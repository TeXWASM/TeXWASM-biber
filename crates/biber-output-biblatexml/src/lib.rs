//! biblatexml (.bltxml) output writer for tool mode.
//!
//! Emits entries in the biblatexml XML format
//! (`http://biblatex-biber.sourceforge.net/biblatexml`).

#![forbid(unsafe_code)]

use biber_core::config::ConfigValue;
use biber_core::entry::Entry;
use biber_core::processor::Biber;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Writer;

const NS: &str = "http://biblatex-biber.sourceforge.net/biblatexml";
const PREFIX: &str = "bltx";

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

fn is_output_field(field_name: &str) -> bool {
    if INTERNAL_FIELDS.contains(&field_name) {
        return false;
    }
    if field_name.ends_with("namehash")
        || field_name.ends_with("fullhash")
        || field_name.ends_with("fullhashraw")
        || field_name.ends_with("bibnamehash")
    {
        return false;
    }
    true
}

/// Write all entries from the tool-mode section as biblatexml.
pub fn write_bltxml(biber: &Biber) -> String {
    let mut buf = Vec::new();

    // schema PI (written before the writer to avoid escaping issues)
    let emit_schema_pi = biber.config.getoption_str("no_bltxml_schema") != Some("1");

    let mut writer = Writer::new_with_indent(&mut buf, b' ', 2);

    // XML declaration
    let _ = writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)));

    // Root element
    let root = BytesStart::new(format!("{}:entries", PREFIX)).with_attributes([("xmlns:bltx", NS)]);
    let _ = writer.write_event(Event::Start(root));

    let section = biber
        .sections
        .get_section(99999)
        .or_else(|| biber.sections.get_section(0));

    let section = match section {
        Some(s) => s,
        None => {
            let _ = writer.write_event(Event::End(BytesEnd::new(format!("{}:entries", PREFIX))));
            let mut out = String::from_utf8(buf).unwrap_or_default();
            maybe_insert_schema_pi(&mut out, emit_schema_pi);
            return out;
        }
    };

    // Get entry order from datalist
    let entries_order: Vec<String> = {
        let lists = biber.datalists.get_lists_for_section(section.number);
        if let Some(list) = lists.first() {
            if !list.state.entries.is_empty() {
                list.state.entries.clone()
            } else {
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

    for citekey in &entries_order {
        let entry = match section.bibentry(citekey) {
            Some(e) => e,
            None => continue,
        };
        write_entry_xml(&mut writer, entry);
    }

    let _ = writer.write_event(Event::End(BytesEnd::new(format!("{}:entries", PREFIX))));

    let mut out = String::from_utf8(buf).unwrap_or_default();
    maybe_insert_schema_pi(&mut out, emit_schema_pi);
    out
}

fn write_entry_xml<W: std::io::Write>(writer: &mut Writer<W>, entry: &Entry) {
    let entrytype = if entry.entrytype.is_empty() {
        "misc"
    } else {
        &entry.entrytype
    };

    let mut elem = BytesStart::new(format!("{}:entry", PREFIX));
    elem.push_attribute(("id", entry.citekey.as_str()));
    elem.push_attribute(("entrytype", entrytype));
    let _ = writer.write_event(Event::Start(elem));

    // Collect all field names (regular fields + name fields)
    let mut output_fields: Vec<&str> = entry.field_names().filter(|n| is_output_field(n)).collect();
    for name in entry.names.keys() {
        if is_output_field(name) && !output_fields.contains(&name.as_str()) {
            output_fields.push(name);
        }
    }
    output_fields.sort();

    // Write name fields first (they have special XML structure)
    for field_name in &output_fields {
        if entry.names.contains_key(*field_name) {
            if let Some(names) = entry.names.get(*field_name) {
                if names.count() > 0 {
                    write_names_field(writer, field_name, names);
                }
            }
        }
    }

    // Write literal fields (non-name fields)
    for field_name in &output_fields {
        if entry.names.contains_key(*field_name) {
            continue;
        }
        if let Some(value) = entry.get_field(field_name) {
            write_literal_field(writer, field_name, value);
        }
    }

    let _ = writer.write_event(Event::End(BytesEnd::new(format!("{}:entry", PREFIX))));
}

fn write_names_field<W: std::io::Write>(
    writer: &mut Writer<W>,
    field_name: &str,
    names: &biber_core::name::Names,
) {
    let mut elem = BytesStart::new(format!("{}:names", PREFIX));
    elem.push_attribute(("type", field_name));
    if names.count() > 1 {
        elem.push_attribute(("morenames", "1"));
    }
    let _ = writer.write_event(Event::Start(elem));

    for name in names.iter() {
        let name_elem = BytesStart::new(format!("{}:name", PREFIX));
        let _ = writer.write_event(Event::Start(name_elem));

        let namepart_types = ["given", "family", "prefix", "suffix"];
        for np_type in &namepart_types {
            if let Some(val) = name.get_namepart(np_type) {
                if !val.is_empty() {
                    let mut np_elem = BytesStart::new(format!("{}:namepart", PREFIX));
                    np_elem.push_attribute(("type", *np_type));
                    let _ = writer.write_event(Event::Start(np_elem));
                    let _ = writer.write_event(Event::Text(BytesText::new(val)));
                    let _ = writer
                        .write_event(Event::End(BytesEnd::new(format!("{}:namepart", PREFIX))));
                }
            }
        }

        let _ = writer.write_event(Event::End(BytesEnd::new(format!("{}:name", PREFIX))));
    }

    let _ = writer.write_event(Event::End(BytesEnd::new(format!("{}:names", PREFIX))));
}

fn write_literal_field<W: std::io::Write>(
    writer: &mut Writer<W>,
    field_name: &str,
    value: &ConfigValue,
) {
    let text = match value {
        ConfigValue::Str(s) => s.trim().to_string(),
        ConfigValue::List(list) => {
            let items: Vec<String> = list
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            items.join(" and ")
        }
        ConfigValue::Map(_) => return,
        ConfigValue::Raw(s) => s.trim().to_string(),
    };

    if text.is_empty() {
        return;
    }

    let elem = BytesStart::new(format!("{}:{}", PREFIX, field_name));
    let _ = writer.write_event(Event::Start(elem));
    let _ = writer.write_event(Event::Text(BytesText::new(&text)));
    let _ = writer.write_event(Event::End(BytesEnd::new(format!(
        "{}:{}",
        PREFIX, field_name
    ))));
}

/// Insert a biblatexml schema PI after the XML declaration, if enabled.
fn maybe_insert_schema_pi(out: &mut String, emit: bool) {
    if emit {
        let pi = "<?xml-model href=\"biblatexml.rng\" type=\"application/xml\" schematypens=\"http://relaxng.org/ns/structure/1.0\"?>\n";
        if let Some(pos) = out.find("?>") {
            let insert_pos = pos + 2;
            out.insert_str(insert_pos, pi);
        }
    }
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
    fn empty_biber_returns_empty_xml() {
        let biber = Biber::new();
        let result = write_bltxml(&biber);
        assert!(result.contains("<bltx:entries"));
        assert!(result.contains("</bltx:entries>"));
    }

    #[test]
    fn simple_entry() {
        let biber = make_biber_with_entry(
            "smith2020",
            "book",
            vec![("title", "A Book"), ("year", "2020")],
        );
        let result = write_bltxml(&biber);
        assert!(result.contains(r#"<bltx:entry id="smith2020" entrytype="book">"#));
        assert!(result.contains("<bltx:title>A Book</bltx:title>"));
        assert!(result.contains("<bltx:year>2020</bltx:year>"));
    }

    #[test]
    fn name_field_is_formatted() {
        let mut biber = Biber::new();
        let mut section = Section::new(99999);
        section.set_allkeys(true);
        let mut entry = Entry::new("named", "book");
        entry.set_field_str("title", "Named Book");
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

        let result = write_bltxml(&biber);
        assert!(result.contains(r#"<bltx:names type="author">"#));
        assert!(result.contains("<bltx:namepart type=\"given\">John</bltx:namepart>"));
        assert!(result.contains("<bltx:namepart type=\"family\">Doe</bltx:namepart>"));
    }

    #[test]
    fn internal_fields_filtered() {
        let mut biber = make_biber_with_entry("test1", "article", vec![("title", "Test")]);
        let section = biber.sections.get_section_mut(99999).unwrap();
        let entry = section.bibentries.get_entry_mut("test1").unwrap();
        entry.set_field_str("namehash", "abc123");
        let result = write_bltxml(&biber);
        assert!(!result.contains("namehash"));
        assert!(result.contains("<bltx:title>Test</bltx:title>"));
    }

    #[test]
    fn xml_model_pi_emitted_by_default() {
        let biber = Biber::new();
        let result = write_bltxml(&biber);
        assert!(result.contains("<?xml-model"));
        assert!(result.contains("biblatexml.rng"));
        assert!(result.contains("schematypens"));
    }

    #[test]
    fn xml_model_pi_suppressed_by_no_bltxml_schema() {
        let mut biber = Biber::new();
        biber.config.setoption_str("no_bltxml_schema", "1");
        let result = write_bltxml(&biber);
        assert!(!result.contains("<?xml-model"));
    }
}
