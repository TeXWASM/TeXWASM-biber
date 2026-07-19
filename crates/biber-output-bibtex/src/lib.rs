//! BibTeX (.bib) output writer for tool mode.
//!
//! Emits entries in standard BibTeX format with proper name handling,
//! field ordering, and entry type preservation. Internal biber fields
//! (citekey, entrytype, datatype, etc.) are filtered out.

#![forbid(unsafe_code)]

use biber_core::config::ConfigValue;
use biber_core::entry::Entry;
use biber_core::latex_recode::{latex_encode_with_set, RecodeSet};
use biber_core::processor::Biber;

/// Internal biber field names that should not appear in the output .bib file.
const INTERNAL_FIELDS: &[&str] = &[
    "citekey",
    "entrytype",
    "datatype",
    "original_type",
    "crossrefsource",
    "nocite",
    "labelname",
    "labeltitle",
    "labelyear",
    "labeldate",
    "labeldatesource",
    "labelmonth",
    "labelday",
    "extradate",
    "extradatescope",
    "namehash",
    "fullhash",
    "fullhashraw",
    "bibnamehash",
    "seenname",
    "seentitle",
    "seenbaretitle",
    "seenwork",
    "seenprimaryauthor",
    "singletitle",
    "uniquetitle",
    "uniquebaretitle",
    "uniquework",
    "uniqueprimaryauthor",
    "sortkey",
    "presort",
    "options",
];

/// Known BibTeX field ordering (approximate). Fields not in this list
/// are appended in alphabetical order after the known fields.
const KNOWN_FIELDS: &[&str] = &[
    "abstract",
    "address",
    "annotation",
    "author",
    "booktitle",
    "chapter",
    "crossref",
    "doi",
    "edition",
    "editor",
    "eprint",
    "howpublished",
    "institution",
    "isbn",
    "issn",
    "journal",
    "journaltitle",
    "key",
    "keywords",
    "language",
    "location",
    "month",
    "note",
    "number",
    "organization",
    "pages",
    "pagetotal",
    "publisher",
    "school",
    "series",
    "shorthand",
    "shorttitle",
    "subtitle",
    "title",
    "translator",
    "type",
    "url",
    "urldate",
    "venue",
    "version",
    "volume",
    "volumes",
    "year",
];

/// Format a single name in BibTeX format.
///
/// Produces the standard `{given} {prefix} {family}` format, or
/// `{family}, {prefix} {given}` when the name has a "family" part
/// and the "given" part is present (to avoid ambiguity).
fn format_name(name: &biber_core::name::Name) -> String {
    let family = name.get_namepart("family").unwrap_or("");
    let given = name.get_namepart("given").unwrap_or("");
    let prefix = name.get_namepart("prefix").unwrap_or("");
    let suffix = name.get_namepart("suffix").unwrap_or("");

    // If we have parsed nameparts, use them.
    // If there's only a family name (no given), just return it.
    // If there's both, format as "family, given" only when needed.
    let has_given = !given.is_empty();
    let has_prefix = !prefix.is_empty();
    let has_suffix = !suffix.is_empty();
    let has_family = !family.is_empty();

    if !has_family && !has_given {
        return String::new();
    }

    // If only raw string is available (no nameparts parsed), return raw
    if !has_family && !has_given && !has_prefix {
        return name.rawstring.clone();
    }

    let mut out = String::new();

    if has_given {
        out.push_str(given);
        if has_prefix {
            out.push(' ');
            out.push_str(prefix);
        }
        if has_family {
            out.push(' ');
            out.push_str(family);
        }
    } else if has_family {
        out.push_str(family);
    }

    if has_suffix {
        out.push_str(", ");
        out.push_str(suffix);
    }

    if out.is_empty() {
        name.rawstring.clone()
    } else {
        out
    }
}

/// Format a name list (e.g. author field) as a BibTeX string.
fn format_names(names: &biber_core::name::Names) -> String {
    let parts: Vec<String> = names.iter().map(format_name).collect();
    parts.join(" and ")
}

/// Format a single field value in BibTeX format.
fn format_field_value(value: &ConfigValue) -> String {
    match value {
        ConfigValue::Str(s) => {
            // Escape special characters and wrap in braces
            let escaped = s
                .replace('\\', "\\\\")
                .replace('{', "\\{")
                .replace('}', "\\}");
            format!("{{{}}}", escaped)
        }
        ConfigValue::List(list) => {
            let items: Vec<String> = list
                .iter()
                .map(|v| match v {
                    ConfigValue::Str(s) => s.clone(),
                    other => format_field_value(other),
                })
                .collect();
            let joined = items.join(" and ");
            format!("{{{}}}", joined)
        }
        ConfigValue::Map(_) => String::new(),
        ConfigValue::Raw(s) => s.clone(),
    }
}

/// Get a field's value for output, handling name fields specially.
fn get_field_value(entry: &Entry, field_name: &str) -> Option<String> {
    // Check if this is a name field
    if entry.names.contains_key(field_name) {
        if let Some(names) = entry.names.get(field_name) {
            if names.count() > 0 {
                return Some(format_names(names));
            }
        }
    }

    // Fallback to raw field value
    entry.get_field(field_name).map(format_field_value)
}

/// Determine if a field should be included in the output.
fn is_output_field(field_name: &str) -> bool {
    if INTERNAL_FIELDS.contains(&field_name) {
        return false;
    }
    // Filter out per-field hash names (e.g. authornamehash, editorfullhashraw)
    if field_name.ends_with("namehash")
        || field_name.ends_with("fullhash")
        || field_name.ends_with("fullhashraw")
        || field_name.ends_with("bibnamehash")
    {
        return false;
    }
    true
}

/// Write all entries from the tool-mode section (99999) as BibTeX.
///
/// Returns the full .bib file content as a string.
pub fn write_bib(biber: &Biber) -> String {
    let mut output = String::new();

    // Determine safechars encoding parameters
    let safechars = biber
        .config
        .getoption_str("output_safechars")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let encode_set: Option<RecodeSet> = if safechars {
        Some(
            biber
                .config
                .getoption_str("output_safecharsset")
                .map(|s| match s {
                    "null" => RecodeSet::Null,
                    "full" => RecodeSet::Full,
                    _ => RecodeSet::Base,
                })
                .unwrap_or(RecodeSet::Base),
        )
    } else {
        None
    };

    // Find the tool-mode section (99999).
    // Also accept section 0 as fallback for testing.
    let section = biber
        .sections
        .get_section(99999)
        .or_else(|| biber.sections.get_section(0));

    let section = match section {
        Some(s) => s,
        None => return output,
    };

    // Get entry order from the first datalist, or from the section's bibentries.
    let entries_order: Vec<String> = {
        let lists = biber.datalists.get_lists_for_section(section.number);
        if let Some(list) = lists.first() {
            if !list.state.entries.is_empty() {
                list.state.entries.clone()
            } else {
                // Fallback: all bibentries in insertion order
                section
                    .bibentries
                    .citekeys()
                    .map(|s| s.to_string())
                    .collect()
            }
        } else {
            section
                .bibentries
                .citekeys()
                .map(|s| s.to_string())
                .collect()
        }
    };

    // Write each entry in order
    for citekey in &entries_order {
        let entry = match section.bibentry(citekey) {
            Some(e) => e,
            None => continue,
        };

        write_entry(&mut output, entry);
    }

    // Apply safechars encoding to entire output
    if let Some(set) = encode_set {
        output = latex_encode_with_set(&output, set);
    }

    output
}

/// Write a single entry in BibTeX format.
fn write_entry(output: &mut String, entry: &Entry) {
    let entrytype = if entry.entrytype.is_empty() {
        "misc"
    } else {
        &entry.entrytype
    };

    output.push('@');
    output.push_str(entrytype);
    output.push('{');
    output.push_str(&entry.citekey);
    output.push_str(",\n");

    // Collect fields to output, sorted by KNOWN_FIELDS order then alphabetically
    let mut output_fields: Vec<&str> = entry.field_names().filter(|n| is_output_field(n)).collect();

    // Sort: known fields first (in order), then alphabetical
    output_fields.sort_by(|a, b| {
        let a_pos = KNOWN_FIELDS.iter().position(|k| *k == *a);
        let b_pos = KNOWN_FIELDS.iter().position(|k| *k == *b);
        match (a_pos, b_pos) {
            (Some(pa), Some(pb)) => pa.cmp(&pb),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.cmp(b),
        }
    });

    // Compute field name width for alignment
    let max_width = output_fields.iter().map(|f| f.len()).max().unwrap_or(0);
    let align_width = max_width + 2; // +2 for the " = " separator

    for field_name in &output_fields {
        let value_str = match get_field_value(entry, field_name) {
            Some(v) => v,
            None => continue,
        };

        if value_str.is_empty() {
            continue;
        }

        output.push_str("  ");
        output.push_str(field_name);
        // Pad to alignment width
        let padding = align_width.saturating_sub(field_name.len());
        for _ in 0..padding {
            output.push(' ');
        }
        output.push_str("= ");
        output.push_str(&value_str);
        output.push_str(",\n");
    }

    output.push_str("}\n\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use biber_core::datalist::DataList;
    use biber_core::entry::Entry;
    use biber_core::name::{Name, Names};
    use biber_core::processor::Biber;
    use biber_core::section::Section;

    fn make_biber_with_entry(citekey: &str, entrytype: &str, fields: Vec<(&str, &str)>) -> Biber {
        let mut biber = Biber::new();
        let mut section = Section::new(99999);
        section.set_allkeys(true);

        let mut entry = Entry::new(citekey, entrytype);
        for (k, v) in fields {
            entry.set_field_str(k, v);
        }
        section.bibentries.add_entry(entry);
        section.add_citekeys(vec![citekey.to_string()]);
        biber.sections.add_section(section);

        // Add a datalist with the entry in order
        let mut dl = DataList::new(
            99999,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        dl.state.entries = vec![citekey.to_string()];
        biber.datalists.add_list(dl);

        biber
    }

    #[test]
    fn empty_biber_returns_empty_string() {
        let biber = Biber::new();
        let result = write_bib(&biber);
        assert!(result.is_empty());
    }

    #[test]
    fn simple_entry() {
        let biber = make_biber_with_entry(
            "smith2020",
            "book",
            vec![
                ("author", "John Smith"),
                ("title", "A Book"),
                ("year", "2020"),
            ],
        );
        let result = write_bib(&biber);
        assert!(result.contains("@book{smith2020,"));
        assert!(result.contains("title"));
        assert!(result.contains("A Book"));
        assert!(result.contains("2020"));
    }

    #[test]
    fn internal_fields_are_filtered() {
        let mut biber = make_biber_with_entry(
            "test1",
            "article",
            vec![("title", "Test"), ("year", "2021")],
        );
        // Add internal fields
        let section = biber.sections.get_section_mut(99999).unwrap();
        let entry = section.bibentries.get_entry_mut("test1").unwrap();
        entry.set_field_str("citekey", "test1");
        entry.set_field_str("entrytype", "article");
        entry.set_field_str("namehash", "abc123");

        let result = write_bib(&biber);
        // Internal fields should not appear
        assert!(!result.contains("namehash"));
        assert!(result.contains("title"));
    }

    #[test]
    fn entry_type_is_preserved() {
        let biber = make_biber_with_entry("misc1", "misc", vec![("note", "A note")]);
        let result = write_bib(&biber);
        assert!(result.contains("@misc{misc1,"));
    }

    #[test]
    fn name_field_is_formatted() {
        let mut biber = Biber::new();
        let mut section = Section::new(99999);
        section.set_allkeys(true);

        let mut entry = Entry::new("named", "book");
        entry.set_field_str("author", "John Doe");
        entry.set_field_str("title", "Named Book");
        entry.set_field_str("year", "2022");

        // Parse the name
        let mut names = Names::new();
        let mut name = Name::new();
        name.set_namepart("family", "Doe");
        name.set_namepart("given", "John");
        names.add_name(name);
        entry.names.insert("author".into(), names);

        section.bibentries.add_entry(entry);
        section.add_citekeys(vec!["named".to_string()]);
        biber.sections.add_section(section);

        let mut dl = DataList::new(
            99999,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        dl.state.entries = vec!["named".to_string()];
        biber.datalists.add_list(dl);

        let result = write_bib(&biber);
        assert!(result.contains("John Doe"));
        assert!(result.contains("author"));
    }

    #[test]
    fn multiple_entries_are_output_in_order() {
        let mut biber = Biber::new();
        let mut section = Section::new(99999);
        section.set_allkeys(true);

        let e1 = {
            let mut e = Entry::new("first", "book");
            e.set_field_str("title", "First");
            e
        };
        let e2 = {
            let mut e = Entry::new("second", "article");
            e.set_field_str("title", "Second");
            e
        };
        section.bibentries.add_entry(e1);
        section.bibentries.add_entry(e2);
        biber.sections.add_section(section);

        let mut dl = DataList::new(
            99999,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        dl.state.entries = vec!["first".to_string(), "second".to_string()];
        biber.datalists.add_list(dl);

        let result = write_bib(&biber);
        let first_pos = result.find("@book{first,").unwrap();
        let second_pos = result.find("@article{second,").unwrap();
        assert!(first_pos < second_pos);
    }

    #[test]
    fn safechars_encodes_unicode_in_bibtex() {
        let mut biber = make_biber_with_entry(
            "muller2020",
            "book",
            vec![
                ("author", "M\u{fc}ller"),
                ("title", "A \u{fc}ber Title"),
                ("year", "2020"),
            ],
        );
        biber.config.setoption_str("output_safechars", "1");
        biber.config.setoption_str("output_safecharsset", "base");

        let result = write_bib(&biber);
        // The ü should be encoded to a LaTeX macro
        assert!(
            result.contains("\\\"{u}") || result.contains("\\\"u"),
            "Expected ü to be encoded to LaTeX macro, got: {}",
            result
        );
    }
}
