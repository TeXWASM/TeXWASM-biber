//! biber.conf XML parser.
//!
//! Reads `biber.conf` (root `<config>`) and applies its settings to a
//! [`Config`] struct. Simple elements (e.g. `<mincrossrefs>5</mincrossrefs>`)
//! are set as biber options. Complex structures (`sourcemap`, `inheritance`,
//! `datamodel`) are stored as [`ConfigValue::Raw`]. Option scopes and
//! sorting templates are parsed into their structured representations.

use roxmltree::Document;

use crate::config::Config;
use crate::constants::OptionScope;

/// Parse a `biber.conf` XML string and apply all settings to `config`.
pub fn parse_biber_config(xml: &str, config: &mut Config) -> Result<(), String> {
    let doc = Document::parse(xml).map_err(|e| format!("XML parse error: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "config" {
        return Err("root element must be <config>".into());
    }
    for node in root.children() {
        if !node.is_element() {
            continue;
        }
        parse_config_element(node, config);
    }
    Ok(())
}

fn parse_config_element(node: roxmltree::Node, config: &mut Config) {
    let name = node.tag_name().name();
    match name {
        // Simple string-valued options
        "mincrossrefs"
        | "minxrefs"
        | "input_encoding"
        | "output_encoding"
        | "output_format"
        | "output_fieldcase"
        | "output_indent"
        | "output_listsep"
        | "output_namesep"
        | "output_annotation_marker"
        | "output_named_annotation_marker"
        | "output_xdatamarker"
        | "output_xdatasep"
        | "output_xnamesep"
        | "annotation_marker"
        | "named_annotation_marker"
        | "xdatamarker"
        | "xdatasep"
        | "xnamesep"
        | "xsvsep"
        | "listsep"
        | "namesep"
        | "others_string"
        | "decodecharsset"
        | "output_safecharsset"
        | "sortlocale"
        | "sortingnamekeytemplatename"
        | "labelalphanametemplatename"
        | "labelalphatemplatename"
        | "uniquenametemplatename"
        | "namehashtemplatename"
        | "collate_options"
        | "logfile"
        | "output_directory"
        | "output_file"
        | "tool_config"
        | "dot_include"
        | "input_format"
        | "mssplit"
        | "recodedata"
        | "sortingnamekeytemplate"
        | "wraplines"
            if !name.is_empty() =>
        {
            let text = node.text().unwrap_or("").trim().to_string();
            if !text.is_empty() {
                config.setoption_str(name, &text);
                config.mark_explicit(name);
            }
        }

        // Boolean options ("0" | "1")
        "debug"
        | "trace"
        | "nolog"
        | "quiet"
        | "sortcase"
        | "sortupper"
        | "tool"
        | "clrmacros"
        | "nostdmacros"
        | "dieondatamodel"
        | "nodieonerror"
        | "noskipduplicates"
        | "no_default_datamodel"
        | "validate_datamodel"
        | "validate_control"
        | "validate_config"
        | "validate_bblxml"
        | "validate_bltxml"
        | "no_bblxml_schema"
        | "no_bltxml_schema"
        | "collate"
        | "convert_control"
        | "fastsort"
        | "fixinits"
        | "onlylog"
        | "output_align"
        | "output_all_macrodefs"
        | "output_legacy_dates"
        | "output_no_macrodefs"
        | "output_resolve_xdata"
        | "output_resolve_crossrefs"
        | "output_resolve_sets"
        | "output_safechars"
        | "output_xname"
        | "ssl-nointernalca"
        | "ssl-noverify-host"
        | "xname" => {
            let val = node.text().unwrap_or("").trim();
            match val {
                "1" | "true" | "yes" => {
                    config.setoption_str(name, "1");
                    config.mark_explicit(name);
                }
                "0" | "false" | "no" => {
                    config.setoption_str(name, "0");
                    config.mark_explicit(name);
                }
                _ => {} // ignore invalid values
            }
        }

        // Option scopes (register scope metadata)
        "optionscope" => {
            parse_optionscope(node, config);
        }

        // Sorting template (stored as ConfigValue::Map)
        "sortingtemplate" => {
            parse_sortingtemplate(node, config);
        }

        // Sourcemap → stored as raw XML for later processing
        "sourcemap" => {
            let raw = node_to_string(node);
            config.setoption("sourcemap", crate::config::ConfigValue::Raw(raw));
            config.mark_explicit("sourcemap");
        }

        // Inheritance → stored as raw XML
        "inheritance" => {
            let raw = node_to_string(node);
            config.setblxoption(None, "inheritance", crate::config::ConfigValue::Raw(raw));
        }

        // Datamodel → stored as raw XML
        "datamodel" => {
            let raw = node_to_string(node);
            config.setblxoption(None, "datamodel", crate::config::ConfigValue::Raw(raw));
        }

        // Transliteration → store as structured rules
        "transliteration" => {
            let entrytype = node.attribute("entrytype").unwrap_or("*");
            for child in node.children() {
                if !child.is_element() || child.tag_name().name() != "translit" {
                    continue;
                }
                let target = match child.attribute("target") {
                    Some(t) => t,
                    None => continue,
                };
                let from = match child.attribute("from") {
                    Some(f) => f,
                    None => continue,
                };
                let to = match child.attribute("to") {
                    Some(t) => t,
                    None => continue,
                };
                let mut m = std::collections::BTreeMap::new();
                m.insert(
                    "entrytype".into(),
                    crate::config::ConfigValue::Str(entrytype.to_string()),
                );
                if let Some(langids) = child.attribute("langids") {
                    m.insert(
                        "langids".into(),
                        crate::config::ConfigValue::Str(langids.to_string()),
                    );
                }
                m.insert(
                    "target".into(),
                    crate::config::ConfigValue::Str(target.to_string()),
                );
                m.insert(
                    "from".into(),
                    crate::config::ConfigValue::Str(from.to_string()),
                );
                m.insert("to".into(), crate::config::ConfigValue::Str(to.to_string()));

                if entrytype == "*" {
                    let existing = config.getblxoption(None, "translit");
                    let mut merged: Vec<crate::config::ConfigValue> = match existing {
                        Some(crate::config::ConfigValue::List(list)) => list.clone(),
                        _ => Vec::new(),
                    };
                    merged.push(crate::config::ConfigValue::Map(m));
                    config.setblxoption(None, "translit", crate::config::ConfigValue::List(merged));
                } else {
                    let existing = config.getblxoption_entrytype(entrytype, "translit");
                    let mut merged: Vec<crate::config::ConfigValue> = match existing {
                        Some(crate::config::ConfigValue::List(list)) => list.clone(),
                        _ => Vec::new(),
                    };
                    merged.push(crate::config::ConfigValue::Map(m));
                    config.setblxoption_entrytype(
                        None,
                        "translit",
                        crate::config::ConfigValue::List(merged),
                        entrytype,
                    );
                }
            }
            config.mark_explicit("transliteration");
        }

        // Noinit / Nolabel / Nosort regex patterns → stored as raw XML
        "noinits" | "nolabels" | "nosort" => {
            let raw = node_to_string(node);
            config.setoption(name, crate::config::ConfigValue::Raw(raw));
            config.mark_explicit(name);
        }

        // Nonamestring → stored as raw XML
        "nonamestring" => {
            let raw = node_to_string(node);
            config.setoption(name, crate::config::ConfigValue::Raw(raw));
            config.mark_explicit(name);
        }

        // Datafield sets
        "datafieldset" => {
            parse_datafieldset(node, config);
        }

        // Uniquename template → stored as raw XML
        "uniquenametemplate" => {
            let raw = node_to_string(node);
            config.setblxoption(
                None,
                "uniquenametemplate",
                crate::config::ConfigValue::Raw(raw),
            );
        }

        // Labelalpha/Name templates → structured parsing for labelalphatemplate
        "labelalphatemplate" => {
            parse_labelalphatemplate_config(node, config);
        }
        "labelalphanametemplate" | "namehashtemplate" | "sortingnamekeytemplate" => {
            let raw = node_to_string(node);
            config.setblxoption(None, name, crate::config::ConfigValue::Raw(raw));
        }

        // Presort with optional type attribute
        "presort" => {
            let text = node.text().unwrap_or("").trim().to_string();
            if !text.is_empty() {
                if let Some(typ) = node.attribute("type") {
                    config.setoption_str(format!("presort_{}", typ), &text);
                } else {
                    config.setoption_str("presort", &text);
                }
                config.mark_explicit("presort");
            }
        }

        // Sort exclusion/inclusion → raw XML
        "sortexclusion" | "sortinclusion" => {
            let raw = node_to_string(node);
            config.setoption(name, crate::config::ConfigValue::Raw(raw));
            config.mark_explicit(name);
        }

        // Unknown elements: store as raw biber option
        _ => {
            let text = node.text().unwrap_or("").trim().to_string();
            if !text.is_empty() {
                config.setoption_str(name, &text);
                config.mark_explicit(name);
            }
        }
    }
}

/// Parse an `<optionscope>` element and register option scope metadata.
fn parse_optionscope(node: roxmltree::Node, config: &mut Config) {
    let scope_str = node.attribute("type").unwrap_or("");
    let scope = match scope_str {
        "GLOBAL" => OptionScope::Global,
        "ENTRYTYPE" => OptionScope::Entrytype,
        "ENTRY" => OptionScope::Entry,
        "NAMELIST" => OptionScope::Namelist,
        "NAME" => OptionScope::Name,
        _ => return,
    };

    for opt in node.children() {
        if !opt.is_element() || opt.tag_name().name() != "option" {
            continue;
        }
        let opt_name = opt.text().unwrap_or("").trim();
        if opt_name.is_empty() {
            continue;
        }
        let datatype = opt.attribute("datatype").unwrap_or("string").to_lowercase();
        let output = opt
            .attribute("backendout")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let input = opt.attribute("backendin").map(|s| s.to_string());

        config.add_optionscope(scope, opt_name, &datatype, output, input);
    }
}

/// Parse a `<sortingtemplate>` element into a `ConfigValue::Map`.
///
/// Matches the format produced by the BCF reader:
/// ```text
/// { "templatename" → Map { "sort_1_1" → "fieldname", ... } }
/// ```
fn parse_sortingtemplate(node: roxmltree::Node, config: &mut Config) {
    let name = node.attribute("name").unwrap_or("tool");
    use std::collections::BTreeMap;

    let mut sort_map = BTreeMap::new();

    for sort in node.children() {
        if !sort.is_element() || sort.tag_name().name() != "sort" {
            continue;
        }
        let sort_order = sort
            .attribute("order")
            .and_then(|o| o.parse::<u32>().ok())
            .unwrap_or(1);

        for item in sort.children() {
            if !item.is_element() || item.tag_name().name() != "sortitem" {
                continue;
            }
            let item_order = item
                .attribute("order")
                .and_then(|o| o.parse::<u32>().ok())
                .unwrap_or(1);
            let field = item.text().unwrap_or("").trim().to_string();
            if !field.is_empty() {
                let key = format!("sort_{sort_order}_{item_order}");
                sort_map.insert(key, crate::config::ConfigValue::Str(field));
            }
        }
    }

    if !sort_map.is_empty() {
        let mut templates = BTreeMap::new();
        templates.insert(name.to_string(), crate::config::ConfigValue::Map(sort_map));
        config.setblxoption(
            None,
            "sortingtemplate",
            crate::config::ConfigValue::Map(templates),
        );
    }
}

/// Parse a `<datafieldset>` element.
fn parse_datafieldset(node: roxmltree::Node, config: &mut Config) {
    let set_name = node.attribute("name").unwrap_or("").to_lowercase();
    if set_name.is_empty() {
        return;
    }

    for member in node.children() {
        if !member.is_element() || member.tag_name().name() != "member" {
            continue;
        }
        let field = member_child_text(member, "field");
        let fieldtype = member.attribute("fieldtype").map(|s| s.to_string());
        let datatype = member.attribute("datatype").map(|s| s.to_string());

        config.add_datafield_set_member(
            &set_name,
            crate::config::DatafieldSetMember {
                field,
                fieldtype,
                datatype,
            },
        );
    }
}

fn member_child_text(node: roxmltree::Node, name: &str) -> Option<String> {
    for child in node.children() {
        if child.is_element() && child.tag_name().name() == name {
            return Some(child.text().unwrap_or("").trim().to_string());
        }
    }
    None
}

/// Serialise a node back to its string form (including children).
fn node_to_string(node: roxmltree::Node) -> String {
    let mut s = String::new();
    write_node(node, &mut s);
    s
}

fn write_node(node: roxmltree::Node, out: &mut String) {
    let name = node.tag_name().name();
    out.push('<');
    out.push_str(name);

    for attr in node.attributes() {
        out.push(' ');
        out.push_str(attr.name());
        out.push_str("=\"");
        out.push_str(attr.value());
        out.push('"');
    }

    // Check if it has children or text
    let has_children = node.children().any(|c| c.is_element());
    let text = node.text().unwrap_or("");

    if has_children {
        out.push('>');
        for child in node.children() {
            match child.node_type() {
                roxmltree::NodeType::Element => write_node(child, out),
                roxmltree::NodeType::Text => out.push_str(child.text().unwrap_or("")),
                _ => {}
            }
        }
        out.push_str("</");
        out.push_str(name);
        out.push('>');
    } else if !text.is_empty() {
        out.push('>');
        out.push_str(text);
        out.push_str("</");
        out.push_str(name);
        out.push('>');
    } else {
        out.push_str("/>");
    }
}

/// Parse a `<labelalphatemplate>` node from biber.conf into structured `ConfigValue::Map`.
fn parse_labelalphatemplate_config(node: roxmltree::Node, config: &mut crate::config::Config) {
    let latype = node.attribute("type").unwrap_or("global").to_string();
    let mut elements: Vec<crate::config::ConfigValue> = Vec::new();
    for elem in node.children().filter(|n| n.has_tag_name("labelelement")) {
        let order = elem
            .attribute("order")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let mut parts: Vec<crate::config::ConfigValue> = Vec::new();
        for part in elem.children().filter(|n| n.has_tag_name("labelpart")) {
            let mut pm = std::collections::BTreeMap::new();
            pm.insert(
                "content".into(),
                crate::config::ConfigValue::Str(part.text().unwrap_or("").trim().to_string()),
            );
            for attr_name in &[
                "final",
                "substring_width",
                "substring_side",
                "substring_width_max",
                "substring_fixed_threshold",
                "pad_char",
                "pad_side",
                "ifnames",
                "names",
                "namessep",
                "noalphaothers",
                "uppercase",
                "lowercase",
            ] {
                if let Some(val) = part.attribute(*attr_name) {
                    pm.insert(
                        (*attr_name).into(),
                        crate::config::ConfigValue::Str(val.into()),
                    );
                }
            }
            parts.push(crate::config::ConfigValue::Map(pm));
        }
        let mut em = std::collections::BTreeMap::new();
        em.insert(
            "order".into(),
            crate::config::ConfigValue::Str(order.to_string()),
        );
        em.insert("parts".into(), crate::config::ConfigValue::List(parts));
        elements.push(crate::config::ConfigValue::Map(em));
    }
    let mut template_map = std::collections::BTreeMap::new();
    template_map.insert(latype, crate::config::ConfigValue::List(elements));
    config.setblxoption(
        None,
        "labelalphatemplate",
        crate::config::ConfigValue::Map(template_map),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn parse_minimal_config() {
        let xml = r#"<config><mincrossrefs>5</mincrossrefs></config>"#;
        let mut config = Config::new();
        parse_biber_config(xml, &mut config).unwrap();
        assert_eq!(config.getoption_str("mincrossrefs"), Some("5"));
        assert!(config.isexplicitoption("mincrossrefs"));
    }

    #[test]
    fn parse_boolean_options() {
        let xml = r#"<config><debug>1</debug><trace>true</trace><sortcase>0</sortcase></config>"#;
        let mut config = Config::new();
        parse_biber_config(xml, &mut config).unwrap();
        assert_eq!(config.getoption_str("debug"), Some("1"));
        assert_eq!(config.getoption_str("trace"), Some("1"));
        assert_eq!(config.getoption_str("sortcase"), Some("0"));
    }

    #[test]
    fn parse_optionscope() {
        let xml = r#"<config>
            <optionscope type="GLOBAL">
                <option datatype="boolean">debug</option>
                <option datatype="string" backendout="1">sortlocale</option>
            </optionscope>
        </config>"#;
        let mut config = Config::new();
        parse_biber_config(xml, &mut config).unwrap();
        assert!(config.optscope.contains_key("debug"));
        assert!(config
            .optscope
            .get("debug")
            .unwrap()
            .contains(&OptionScope::Global));
        assert_eq!(
            config.opttype.get("debug").map(|s| s.as_str()),
            Some("boolean")
        );
        assert!(config.biblatex_option_meta[&OptionScope::Global].contains_key("sortlocale"));
    }

    #[test]
    fn parse_sortingtemplate() {
        let xml = r#"<config>
            <sortingtemplate name="nty">
                <sort order="1">
                    <sortitem order="1">author</sortitem>
                    <sortitem order="2">year</sortitem>
                </sort>
            </sortingtemplate>
        </config>"#;
        let mut config = Config::new();
        parse_biber_config(xml, &mut config).unwrap();
        let tmpl = config.getblxoption(None, "sortingtemplate");
        assert!(tmpl.is_some());
    }

    #[test]
    fn parse_sourcemap_as_raw() {
        let xml = r#"<config>
            <sourcemap>
                <maps datatype="bibtex">
                    <map>
                        <map_step map_field_source="author" match=".*" map_final="1"/>
                    </map>
                </maps>
            </sourcemap>
        </config>"#;
        let mut config = Config::new();
        parse_biber_config(xml, &mut config).unwrap();
        let sm = config.getoption("sourcemap");
        assert!(sm.is_some());
        assert!(matches!(sm.unwrap(), crate::config::ConfigValue::Raw(_)));
    }

    #[test]
    fn parse_presort_with_type() {
        let xml = r#"<config>
            <presort type="mn">MM</presort>
        </config>"#;
        let mut config = Config::new();
        parse_biber_config(xml, &mut config).unwrap();
        assert_eq!(config.getoption_str("presort_mn"), Some("MM"));
        assert!(config.isexplicitoption("presort"));
    }

    #[test]
    fn missing_root_error() {
        let result = parse_biber_config("<notconfig/>", &mut Config::new());
        assert!(result.is_err());
    }

    #[test]
    fn unknown_element_stored_as_option() {
        let xml = r#"<config><mycustomoption>myvalue</mycustomoption></config>"#;
        let mut config = Config::new();
        parse_biber_config(xml, &mut config).unwrap();
        assert_eq!(config.getoption_str("mycustomoption"), Some("myvalue"));
    }
}
