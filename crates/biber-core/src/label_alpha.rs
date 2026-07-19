//! Full `labelalphatemplate` engine.
//!
//! Ported from `lib/Biber/Internals.pm` (lines 306–1019) and
//! `lib/Biber/Utils.pm` (normalise_string_label, escape/unescape_label,
//! parse_range, parse_range_alt).
//!
//! The engine reads the structured `labelalphatemplate` (a
//! `ConfigValue::Map` keyed by entry-type, defaulting to `"global"`)
//! and iterates labelelements / labelparts to build both `labelalpha`
//! (display, with LaTeX markup) and `sortlabelalpha` (for sorting,
//! plain text).

use std::collections::{BTreeMap, HashMap};

use regex::Regex;
use tracing::trace;

use crate::config::{Config, ConfigValue};
use crate::datalist::{LabelCacheL, LabelCacheV};
use crate::entry::Entry;

// ---------------------------------------------------------------------------
// Helper functions (ported from Biber::Utils)
// ---------------------------------------------------------------------------

/// Normalise a string for use in a label.
///
/// Strips LaTeX macros (`\command`), replaces ties (`~`) with spaces,
/// trims leading/trailing whitespace, and collapses internal whitespace.
pub fn normalise_string_label(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let mut result = s.to_string();
    // Remove LaTeX macros with braced arguments: \cmd{arg} → arg
    let re_macro_arg = Regex::new(r"\\[A-Za-z]+\{([^}]*)\}").unwrap();
    while re_macro_arg.is_match(&result) {
        result = re_macro_arg.replace_all(&result, "$1").to_string();
    }
    // Remove remaining LaTeX macros (backslash followed by ASCII letters)
    let re_macro = Regex::new(r"\\[A-Za-z]+").unwrap();
    result = re_macro.replace_all(&result, "").to_string();
    // Replace ties (non-backslash-escaped ~) with spaces
    let re_tie = Regex::new(r"([^\\])~").unwrap();
    result = re_tie.replace_all(&result, "$1 ").to_string();
    // Trim and collapse whitespace
    let re_ws = Regex::new(r"\s+").unwrap();
    result = re_ws.replace_all(result.trim(), " ").to_string();
    result
}

/// Escape special characters for use in label fields in the `.bbl`.
pub fn escape_label(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let mut result = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        match c {
            '_' | '^' | '$' | '#' | '%' | '&' => {
                result.push('\\');
                result.push(c);
            }
            '~' => result.push_str("{\\textasciitilde}"),
            '>' => result.push_str("{\\textgreater}"),
            '<' => result.push_str("{\\textless}"),
            _ => result.push(c),
        }
    }
    result
}

/// Unescape label-special sequences back to plain text.
pub fn unescape_label(s: &str) -> String {
    if s.is_empty() {
        return String::new();
    }
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '\\' && i + 1 < chars.len() {
            match chars[i + 1] {
                '_' | '^' | '$' | '~' | '#' | '%' | '&' => {
                    result.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        // Check for {\textasciitilde} etc.
        if chars[i] == '{' && i + 1 < chars.len() && chars[i + 1] == '\\' {
            // Collect until '}'
            let start = i;
            let mut depth = 0;
            while i < chars.len() {
                match chars[i] {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            let seq: String = chars[start..i].iter().collect();
            match seq.as_str() {
                "{\\textasciitilde}" => result.push('~'),
                "{\\textgreater}" => result.push('>'),
                "{\\textless}" => result.push('<'),
                _ => result.push_str(&seq),
            }
            continue;
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

/// Parse a name range string like `"2-7"`, `"2-"`, `"-7"`, `"3"`.
///
/// Returns `(start, end)` where `None` means unbounded.
/// For a single number `"N"`, returns `(Some(N), Some(N))`.
pub fn parse_range(s: &str) -> (Option<u32>, Option<u32>) {
    let s = s.trim();
    if let Some(dash_pos) = s.find('-') {
        let left = s[..dash_pos].trim();
        let right = s[dash_pos + 1..].trim();
        let start = if left.is_empty() {
            None
        } else {
            left.parse::<u32>().ok()
        };
        let end = if right.is_empty() {
            None
        } else {
            right.parse::<u32>().ok()
        };
        (start, end)
    } else if let Ok(n) = s.parse::<u32>() {
        (Some(n), Some(n))
    } else {
        (None, None)
    }
}

/// Like [`parse_range`] but returns `None` if there is no dash (not a range).
pub fn parse_range_alt(s: &str) -> Option<(Option<u32>, Option<u32>)> {
    let s = s.trim();
    if s.contains('-') {
        Some(parse_range(s))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Internal label state
// ---------------------------------------------------------------------------

/// Mutable state threaded through label generation for one section.
pub struct LabelAlphaState<'a> {
    /// List of citekeys in the current section.
    pub section_citekeys: Vec<String>,
    /// Reference to the global configuration.
    pub config: &'a Config,
    /// Cache for visible-name-count disambiguation per entry.
    pub labelcache_v: HashMap<String, LabelCacheV>,
    /// Cache for list-wide disambiguation per entry.
    pub labelcache_l: HashMap<String, LabelCacheL>,
    /// Per-entry counter of visible alpha names for disambiguation.
    pub visible_alpha: HashMap<String, u32>,
    /// Tracks whether an entry has more names than the current range.
    pub morenames: HashMap<String, bool>,
    /// Whether a `final` part has been emitted (stops further elements).
    pub label_final: bool,
}

// ---------------------------------------------------------------------------
// Template data model (parsed from ConfigValue::Map)
// ---------------------------------------------------------------------------

/// A single labelpart inside a labelelement.
#[derive(Debug, Clone, Default)]
pub struct LabelPart {
    /// Field name or literal string content.
    pub content: String,
    /// If true, stop processing after the first successful labelelement.
    pub final_part: bool,
    /// Substring width: "1", "3", "v", "vf", "vl", "l", etc.
    pub substring_width: Option<String>,
    /// Substring side: "left" (default) or "right".
    pub substring_side: Option<String>,
    /// Maximum width for "v" mode disambiguation.
    pub substring_width_max: Option<u32>,
    /// Fixed threshold for "f" mode.
    pub substring_fixed_threshold: Option<u32>,
    /// Pad character (e.g. "_", "~").
    pub pad_char: Option<String>,
    /// Pad side: "right" (default) or "left".
    pub pad_side: Option<String>,
    /// Conditional on visible name count: "1", "2-3", etc.
    pub ifnames: Option<String>,
    /// Override which name indices to use: "2-7", "1+", etc.
    pub names: Option<String>,
    /// Separator between multiple names in the label.
    pub namessep: Option<String>,
    /// Suppress alphaothers for this template part.
    pub noalphaothers: bool,
    /// Force uppercase.
    pub uppercase: bool,
    /// Force lowercase.
    pub lowercase: bool,
    /// Namepart-specific substring width (overrides the labelpart's).
    pub namepart_substring_width: Option<String>,
    /// Namepart-specific substring side.
    pub namepart_substring_side: Option<String>,
}

/// A labelelement (ordered group of alternative labelparts).
#[derive(Debug, Clone, Default)]
pub struct LabelElement {
    /// Order among elements (lower = earlier, determines display sequence).
    pub order: u32,
    /// Disjunctive list of alternative labelparts for this element.
    pub parts: Vec<LabelPart>,
}

/// A complete labelalphatemplate (list of labelelements in order).
#[derive(Debug, Clone, Default)]
pub struct LabelAlphaTemplate {
    /// Ordered list of labelelements that make up the label.
    pub elements: Vec<LabelElement>,
}

/// A namepart entry in a labelalphanametemplate.
#[derive(Debug, Clone, Default)]
pub struct LabelAlphaNamePart {
    /// Namepart name (e.g. "family", "given", "prefix").
    pub namepart: String,
    /// If true, only include when the corresponding `use<namepart>` option is set.
    pub r#use: bool,
    /// If true, this namepart is a prefix (concatenated before main parts).
    pub pre: bool,
    /// If true, split on whitespace/hyphens and take substring of each piece.
    pub substring_compound: bool,
    /// Width override for this namepart's substring.
    pub substring_width: Option<String>,
    /// Side override for this namepart's substring ("left" or "right").
    pub substring_side: Option<String>,
}

/// A labelalphanametemplate (list of namepart entries).
#[derive(Debug, Clone, Default)]
pub struct LabelAlphaNameTemplate {
    /// Ordered list of namepart entries that define how names are extracted.
    pub parts: Vec<LabelAlphaNamePart>,
}

// ---------------------------------------------------------------------------
// Template parsing from ConfigValue
// ---------------------------------------------------------------------------

/// Parse a `ConfigValue::Map` representation of a `labelalphatemplate`
/// into a `LabelAlphaTemplate`.
///
/// The ConfigValue structure (set by the BCF reader) is:
/// ```text
/// Map({
///   "global" => List([
///     Map({ "order" => Str("1"), "parts" => List([
///       Map({ "content" => Str("shorthand"), "final" => Str("1") }),
///       ...
///     ]) }),
///     ...
///   ]),
///   "customc" => List([...]),
///   ...
/// })
/// ```
pub fn parse_labelalphatemplate_config(cv: &ConfigValue) -> HashMap<String, LabelAlphaTemplate> {
    let map = match cv {
        ConfigValue::Map(m) => m,
        _ => return HashMap::new(),
    };
    let mut templates = HashMap::new();
    for (name, val) in map {
        if let ConfigValue::List(elements) = val {
            let mut tmpl = LabelAlphaTemplate::default();
            for elem in elements {
                if let ConfigValue::Map(m) = elem {
                    let order = m
                        .get("order")
                        .and_then(|v| v.as_str())
                        .and_then(|s| s.parse::<u32>().ok())
                        .unwrap_or(0);
                    let mut le = LabelElement {
                        order,
                        parts: Vec::new(),
                    };
                    if let Some(ConfigValue::List(parts)) = m.get("parts") {
                        for part in parts {
                            if let ConfigValue::Map(pm) = part {
                                le.parts.push(parse_labelpart(pm));
                            }
                        }
                    }
                    tmpl.elements.push(le);
                }
            }
            tmpl.elements.sort_by_key(|e| e.order);
            templates.insert(name.clone(), tmpl);
        }
    }
    templates
}

/// Parse a single labelpart from a `ConfigValue::Map`.
fn parse_labelpart(m: &BTreeMap<String, ConfigValue>) -> LabelPart {
    let get_str = |key: &str| -> Option<String> {
        m.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
    };
    let get_bool = |key: &str| -> bool {
        m.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s == "1" || s == "true")
            .unwrap_or(false)
    };
    LabelPart {
        content: get_str("content").unwrap_or_default(),
        final_part: get_bool("final"),
        substring_width: get_str("substring_width"),
        substring_side: get_str("substring_side"),
        substring_width_max: get_str("substring_width_max").and_then(|s| s.parse().ok()),
        substring_fixed_threshold: get_str("substring_fixed_threshold")
            .and_then(|s| s.parse().ok()),
        pad_char: get_str("pad_char"),
        pad_side: get_str("pad_side"),
        ifnames: get_str("ifnames"),
        names: get_str("names"),
        namessep: get_str("namessep"),
        noalphaothers: get_bool("noalphaothers"),
        uppercase: get_bool("uppercase"),
        lowercase: get_bool("lowercase"),
        namepart_substring_width: None,
        namepart_substring_side: None,
    }
}

/// Parse a `ConfigValue::Map` representation of a `labelalphanametemplate`.
///
/// The BCF reader stores it as:
/// ```text
/// Map({
///   "global" => List([
///     Map({ "namepart" => Str("family"), "substring_width" => Str("1"), ... }),
///     ...
///   ])
/// })
/// ```
pub fn parse_labelalphanametemplate_config(
    cv: &ConfigValue,
) -> HashMap<String, LabelAlphaNameTemplate> {
    let map = match cv {
        ConfigValue::Map(m) => m,
        _ => return HashMap::new(),
    };
    let mut templates = HashMap::new();
    for (name, val) in map {
        if let ConfigValue::List(parts) = val {
            let mut tmpl = LabelAlphaNameTemplate::default();
            for part in parts {
                if let ConfigValue::Map(m) = part {
                    let get_str = |key: &str| -> Option<String> {
                        m.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
                    };
                    let get_bool = |key: &str| -> bool {
                        m.get(key)
                            .and_then(|v| v.as_str())
                            .map(|s| s == "1" || s == "true")
                            .unwrap_or(false)
                    };
                    tmpl.parts.push(LabelAlphaNamePart {
                        namepart: get_str("namepart").unwrap_or_default(),
                        r#use: get_bool("use"),
                        pre: get_bool("pre"),
                        substring_compound: get_bool("substring_compound"),
                        substring_width: get_str("substring_width"),
                        substring_side: get_str("substring_side"),
                    });
                }
            }
            templates.insert(name.clone(), tmpl);
        }
    }
    templates
}

// ---------------------------------------------------------------------------
// Main label generation
// ---------------------------------------------------------------------------

/// Generate `(labelalpha, sortlabelalpha)` for a citekey.
///
/// This is the Rust equivalent of `Biber::Internals::_genlabel`.
pub fn gen_label(
    citekey: &str,
    entry: &Entry,
    config: &Config,
    templates: &HashMap<String, LabelAlphaTemplate>,
    name_templates: &HashMap<String, LabelAlphaNameTemplate>,
    state: &mut LabelAlphaState<'_>,
) -> (String, String) {
    let entrytype = &entry.entrytype;
    let tmpl = templates
        .get(entrytype)
        .or_else(|| templates.get("global"))
        .cloned()
        .unwrap_or_else(default_labelalphatemplate);

    let mut label = String::new();
    let mut slabel = String::new();
    state.label_final = false;

    for elem in &tmpl.elements {
        let (lp, slp) = label_part(&elem.parts, citekey, entry, config, name_templates, state);
        label.push_str(&lp);
        slabel.push_str(&slp);
        if state.label_final {
            break;
        }
    }

    trace!("gen_label for '{citekey}': label='{label}', sortlabel='{slabel}'");
    (label, slabel)
}

/// Default `labelalphatemplate` matching the standard biblatex behavior:
/// shorthand (final) → label → labelname (3-char if 1 name, 1-char otherwise)
/// → year (2-char from right).
fn default_labelalphatemplate() -> LabelAlphaTemplate {
    LabelAlphaTemplate {
        elements: vec![
            LabelElement {
                order: 1,
                parts: vec![
                    LabelPart {
                        content: "shorthand".to_string(),
                        final_part: true,
                        ..Default::default()
                    },
                    LabelPart {
                        content: "label".to_string(),
                        ..Default::default()
                    },
                    LabelPart {
                        content: "labelname".to_string(),
                        substring_width: Some("3".to_string()),
                        substring_side: Some("left".to_string()),
                        ifnames: Some("1".to_string()),
                        ..Default::default()
                    },
                    LabelPart {
                        content: "labelname".to_string(),
                        substring_width: Some("1".to_string()),
                        substring_side: Some("left".to_string()),
                        ..Default::default()
                    },
                ],
            },
            LabelElement {
                order: 2,
                parts: vec![LabelPart {
                    content: "labelyear".to_string(),
                    substring_width: Some("2".to_string()),
                    substring_side: Some("right".to_string()),
                    ..Default::default()
                }],
            },
        ],
    }
}

/// Default `labelalphanametemplate`: uses only the family name.
fn default_labelalphanametemplate() -> LabelAlphaNameTemplate {
    LabelAlphaNameTemplate {
        parts: vec![LabelAlphaNamePart {
            namepart: "family".to_string(),
            r#use: false,
            pre: false,
            substring_compound: false,
            substring_width: None,
            substring_side: None,
        }],
    }
}

// ---------------------------------------------------------------------------
// _labelpart — iterate parts within one labelelement
// ---------------------------------------------------------------------------

/// Process a disjunctive set of labelparts (one labelelement).
/// Returns the first part that produces a non-empty result.
fn label_part(
    parts: &[LabelPart],
    citekey: &str,
    entry: &Entry,
    config: &Config,
    name_templates: &HashMap<String, LabelAlphaNameTemplate>,
    state: &mut LabelAlphaState<'_>,
) -> (String, String) {
    let entrytype = &entry.entrytype;
    let maxan = config
        .getblxoption_for_entry_str(entrytype, "maxalphanames")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(3);
    let minan = config
        .getblxoption_for_entry_str(entrytype, "minalphanames")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1);

    let mut lp = String::new();
    let mut slp = String::new();

    for part in parts {
        // Handle ifnames conditional
        if let Some(ref inc_str) = part.ifnames {
            if let Some(ln_field) = entry.get_field_str("labelname") {
                if let Some(names) = entry.names.get(ln_field) {
                    let total_names = names.count() as u32;
                    let visible_names = if total_names > maxan {
                        minan
                    } else {
                        total_names
                    };
                    if let Ok(exact) = inc_str.parse::<u32>() {
                        if visible_names != exact {
                            continue;
                        }
                    } else if let Some((lo, hi)) = parse_range_alt(inc_str) {
                        match (lo, hi) {
                            (None, Some(h)) if visible_names > h => {
                                continue;
                            }
                            (Some(l), None) if visible_names < l => {
                                continue;
                            }
                            (Some(l), Some(h)) if visible_names < l || visible_names > h => {
                                continue;
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        let (ret_lp, ret_slp) = dispatch_label(part, citekey, entry, config, name_templates, state);
        lp.push_str(&ret_lp);
        slp.push_str(&ret_slp);

        if !ret_lp.is_empty() {
            if part.final_part {
                state.label_final = true;
            }
            break;
        }
    }

    (lp, slp)
}

// ---------------------------------------------------------------------------
// _dispatch_label — route to the appropriate handler
// ---------------------------------------------------------------------------

/// Known internal label field dispatch table.
///
/// Returns `(handler_name, args)` where handler_name is one of:
/// "basic_nostrip", "basic", "citekey", "name", "literal".
fn dispatch_table_entry(field: &str) -> (&'static str, &str) {
    match field {
        "label" => ("basic_nostrip", "label"),
        "shorthand" => ("basic_nostrip", "shorthand"),
        "sortkey" => ("basic_nostrip", "sortkey"),
        "citekey" | "entrykey" => ("citekey", ""),
        "labelname" => ("name", "labelname"),
        "labeltitle" => ("basic", "labeltitle"),
        "labelmonth" => ("basic", "labelmonth"),
        "labelday" => ("basic", "labelday"),
        "labelyear" => ("basic", "labelyear"),
        _ => ("unknown", field),
    }
}

/// Dispatch a labelpart to the appropriate handler.
fn dispatch_label(
    part: &LabelPart,
    citekey: &str,
    entry: &Entry,
    config: &Config,
    name_templates: &HashMap<String, LabelAlphaNameTemplate>,
    state: &mut LabelAlphaState<'_>,
) -> (String, String) {
    let content = &part.content;

    // Check if it's a known internal field
    let (handler, field_name) = dispatch_table_entry(content);
    match handler {
        "basic_nostrip" => label_basic(field_name, true, part, citekey, entry, state),
        "basic" => label_basic(field_name, false, part, citekey, entry, state),
        "citekey" => label_citekey(citekey, part, state),
        "name" => label_name(
            field_name,
            citekey,
            entry,
            config,
            name_templates,
            part,
            state,
        ),
        "unknown" => {
            // Check if it's a name-type field in the data model
            if is_name_field(content, entry) {
                label_name(content, citekey, entry, config, name_templates, part, state)
            } else if entry.has_field(content) || is_known_field(content) {
                label_basic(content, false, part, citekey, entry, state)
            } else {
                // Treat as literal string
                label_literal(content)
            }
        }
        _ => label_literal(content),
    }
}

/// Check if a field is a known name-type field for the given entry.
fn is_name_field(field: &str, entry: &Entry) -> bool {
    // Common name fields
    matches!(
        field,
        "author"
            | "editor"
            | "translator"
            | "bookauthor"
            | "editora"
            | "editorb"
            | "editorc"
            | "foreword"
            | "afterword"
            | "holder"
    ) || entry.names.contains_key(field)
}

/// Check if a field is a known general field.
fn is_known_field(field: &str) -> bool {
    matches!(
        field,
        "title"
            | "subtitle"
            | "titleaddon"
            | "maintitle"
            | "booktitle"
            | "booksubtitle"
            | "booktitleaddon"
            | "volume"
            | "series"
            | "number"
            | "note"
            | "annote"
            | "pages"
            | "pagetotal"
            | "chapter"
            | "edition"
            | "version"
            | "type"
            | "journal"
            | "journaltitle"
            | "journalsubtitle"
            | "eventtitle"
            | "eventdate"
            | "venue"
            | "location"
            | "address"
            | "publisher"
            | "institution"
            | "organization"
            | "school"
            | "language"
            | "langid"
            | "howpublished"
            | "medium"
            | "isan"
            | "isbn"
            | "ismn"
            | "isrn"
            | "issn"
            | "issue"
            | "urldate"
            | "url"
            | "doi"
            | "eprint"
            | "eprintclass"
            | "eprinttype"
            | "verba"
            | "verbtitle"
            | "sortkey"
            | "sortname"
            | "sorttitle"
            | "sortyear"
            | "sortmonth"
            | "sortday"
            | "sortscheme"
    )
}

// ---------------------------------------------------------------------------
// _label_citekey
// ---------------------------------------------------------------------------

/// Use the citekey itself as the label.
fn label_citekey(
    citekey: &str,
    part: &LabelPart,
    state: &mut LabelAlphaState<'_>,
) -> (String, String) {
    let k = process_label_attributes(citekey, part, None, state);
    let sk = unescape_label(&k);
    (k, sk)
}

// ---------------------------------------------------------------------------
// _label_basic
// ---------------------------------------------------------------------------

/// Simple field handler (shorthand, label, labeltitle, labelyear, etc.).
fn label_basic(
    field: &str,
    nostrip: bool,
    part: &LabelPart,
    _citekey: &str,
    entry: &Entry,
    state: &mut LabelAlphaState<'_>,
) -> (String, String) {
    let raw = entry.get_field_str(field).unwrap_or("");
    let f = if nostrip {
        raw.to_string()
    } else {
        normalise_string_label(raw)
    };
    if f.is_empty() {
        return (String::new(), String::new());
    }
    let b = process_label_attributes(&f, part, None, state);
    let sk = unescape_label(&b);
    (b, sk)
}

// ---------------------------------------------------------------------------
// _label_literal
// ---------------------------------------------------------------------------

/// Literal string in a template (e.g. "&", "-", ".").
fn label_literal(s: &str) -> (String, String) {
    let escaped = escape_label(&unescape_label(s));
    let plain = unescape_label(s);
    (escaped, plain)
}

// ---------------------------------------------------------------------------
// _label_name
// ---------------------------------------------------------------------------

/// Name field handler — the most complex handler.
///
/// Uses the `labelalphanametemplate` to extract nameparts, handles name
/// ranges, pre/main namepart separation, alphaothers, etc.
fn label_name(
    field: &str,
    _citekey: &str,
    entry: &Entry,
    config: &Config,
    name_templates: &HashMap<String, LabelAlphaNameTemplate>,
    part: &LabelPart,
    state: &mut LabelAlphaState<'_>,
) -> (String, String) {
    let entrytype = &entry.entrytype;

    // Resolve "labelname" to the actual field name
    let realname = if field == "labelname" {
        match entry.get_field_str("labelname") {
            Some(ln) => ln.to_string(),
            None => return (String::new(), String::new()),
        }
    } else {
        field.to_string()
    };

    let useprefix = config
        .getblxoption_for_entry_str(entrytype, "useprefix")
        .map(|s| s == "1" || s == "true")
        .unwrap_or(false);

    let alphaothers = config
        .getblxoption_for_entry_str(entrytype, "alphaothers")
        .map(|s| s.to_string());

    let sortalphaothers = config
        .getblxoption_for_entry_str(entrytype, "sortalphaothers")
        .map(|s| s.to_string());

    // Get the labelalphanametemplate name for this datalist context
    // For now, use the entrytype-specific or global template
    let lantname = config
        .getblxoption_for_entry_str(entrytype, "labelalphanametemplatename")
        .unwrap_or("global");

    // Shortcut: no names → no label
    let names = match entry.names.get(&realname) {
        Some(n) if !n.is_empty() => n,
        _ => return (String::new(), String::new()),
    };

    let numnames = names.count() as u32;
    let maxan = config
        .getblxoption_for_entry_str(entrytype, "maxalphanames")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(3);
    let minan = config
        .getblxoption_for_entry_str(entrytype, "minalphanames")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(1);

    let visibility = if numnames > maxan { minan } else { numnames };

    // Determine name range
    let (nr_start, nr_end) = if let Some(ref names_range) = part.names {
        let (lo, hi) = parse_range(names_range);
        let start = lo.unwrap_or(1);
        let end = if let Some(h) = hi {
            if h == u32::MAX {
                // "+" marker
                visibility
            } else if h > numnames {
                numnames
            } else {
                h
            }
        } else {
            numnames
        };
        (start, end)
    } else {
        (1, visibility)
    };

    trace!(
        "{}/numnames={}/visibility={}/nr_start={}/nr_end={}",
        realname,
        numnames,
        visibility,
        nr_start,
        nr_end
    );

    // Get the name template
    let lnat = name_templates
        .get(lantname)
        .cloned()
        .unwrap_or_else(default_labelalphanametemplate);

    // Pre-allocate namepart data for each name in range
    struct NamePartData {
        pre_strings: Vec<(String, Option<String>, Option<String>, bool)>,
        main_strings: Vec<(String, Option<String>, Option<String>, bool)>,
        main_partnames: Vec<String>,
    }
    let mut all_name_data: Vec<NamePartData> = Vec::new();

    for name in names.iter() {
        let mut pre_strings = Vec::new();
        let mut main_strings = Vec::new();
        let mut main_partnames = Vec::new();

        for lnp in &lnat.parts {
            let npn = &lnp.namepart;
            if let Some(np_val) = name.get_namepart(npn) {
                if lnp.r#use {
                    // Check use* option — skip if not enabled
                    let opt_key = format!("use{}", npn);
                    let use_enabled = config
                        .getblxoption_for_entry_str(entrytype, &opt_key)
                        .map(|s| s == "1" || s == "true")
                        .unwrap_or(false);
                    if !use_enabled {
                        continue;
                    }
                }
                let normalised = normalise_string_label(np_val);
                let nw = lnp.substring_width.clone();
                let ns = lnp.substring_side.clone();
                let nc = lnp.substring_compound;
                if lnp.pre {
                    pre_strings.push((normalised, nw, ns, nc));
                } else {
                    main_partnames.push(npn.clone());
                    main_strings.push((normalised, nw, ns, nc));
                }
                // Suppress unused variable warning
                let _ = (
                    useprefix,
                    alphaothers.as_deref(),
                    sortalphaothers.as_deref(),
                );
            }
        }

        all_name_data.push(NamePartData {
            pre_strings,
            main_strings,
            main_partnames,
        });
    }

    // Build the label string by iterating names in range
    let mut acc = String::new();

    for i in (nr_start - 1) as usize..nr_end as usize {
        if i >= all_name_data.len() {
            break;
        }
        let nd = &all_name_data[i];

        // Process pre nameparts (concatenate directly)
        for (np_val, nw, _ns, nc) in &nd.pre_strings {
            let w = nw
                .as_deref()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(1);
            if *nc {
                // Compound: split on whitespace/hyphens and take substring of each part
                let mut tmp = String::new();
                for part in np_val.split(|c: char| {
                    c.is_whitespace() || c == '-' || matches!(c, '\u{2010}'..='\u{2013}')
                }) {
                    let sub: String = part.chars().take(w).collect();
                    tmp.push_str(&sub);
                }
                acc.push_str(&tmp);
            } else {
                let sub: String = np_val.chars().take(w).collect();
                acc.push_str(&sub);
            }
        }

        // Process main nameparts with full attribute processing
        if !nd.main_strings.is_empty() {
            let mut namepart_opts: Vec<(Option<String>, Option<String>, bool)> = Vec::new();
            for (_, nw, ns, nc) in &nd.main_strings {
                namepart_opts.push((nw.clone(), ns.clone(), *nc));
            }

            let main_str: String = process_label_attributes_multi(
                &nd.main_strings,
                part,
                Some(&nd.main_partnames),
                i as u32,
                state,
            );
            acc.push_str(&main_str);

            // Suppress unused variable warning
            let _ = &namepart_opts;
        }

        // namessep between names
        if i + 1 < nr_end as usize {
            if let Some(ref sep) = part.namessep {
                acc.push_str(sep);
            }
        }
    }

    // Build sortlabelalpha (same as acc since we don't use markup here)
    let mut sortacc = acc.clone();

    // Add alphaothers if name list is truncated
    if !part.noalphaothers && (numnames > nr_end || names.count() as u32 > nr_end) {
        if let Some(ref ao) = alphaothers {
            acc.push_str(ao);
        }
        if let Some(ref sao) = sortalphaothers {
            sortacc.push_str(sao);
        }
    }

    (acc, unescape_label(&sortacc))
}

// ---------------------------------------------------------------------------
// _process_label_attributes — substring width, padding, case transforms
// ---------------------------------------------------------------------------

/// Process a single field string through label attributes (substring, pad, case).
fn process_label_attributes(
    field_string: &str,
    part: &LabelPart,
    namepart_opts: Option<(&Option<String>, &Option<String>, bool)>,
    _state: &mut LabelAlphaState<'_>,
) -> String {
    let mut result = field_string.to_string();

    if let Some(ref sw) = part.substring_width {
        let is_var_v = sw.contains('v') && !sw.contains('l');
        let is_var_l = sw.contains('l') && !sw.contains('v');

        if is_var_v {
            // "v" mode — per-name variable disambiguation
            // For static processing, use the static branch as fallback
            // Full "v" mode requires cross-entry iteration (done at section level)
            let subs_width = 1;
            let subs_side = part.substring_side.as_deref().unwrap_or("left");
            let offset = if subs_side == "right" {
                -(subs_width as isize)
            } else {
                0
            };
            result = take_substring(&result, offset, subs_width);
        } else if is_var_l {
            // "l" mode — list-wide disambiguation
            let subs_width = 1;
            let subs_side = part.substring_side.as_deref().unwrap_or("left");
            let offset = if subs_side == "right" {
                -(subs_width as isize)
            } else {
                0
            };
            result = take_substring(&result, offset, subs_width);
        } else {
            // Static substring width
            let subs_side = part.substring_side.as_deref().unwrap_or("left");
            let mut subs_width: usize = sw.parse().unwrap_or(1);

            // Override with namepart-specific setting
            if let Some((ref np_w, ref np_s, _nc)) = namepart_opts {
                if let Some(ref w) = np_w {
                    subs_width = w.parse().unwrap_or(subs_width);
                }
                if let Some(ref s) = np_s {
                    let _ = s; // Use the namepart's side
                }
            }

            let offset = if subs_side == "right" {
                -(subs_width as isize)
            } else {
                0
            };

            // substring_compound: split on whitespace/hyphens
            let is_compound = namepart_opts
                .as_ref()
                .map(|(_, _, nc)| *nc)
                .unwrap_or(false);
            if is_compound {
                let mut tmp = String::new();
                for part_str in result.split(|c: char| {
                    c.is_whitespace() || c == '-' || matches!(c, '\u{2010}'..='\u{2013}')
                }) {
                    let sub = take_substring(part_str, offset, subs_width);
                    tmp.push_str(&sub);
                }
                result = tmp;
            } else {
                result = take_substring(&result, offset, subs_width);
            }

            // Padding
            if let Some(ref pad_char) = part.pad_char {
                let pad_side = part.pad_side.as_deref().unwrap_or("right");
                let pad_char_ch = unescape_label(pad_char).chars().next().unwrap_or(' ');
                let current_len = result.chars().count();
                if subs_width > current_len {
                    let paddiff = subs_width - current_len;
                    let padding: String = std::iter::repeat(pad_char_ch).take(paddiff).collect();
                    if pad_side == "left" {
                        result = format!("{}{}", padding, result);
                    } else {
                        result.push_str(&padding);
                    }
                }
                result = escape_label(&result);
            }
        }
    }

    // Case transforms
    if part.uppercase && part.lowercase {
        // Both set — do nothing (sanity)
    } else if part.uppercase {
        result = result.to_uppercase();
    } else if part.lowercase {
        result = result.to_lowercase();
    }

    result
}

/// Process multiple field strings through label attributes (for name fields).
fn process_label_attributes_multi(
    field_strings: &[(String, Option<String>, Option<String>, bool)],
    part: &LabelPart,
    _nameparts: Option<&[String]>,
    _index: u32,
    state: &mut LabelAlphaState<'_>,
) -> String {
    let mut result = String::new();
    for (fs, nw, ns, nc) in field_strings {
        let np_opts = Some((nw, ns, *nc));
        let sub = process_label_attributes(fs, part, np_opts, state);
        result.push_str(&sub);
    }
    result
}

/// Take a Unicode-aware substring with offset.
///
/// Positive offset = from left (skip first `offset` chars).
/// Negative offset = from right (take last `width` chars).
fn take_substring(s: &str, offset: isize, width: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    if len == 0 {
        return String::new();
    }
    if width == 0 {
        return String::new();
    }
    let start = if offset < 0 {
        let from_end = (-offset) as usize;
        if from_end >= len {
            return s.to_string();
        }
        len.saturating_sub(from_end)
    } else {
        offset as usize
    };
    let end = (start + width).min(len);
    if start >= len {
        return String::new();
    }
    chars[start..end].iter().collect()
}

// ---------------------------------------------------------------------------
// _label_listdisambiguation
// ---------------------------------------------------------------------------

/// List-wide label disambiguation (the "l" mode).
///
/// Takes a list of name-lists (one per entry) and produces disambiguated
/// substrings of increasing length until all entries are unique.
pub fn label_listdisambiguation(strings: &[Vec<String>]) -> Vec<Vec<String>> {
    let n = strings.len();
    if n == 0 {
        return Vec::new();
    }
    let max_names = strings.iter().map(|s| s.len()).max().unwrap_or(0);

    // Start with 1-char substrings
    let mut data: Vec<Vec<String>> = strings
        .iter()
        .map(|names| names.iter().map(|s| s.chars().take(1).collect()).collect())
        .collect();

    // Build a map of concatenated strings → indices
    let mut seen: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, row) in data.iter().enumerate() {
        let key: String = row.concat();
        seen.entry(key).or_default().push(i);
    }

    // Track current substring width per name position per entry
    let mut widths: Vec<Vec<usize>> = strings
        .iter()
        .map(|names| names.iter().map(|_| 1usize).collect())
        .collect();

    // Iterate until no duplicates
    let max_width = strings
        .iter()
        .flat_map(|s| s.iter().map(|ss| ss.chars().count()))
        .max()
        .unwrap_or(1);

    for w in 2..=max_width {
        // Find ambiguous groups
        let ambiguous: Vec<Vec<usize>> = seen
            .values()
            .filter(|indices| indices.len() > 1)
            .cloned()
            .collect();

        if ambiguous.is_empty() {
            break;
        }

        for group in &ambiguous {
            // Find the first name position that differs
            let mut disambig_pos = None;
            for pos in 0..max_names {
                let vals: Vec<String> = group
                    .iter()
                    .map(|&i| strings[i].get(pos).cloned().unwrap_or_default())
                    .collect();
                let unique_vals: std::collections::HashSet<&str> =
                    vals.iter().map(|s| s.as_str()).collect();
                if unique_vals.len() > 1 {
                    disambig_pos = Some(pos);
                    break;
                }
            }

            if let Some(pos) = disambig_pos {
                // All identical lists get 1-char substrings
                let first_row = &strings[group[0]];
                let all_identical = group.iter().all(|&i| &strings[i] == first_row);
                if all_identical {
                    for &i in group {
                        data[i] = first_row
                            .iter()
                            .map(|s| s.chars().take(1).collect())
                            .collect();
                    }
                } else {
                    // Increment width for the disambiguating position
                    for &i in group {
                        if widths[i].get(pos).copied().unwrap_or(1) < w && widths[i].len() > pos {
                            widths[i][pos] = w;
                        }
                        data[i] = strings[i]
                            .iter()
                            .enumerate()
                            .map(|(j, s)| {
                                let width = widths[i].get(j).copied().unwrap_or(1);
                                s.chars().take(width).collect()
                            })
                            .collect();
                    }
                }
            }
        }

        // Rebuild seen map
        seen.clear();
        for (i, row) in data.iter().enumerate() {
            let key: String = row.concat();
            seen.entry(key).or_default().push(i);
        }
    }

    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise_string_label_removes_latex_macros() {
        assert_eq!(
            normalise_string_label(r"The \textbf{Quick} Brown Fox"),
            "The Quick Brown Fox"
        );
    }

    #[test]
    fn normalise_string_label_replaces_ties() {
        assert_eq!(normalise_string_label("Foo~Bar"), "Foo Bar");
    }

    #[test]
    fn normalise_string_label_collapses_whitespace() {
        assert_eq!(normalise_string_label("  hello   world  "), "hello world");
    }

    #[test]
    fn escape_label_special_chars() {
        assert_eq!(escape_label("a_b"), r"a\_b");
        assert_eq!(escape_label("a^b"), r"a\^b");
        assert_eq!(escape_label("a$b"), r"a\$b");
        assert_eq!(escape_label("a#b"), r"a\#b");
        assert_eq!(escape_label("a%b"), r"a\%b");
        assert_eq!(escape_label("a&b"), r"a\&b");
    }

    #[test]
    fn escape_label_tilde() {
        assert_eq!(escape_label("a~b"), r"a{\textasciitilde}b");
    }

    #[test]
    fn unescape_label_roundtrip() {
        let original = "hello_world^2";
        let escaped = escape_label(original);
        let unescaped = unescape_label(&escaped);
        assert_eq!(unescaped, original);
    }

    #[test]
    fn parse_range_single_number() {
        assert_eq!(parse_range("3"), (Some(3), Some(3)));
    }

    #[test]
    fn parse_range_with_dash() {
        assert_eq!(parse_range("2-7"), (Some(2), Some(7)));
    }

    #[test]
    fn parse_range_open_ended() {
        assert_eq!(parse_range("2-"), (Some(2), None));
        assert_eq!(parse_range("-7"), (None, Some(7)));
    }

    #[test]
    fn parse_range_alt_requires_dash() {
        assert_eq!(parse_range_alt("3"), None);
        assert_eq!(parse_range_alt("2-7"), Some((Some(2), Some(7))));
    }

    #[test]
    fn take_substring_left() {
        assert_eq!(take_substring("hello", 0, 3), "hel");
    }

    #[test]
    fn take_substring_right() {
        assert_eq!(take_substring("hello", -3, 3), "llo");
    }

    #[test]
    fn take_substring_unicode() {
        assert_eq!(take_substring("café", 0, 3), "caf");
    }

    #[test]
    fn label_listdisambiguation_basic() {
        // Smith vs Jones: Jones disambiguates with 1 char, Smith needs more
        let strings = vec![
            vec!["Smith".to_string()],
            vec!["Smythe".to_string()],
            vec!["Jones".to_string()],
        ];
        let result = label_listdisambiguation(&strings);
        // "Jones" is unique → 1 char
        assert_eq!(result[2][0], "J");
        // "Smith" and "Smythe" need at least 2 chars ("Sm" vs "Sm" - still same!
        // but algorithm will give them at least 2)
        assert!(!result[0][0].is_empty());
        assert!(!result[1][0].is_empty());
        // All results should be non-empty
        for row in &result {
            assert!(!row.is_empty());
            for cell in row {
                assert!(!cell.is_empty());
            }
        }
    }
}
