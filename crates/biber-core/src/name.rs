//! Name model — authors, editors, etc.
//!
//! Ported from `lib/Biber/Entry/Name.pm` and `lib/Biber/Entry/Names.pm`.

use std::collections::BTreeMap;

use md5::{Digest, Md5};
use tracing::{debug, trace};

use crate::config::ConfigValue;

/// A single name with its nameparts (family, given, prefix, suffix).
///
/// In Perl, `Biber::Entry::Name` is a blessed hash with dynamic namepart
/// keys. Here we use a struct with a map for extensibility.
#[derive(Debug, Clone, Default)]
pub struct Name {
    /// Nameparts: "family", "given", "prefix", "suffix" → value.
    pub nameparts: BTreeMap<String, String>,
    /// Raw name string (as parsed from the .bib).
    pub rawstring: String,
    /// Per-name options (from NAME scope).
    pub options: BTreeMap<String, String>,
    /// A unique ID for this name within its namelist.
    pub id: Option<String>,
    /// Disambiguation flag (0 or 1). Set by process_namedis.
    pub un: u32,
    /// Which namepart was used for disambiguation (e.g. "base").
    pub uniquepart: String,
    /// Unique part of given name (when given name helps disambiguate).
    pub givenun: String,
}

impl Name {
    /// Create a new empty name.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a name from a raw string (used before namepart parsing).
    pub fn from_raw(raw: impl Into<String>) -> Self {
        Self {
            rawstring: raw.into(),
            ..Default::default()
        }
    }

    /// Get a namepart value.
    pub fn get_namepart(&self, part: &str) -> Option<&str> {
        self.nameparts.get(part).map(|s| s.as_str())
    }

    /// Set a namepart value.
    pub fn set_namepart(&mut self, part: impl Into<String>, value: impl Into<String>) {
        self.nameparts.insert(part.into(), value.into());
    }

    /// Get the family name part.
    pub fn family(&self) -> Option<&str> {
        self.get_namepart("family")
    }

    /// Get the given name part.
    pub fn given(&self) -> Option<&str> {
        self.get_namepart("given")
    }
}

/// A list of names (e.g. the `author` field of an entry).
///
/// Ported from `lib/Biber/Entry/Names.pm`.
#[derive(Debug, Clone, Default)]
pub struct Names {
    /// The list of names.
    pub names: Vec<Name>,
    /// Per-namelist options (from NAMELIST scope).
    pub options: BTreeMap<String, String>,
}

impl Names {
    /// Create an empty name list.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a name.
    pub fn add_name(&mut self, name: Name) {
        self.names.push(name);
    }

    /// Number of names.
    pub fn count(&self) -> usize {
        self.names.len()
    }

    /// Get a name by index.
    pub fn get(&self, idx: usize) -> Option<&Name> {
        self.names.get(idx)
    }

    /// Iterate over names.
    pub fn iter(&self) -> impl Iterator<Item = &Name> {
        self.names.iter()
    }

    /// Is the list empty?
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// Parse a name list string into a `Names` object.
///
/// Splits on " and " and parses each name using the standard BibTeX
/// name algorithm (see `parse_name`).
pub fn parse_names(namestr: &str) -> Names {
    let mut names = Names::new();
    for part in namestr.split(" and ") {
        let part = part.trim();
        if part.is_empty() || part.eq_ignore_ascii_case("others") {
            continue;
        }
        names.add_name(parse_name(part));
    }
    names
}

/// Parse a BibTeX name in standard format.
///
/// Implements the BibTeX name algorithm as described in the LaTeX companion
/// and the btparse source code. Handles:
/// - `First von Last`
/// - `von Last, First`
/// - `von Last, Jr, First`
/// - Braced/quoted segments (not split on spaces)
/// - "others" (et al marker)
pub fn parse_name(namestr: &str) -> Name {
    let mut namestr = namestr.trim().to_string();
    let mut result = String::new();
    let mut prev_ws = false;
    for c in namestr.chars() {
        if c.is_whitespace() {
            if !prev_ws {
                result.push(' ');
            }
            prev_ws = true;
        } else {
            result.push(c);
            prev_ws = false;
        }
    }
    namestr = result;

    if namestr.starts_with("{{") && namestr.ends_with("}}") {
        let inner = &namestr[1..namestr.len() - 1];
        namestr = format!("{{{}}}", inner.trim_matches(|c| c == '{' || c == '}'));
    }

    let comma_parts: Vec<&str> = split_top_level_commas(&namestr);

    let (family, given, prefix, suffix) = match comma_parts.len() {
        1 => split_first_von_last(comma_parts[0]),
        2 => {
            let (f, g, p, s) = split_von_last_first(comma_parts[0], comma_parts[1]);
            (f, g, p, s)
        }
        3 => {
            let (f, _, p, _) = split_von_last_first(comma_parts[0], "");
            let suffix = comma_parts[1].trim();
            let given = comma_parts[2].trim();
            (f, given.to_string(), p, suffix.to_string())
        }
        _ => (namestr.clone(), String::new(), String::new(), String::new()),
    };

    let mut name = Name::new();
    name.rawstring = namestr;
    if !family.is_empty() {
        name.set_namepart("family", &family);
    }
    if !given.is_empty() {
        name.set_namepart("given", &given);
    }
    if !prefix.is_empty() {
        name.set_namepart("prefix", &prefix);
    }
    if !suffix.is_empty() {
        name.set_namepart("suffix", &suffix);
    }
    name
}

/// Parse a name in biblatex extended format: `family=Doe, given=John`.
pub fn parse_name_x(namestr: &str, xnamesep: char) -> Name {
    let mut name = Name::new();
    name.rawstring = namestr.to_string();
    let parts = split_xsv(namestr);

    for part in &parts {
        let part = part.trim();
        if let Some(eq_pos) = part.find(xnamesep) {
            let npn = part[..eq_pos].trim().to_lowercase();
            let npv = part[eq_pos + xnamesep.len_utf8()..].trim();
            if !npn.is_empty() && !npv.is_empty() {
                name.set_namepart(&npn, npv);
            }
        }
    }

    if name.nameparts.is_empty() {
        return parse_name(namestr);
    }

    name
}

/// Parse all name fields on an entry, storing parsed `Names` in `entry.names`.
///
/// Called before namehash computation. Skips fields that are already parsed.
pub fn parse_entry_names(entry: &mut crate::entry::Entry) {
    let name_fields = ["author", "editor", "translator", "bookauthor"];
    for field in &name_fields {
        if entry.names.contains_key(*field) {
            continue;
        }
        if let Some(raw) = entry.get_field_str(field) {
            let names = parse_names(raw);
            let count = names.count();
            if count > 0 {
                debug!(
                    "Parsed {count} names from '{field}' for '{}'",
                    entry.citekey
                );
                trace!(
                    "parse_entry_names: field='{field}', citekey='{}', count={count}",
                    entry.citekey
                );
                entry.names.insert(field.to_string(), names);
            }
        }
    }
}

/// Split a string on commas at the top level (not inside braces).
fn split_top_level_commas(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0;

    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

/// Split "First [von] Last" format into (family, given, prefix, suffix).
fn split_first_von_last(s: &str) -> (String, String, String, String) {
    let words = split_name_words(s);
    if words.len() == 1 {
        return (
            words[0].clone(),
            String::new(),
            String::new(),
            String::new(),
        );
    }

    let lower_indices: Vec<usize> = (0..words.len() - 1)
        .filter(|&i| is_von_word(&words[i]))
        .collect();

    if lower_indices.is_empty() {
        let family = words.last().unwrap().clone();
        let given = words[..words.len() - 1].join(" ");
        (family, given, String::new(), String::new())
    } else {
        let von_start = lower_indices[0];
        let von_end = *lower_indices.last().unwrap();

        let given = if von_start > 0 {
            words[..von_start].join(" ")
        } else {
            String::new()
        };
        let prefix = words[von_start..=von_end].join(" ");
        let family = words[von_end + 1..].join(" ");

        (family, given, prefix, String::new())
    }
}

/// Split "[von] Last, First" format into (family, given, prefix, suffix).
fn split_von_last_first(last_part: &str, first_part: &str) -> (String, String, String, String) {
    let words = split_name_words(last_part);

    if words.len() == 1 {
        return (
            words[0].clone(),
            first_part.trim().to_string(),
            String::new(),
            String::new(),
        );
    }

    let lower_indices: Vec<usize> = (0..words.len())
        .filter(|&i| is_von_word(&words[i]))
        .collect();

    if lower_indices.is_empty() || *lower_indices.last().unwrap() == words.len() - 1 {
        let family = words.join(" ");
        return (
            family,
            first_part.trim().to_string(),
            String::new(),
            String::new(),
        );
    }

    let von_start = lower_indices[0];
    let von_end = *lower_indices.last().unwrap();
    let prefix = words[von_start..=von_end].join(" ");
    let family = words[von_end + 1..].join(" ");
    let given = first_part.trim().to_string();

    (family, given, prefix, String::new())
}

/// Split a name string into words, respecting braces and tildes.
fn split_name_words(s: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut depth = 0;

    for c in s.chars() {
        match c {
            '{' => {
                depth += 1;
                current.push(c);
            }
            '}' => {
                depth -= 1;
                current.push(c);
            }
            ' ' | '\t' | '\n' if depth == 0 => {
                if !current.is_empty() {
                    words.push(current.clone());
                    current.clear();
                }
            }
            '~' if depth == 0 => {
                if !current.is_empty() {
                    words.push(current.clone());
                    current.clear();
                }
                current.push('~');
                words.push(current.clone());
                current.clear();
            }
            _ => {
                current.push(c);
            }
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Check if a word is a "von" word (starts with a lowercase letter outside braces).
fn is_von_word(word: &str) -> bool {
    let mut depth = 0;
    for c in word.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            _ if depth == 0 && c.is_alphabetic() => {
                return c.is_lowercase();
            }
            _ => {}
        }
    }
    false
}

/// A parsed name part (string + initials).
#[derive(Debug, Clone, Default)]
pub struct NamePart {
    /// The name part string (e.g. "John").
    pub string: String,
    /// Initials (e.g. ["J"]).
    pub initials: Vec<String>,
}

/// Generate initials from a space-separated name part.
///
/// Handles braced compound initials like `{IJ}` → "IJ".
pub fn gen_initials(s: &str) -> Vec<String> {
    let mut initials = Vec::new();
    let mut current = String::new();
    let mut in_brace = false;

    for c in s.chars() {
        match c {
            '{' => {
                in_brace = true;
                current.clear();
            }
            '}' => {
                if in_brace && !current.is_empty() {
                    initials.push(current.clone());
                }
                in_brace = false;
                current.clear();
            }
            ' ' | '\t' | '\n' if !in_brace => {
                if !current.is_empty() {
                    initials.push(current.clone());
                }
                current.clear();
            }
            '~' if !in_brace => {
                if !current.is_empty() {
                    initials.push(current.clone());
                }
                current.clear();
            }
            _ if !in_brace => {
                if current.is_empty() && c.is_alphabetic() {
                    current.push(c);
                }
            }
            _ if in_brace => {
                current.push(c);
            }
            _ => {}
        }
    }
    if !current.is_empty() {
        initials.push(current);
    }
    initials
}

/// Split a CSV-like string (comma-separated, respecting braces and quotes).
fn split_xsv(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    let mut in_quote = false;

    for c in s.chars() {
        match c {
            '{' if !in_quote => depth += 1,
            '}' if !in_quote => depth -= 1,
            '"' if depth == 0 => in_quote = !in_quote,
            ',' if depth == 0 && !in_quote => {
                parts.push(current.clone());
                current.clear();
            }
            _ => current.push(c),
        }
    }
    parts.push(current);
    parts
}

/// Compute an individual name's hash using the namehashtemplate from config.
///
/// Concatenates the selected nameparts in template order (like Perl's
/// `Biber::Entry::Name::get_hash`) and returns the MD5 hex digest.
pub fn compute_name_hash(name: &Name, config: &crate::config::Config) -> String {
    let template = config
        .getblxoption(None, "namehashtemplate")
        .and_then(|v| match v {
            ConfigValue::Map(m) => m.get("global"),
            _ => None,
        });
    let items = match template {
        Some(ConfigValue::List(items)) => items,
        _ => {
            // Default: use all standard nameparts
            let mut s = String::new();
            for np in &["family", "given", "prefix", "suffix"] {
                if let Some(val) = name.get_namepart(np) {
                    s.push_str(val);
                }
            }
            if s.is_empty() {
                s = name.rawstring.clone();
            }
            return hex::encode(Md5::digest(s.as_bytes()));
        }
    };
    let mut concat = String::new();
    for item in items {
        if let ConfigValue::Map(m) = item {
            let namepart = m.get("namepart").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(val) = name.get_namepart(namepart) {
                concat.push_str(val);
            }
        }
    }
    if concat.is_empty() {
        concat = name.rawstring.clone();
    }
    hex::encode(Md5::digest(concat.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_parts() {
        let mut n = Name::from_raw("Doe, John");
        n.set_namepart("family", "Doe");
        n.set_namepart("given", "John");
        assert_eq!(n.family(), Some("Doe"));
        assert_eq!(n.given(), Some("John"));
    }

    #[test]
    fn names_collection() {
        let mut ns = Names::new();
        assert!(ns.is_empty());
        ns.add_name(Name::from_raw("Doe"));
        ns.add_name(Name::from_raw("Smith"));
        assert_eq!(ns.count(), 2);
        assert_eq!(ns.get(0).unwrap().rawstring, "Doe");
    }

    #[test]
    fn compute_name_hash_default_template() {
        use crate::config::Config;
        let mut n = Name::from_raw("Doe, John");
        n.set_namepart("family", "Doe");
        n.set_namepart("given", "John");
        let config = Config::new();
        // No namehashtemplate set — uses default (all nameparts)
        let hash = super::compute_name_hash(&n, &config);
        // Default template concatenates "DoeJohn"
        let expected = hex::encode(md5::Md5::digest(b"DoeJohn"));
        assert_eq!(
            hash, expected,
            "default template should hash concatenated nameparts"
        );
    }

    #[test]
    fn compute_name_hash_with_custom_template() {
        use crate::config::{Config, ConfigValue};
        use std::collections::BTreeMap;
        let mut n = Name::from_raw("Doe, John");
        n.set_namepart("family", "Doe");
        n.set_namepart("given", "John");
        n.set_namepart("prefix", "van");
        n.set_namepart("suffix", "Jr.");
        let mut config = Config::new();

        // Custom template: only family + prefix
        let mut part1 = BTreeMap::new();
        part1.insert("namepart".into(), ConfigValue::Str("family".into()));
        let mut part2 = BTreeMap::new();
        part2.insert("namepart".into(), ConfigValue::Str("prefix".into()));
        let mut template = BTreeMap::new();
        template.insert(
            "global".into(),
            ConfigValue::List(vec![ConfigValue::Map(part1), ConfigValue::Map(part2)]),
        );
        config.setblxoption(None, "namehashtemplate", ConfigValue::Map(template));
        let hash = super::compute_name_hash(&n, &config);
        // Concatenates "Doe" + "van" = "Doevan"
        let expected = hex::encode(md5::Md5::digest(b"Doevan"));
        assert_eq!(
            hash, expected,
            "custom template should hash selected nameparts only"
        );
    }

    #[test]
    fn compute_name_hash_empty_nameparts_falls_back_to_rawstring() {
        use crate::config::Config;
        // Name with no explicit nameparts set
        let n = Name::from_raw("Aristotle");
        let config = Config::new();
        let hash = super::compute_name_hash(&n, &config);
        let expected = hex::encode(md5::Md5::digest(b"Aristotle"));
        assert_eq!(
            hash, expected,
            "empty nameparts should fall back to rawstring"
        );
    }

    #[test]
    fn compute_name_hash_different_orders_produce_different_hashes() {
        use crate::config::{Config, ConfigValue};
        use std::collections::BTreeMap;
        let mut n = Name::from_raw("Doe, John");
        n.set_namepart("family", "Doe");
        n.set_namepart("given", "John");

        // Template: given then family (reverse order)
        let mut part1 = BTreeMap::new();
        part1.insert("namepart".into(), ConfigValue::Str("given".into()));
        let mut part2 = BTreeMap::new();
        part2.insert("namepart".into(), ConfigValue::Str("family".into()));
        let mut template = BTreeMap::new();
        template.insert(
            "global".into(),
            ConfigValue::List(vec![ConfigValue::Map(part1), ConfigValue::Map(part2)]),
        );
        let mut config = Config::new();
        config.setblxoption(None, "namehashtemplate", ConfigValue::Map(template));

        let hash_given_first = super::compute_name_hash(&n, &config);

        // Template: family then given (normal order)
        let part1 = {
            let mut m = BTreeMap::new();
            m.insert("namepart".into(), ConfigValue::Str("family".into()));
            ConfigValue::Map(m)
        };
        let part2 = {
            let mut m = BTreeMap::new();
            m.insert("namepart".into(), ConfigValue::Str("given".into()));
            ConfigValue::Map(m)
        };
        let mut template = BTreeMap::new();
        template.insert("global".into(), ConfigValue::List(vec![part1, part2]));
        let mut config2 = Config::new();
        config2.setblxoption(None, "namehashtemplate", ConfigValue::Map(template));

        let hash_family_first = super::compute_name_hash(&n, &config2);

        assert_ne!(
            hash_given_first, hash_family_first,
            "different namepart orders should produce different hashes"
        );
    }
}
