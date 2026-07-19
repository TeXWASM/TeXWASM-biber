//! BiblateXML (.bltxml) reader — pure-Rust replacement for the Perl
//! `Biber::Input::file::biblatexml` module (1592 lines).
//!
//! Parses `.bltxml` files in the
//! `http://biblatex-biber.sourceforge.net/biblatexml` namespace into
//! the same entry map format used by the BibTeX reader.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use roxmltree::Document;

const NS: &str = "http://biblatex-biber.sourceforge.net/biblatexml";

/// A parsed biblatexml entry, matching the `BibEntry` interface.
#[derive(Debug, Clone)]
pub struct BltxEntry {
    /// Entry type (e.g. "book", "article").
    pub typ: String,
    /// Citekey.
    pub key: String,
    /// Fields: field name → value.
    pub fields: Vec<(String, String)>,
    /// Whether parsing succeeded.
    pub parse_ok: bool,
}

impl BltxEntry {
    /// Get a field value by name (case-insensitive lookup).
    pub fn get(&self, name: &str) -> Option<&str> {
        let lower = name.to_lowercase();
        self.fields
            .iter()
            .find(|(k, _)| *k == lower)
            .map(|(_, v)| v.as_str())
    }

    /// Get the list of field names in order.
    pub fn field_list(&self) -> Vec<&str> {
        self.fields.iter().map(|(k, _)| k.as_str()).collect()
    }
}

/// Result of parsing a `.bltxml` file into a key→entry map.
///
/// Returns (map, key_order, preambles) matching `BibMap` from the bib reader.
pub type BltxMap = (HashMap<String, BltxEntry>, Vec<String>, Vec<String>);

/// Error type for biblatexml parsing.
#[derive(Debug)]
pub enum BltxError {
    /// The XML could not be parsed.
    Xml(String),
    /// A required attribute is missing.
    MissingAttr(String),
}

impl std::fmt::Display for BltxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Xml(s) => write!(f, "XML parse error: {s}"),
            Self::MissingAttr(s) => write!(f, "missing attribute: {s}"),
        }
    }
}

impl std::error::Error for BltxError {}

/// Parse a biblatexml string and return all entries.
pub fn parse_bltxml(input: &str) -> Result<Vec<BltxEntry>, BltxError> {
    let doc = Document::parse(input).map_err(|e| BltxError::Xml(e.to_string()))?;

    let mut entries = Vec::new();

    // Find all <bltx:entry> elements
    for node in doc.descendants() {
        if !node.is_element() {
            continue;
        }
        if node.tag_name().namespace() != Some(NS) {
            continue;
        }
        if node.tag_name().name() != "entry" {
            continue;
        }

        let entry = parse_entry_node(node)?;
        entries.push(entry);
    }

    Ok(entries)
}

fn parse_entry_node(node: roxmltree::Node) -> Result<BltxEntry, BltxError> {
    let id = node
        .attribute("id")
        .ok_or_else(|| BltxError::MissingAttr("id on <bltx:entry>".into()))?
        .to_string();

    let entrytype = node
        .attribute("entrytype")
        .ok_or_else(|| BltxError::MissingAttr("entrytype on <bltx:entry>".into()))?
        .to_string();

    let mut fields: Vec<(String, String)> = Vec::new();

    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        if child.tag_name().namespace() != Some(NS) {
            continue;
        }

        let local = child.tag_name().name();
        match local {
            "names" => {
                // Extract name field
                let name_type = child.attribute("type").unwrap_or("author").to_string();
                let text = extract_names_text(child);
                if !text.is_empty() {
                    fields.push((name_type, text));
                }
            }
            "annotation" | "ids" | "options" | "related" => {
                // Skip annotation/ids/options/related for v1 — they map to
                // metadata that is not in the core BibEntry model.
            }
            _ => {
                // Literal field
                let text = extract_field_text(child);
                if !text.is_empty() {
                    fields.push((local.to_string(), text));
                }
            }
        }
    }

    Ok(BltxEntry {
        typ: entrytype,
        key: id,
        fields,
        parse_ok: true,
    })
}

/// Extract text from a name field, producing a simple BibTeX-like string.
fn extract_names_text(node: roxmltree::Node) -> String {
    let mut parts: Vec<String> = Vec::new();

    for name_node in node.children() {
        if !name_node.is_element() {
            continue;
        }
        if name_node.tag_name().namespace() != Some(NS) {
            continue;
        }
        if name_node.tag_name().name() != "name" {
            continue;
        }

        let mut given = String::new();
        let mut family = String::new();
        let mut prefix = String::new();
        let mut suffix = String::new();

        for np in name_node.children() {
            if !np.is_element() {
                continue;
            }
            if np.tag_name().namespace() != Some(NS) {
                continue;
            }
            if np.tag_name().name() != "namepart" {
                continue;
            }
            let np_type = np.attribute("type").unwrap_or("");
            let text = extract_field_text(np);
            match np_type {
                "given" => given = text,
                "family" => family = text,
                "prefix" => prefix = text,
                "suffix" => suffix = text,
                _ => {}
            }
        }

        let name_str = if !given.is_empty() && !family.is_empty() {
            if prefix.is_empty() {
                format!("{given} {family}")
            } else {
                format!("{given} {prefix} {family}")
            }
        } else if !family.is_empty() {
            if !suffix.is_empty() {
                format!("{family}, {suffix}")
            } else {
                family
            }
        } else if !given.is_empty() {
            given
        } else {
            String::new()
        };

        if !name_str.is_empty() {
            parts.push(name_str);
        }
    }

    parts.join(" and ")
}

/// Extract text content from a field element (handles mixed text+children).
fn extract_field_text(node: roxmltree::Node) -> String {
    let mut text = String::new();
    for child in node.children() {
        match child.node_type() {
            roxmltree::NodeType::Text => {
                text.push_str(child.text().unwrap_or(""));
            }
            roxmltree::NodeType::Element => {
                // For list items, extract text
                if child.tag_name().name() == "item" {
                    if !text.is_empty() {
                        text.push_str(" and ");
                    }
                    text.push_str(&extract_field_text(child));
                }
                // For start/end in range fields
                if child.tag_name().name() == "start" {
                    text.push_str(&extract_field_text(child));
                }
                if child.tag_name().name() == "end" {
                    text.push('/');
                    text.push_str(&extract_field_text(child));
                }
            }
            _ => {}
        }
    }
    text
}

/// Parse a biblatexml string into a key→entry map (matching `BibMap`).
pub fn parse_bltxml_into_map(input: &str) -> Result<BltxMap, BltxError> {
    let entries = parse_bltxml(input)?;
    let mut map = HashMap::new();
    let mut order = Vec::new();

    for entry in entries {
        let key = entry.key.clone();
        if !map.contains_key(&key) {
            order.push(key.clone());
        }
        map.insert(key, entry);
    }

    Ok((map, order, Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<bltx:entries xmlns:bltx="http://biblatex-biber.sourceforge.net/biblatexml">
  <bltx:entry id="bltx1" entrytype="book">
    <bltx:title>Test Book</bltx:title>
    <bltx:year>2020</bltx:year>
    <bltx:location>Москва</bltx:location>
  </bltx:entry>
  <bltx:entry id="bltx2" entrytype="article">
    <bltx:title>Test Article</bltx:title>
    <bltx:year>2021</bltx:year>
    <bltx:author>John Doe</bltx:author>
  </bltx:entry>
</bltx:entries>"#;

    #[test]
    fn parse_simple_entries() {
        let (map, order, _) = parse_bltxml_into_map(TEST_XML).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(order.len(), 2);

        let e1 = map.get("bltx1").unwrap();
        assert_eq!(e1.typ, "book");
        assert_eq!(e1.get("title"), Some("Test Book"));
        assert_eq!(e1.get("year"), Some("2020"));
        assert_eq!(e1.get("location"), Some("Москва"));
    }

    #[test]
    fn parse_entry_with_names() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<bltx:entries xmlns:bltx="http://biblatex-biber.sourceforge.net/biblatexml">
  <bltx:entry id="named" entrytype="book">
    <bltx:names type="author">
      <bltx:name>
        <bltx:namepart type="given">John</bltx:namepart>
        <bltx:namepart type="family">Doe</bltx:namepart>
      </bltx:name>
      <bltx:name>
        <bltx:namepart type="given">Jane</bltx:namepart>
        <bltx:namepart type="family">Smith</bltx:namepart>
      </bltx:name>
    </bltx:names>
  </bltx:entry>
</bltx:entries>"#;

        let (map, _, _) = parse_bltxml_into_map(xml).unwrap();
        let entry = map.get("named").unwrap();
        assert_eq!(entry.get("author"), Some("John Doe and Jane Smith"));
    }

    #[test]
    fn parse_from_fixture() {
        let fixture = include_str!("../../../t/tdata/biblatexml.bltxml");
        let (map, order, _) = parse_bltxml_into_map(fixture).unwrap();
        assert_eq!(map.len(), 1);
        assert_eq!(order, vec!["bltx1"]);

        let entry = map.get("bltx1").unwrap();
        assert_eq!(entry.typ, "book");
        assert_eq!(
            entry.get("title"),
            Some("Мухаммад ибн муса ал-Хорезми. Около 783 – около 850")
        );
        assert_eq!(entry.get("location"), Some("Москва"));
        assert!(entry.get("author").is_some());
    }

    #[test]
    fn missing_attributes_error() {
        let xml = r#"<bltx:entries xmlns:bltx="http://biblatex-biber.sourceforge.net/biblatexml">
  <bltx:entry>no id or entrytype</bltx:entry>
</bltx:entries>"#;
        let result = parse_bltxml_into_map(xml);
        assert!(result.is_err());
    }

    #[test]
    fn empty_xml_returns_no_entries() {
        let xml =
            r#"<bltx:entries xmlns:bltx="http://biblatex-biber.sourceforge.net/biblatexml"/>"#;
        let (map, order, _) = parse_bltxml_into_map(xml).unwrap();
        assert!(map.is_empty());
        assert!(order.is_empty());
    }
}
