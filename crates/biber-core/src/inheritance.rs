//! CrossRef/XDATA inheritance engine.
//!
//! Ported from `lib/Biber/Entry.pm` (`inherit_from`, `resolve_xdata`) and
//! `lib/Biber.pm` (`process_interentry`, `resolve_xdata`, `calculate_interentry`).
//!
//! Parses the `<bcf:inheritance>` XML from the BCF config, then applies
//! inheritance rules to pass fields from parent entries to children.

use std::collections::{HashMap, HashSet};

use roxmltree::Document;
use tracing::{debug, warn};

use crate::config::ConfigValue;
use crate::processor::Biber;

/// A parsed inheritance scheme from `<bcf:inheritance>`.
#[derive(Debug, Clone, Default)]
pub struct InheritanceScheme {
    /// Global defaults for all inheritance rules.
    pub defaults: InheritanceDefaults,
    /// Ordered list of inheritance rules.
    pub rules: Vec<InheritanceRule>,
}

/// Global defaults for the inheritance scheme.
#[derive(Debug, Clone, Default)]
pub struct InheritanceDefaults {
    /// Whether to inherit unlisted fields (default: false).
    pub inherit_all: bool,
    /// Whether to overwrite existing fields (default: false).
    pub override_target: bool,
    /// Fields to ignore globally.
    pub ignore: Vec<String>,
    /// Per-type-pair overrides of defaults.
    pub type_pair_overrides: HashMap<(String, String), TypePairOverride>,
}

/// Per-type-pair override of inheritance defaults.
#[derive(Debug, Clone, Copy)]
pub struct TypePairOverride {
    /// Whether to inherit all unlisted fields for this type pair.
    pub inherit_all: bool,
}

/// A single inheritance rule (from `<bcf:inherit>`).
#[derive(Debug, Clone)]
pub struct InheritanceRule {
    /// Source entry type to match (or `"*"` for any).
    pub source_type: String,
    /// Target entry type to match (or `"*"` for any).
    pub target_type: String,
    /// Field mappings for this rule.
    pub fields: Vec<InheritanceField>,
}

/// A field mapping within an inheritance rule.
#[derive(Debug, Clone)]
pub struct InheritanceField {
    /// Name of the field in the source (parent) entry.
    pub source: String,
    /// Name of the field in the target (child) entry; defaults to `source` if `None`.
    pub target: Option<String>,
    /// Whether to overwrite an existing field in the child.
    pub override_target: bool,
    /// If true, skip inheritance of this field entirely.
    pub skip: bool,
}

/// Parse the `<bcf:inheritance>` XML into an `InheritanceScheme`.
pub fn parse_inheritance_xml(xml: &str) -> Option<InheritanceScheme> {
    let clean = xml.replace("bcf:", "");
    let doc = Document::parse(&clean).ok()?;
    let root = doc.root_element();

    let mut scheme = InheritanceScheme::default();

    // Parse <defaults>
    if let Some(defaults_node) = root
        .children()
        .find(|n| n.is_element() && n.tag_name().name() == "defaults")
    {
        scheme.defaults.inherit_all = defaults_node
            .attribute("inherit_all")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        scheme.defaults.override_target = defaults_node
            .attribute("override_target")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        if let Some(ign) = defaults_node.attribute("ignore") {
            scheme.defaults.ignore = ign.split(',').map(|s| s.trim().to_string()).collect();
        }

        // Parse per-type-pair overrides
        for child in defaults_node.children() {
            if child.is_element() && child.tag_name().name() == "type_pair" {
                let source = child.attribute("source").unwrap_or("*").to_string();
                let target = child.attribute("target").unwrap_or("*").to_string();
                let inherit_all = child
                    .attribute("inherit_all")
                    .map(|v| v == "true" || v == "1")
                    .unwrap_or(false);
                scheme
                    .defaults
                    .type_pair_overrides
                    .insert((source, target), TypePairOverride { inherit_all });
            }
        }
    }

    // Parse <inherit> rules
    for inherit_node in root.children() {
        if !inherit_node.is_element() || inherit_node.tag_name().name() != "inherit" {
            continue;
        }

        let mut rule = InheritanceRule {
            source_type: String::new(),
            target_type: String::new(),
            fields: Vec::new(),
        };

        // Find the type_pair first
        for child in inherit_node.children() {
            if child.is_element() && child.tag_name().name() == "type_pair" {
                rule.source_type = child.attribute("source").unwrap_or("*").to_string();
                rule.target_type = child.attribute("target").unwrap_or("*").to_string();
                break;
            }
        }

        // Parse field mappings
        for child in inherit_node.children() {
            if !child.is_element() || child.tag_name().name() != "field" {
                continue;
            }

            let source = child.attribute("source").unwrap_or("").to_string();
            let target = child.attribute("target").map(|s| s.to_string());
            let override_target = child
                .attribute("override_target")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(scheme.defaults.override_target);
            let skip = child
                .attribute("skip")
                .map(|v| v == "true" || v == "1")
                .unwrap_or(false);

            if !source.is_empty() {
                rule.fields.push(InheritanceField {
                    source,
                    target,
                    override_target,
                    skip,
                });
            }
        }

        scheme.rules.push(rule);
    }

    Some(scheme)
}

/// Get the fields from a `noinherit` datafield set for a given entry.
fn get_noinherit_fields(biber: &Biber, secnum: u32, target_key: &str) -> HashSet<String> {
    let mut excluded = HashSet::new();

    // Check per-entry options for "noinherit"
    let noinherit_val = get_entry_option(biber, secnum, target_key, "noinherit");
    if let Some(set_name) = noinherit_val {
        // Look up the datafield set
        if let Some(members) = biber.config.datafield_sets.get(&set_name.to_lowercase()) {
            for member in members {
                if let Some(ref field) = member.field {
                    excluded.insert(field.clone());
                }
            }
        }
    }

    excluded
}

/// Get a per-entry option value by scanning all citekeys' options fields.
fn get_entry_option(biber: &Biber, secnum: u32, key: &str, opt_name: &str) -> Option<String> {
    let section = biber.sections.get_section(secnum)?;
    let be = section.bibentry(key)?;

    // Options are stored as a comma-separated list in the "options" field
    // Format: "opt1=val1,opt2=val2,opt3"
    if let Some(options_str) = be.get_field_str("options") {
        for part in options_str.split(',') {
            let part = part.trim();
            if let Some(eq_pos) = part.find('=') {
                let name = part[..eq_pos].trim();
                let value = part[eq_pos + 1..].trim();
                if name == opt_name {
                    return Some(value.to_string());
                }
            } else if part == opt_name {
                return Some("1".to_string());
            }
        }
    }

    None
}

/// Perform crossref inheritance: copy fields from parent to child.
///
/// This implements the Perl `$be->inherit_from($parent)` logic.
pub fn inherit_from(biber: &mut Biber, secnum: u32, child_key: &str, parent_key: &str) {
    debug!("inherit_from: '{child_key}' <- '{parent_key}'");

    // --- Loop detection ---
    if is_inheritance_path(biber, "crossref", child_key, parent_key) {
        warn!("Circular crossref inheritance detected: '{child_key}' -> '{parent_key}'");
        return;
    }
    record_inheritance_mut(biber, "crossref", child_key, parent_key);

    // Resolve alias for parent
    let parent_real = {
        let section = biber.sections.get_section(secnum);
        section.and_then(|s| s.get_citekey_alias(parent_key).map(|s| s.to_string()))
    };
    let parent_key = parent_real.as_deref().unwrap_or(parent_key);

    // Get child and parent entries
    let (parent_entrytype, child_entrytype) = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        let child = match section.bibentry(child_key) {
            Some(e) => e,
            None => return,
        };
        let parent = match section.bibentry(parent_key) {
            Some(e) => e,
            None => return,
        };
        (parent.entrytype.clone(), child.entrytype.clone())
    };

    // --- Cascading crossrefs: if parent has a crossref, resolve it first ---
    let grandparent = {
        let section = biber.sections.get_section(secnum);
        section
            .and_then(|s| s.bibentry(parent_key))
            .and_then(|be| be.get_field_str("crossref"))
            .map(|s| s.to_string())
    };
    if let Some(ref gp) = grandparent {
        if !is_inheritance_path(biber, "crossref", parent_key, gp) {
            inherit_from(biber, secnum, parent_key, gp);
        }
    }

    // --- Parse inheritance scheme ---
    let inheritance_raw = biber
        .config
        .getblxoption(None, "inheritance")
        .and_then(|v| match v {
            ConfigValue::Raw(s) => Some(s.clone()),
            _ => None,
        });

    let scheme = inheritance_raw.as_deref().and_then(parse_inheritance_xml);

    let scheme = match scheme {
        Some(s) => s,
        None => {
            debug!("No inheritance scheme found, skipping inherit_from");
            return;
        }
    };

    // Determine effective defaults for this type pair
    let tp = (child_entrytype.clone(), parent_entrytype.clone());
    let tp_rev = (parent_entrytype.clone(), child_entrytype.clone());

    let effective_inherit_all = scheme
        .defaults
        .type_pair_overrides
        .get(&tp)
        .or_else(|| scheme.defaults.type_pair_overrides.get(&tp_rev))
        .map(|o| o.inherit_all)
        .unwrap_or(scheme.defaults.inherit_all);

    let effective_override_target = scheme.defaults.override_target;

    // Collect noinherit fields for child
    let noinherit_fields = get_noinherit_fields(biber, secnum, child_key);

    // Track which parent fields have been handled
    let mut handled_source_fields: HashSet<String> = HashSet::new();

    // --- Apply explicit rules ---
    for rule in &scheme.rules {
        // Match type pair: both source and target can be "*" (wildcard)
        let source_match = rule.source_type == "*" || rule.source_type == parent_entrytype;
        let target_match = rule.target_type == "*" || rule.target_type == child_entrytype;
        if !source_match || !target_match {
            continue;
        }

        for field in &rule.fields {
            if field.skip {
                handled_source_fields.insert(field.source.clone());
                continue;
            }

            let target_field = match &field.target {
                Some(t) => t.clone(),
                None => field.source.clone(),
            };

            // Check noinherit
            if noinherit_fields.contains(&target_field) {
                handled_source_fields.insert(field.source.clone());
                continue;
            }

            // Check if child already has the field
            let child_has_field = {
                let section = biber.sections.get_section(secnum);
                section
                    .and_then(|s| s.bibentry(child_key))
                    .map(|be| be.has_field(&target_field))
                    .unwrap_or(false)
            };

            if child_has_field && !field.override_target && !effective_override_target {
                handled_source_fields.insert(field.source.clone());
                continue;
            }

            // Copy the field value from parent
            let parent_value = {
                let section = biber.sections.get_section(secnum);
                section
                    .and_then(|s| s.bibentry(parent_key))
                    .and_then(|be| be.get_field(&field.source))
                    .cloned()
            };

            if let Some(value) = parent_value {
                let section = biber.sections.get_section_mut(secnum);
                if let Some(be) = section.and_then(|s| s.bibentries.get_entry_mut(child_key)) {
                    be.set_field(&target_field, value.clone());
                    debug!(
                        "Inherited field '{}' for '{child_key}': {}.{} -> {}.{}",
                        field.source, parent_entrytype, field.source, child_entrytype, target_field
                    );
                }
            }

            handled_source_fields.insert(field.source.clone());
        }
    }

    // --- Inherit all remaining unhandled fields ---
    if effective_inherit_all {
        let parent_fields: Vec<(String, ConfigValue)> = {
            let section = biber.sections.get_section(secnum);
            match section.and_then(|s| s.bibentry(parent_key)) {
                Some(be) => be
                    .fields
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                None => return,
            }
        };

        for (field_name, field_value) in &parent_fields {
            // Skip already-handled fields
            if handled_source_fields.contains(field_name) {
                continue;
            }

            // Skip ignored fields
            if scheme.defaults.ignore.contains(field_name) {
                continue;
            }

            // Skip internal/private fields
            if field_name.starts_with("_")
                || field_name == "ids"
                || field_name == "crossref"
                || field_name == "xref"
                || field_name == "xdata"
            {
                continue;
            }

            // Check noinherit
            if noinherit_fields.contains(field_name) {
                continue;
            }

            // Datepart inheritance blocking: if child has any datepart field,
            // and this is a datepart field, don't inherit
            if is_datepart_field(field_name) {
                let child_has_datepart = {
                    let section = biber.sections.get_section(secnum);
                    section
                        .and_then(|s| s.bibentry(child_key))
                        .map(|be| DATE_PARTS.iter().any(|dp| be.has_field(dp)))
                        .unwrap_or(false)
                };
                if child_has_datepart {
                    continue;
                }
            }

            // Check if child already has the field
            let child_has_field = {
                let section = biber.sections.get_section(secnum);
                section
                    .and_then(|s| s.bibentry(child_key))
                    .map(|be| be.has_field(field_name))
                    .unwrap_or(false)
            };

            if child_has_field && !effective_override_target {
                continue;
            }

            // Copy field
            let section = biber.sections.get_section_mut(secnum);
            if let Some(be) = section.and_then(|s| s.bibentries.get_entry_mut(child_key)) {
                be.set_field(field_name.clone(), field_value.clone());
                debug!(
                    "Inherited field (all) '{field_name}' for '{child_key}' from '{parent_key}'"
                );
            }
        }
    }
}

const DATE_PARTS: &[&str] = &[
    "year",
    "month",
    "day",
    "endyear",
    "endmonth",
    "endday",
    "season",
    "endseason",
];

fn is_datepart_field(name: &str) -> bool {
    DATE_PARTS.contains(&name)
}

// ---- XDATA resolution ----

/// Resolve XDATA references for all entries in a section.
///
/// This implements the Perl `Biber::resolve_xdata` logic for whole XDATA.
/// Granular XDATA references (per-field `xdata=entry-field` markers) are
/// deferred (post-MVP).
pub fn resolve_xdata_section(biber: &mut Biber, secnum: u32) {
    debug!("resolve_xdata for section {secnum}");

    let all_keys: Vec<String> = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        section
            .bibentries
            .citekeys()
            .map(|s| s.to_string())
            .collect()
    };

    for key in &all_keys {
        // Skip xdata entries themselves
        let is_xdata = {
            let section = biber.sections.get_section(secnum);
            section
                .and_then(|s| s.bibentry(key))
                .map(|be| be.entrytype == "xdata")
                .unwrap_or(false)
        };
        if is_xdata {
            continue;
        }

        // Get the xdata field value
        let xdata_value = {
            let section = biber.sections.get_section(secnum);
            section
                .and_then(|s| s.bibentry(key))
                .and_then(|be| be.get_field_str("xdata"))
                .map(|s| s.to_string())
        };

        let xdata_value = match xdata_value {
            Some(v) => v,
            None => continue,
        };

        // Parse comma-separated XDATA keys
        let xdata_keys: Vec<String> = xdata_value
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if xdata_keys.is_empty() {
            continue;
        }

        debug!("Resolving XDATA for '{key}': keys = {:?}", xdata_keys);

        // Resolve each xdata key
        for xd_key in &xdata_keys {
            // Check if the xdata entry exists
            let xd_entrytype = {
                let section = biber.sections.get_section(secnum);
                section
                    .and_then(|s| s.bibentry(xd_key))
                    .map(|be| be.entrytype.clone())
            };

            let xd_entrytype = match xd_entrytype {
                Some(et) => et,
                None => {
                    warn!("XDATA entry '{xd_key}' not found (referenced by '{key}')");
                    continue;
                }
            };

            if xd_entrytype != "xdata" {
                warn!("XDATA reference '{xd_key}' is not an xdata entry (type='{xd_entrytype}')");
                continue;
            }

            // Loop detection
            if is_inheritance_path(biber, "xdata", key, xd_key) {
                warn!("Circular XDATA reference detected: '{key}' -> '{xd_key}'");
                continue;
            }
            record_inheritance_mut(biber, "xdata", key, xd_key);

            // Recursively resolve XDATA refs of the XDATA entry itself
            let xd_has_xdata = {
                let section = biber.sections.get_section(secnum);
                section
                    .and_then(|s| s.bibentry(xd_key))
                    .map(|be| be.has_field("xdata"))
                    .unwrap_or(false)
            };
            if xd_has_xdata {
                resolve_xdata_single(biber, secnum, xd_key);
            }

            // Copy all fields from the XDATA entry to the target (except ids)
            let xd_fields: Vec<(String, ConfigValue)> = {
                let section = biber.sections.get_section(secnum);
                match section.and_then(|s| s.bibentry(xd_key)) {
                    Some(be) => be
                        .fields
                        .iter()
                        .filter(|(k, _)| *k != "ids" && *k != "xdata")
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                    None => continue,
                }
            };

            let section = biber.sections.get_section_mut(secnum);
            if let Some(be) = section.and_then(|s| s.bibentries.get_entry_mut(key)) {
                for (field_name, field_value) in &xd_fields {
                    // Don't overwrite existing fields
                    if !be.has_field(field_name) {
                        be.set_field(field_name.clone(), field_value.clone());
                        debug!("Inherited field '{field_name}' from XDATA '{xd_key}' to '{key}'");
                    }
                }
            }
        }
    }
}

/// Resolve XDATA for a single entry (used for recursive resolution).
fn resolve_xdata_single(biber: &mut Biber, secnum: u32, key: &str) {
    let xdata_value = {
        let section = biber.sections.get_section(secnum);
        section
            .and_then(|s| s.bibentry(key))
            .and_then(|be| be.get_field_str("xdata"))
            .map(|s| s.to_string())
    };

    let xdata_value = match xdata_value {
        Some(v) => v,
        None => return,
    };

    let xdata_keys: Vec<String> = xdata_value
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    for xd_key in &xdata_keys {
        let is_xdata_entry = {
            let section = biber.sections.get_section(secnum);
            section
                .and_then(|s| s.bibentry(xd_key))
                .map(|be| be.entrytype == "xdata")
                .unwrap_or(false)
        };
        if !is_xdata_entry {
            continue;
        }

        if is_inheritance_path(biber, "xdata", key, xd_key) {
            continue;
        }
        record_inheritance_mut(biber, "xdata", key, xd_key);

        // Recurse
        let xd_has_xdata = {
            let section = biber.sections.get_section(secnum);
            section
                .and_then(|s| s.bibentry(xd_key))
                .map(|be| be.has_field("xdata"))
                .unwrap_or(false)
        };
        if xd_has_xdata {
            resolve_xdata_single(biber, secnum, xd_key);
        }

        // Copy fields
        let xd_fields: Vec<(String, ConfigValue)> = {
            let section = biber.sections.get_section(secnum);
            match section.and_then(|s| s.bibentry(xd_key)) {
                Some(be) => be
                    .fields
                    .iter()
                    .filter(|(k, _)| *k != "ids" && *k != "xdata")
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
                None => continue,
            }
        };

        let section = biber.sections.get_section_mut(secnum);
        if let Some(be) = section.and_then(|s| s.bibentries.get_entry_mut(key)) {
            for (field_name, field_value) in &xd_fields {
                if !be.has_field(field_name) {
                    be.set_field(field_name.clone(), field_value.clone());
                }
            }
        }
    }
}

// ---- Inheritance state tracking (on Config) ----

/// Check if there's a circular inheritance path from `start` to `target`.
pub fn is_inheritance_path(biber: &Biber, r#type: &str, start: &str, target: &str) -> bool {
    if start == target {
        return true;
    }

    let edges = match biber.config.inheritance_edges.get(r#type) {
        Some(e) => e,
        None => return false,
    };

    // DFS from start to target
    let mut visited: HashSet<&str> = HashSet::new();
    let mut stack: Vec<&str> = vec![start];

    while let Some(current) = stack.pop() {
        if current == target {
            return true;
        }
        if !visited.insert(current) {
            continue;
        }
        for (src, tgt) in edges {
            if src == current {
                stack.push(tgt);
            }
        }
    }

    false
}

/// Record an inheritance edge in the config.
pub fn record_inheritance_mut(biber: &mut Biber, r#type: &str, source: &str, target: &str) {
    biber
        .config
        .inheritance_edges
        .entry(r#type.to_string())
        .or_default()
        .push((source.to_string(), target.to_string()));
}

/// Remove all recorded inheritance edges (called at start of section processing).
pub fn clear_inheritance(biber: &mut Biber) {
    biber.config.inheritance_edges.clear();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::Entry;
    use crate::section::Section;

    fn make_biber() -> Biber {
        let mut b = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("child");
        section.add_cite("parent");
        let mut child = Entry::new("child", "inbook");
        child.set_field_str("author", "Child Author");
        child.set_field_str("title", "Child Title");
        child.set_field_str("crossref", "parent");
        section.bibentries.add_entry(child);

        let mut parent = Entry::new("parent", "book");
        parent.set_field_str("author", "Parent Author");
        parent.set_field_str("title", "Parent Title");
        parent.set_field_str("publisher", "Parent Publisher");
        parent.set_field_str("year", "2020");
        section.bibentries.add_entry(parent);

        b.sections.add_section(section);
        b
    }

    fn set_inheritance_xml(biber: &mut Biber, xml: &str) {
        biber
            .config
            .setblxoption(None, "inheritance", ConfigValue::Raw(xml.to_string()));
    }

    #[test]
    fn parse_simple_inheritance() {
        let xml = r#"<inheritance>
            <defaults inherit_all="true" override_target="false"/>
            <inherit>
                <type_pair source="book" target="inbook"/>
                <field source="title" target="booktitle" override_target="true"/>
            </inherit>
        </inheritance>"#;

        let scheme = parse_inheritance_xml(xml).unwrap();
        assert!(scheme.defaults.inherit_all);
        assert!(!scheme.defaults.override_target);
        assert_eq!(scheme.rules.len(), 1);
        assert_eq!(scheme.rules[0].fields.len(), 1);
        assert_eq!(scheme.rules[0].fields[0].source, "title");
        assert_eq!(
            scheme.rules[0].fields[0].target,
            Some("booktitle".to_string())
        );
        assert!(scheme.rules[0].fields[0].override_target);
    }

    #[test]
    fn parse_inheritance_with_bcf_prefix() {
        let xml = r#"<bcf:inheritance>
            <bcf:defaults inherit_all="true"/>
            <bcf:inherit>
                <bcf:type_pair source="book" target="inbook"/>
                <bcf:field source="title" target="booktitle"/>
            </bcf:inherit>
        </bcf:inheritance>"#;

        let scheme = parse_inheritance_xml(xml).unwrap();
        assert!(scheme.defaults.inherit_all);
        assert_eq!(scheme.rules.len(), 1);
    }

    #[test]
    fn parse_defaults_with_type_pair_overrides() {
        let xml = r#"<inheritance>
            <defaults inherit_all="true" override_target="false">
                <type_pair source="*" target="incollection" inherit_all="false"/>
            </defaults>
        </inheritance>"#;

        let scheme = parse_inheritance_xml(xml).unwrap();
        assert!(scheme.defaults.inherit_all);
        let override_tp = scheme
            .defaults
            .type_pair_overrides
            .get(&("*".to_string(), "incollection".to_string()))
            .unwrap();
        assert!(!override_tp.inherit_all);
    }

    #[test]
    fn parse_fields_with_skip() {
        let xml = r#"<inheritance>
            <defaults inherit_all="true"/>
            <inherit>
                <type_pair source="*" target="*"/>
                <field source="publisher" skip="true"/>
            </inherit>
        </inheritance>"#;

        let scheme = parse_inheritance_xml(xml).unwrap();
        assert_eq!(scheme.rules.len(), 1);
        assert!(scheme.rules[0].fields[0].skip);
    }

    #[test]
    fn inherit_simple_field() {
        let mut biber = make_biber();
        set_inheritance_xml(
            &mut biber,
            r#"<inheritance>
                <defaults inherit_all="false" override_target="false"/>
                <inherit>
                    <type_pair source="book" target="inbook"/>
                    <field source="publisher" target="publisher"/>
                </inherit>
            </inheritance>"#,
        );

        inherit_from(&mut biber, 0, "child", "parent");

        let section = biber.sections.get_section(0).unwrap();
        let child = section.bibentry("child").unwrap();
        assert_eq!(child.get_field_str("publisher"), Some("Parent Publisher"));
    }

    #[test]
    fn inherit_all_unhandled_fields() {
        let mut biber = make_biber();
        set_inheritance_xml(
            &mut biber,
            r#"<inheritance>
                <defaults inherit_all="true" override_target="false"/>
            </inheritance>"#,
        );

        inherit_from(&mut biber, 0, "child", "parent");

        let section = biber.sections.get_section(0).unwrap();
        let child = section.bibentry("child").unwrap();
        // year should be inherited (child doesn't have it)
        assert_eq!(child.get_field_str("year"), Some("2020"));
        // publisher should be inherited
        assert_eq!(child.get_field_str("publisher"), Some("Parent Publisher"));
        // title should NOT be overwritten (child already has it and override is false)
        assert_eq!(child.get_field_str("title"), Some("Child Title"));
        // author should NOT be overwritten
        assert_eq!(child.get_field_str("author"), Some("Child Author"));
    }

    #[test]
    fn inherit_rule_maps_field_with_rename() {
        let mut biber = make_biber();
        set_inheritance_xml(
            &mut biber,
            r#"<inheritance>
                <defaults inherit_all="false" override_target="false"/>
                <inherit>
                    <type_pair source="book" target="inbook"/>
                    <field source="title" target="booktitle" override_target="true"/>
                </inherit>
            </inheritance>"#,
        );

        inherit_from(&mut biber, 0, "child", "parent");

        let section = biber.sections.get_section(0).unwrap();
        let child = section.bibentry("child").unwrap();
        // booktitle should be inherited from parent's title
        assert_eq!(child.get_field_str("booktitle"), Some("Parent Title"));
    }

    #[test]
    fn inherit_does_not_override_existing_field() {
        let mut biber = make_biber();
        set_inheritance_xml(
            &mut biber,
            r#"<inheritance>
                <defaults inherit_all="true" override_target="false"/>
            </inheritance>"#,
        );

        inherit_from(&mut biber, 0, "child", "parent");

        let section = biber.sections.get_section(0).unwrap();
        let child = section.bibentry("child").unwrap();
        // Child has its own author, should not be overwritten
        assert_eq!(child.get_field_str("author"), Some("Child Author"));
        // Child has its own title, should not be overwritten
        assert_eq!(child.get_field_str("title"), Some("Child Title"));
    }

    #[test]
    fn inherit_skip_field() {
        let mut biber = make_biber();
        set_inheritance_xml(
            &mut biber,
            r#"<inheritance>
                <defaults inherit_all="true" override_target="false"/>
                <inherit>
                    <type_pair source="*" target="*"/>
                    <field source="publisher" skip="true"/>
                </inherit>
            </inheritance>"#,
        );

        inherit_from(&mut biber, 0, "child", "parent");

        let section = biber.sections.get_section(0).unwrap();
        let child = section.bibentry("child").unwrap();
        // publisher should be skipped
        assert_eq!(child.get_field_str("publisher"), None);
    }

    #[test]
    fn resolve_xdata_whole() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("target");

        let mut target = Entry::new("target", "book");
        target.set_field_str("author", "Target Author");
        target.set_field_str("xdata", "shared");
        section.bibentries.add_entry(target);

        let mut xdata_entry = Entry::new("shared", "xdata");
        xdata_entry.set_field_str("publisher", "Shared Publisher");
        xdata_entry.set_field_str("year", "2021");
        section.bibentries.add_entry(xdata_entry);

        biber.sections.add_section(section);
        biber.set_current_section(0);

        resolve_xdata_section(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();
        let target = section.bibentry("target").unwrap();
        assert_eq!(target.get_field_str("publisher"), Some("Shared Publisher"));
        assert_eq!(target.get_field_str("year"), Some("2021"));
        assert_eq!(target.get_field_str("author"), Some("Target Author"));
    }

    #[test]
    fn resolve_xdata_with_cascading() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("target");

        let mut target = Entry::new("target", "book");
        target.set_field_str("author", "Target Author");
        target.set_field_str("xdata", "middle");
        section.bibentries.add_entry(target);

        let mut middle = Entry::new("middle", "xdata");
        middle.set_field_str("publisher", "Middle Publisher");
        middle.set_field_str("xdata", "base");
        section.bibentries.add_entry(middle);

        let mut base = Entry::new("base", "xdata");
        base.set_field_str("year", "2022");
        base.set_field_str("note", "Base Note");
        section.bibentries.add_entry(base);

        biber.sections.add_section(section);
        biber.set_current_section(0);

        resolve_xdata_section(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();
        let target = section.bibentry("target").unwrap();
        assert_eq!(target.get_field_str("publisher"), Some("Middle Publisher"));
        assert_eq!(target.get_field_str("year"), Some("2022"));
        assert_eq!(target.get_field_str("note"), Some("Base Note"));
    }

    #[test]
    fn resolve_xdata_circular_detection() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("entry1");

        let mut entry1 = Entry::new("entry1", "book");
        entry1.set_field_str("xdata", "x1, x2");
        section.bibentries.add_entry(entry1);

        let mut x1 = Entry::new("x1", "xdata");
        x1.set_field_str("xdata", "x2");
        x1.set_field_str("note", "From x1");
        section.bibentries.add_entry(x1);

        let mut x2 = Entry::new("x2", "xdata");
        x2.set_field_str("xdata", "x1");
        x2.set_field_str("publisher", "From x2");
        section.bibentries.add_entry(x2);

        biber.sections.add_section(section);
        biber.set_current_section(0);
        // Should not panic due to circular reference
        resolve_xdata_section(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();
        let entry1 = section.bibentry("entry1").unwrap();
        // x1 should be resolved first (note from x1)
        // Then when resolving x1's xdata (which points to x2), x2
        // should resolve x1 but the circular detection prevents infinite recursion
        assert_eq!(entry1.get_field_str("note"), Some("From x1"));
        assert_eq!(entry1.get_field_str("publisher"), Some("From x2"));
    }
}
