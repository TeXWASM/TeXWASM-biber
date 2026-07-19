//! BBL XML (.bblxml) output writer.
//!
//! Ported from `lib/Biber/Output/bblxml.pm` (820 lines). Emits the
//! bibliography data as XML in the `https://sourceforge.net/projects/biblatex/bblxml`
//! namespace, following the same section/datalist/entry structure as
//! the `.bbl` writer.

use biber_core::config::ConfigValue;
use biber_core::entry::Entry;
use biber_core::processor::Biber;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Writer;

const NS: &str = "https://sourceforge.net/projects/biblatex/bblxml";
const PREFIX: &str = "bbl";

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

/// Generate the `.bblxml` output from a processed `Biber` struct.
pub fn write_bblxml(biber: &Biber) -> String {
    let mut buf = Vec::new();
    let mut writer = Writer::new_with_indent(&mut buf, b' ', 2);

    let _ = writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)));

    let root =
        BytesStart::new(format!("{}:refsections", PREFIX)).with_attributes([("xmlns:bbl", NS)]);
    let _ = writer.write_event(Event::Start(root));

    for section in biber.sections.get_sections() {
        let secnum = section.number;
        let sec_start = BytesStart::new(format!("{}:refsection", PREFIX))
            .with_attributes([("id", secnum.to_string().as_str())]);
        let _ = writer.write_event(Event::Start(sec_start));

        let global_ss = biber
            .config
            .getblxoption_str("sortingtemplatename")
            .unwrap_or("nty")
            .to_string();
        let lists = biber.datalists.get_lists_for_section(secnum);
        let mut non_global: Vec<&biber_core::datalist::DataList> = lists
            .iter()
            .copied()
            .filter(|l| !(l.r#type == "entry" && l.sortingtemplatename == global_ss))
            .collect();
        non_global.sort_by(|a, b| a.sortingtemplatename.cmp(&b.sortingtemplatename));
        let global_lists: Vec<&biber_core::datalist::DataList> = lists
            .iter()
            .copied()
            .filter(|l| l.r#type == "entry" && l.sortingtemplatename == global_ss)
            .collect();
        let ordered_lists: Vec<&biber_core::datalist::DataList> =
            non_global.into_iter().chain(global_lists).collect();

        for list in ordered_lists {
            if list.state.entries.is_empty() {
                continue;
            }
            let mut dl_start = BytesStart::new(format!("{}:datalist", PREFIX));
            dl_start.push_attribute(("type", list.r#type.as_str()));
            dl_start.push_attribute(("id", list.name.as_str()));
            let _ = writer.write_event(Event::Start(dl_start));

            for key in &list.state.entries {
                if let Some(be) = section.bibentry(key) {
                    write_entry_xml(&mut writer, be, list, key);
                }
            }

            let _ = writer.write_event(Event::End(BytesEnd::new(format!("{}:datalist", PREFIX))));
        }

        // Keyalias section
        let mut keyalias_written = false;
        for (alias, target) in section.get_citekey_aliases() {
            if !keyalias_written {
                keyalias_written = true;
            }
            let ka = BytesStart::new(format!("{}:keyalias", PREFIX))
                .with_attributes([("key", alias), ("target", target)]);
            let _ = writer.write_event(Event::Empty(ka));
        }

        // Missing keys
        for mk in section.get_undef_citekeys() {
            let miss = BytesStart::new(format!("{}:missing", PREFIX))
                .with_attributes([("key", mk.as_str())]);
            let _ = writer.write_event(Event::Empty(miss));
        }

        let _ = writer.write_event(Event::End(BytesEnd::new(format!("{}:refsection", PREFIX))));
    }

    let _ = writer.write_event(Event::End(BytesEnd::new(format!("{}:refsections", PREFIX))));
    String::from_utf8(buf).unwrap_or_default()
}

fn write_entry_xml<W: std::io::Write>(
    writer: &mut Writer<W>,
    entry: &Entry,
    list: &biber_core::datalist::DataList,
    key: &str,
) {
    let entrytype = if entry.entrytype.is_empty() {
        "misc"
    } else {
        &entry.entrytype
    };

    let mut elem = BytesStart::new(format!("{}:entry", PREFIX));
    elem.push_attribute(("key", entry.citekey.as_str()));
    elem.push_attribute(("type", entrytype));
    let _ = writer.write_event(Event::Start(elem));

    // BDS fields from list state
    if let Some(sortinit) = list.state.sortinit.get(key) {
        field_elem(writer, "sortinit", sortinit);
    }
    if let Some(sortinithash) = list.state.sortinithash.get(key) {
        field_elem(writer, "sortinithash", sortinithash);
    }
    if let Some(labelprefix) = list.state.labelprefix_data.get(key) {
        field_elem(writer, "labelprefix", labelprefix);
    }
    if let Some(labelalpha) = list.state.labelalphadata.get(key) {
        field_elem(writer, "labelalpha", labelalpha);
    }
    if let Some(extratitle) = list.state.extratitledata.get(key) {
        field_elem(writer, "extratitle", extratitle);
    }

    // Collect all field names (regular fields + name fields)
    let mut output_fields: Vec<&str> = Vec::new();
    for name in entry.field_names() {
        if is_output_field(name) {
            output_fields.push(name);
        }
    }
    for name in entry.names.keys() {
        if is_output_field(name) && !output_fields.contains(&name.as_str()) {
            output_fields.push(name);
        }
    }
    output_fields.sort();

    // Write name fields first
    for field_name in &output_fields {
        if entry.names.contains_key(*field_name) {
            if let Some(names) = entry.names.get(*field_name) {
                if names.count() > 0 {
                    write_names_field_xml(writer, field_name, names);
                }
            }
        }
    }

    // Write literal fields
    for field_name in &output_fields {
        if entry.names.contains_key(*field_name) {
            continue;
        }
        if let Some(value) = entry.get_field(field_name) {
            write_literal_field_xml(writer, field_name, value);
        }
    }

    let _ = writer.write_event(Event::End(BytesEnd::new(format!("{}:entry", PREFIX))));
}

fn field_elem<W: std::io::Write>(writer: &mut Writer<W>, name: &str, value: &str) {
    let mut elem = BytesStart::new(format!("{}:field", PREFIX));
    elem.push_attribute(("name", name));
    let _ = writer.write_event(Event::Start(elem));
    let _ = writer.write_event(Event::Text(BytesText::new(value)));
    let _ = writer.write_event(Event::End(BytesEnd::new(format!("{}:field", PREFIX))));
}

fn write_names_field_xml<W: std::io::Write>(
    writer: &mut Writer<W>,
    field_name: &str,
    names: &biber_core::name::Names,
) {
    let mut elem = BytesStart::new(format!("{}:names", PREFIX));
    elem.push_attribute(("type", field_name));
    elem.push_attribute(("count", names.count().to_string().as_str()));
    let _ = writer.write_event(Event::Start(elem));

    for name in names.iter() {
        let name_elem = BytesStart::new(format!("{}:name", PREFIX));
        let _ = writer.write_event(Event::Start(name_elem));

        let namepart_types = ["family", "given", "prefix", "suffix"];
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

fn write_literal_field_xml<W: std::io::Write>(
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

    let mut elem = BytesStart::new(format!("{}:field", PREFIX));
    elem.push_attribute(("name", field_name));
    let _ = writer.write_event(Event::Start(elem));
    let _ = writer.write_event(Event::Text(BytesText::new(&text)));
    let _ = writer.write_event(Event::End(BytesEnd::new(format!("{}:field", PREFIX))));
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
        let mut section = Section::new(0);
        section.set_allkeys(true);
        let mut entry = Entry::new(citekey, entrytype);
        for (k, v) in fields {
            entry.set_field_str(k, v);
        }
        section.bibentries.add_entry(entry);
        section.add_citekeys(vec![citekey.to_string()]);
        biber.sections.add_section(section);
        let mut dl = DataList::new(
            0,
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
        let result = write_bblxml(&biber);
        assert!(result.contains("<bbl:refsections"));
        assert!(result.contains("</bbl:refsections>"));
    }

    #[test]
    fn simple_entry() {
        let biber = make_biber_with_entry("smith2020", "book", vec![("title", "A Book")]);
        let result = write_bblxml(&biber);
        assert!(result.contains(r#"<bbl:entry key="smith2020" type="book">"#));
        assert!(result.contains("<bbl:field name=\"title\">A Book</bbl:field>"));
    }

    #[test]
    fn name_field_is_formatted() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
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
            0,
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

        let result = write_bblxml(&biber);
        assert!(result.contains(r#"<bbl:names type="author" count="1">"#));
        assert!(result.contains("<bbl:namepart type=\"given\">John</bbl:namepart>"));
        assert!(result.contains("<bbl:namepart type=\"family\">Doe</bbl:namepart>"));
    }

    #[test]
    fn bds_fields_appear() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.set_allkeys(true);
        let mut entry = Entry::new("test1", "article");
        entry.set_field_str("title", "Test");
        section.bibentries.add_entry(entry);
        section.add_citekeys(vec!["test1".to_string()]);
        biber.sections.add_section(section);
        let mut dl = DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        dl.state.entries = vec!["test1".to_string()];
        dl.state
            .sortinit
            .insert("test1".to_string(), "Smith".to_string());
        biber.datalists.add_list(dl);

        let result = write_bblxml(&biber);
        assert!(result.contains("<bbl:field name=\"sortinit\">Smith</bbl:field>"));
    }

    #[test]
    fn internal_fields_filtered() {
        let mut biber = make_biber_with_entry("test1", "article", vec![("title", "Test")]);
        let section = biber.sections.get_section_mut(0).unwrap();
        let entry = section.bibentries.get_entry_mut("test1").unwrap();
        entry.set_field_str("namehash", "abc123");
        let result = write_bblxml(&biber);
        assert!(!result.contains("namehash"));
        assert!(result.contains("<bbl:field name=\"title\">Test</bbl:field>"));
    }

    #[test]
    fn keyalias_and_missing() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.set_allkeys(true);
        let entry = Entry::new("real", "book");
        section.bibentries.add_entry(entry);
        section.add_citekeys(vec!["real".to_string()]);
        section.set_citekey_alias("oldkey", "real");
        section.add_undef_citekey("missing1".to_string());
        biber.sections.add_section(section);
        let mut dl = DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        dl.state.entries = vec!["real".to_string()];
        biber.datalists.add_list(dl);

        let result = write_bblxml(&biber);
        assert!(result.contains(r#"<bbl:keyalias key="oldkey" target="real""#));
        assert!(result.contains(r#"<bbl:missing key="missing1""#));
    }
}
