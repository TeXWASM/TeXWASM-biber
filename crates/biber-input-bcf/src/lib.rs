//! `.bcf` control-file reader.
//!
//! Ported from `Biber::parse_ctrlfile` (`lib/Biber.pm:345`). Parses the
//! XML control file produced by biblatex and populates a [`Biber`] object
//! with sections, datasources, datalists, options, and the data model.
//!
//! Uses `roxmltree` for DOM-based parsing. The BCF files are small
//! (typically < 2000 lines), so DOM is simpler and less error-prone than
//! streaming for this complex, deeply-nested XML.

use biber_core::config::ConfigValue;
use biber_core::constants::{OptionScope, BCF_VERSION};
use biber_core::data_model::{
    ConditionalPart, DataModel, EntryFields, MandatoryField, ModelConstant, ModelConstraint,
    ModelConstraints, ModelField,
};
use biber_core::datalist::{DataList, ListFilter, ListFilterOr};
use biber_core::processor::Biber;
use biber_core::section::{DatasourceRef, Section};
use tracing::{debug, info, warn};

/// Convenience alias for the two-lifetime roxmltree Node.
type Node<'a, 'input> = roxmltree::Node<'a, 'input>;

/// Parse a `.bcf` control file (as UTF-8 string) and return a populated
/// [`Biber`] processor.
///
/// This is the Rust equivalent of `Biber::parse_ctrlfile()`.
pub fn parse_bcf(bcf_xml: &str) -> Result<Biber, ParseError> {
    let doc = roxmltree::Document::parse(bcf_xml).map_err(|e| ParseError::Xml(e.to_string()))?;

    let root = doc.root_element();
    let local = lname(root);
    if local != "controlfile" {
        return Err(ParseError::InvalidRoot(local.to_string()));
    }

    let controlversion = root.attribute("version").unwrap_or("");
    if controlversion != BCF_VERSION {
        warn!(
            "BCF version mismatch: found {}, expected {}",
            controlversion, BCF_VERSION
        );
    }

    let mut biber = Biber::new();
    biber.config.setoption_str("controlversion", controlversion);

    for node in root.children() {
        if !node.is_element() {
            continue;
        }
        let name = lname(node);
        match name {
            "optionscope" => parse_optionscope(node, &mut biber),
            "options" => parse_options(node, &mut biber),
            "datafieldset" => parse_datafieldset(node, &mut biber),
            "sourcemap" => parse_sourcemap(node, &mut biber),
            "labelalphanametemplate" => parse_labelalphanametemplate(node, &mut biber),
            "labelalphatemplate" => parse_labelalphatemplate(node, &mut biber),
            "extradatespec" => parse_extradatespec(node, &mut biber),
            "inheritance" => parse_inheritance(node, &mut biber),
            "noinits" => parse_filter_list(node, &mut biber, "noinit"),
            "nolabels" => parse_filter_list(node, &mut biber, "nolabel"),
            "nolabelwidthcounts" => parse_filter_list(node, &mut biber, "nolabelwidthcount"),
            "nosorts" => parse_filter_list(node, &mut biber, "nosort"),
            "nonamestrings" => parse_filter_list(node, &mut biber, "nonamestring"),
            "uniquenametemplate" => parse_uniquenametemplate(node, &mut biber),
            "namehashtemplate" => parse_namehashtemplate(node, &mut biber),
            "sortingnamekeytemplate" => parse_sortingnamekeytemplate(node, &mut biber),
            "transliteration" => parse_transliteration(node, &mut biber),
            "sortexclusion" => parse_sortex_inclusion(node, &mut biber, true),
            "sortinclusion" => parse_sortex_inclusion(node, &mut biber, false),
            "presort" => parse_presort(node, &mut biber),
            "sortingtemplate" => parse_sortingtemplate(node, &mut biber),
            "datamodel" => parse_datamodel(node, &mut biber),
            "bibdata" => parse_bibdata(node, &mut biber),
            "section" => parse_section(node, &mut biber),
            "datalist" => parse_datalist(node, &mut biber),
            _ => {
                debug!("unhandled BCF element: {}", name);
            }
        }
    }

    ensure_global_datalists(&mut biber);

    info!(
        "Parsed BCF: version {}, {} sections, {} datalists",
        controlversion,
        biber.sections.len(),
        biber.datalists.len()
    );

    Ok(biber)
}

// ---- Helpers ----

fn lname<'a, 'input>(node: Node<'a, 'input>) -> &'a str {
    node.tag_name().name()
}

fn text_content<'a, 'input>(node: Node<'a, 'input>) -> String {
    node.text().unwrap_or("").trim().to_string()
}

fn children_by_name<'a, 'input>(
    node: Node<'a, 'input>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'input>> + 'a {
    node.children()
        .filter(move |c| c.is_element() && c.tag_name().name() == name)
}

fn first_child<'a, 'input>(node: Node<'a, 'input>, name: &'a str) -> Option<Node<'a, 'input>> {
    children_by_name(node, name).next()
}

fn attr<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<&'a str> {
    node.attribute(name)
}

fn node_to_string<'a, 'input>(node: Node<'a, 'input>) -> String {
    let name = lname(node);
    let text = text_content(node);
    format!("<{}>{}</{}>", name, text, name)
}

// ---- Option scope ----

fn parse_optionscope(node: Node, biber: &mut Biber) {
    let scope_str = attr(node, "type").unwrap_or("");
    let scope = match OptionScope::from_bcf_str(scope_str) {
        Some(s) => s,
        None => {
            warn!("unknown optionscope type: {}", scope_str);
            return;
        }
    };

    for opt in children_by_name(node, "option") {
        let opt_name = text_content(opt);
        let datatype = attr(opt, "datatype").unwrap_or("string").to_lowercase();
        let output = attr(opt, "backendout")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let input = attr(opt, "backendin").map(|s| s.to_string());

        biber
            .config
            .add_optionscope(scope, &opt_name, &datatype, output, input);
    }
}

// ---- Options ----

fn parse_options(node: Node, biber: &mut Biber) {
    let component = attr(node, "component").unwrap_or("");
    let opt_type = attr(node, "type").unwrap_or("global");

    for opt in children_by_name(node, "option") {
        let opt_type_attr = attr(opt, "type").unwrap_or("singlevalued");
        let key_node = first_child(opt, "key");
        let key = text_content(key_node.expect("option key"));

        let values: Vec<(Option<u32>, String)> = children_by_name(opt, "value")
            .map(|v| {
                let order = attr(v, "order").and_then(|s| s.parse::<u32>().ok());
                let content = text_content(v);
                (order, content)
            })
            .collect();

        match component {
            "biber" => {
                if biber.config.isexplicitoption(&key) {
                    continue;
                }
                if opt_type_attr == "singlevalued" {
                    if let Some((_, val)) = values.first() {
                        biber.config.setoption_str(&key, val.clone());
                    }
                } else if opt_type_attr == "multivalued" {
                    let mut sorted: Vec<(u32, String)> = values
                        .into_iter()
                        .map(|(o, v)| (o.unwrap_or(0), v))
                        .collect();
                    sorted.sort_by_key(|(o, _)| *o);
                    let list: Vec<ConfigValue> = sorted
                        .into_iter()
                        .map(|(_, v)| ConfigValue::Str(v))
                        .collect();
                    biber.config.setoption(&key, ConfigValue::List(list));
                }
            }
            "biblatex" => {
                if opt_type == "global" {
                    if opt_type_attr == "singlevalued" {
                        if let Some((_, val)) = values.first() {
                            biber.config.setblxoption(None, &key, val.clone().into());
                        }
                    } else if opt_type_attr == "multivalued" {
                        let mut sorted: Vec<(u32, String)> = values
                            .into_iter()
                            .map(|(o, v)| (o.unwrap_or(0), v))
                            .collect();
                        sorted.sort_by_key(|(o, _)| *o);
                        let list: Vec<ConfigValue> = sorted
                            .into_iter()
                            .map(|(_, v)| ConfigValue::Str(v))
                            .collect();
                        biber
                            .config
                            .setblxoption(None, &key, ConfigValue::List(list));
                    }
                } else {
                    if opt_type_attr == "singlevalued" {
                        if let Some((_, val)) = values.first() {
                            biber.config.setblxoption_entrytype(
                                None,
                                &key,
                                val.clone().into(),
                                opt_type,
                            );
                        }
                    } else if opt_type_attr == "multivalued" {
                        let mut sorted: Vec<(u32, String)> = values
                            .into_iter()
                            .map(|(o, v)| (o.unwrap_or(0), v))
                            .collect();
                        sorted.sort_by_key(|(o, _)| *o);
                        let list: Vec<ConfigValue> = sorted
                            .into_iter()
                            .map(|(_, v)| ConfigValue::Str(v))
                            .collect();
                        biber.config.setblxoption_entrytype(
                            None,
                            &key,
                            ConfigValue::List(list),
                            opt_type,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

// ---- Datafield sets ----

fn parse_datafieldset(node: Node, biber: &mut Biber) {
    let name = attr(node, "name").unwrap_or("").to_lowercase();
    for member in children_by_name(node, "member") {
        let field = first_child(member, "field").map(|f| text_content(f));
        let fieldtype = attr(member, "fieldtype").map(|s| s.to_string());
        let datatype = attr(member, "datatype").map(|s| s.to_string());

        biber.config.add_datafield_set_member(
            &name,
            biber_core::config::DatafieldSetMember {
                field,
                fieldtype,
                datatype,
            },
        );
    }
}

// ---- Sourcemap ----

fn parse_sourcemap(node: Node, biber: &mut Biber) {
    biber
        .config
        .setoption("sourcemap", ConfigValue::Raw(node_to_string(node)));
}

// ---- Label alpha name template ----

fn parse_labelalphanametemplate(node: Node, biber: &mut Biber) {
    let name = attr(node, "name").unwrap_or("global").to_string();
    let mut parts: Vec<ConfigValue> = Vec::new();
    for np in children_by_name(node, "namepart") {
        let mut part = std::collections::BTreeMap::new();
        part.insert("namepart".into(), ConfigValue::Str(text_content(np)));
        if let Some(u) = attr(np, "use") {
            part.insert("use".into(), ConfigValue::Str(u.into()));
        }
        if let Some(p) = attr(np, "pre") {
            part.insert("pre".into(), ConfigValue::Str(p.into()));
        }
        if let Some(sc) = attr(np, "substring_compound") {
            part.insert("substring_compound".into(), ConfigValue::Str(sc.into()));
        }
        if let Some(ss) = attr(np, "substring_side") {
            part.insert("substring_side".into(), ConfigValue::Str(ss.into()));
        }
        if let Some(sw) = attr(np, "substring_width") {
            part.insert("substring_width".into(), ConfigValue::Str(sw.into()));
        }
        parts.push(ConfigValue::Map(part));
    }
    let mut template = std::collections::BTreeMap::new();
    template.insert(name, ConfigValue::List(parts));
    biber
        .config
        .setblxoption(None, "labelalphanametemplate", ConfigValue::Map(template));
}

// ---- Label alpha template ----

fn parse_labelalphatemplate(node: Node, biber: &mut Biber) {
    let latype = attr(node, "type").unwrap_or("global").to_string();
    let mut elements: Vec<ConfigValue> = Vec::new();
    for elem in children_by_name(node, "labelelement") {
        let order = attr(elem, "order")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let mut parts: Vec<ConfigValue> = Vec::new();
        for part in children_by_name(elem, "labelpart") {
            let mut pm = std::collections::BTreeMap::new();
            pm.insert("content".into(), ConfigValue::Str(text_content(part)));
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
                if let Some(val) = attr(part, attr_name) {
                    pm.insert((*attr_name).into(), ConfigValue::Str(val.into()));
                }
            }
            parts.push(ConfigValue::Map(pm));
        }
        let mut em = std::collections::BTreeMap::new();
        em.insert("order".into(), ConfigValue::Str(order.to_string()));
        em.insert("parts".into(), ConfigValue::List(parts));
        elements.push(ConfigValue::Map(em));
    }
    let mut template_map = std::collections::BTreeMap::new();
    template_map.insert(latype.clone(), ConfigValue::List(elements));
    // Merge with existing global template if present (for multiple type entries)
    if let Some(ConfigValue::Map(mut existing_map)) = biber
        .config
        .getblxoption(None, "labelalphatemplate")
        .cloned()
    {
        for (k, v) in template_map {
            existing_map.insert(k, v);
        }
        biber
            .config
            .setblxoption(None, "labelalphatemplate", ConfigValue::Map(existing_map));
        return;
    }
    biber
        .config
        .setblxoption(None, "labelalphatemplate", ConfigValue::Map(template_map));
}

// ---- Extra date spec ----

fn parse_extradatespec(node: Node, biber: &mut Biber) {
    let mut scopes: Vec<ConfigValue> = Vec::new();
    for scope in children_by_name(node, "scope") {
        let mut sorted: Vec<(u32, String)> = children_by_name(scope, "field")
            .map(|f| {
                let order = attr(f, "order")
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                (order, text_content(f))
            })
            .collect();
        sorted.sort_by_key(|(o, _)| *o);
        let fields: Vec<ConfigValue> = sorted
            .into_iter()
            .map(|(_, f)| ConfigValue::Str(f))
            .collect();
        scopes.push(ConfigValue::List(fields));
    }
    biber
        .config
        .setblxoption(None, "extradatespec", ConfigValue::List(scopes));
}

// ---- Inheritance ----

fn parse_inheritance(node: Node, biber: &mut Biber) {
    biber
        .config
        .setblxoption(None, "inheritance", ConfigValue::Raw(node_to_string(node)));
}

// ---- Filter lists ----

fn parse_filter_list(node: Node, biber: &mut Biber, option_name: &str) {
    let child_name = option_name; // same name as the option, minus the 's'

    let mut items: Vec<ConfigValue> = Vec::new();
    for item in children_by_name(node, child_name) {
        let mut m = std::collections::BTreeMap::new();
        if let Some(v) = first_child(item, "value") {
            m.insert("value".into(), ConfigValue::Str(text_content(v)));
        }
        if let Some(f) = first_child(item, "field") {
            m.insert("name".into(), ConfigValue::Str(text_content(f)));
        }
        items.push(ConfigValue::Map(m));
    }
    if !items.is_empty() {
        biber
            .config
            .setoption(option_name, ConfigValue::List(items));
    }
}

// ---- Uniquename template ----

fn parse_uniquenametemplate(node: Node, biber: &mut Biber) {
    let name = attr(node, "name").unwrap_or("global").to_string();
    let mut nps: Vec<(u32, ConfigValue)> = children_by_name(node, "namepart")
        .map(|np| {
            let order = attr(np, "order")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            let mut m = std::collections::BTreeMap::new();
            m.insert("namepart".into(), ConfigValue::Str(text_content(np)));
            if let Some(u) = attr(np, "use") {
                m.insert("use".into(), ConfigValue::Str(u.into()));
            }
            if let Some(d) = attr(np, "disambiguation") {
                m.insert("disambiguation".into(), ConfigValue::Str(d.into()));
            }
            if let Some(b) = attr(np, "base") {
                m.insert("base".into(), ConfigValue::Str(b.into()));
            }
            (order, ConfigValue::Map(m))
        })
        .collect();
    nps.sort_by_key(|(o, _)| *o);
    let parts: Vec<ConfigValue> = nps.into_iter().map(|(_, v)| v).collect();
    let mut template = std::collections::BTreeMap::new();
    template.insert(name, ConfigValue::List(parts));
    biber
        .config
        .setblxoption(None, "uniquenametemplate", ConfigValue::Map(template));
}

// ---- Name hash template ----

fn parse_namehashtemplate(node: Node, biber: &mut Biber) {
    let name = attr(node, "name").unwrap_or("global").to_string();
    let mut nps: Vec<(u32, ConfigValue)> = children_by_name(node, "namepart")
        .map(|np| {
            let order = attr(np, "order")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            let mut m = std::collections::BTreeMap::new();
            m.insert("namepart".into(), ConfigValue::Str(text_content(np)));
            if let Some(hs) = attr(np, "hashscope") {
                m.insert("hashscope".into(), ConfigValue::Str(hs.into()));
            }
            (order, ConfigValue::Map(m))
        })
        .collect();
    nps.sort_by_key(|(o, _)| *o);
    let parts: Vec<ConfigValue> = nps.into_iter().map(|(_, v)| v).collect();
    let mut template = std::collections::BTreeMap::new();
    template.insert(name, ConfigValue::List(parts));
    biber
        .config
        .setblxoption(None, "namehashtemplate", ConfigValue::Map(template));
}

// ---- Sorting name key template ----

fn parse_sortingnamekeytemplate(node: Node, biber: &mut Biber) {
    let name = attr(node, "name").unwrap_or("global").to_string();
    let visibility = attr(node, "visibility").unwrap_or("sort").to_string();

    let mut kps: Vec<(u32, Vec<ConfigValue>)> = children_by_name(node, "keypart")
        .map(|kp| {
            let kp_order = attr(kp, "order")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            let mut parts: Vec<(u32, ConfigValue)> = children_by_name(kp, "part")
                .map(|p| {
                    let p_order = attr(p, "order")
                        .and_then(|s| s.parse::<u32>().ok())
                        .unwrap_or(0);
                    let ptype = attr(p, "type").unwrap_or("namepart");
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("type".into(), ConfigValue::Str(ptype.into()));
                    m.insert("value".into(), ConfigValue::Str(text_content(p)));
                    if let Some(u) = attr(p, "use") {
                        m.insert("use".into(), ConfigValue::Str(u.into()));
                    }
                    if let Some(i) = attr(p, "inits") {
                        m.insert("inits".into(), ConfigValue::Str(i.into()));
                    }
                    (p_order, ConfigValue::Map(m))
                })
                .collect();
            parts.sort_by_key(|(o, _)| *o);
            (kp_order, parts.into_iter().map(|(_, v)| v).collect())
        })
        .collect();
    kps.sort_by_key(|(o, _)| *o);
    let keyparts: Vec<ConfigValue> = kps
        .into_iter()
        .map(|(_, parts)| ConfigValue::List(parts))
        .collect();

    let mut template = std::collections::BTreeMap::new();
    let mut inner = std::collections::BTreeMap::new();
    inner.insert("visibility".into(), ConfigValue::Str(visibility));
    inner.insert("template".into(), ConfigValue::List(keyparts));
    template.insert(name, ConfigValue::Map(inner));
    biber
        .config
        .setblxoption(None, "sortingnamekeytemplate", ConfigValue::Map(template));
}

// ---- Transliteration ----

fn parse_transliteration(node: Node, biber: &mut Biber) {
    let entrytype = node.attribute("entrytype").unwrap_or("*");
    let mut rules: Vec<ConfigValue> = Vec::new();

    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let cname = child.tag_name().name();
        if !cname.ends_with("translit") {
            continue;
        }
        let langids = child.attribute("langids");
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
        m.insert("entrytype".into(), ConfigValue::Str(entrytype.to_string()));
        if let Some(l) = langids {
            m.insert("langids".into(), ConfigValue::Str(l.to_string()));
        }
        m.insert("target".into(), ConfigValue::Str(target.to_string()));
        m.insert("from".into(), ConfigValue::Str(from.to_string()));
        m.insert("to".into(), ConfigValue::Str(to.to_string()));
        rules.push(ConfigValue::Map(m));
    }

    if rules.is_empty() {
        return;
    }

    if entrytype == "*" {
        // Merge with existing global rules, if any
        let existing = biber.config.getblxoption(None, "translit");
        let mut merged: Vec<ConfigValue> = match existing {
            Some(ConfigValue::List(list)) => list.clone(),
            _ => Vec::new(),
        };
        merged.extend(rules);
        biber
            .config
            .setblxoption(None, "translit", ConfigValue::List(merged));
    } else {
        // Per-entrytype rules
        let existing = biber.config.getblxoption_entrytype(entrytype, "translit");
        let mut merged: Vec<ConfigValue> = match existing {
            Some(ConfigValue::List(list)) => list.clone(),
            _ => Vec::new(),
        };
        merged.extend(rules);
        biber
            .config
            .setblxoption_entrytype(None, "translit", ConfigValue::List(merged), entrytype);
    }
}

// ---- Sort exclusion/inclusion ----

fn parse_sortex_inclusion(node: Node, biber: &mut Biber, is_exclusion: bool) {
    let entrytype = attr(node, "type").unwrap_or("").to_string();
    let child_name = if is_exclusion {
        "exclusion"
    } else {
        "inclusion"
    };
    let mut items = std::collections::BTreeMap::new();
    for child in children_by_name(node, child_name) {
        let content = text_content(child);
        items.insert(content, ConfigValue::Str("1".into()));
    }
    let opt_name = if is_exclusion {
        "sortexclusion"
    } else {
        "sortinclusion"
    };
    biber
        .config
        .setblxoption_entrytype(None, opt_name, ConfigValue::Map(items), &entrytype);
}

// ---- Presort ----

fn parse_presort(node: Node, biber: &mut Biber) {
    let content = text_content(node);
    if let Some(ptype) = attr(node, "type") {
        biber
            .config
            .setblxoption_entrytype(None, "presort", content.into(), ptype);
    } else {
        biber.config.setblxoption(None, "presort", content.into());
    }
}

// ---- Sorting template ----

fn parse_sortingtemplate(node: Node, biber: &mut Biber) {
    let name = attr(node, "name").unwrap_or("").to_string();
    let locale = attr(node, "locale").map(|s| s.to_string());

    let mut sorted_sorts: Vec<(u32, Node)> = children_by_name(node, "sort")
        .map(|s| {
            let order = attr(s, "order")
                .and_then(|o| o.parse::<u32>().ok())
                .unwrap_or(0);
            (order, s)
        })
        .collect();
    sorted_sorts.sort_by_key(|(o, _)| *o);

    let mut sorts: Vec<ConfigValue> = Vec::new();
    for (_, sort_node) in sorted_sorts {
        let mut sort_opts: std::collections::BTreeMap<String, ConfigValue> =
            std::collections::BTreeMap::new();
        if let Some(f) = attr(sort_node, "final") {
            sort_opts.insert("final".into(), ConfigValue::Str(f.into()));
        }
        if let Some(sd) = attr(sort_node, "sort_direction") {
            sort_opts.insert("sort_direction".into(), ConfigValue::Str(sd.into()));
        }
        if let Some(sc) = attr(sort_node, "sortcase") {
            sort_opts.insert("sortcase".into(), ConfigValue::Str(sc.into()));
        }
        if let Some(su) = attr(sort_node, "sortupper") {
            sort_opts.insert("sortupper".into(), ConfigValue::Str(su.into()));
        }
        if let Some(l) = attr(sort_node, "locale") {
            sort_opts.insert("locale".into(), ConfigValue::Str(l.into()));
        }

        let mut sorted_items: Vec<(u32, Node)> = children_by_name(sort_node, "sortitem")
            .map(|si| {
                let order = attr(si, "order")
                    .and_then(|o| o.parse::<u32>().ok())
                    .unwrap_or(0);
                (order, si)
            })
            .collect();
        sorted_items.sort_by_key(|(o, _)| *o);

        let mut sortitems: Vec<ConfigValue> = Vec::new();
        for (_, item_node) in sorted_items {
            let mut item_attrs = std::collections::BTreeMap::new();
            if let Some(ss) = attr(item_node, "substring_side") {
                item_attrs.insert("substring_side".into(), ConfigValue::Str(ss.into()));
            }
            if let Some(sw) = attr(item_node, "substring_width") {
                item_attrs.insert("substring_width".into(), ConfigValue::Str(sw.into()));
            }
            if let Some(pw) = attr(item_node, "pad_width") {
                item_attrs.insert("pad_width".into(), ConfigValue::Str(pw.into()));
            }
            if let Some(pc) = attr(item_node, "pad_char") {
                item_attrs.insert("pad_char".into(), ConfigValue::Str(pc.into()));
            }
            if let Some(ps) = attr(item_node, "pad_side") {
                item_attrs.insert("pad_side".into(), ConfigValue::Str(ps.into()));
            }
            if let Some(lit) = attr(item_node, "literal") {
                item_attrs.insert("literal".into(), ConfigValue::Str(lit.into()));
            }
            let mut item_map = std::collections::BTreeMap::new();
            item_map.insert(text_content(item_node), ConfigValue::Map(item_attrs));
            sortitems.push(ConfigValue::Map(item_map));
        }

        if !sortitems.is_empty() {
            let mut sort_entry = std::collections::BTreeMap::new();
            for (k, v) in &sort_opts {
                sort_entry.insert(k.clone(), v.clone());
            }
            sort_entry.insert("items".into(), ConfigValue::List(sortitems));
            sorts.push(ConfigValue::Map(sort_entry));
        }
    }

    let mut template = std::collections::BTreeMap::new();
    if let Some(l) = locale {
        template.insert("locale".into(), ConfigValue::Str(l));
    }
    template.insert("spec".into(), ConfigValue::List(sorts));

    let existing = biber.config.getblxoption(None, "sortingtemplate");
    let mut templates = match existing {
        Some(ConfigValue::Map(m)) => m.clone(),
        _ => std::collections::BTreeMap::new(),
    };
    templates.insert(name, ConfigValue::Map(template));
    biber
        .config
        .setblxoption(None, "sortingtemplate", ConfigValue::Map(templates));
}

// ---- Data model ----

fn parse_datamodel(node: Node, biber: &mut Biber) {
    let mut dm = DataModel::new();

    if let Some(constants_node) = first_child(node, "constants") {
        for c in children_by_name(constants_node, "constant") {
            let ctype = attr(c, "type").unwrap_or("string").to_string();
            let cname = attr(c, "name").unwrap_or("").to_string();
            let cvalue = text_content(c);
            dm.constants.insert(
                cname.clone(),
                ModelConstant {
                    r#type: ctype,
                    name: cname,
                    value: cvalue,
                },
            );
        }
    }

    if let Some(et_node) = first_child(node, "entrytypes") {
        for et in children_by_name(et_node, "entrytype") {
            dm.entrytypes.insert(text_content(et));
        }
    }

    if let Some(f_node) = first_child(node, "fields") {
        for f in children_by_name(f_node, "field") {
            let fname = text_content(f);
            let fieldtype = attr(f, "fieldtype").unwrap_or("field").to_string();
            let datatype = attr(f, "datatype").unwrap_or("literal").to_string();
            let nullok = attr(f, "nullok").map(|v| v == "true").unwrap_or(false);
            let label = attr(f, "label").map(|v| v == "true").unwrap_or(false);
            let skip_output = attr(f, "skip_output").map(|v| v == "true").unwrap_or(false);
            let format = attr(f, "format").map(|s| s.to_string());

            dm.fields.insert(
                fname,
                ModelField {
                    fieldtype,
                    datatype,
                    nullok,
                    label,
                    skip_output,
                    format,
                },
            );
        }
    }

    for ef in children_by_name(node, "entryfields") {
        let mut entrytypes = Vec::new();
        let mut fields = Vec::new();
        for et in children_by_name(ef, "entrytype") {
            entrytypes.push(text_content(et));
        }
        for f in children_by_name(ef, "field") {
            fields.push(text_content(f));
        }
        dm.entryfields.push(EntryFields { entrytypes, fields });
    }

    if let Some(ms_node) = first_child(node, "multiscriptfields") {
        for f in children_by_name(ms_node, "field") {
            dm.multiscriptfields.insert(text_content(f));
        }
    }

    for cs_node in children_by_name(node, "constraints") {
        let mut entrytypes = Vec::new();
        for et in children_by_name(cs_node, "entrytype") {
            entrytypes.push(text_content(et));
        }
        let mut constraints = Vec::new();
        for c in children_by_name(cs_node, "constraint") {
            let ctype = attr(c, "type").unwrap_or("mandatory");
            match ctype {
                "mandatory" => {
                    let mut fields = Vec::new();
                    for child in c.children() {
                        if !child.is_element() {
                            continue;
                        }
                        match lname(child) {
                            "field" => {
                                fields.push(MandatoryField::Field(text_content(child)));
                            }
                            "fieldor" => {
                                let mut alternatives = Vec::new();
                                for f in children_by_name(child, "field") {
                                    alternatives.push(text_content(f));
                                }
                                fields.push(MandatoryField::FieldOr(alternatives));
                            }
                            _ => {}
                        }
                    }
                    constraints.push(ModelConstraint::Mandatory { fields });
                }
                "data" => {
                    let datatype = attr(c, "datatype").unwrap_or("integer").to_string();
                    let rangemin = attr(c, "rangemin").map(|s| s.to_string());
                    let rangemax = attr(c, "rangemax").map(|s| s.to_string());
                    let pattern = attr(c, "pattern").map(|s| s.to_string());
                    let fields: Vec<String> = children_by_name(c, "field")
                        .map(|f| text_content(f))
                        .collect();
                    constraints.push(ModelConstraint::Data {
                        datatype,
                        rangemin,
                        rangemax,
                        pattern,
                        fields,
                    });
                }
                "conditional" => {
                    let ant_node = first_child(c, "antecedent");
                    let con_node = first_child(c, "consequent");
                    let antecedent = ant_node
                        .map(|n| ConditionalPart {
                            quant: attr(n, "quant").unwrap_or("all").to_string(),
                            fields: children_by_name(n, "field")
                                .map(|f| text_content(f))
                                .collect(),
                        })
                        .unwrap_or(ConditionalPart {
                            quant: "all".into(),
                            fields: vec![],
                        });
                    let consequent = con_node
                        .map(|n| ConditionalPart {
                            quant: attr(n, "quant").unwrap_or("all").to_string(),
                            fields: children_by_name(n, "field")
                                .map(|f| text_content(f))
                                .collect(),
                        })
                        .unwrap_or(ConditionalPart {
                            quant: "all".into(),
                            fields: vec![],
                        });
                    constraints.push(ModelConstraint::Conditional {
                        antecedent,
                        consequent,
                    });
                }
                _ => {
                    debug!("unknown constraint type: {}", ctype);
                }
            }
        }
        dm.constraints.push(ModelConstraints {
            entrytypes,
            constraints,
        });
    }

    biber.datamodel = dm;

    biber
        .config
        .setblxoption(None, "datamodel", ConfigValue::Raw(node_to_string(node)));
}

// ---- Bibdata ----

fn parse_bibdata(node: Node, biber: &mut Biber) {
    let section: u32 = attr(node, "section")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    for ds in children_by_name(node, "datasource") {
        let ds_type = attr(ds, "type").unwrap_or("file").to_string();
        let ds_name = text_content(ds);
        let datatype = attr(ds, "datatype").unwrap_or("bibtex").to_string();
        let encoding = attr(ds, "encoding").map(|s| s.to_string());
        let glob = attr(ds, "glob")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let ds_ref = DatasourceRef {
            r#type: ds_type,
            name: ds_name,
            datatype,
            encoding,
            glob,
        };

        if biber.sections.get_section(section).is_none() {
            biber.sections.add_section(Section::new(section));
        }

        let section_obj = biber.sections.get_section_mut(section).unwrap();
        if section_obj.get_datasources().is_empty() {
            section_obj.set_datasources(vec![ds_ref]);
        } else {
            let exists = section_obj
                .get_datasources()
                .iter()
                .any(|d| d.r#type == ds_ref.r#type && d.name == ds_ref.name);
            if !exists {
                section_obj.add_datasource(ds_ref);
            }
        }
    }
}

// ---- Sections ----

fn parse_section(node: Node, biber: &mut Biber) {
    let secnum: u32 = attr(node, "number")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    if biber.sections.get_section(secnum).is_none() {
        biber.sections.add_section(Section::new(secnum));
    }
    let section = biber.sections.get_section_mut(secnum).unwrap();

    let mut keys: Vec<String> = Vec::new();

    for keyc in children_by_name(node, "citekey") {
        let key = text_content(keyc);
        let nocite = attr(keyc, "nocite").map(|v| v == "1").unwrap_or(false);
        let order = attr(keyc, "order").and_then(|s| s.parse::<u32>().ok());
        let intorder = attr(keyc, "intorder").and_then(|s| s.parse::<u32>().ok());
        let key_type = attr(keyc, "type").map(|s| s.to_string());
        let members = attr(keyc, "members").map(|s| s.to_string());

        if key_type.as_deref() == Some("set") {
            if let Some(m) = members {
                let member_list: Vec<String> = m
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                section.set_dynamic_set(key.clone(), member_list);
            }
            keys.push(key.clone());
            section.incr_seenkey(&key);
        } else if key == "*" {
            section.set_allkeys(true);
            if nocite {
                section.set_allkeys_nocite(true);
            }
            section.incr_seenkey(&key);
        } else if section.get_seenkey(&key) == 0 {
            if nocite {
                section.add_nocite(&key);
            } else {
                section.add_cite(&key);
            }
            if let Some(o) = order {
                biber.config.set_keyorder(secnum, &key, o);
            }
            if let Some(io) = intorder {
                biber.config.set_internal_keyorder(secnum, &key, io);
            }
            keys.push(key.clone());
            section.incr_seenkey(&key);
        }
    }

    for keycount in children_by_name(node, "citekeycount") {
        let key = text_content(keycount);
        let count: u32 = attr(keycount, "count")
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        section.set_citecount(key, count);
    }

    if section.is_allkeys() {
        section.del_citekeys();
    } else {
        section.add_citekeys(keys);
    }
}

// ---- Datalists ----

fn parse_datalist(node: Node, biber: &mut Biber) {
    let ltype = attr(node, "type").unwrap_or("entry");
    let lstn = attr(node, "sortingtemplatename").unwrap_or("");
    let lsnksn = attr(node, "sortingnamekeytemplatename").unwrap_or("global");
    let luntn = attr(node, "uniquenametemplatename").unwrap_or("global");
    let llantn = attr(node, "labelalphanametemplatename").unwrap_or("global");
    let lnhtn = attr(node, "namehashtemplatename").unwrap_or("global");
    let lpn = attr(node, "labelprefix").unwrap_or("");
    let lname = attr(node, "name").unwrap_or("");
    let lsection: u32 = attr(node, "section")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);

    if biber.datalists.has_list(lsection, lname, ltype, lstn) {
        return;
    }

    let mut dl = DataList::new(lsection, lstn, lsnksn, luntn, llantn, lnhtn, lpn, lname);
    dl.set_type(ltype);

    for filter in children_by_name(node, "filter") {
        dl.add_filter(ListFilter {
            r#type: attr(filter, "type").unwrap_or("").to_string(),
            value: text_content(filter),
        });
    }

    for orfilter in children_by_name(node, "filteror") {
        let filters: Vec<ListFilter> = children_by_name(orfilter, "filter")
            .map(|f| ListFilter {
                r#type: attr(f, "type").unwrap_or("").to_string(),
                value: text_content(f),
            })
            .collect();
        if !filters.is_empty() {
            dl.add_filteror(ListFilterOr { filters });
        }
    }

    biber.datalists.add_list(dl);
}

// ---- Ensure global datalists ----

fn ensure_global_datalists(biber: &mut Biber) {
    let globalss = biber
        .config
        .getblxoption_str("sortingtemplatename")
        .unwrap_or("nty")
        .to_string();

    let section_nums: Vec<u32> = biber
        .sections
        .get_sections()
        .iter()
        .map(|s| s.number)
        .collect();

    for secnum in section_nums {
        let list_name = format!("{}/global//global/global/global", globalss);
        let has_global = biber
            .datalists
            .has_list(secnum, &list_name, "entry", &globalss);

        if !has_global {
            let mut dl = DataList::new(
                secnum, &globalss, "global", "global", "global", "global", "", &list_name,
            );
            dl.set_type("entry");
            biber.datalists.add_list(dl);
        }
    }
}

/// Error type for BCF parsing.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// XML parsing error.
    #[error("XML parse error: {0}")]
    Xml(String),
    /// Invalid root element.
    #[error("invalid BCF root element: {0} (expected 'controlfile')")]
    InvalidRoot(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn repo_root() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .unwrap()
            .to_path_buf()
    }

    #[test]
    fn parse_minimal_bcf() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<bcf:controlfile version="3.11" bltxversion="3.21" xmlns:bcf="https://sourceforge.net/projects/biblatex">
  <bcf:options component="biber" type="global">
    <bcf:option type="singlevalued">
      <bcf:key>input_encoding</bcf:key>
      <bcf:value>utf8</bcf:value>
    </bcf:option>
    <bcf:option type="singlevalued">
      <bcf:key>output_encoding</bcf:key>
      <bcf:value>utf8</bcf:value>
    </bcf:option>
  </bcf:options>
  <bcf:bibdata section="0">
    <bcf:datasource type="file" datatype="bibtex">test.bib</bcf:datasource>
  </bcf:bibdata>
  <bcf:section number="0">
    <bcf:citekey order="1" intorder="1">key1</bcf:citekey>
    <bcf:citekey order="2" intorder="1">key2</bcf:citekey>
  </bcf:section>
  <bcf:datalist sortingnamekeytemplatename="global" section="0" name="nty/global//global/global/global" sortingtemplatename="nty" type="entry" labelprefix="" uniquenametemplatename="global" labelalphanametemplatename="global" namehashtemplatename="global"/>
</bcf:controlfile>"#;

        let biber = parse_bcf(xml).expect("parse should succeed");

        assert_eq!(biber.config.getoption_str("input_encoding"), Some("utf8"));
        assert_eq!(biber.config.getoption_str("output_encoding"), Some("utf8"));
        assert_eq!(biber.config.getoption_str("controlversion"), Some("3.11"));

        let section = biber.sections.get_section(0).expect("section 0");
        assert_eq!(section.get_citekeys().len(), 2);
        assert_eq!(section.get_citekeys()[0], "key1");
        assert_eq!(section.get_citekeys()[1], "key2");

        assert_eq!(section.get_datasources().len(), 1);
        assert_eq!(section.get_datasources()[0].name, "test.bib");
        assert_eq!(section.get_datasources()[0].datatype, "bibtex");
    }

    #[test]
    fn parse_real_fixture() {
        let bcf_path = repo_root().join("t/tdata/full-bbl.bcf");
        let bcf_xml = fs::read_to_string(&bcf_path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", bcf_path.display()));

        let biber = parse_bcf(&bcf_xml).expect("parse should succeed");

        assert_eq!(biber.config.getoption_str("controlversion"), Some("3.11"));
        assert!(!biber.sections.is_empty());

        let section = biber.sections.get_section(0).expect("section 0");
        assert!(!section.get_citekeys().is_empty() || section.is_allkeys());
        assert!(!section.get_datasources().is_empty());
        assert!(!biber.datalists.is_empty());
        assert!(biber
            .config
            .getblxoption_str("sortingtemplatename")
            .is_some());
        assert!(!biber.config.optscope.is_empty());
    }

    #[test]
    fn parse_all_fixtures() {
        let tdata = repo_root().join("t/tdata");
        let entries = fs::read_dir(&tdata).expect("reading t/tdata");
        let mut count = 0;
        let mut errors = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "bcf") {
                continue;
            }
            count += 1;
            let xml = match fs::read_to_string(&path) {
                Ok(s) => s,
                Err(e) => {
                    errors.push(format!("{}: read error: {e}", path.display()));
                    continue;
                }
            };
            match parse_bcf(&xml) {
                Ok(_) => {}
                Err(e) => errors.push(format!("{}: {e}", path.display())),
            }
        }

        assert!(count >= 50, "expected >=50 .bcf fixtures, found {count}");
        if !errors.is_empty() {
            panic!(
                "{} of {} fixtures failed to parse:\n{}",
                errors.len(),
                count,
                errors.join("\n")
            );
        }
    }
}
