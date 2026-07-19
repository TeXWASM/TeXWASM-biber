//! Annotation model — stores field, item, and namepart annotations.
//!
//! Ported from `lib/Biber/Annotation.pm`. Annotations are metadata
//! attached to fields or name parts of bibliography entries, specified
//! in `.bib` files via the `+an` field-name marker suffix.

use std::collections::{HashMap, HashSet};

// Type aliases for nested map structures
type FMap = HashMap<String, Annotation>; // field scope: name → Annotation
type IMap = HashMap<String, HashMap<u32, Annotation>>; // item scope: name → count → Annotation
type PMap = HashMap<String, HashMap<u32, HashMap<String, Annotation>>>; // part scope: name → count → part → Annotation

/// Scope of an annotation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnnotationScope {
    /// Annotation on the entire field value.
    Field,
    /// Annotation on a specific list item (by 1-based index).
    Item,
    /// Annotation on a specific name part (by index and part name).
    Part,
}

impl AnnotationScope {
    /// Return the string representation used in BBL output.
    pub fn as_str(&self) -> &'static str {
        match self {
            AnnotationScope::Field => "field",
            AnnotationScope::Item => "item",
            AnnotationScope::Part => "part",
        }
    }
}

/// A single annotation value, stored at any scope level.
#[derive(Debug, Clone)]
pub struct Annotation {
    /// The annotation text.
    pub value: String,
    /// Whether the value was quoted (a literal string, not parsed).
    pub literal: bool,
    /// The annotation name / key (e.g. "default", "french").
    pub name: String,
}

impl Annotation {
    /// Create a new `Annotation` with the given value, literal flag, and name.
    pub fn new(value: impl Into<String>, literal: bool, name: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            literal,
            name: name.into(),
        }
    }
}

/// Storage for all annotations in a section.
///
/// Three-level maps keyed by scope:
/// - `field`: citekey → fieldname → name → Annotation
/// - `item`: citekey → fieldname → (name → (count → Annotation))
/// - `part`: citekey → fieldname → (name → (count → (part → Annotation)))
///
/// Additional indexes for fast lookups (matching Perl's `$ANN{fields}`
/// and `$ANN{names}`).
#[derive(Debug, Clone, Default)]
pub struct AnnotationStore {
    /// Field-scope annotations: citekey → fieldname → name → Annotation.
    field: HashMap<String, HashMap<String, FMap>>,
    /// Item-scope annotations: citekey → fieldname → name → (count → Annotation).
    item: HashMap<String, HashMap<String, IMap>>,
    /// Part-scope annotations: citekey → fieldname → name → (count → (part → Annotation)).
    part: HashMap<String, HashMap<String, PMap>>,
    /// Quick lookup: citekey → set of annotated field names.
    annotated_fields: HashMap<String, HashSet<String>>,
    /// Quick lookup: citekey → fieldname → set of annotation names.
    annotation_names: HashMap<String, HashMap<String, HashSet<String>>>,
}

impl AnnotationStore {
    /// Create an empty `AnnotationStore`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Store a single annotation value.
    #[allow(clippy::too_many_arguments)]
    pub fn set_annotation(
        &mut self,
        scope: AnnotationScope,
        citekey: impl Into<String>,
        field: impl Into<String>,
        name: impl Into<String>,
        value: impl Into<String>,
        literal: bool,
        count: Option<u32>,
        part: Option<String>,
    ) {
        let citekey = citekey.into();
        let field = field.into();
        let name = name.into();
        let value = value.into();
        let annotation = Annotation::new(value, literal, &name);

        // Track annotated fields and names
        self.annotated_fields
            .entry(citekey.clone())
            .or_default()
            .insert(field.clone());
        self.annotation_names
            .entry(citekey.clone())
            .or_default()
            .entry(field.clone())
            .or_default()
            .insert(name.clone());

        match scope {
            AnnotationScope::Field => {
                self.field
                    .entry(citekey)
                    .or_default()
                    .entry(field)
                    .or_default()
                    .insert(name, annotation);
            }
            AnnotationScope::Item => {
                let c = count.unwrap_or(0);
                self.item
                    .entry(citekey)
                    .or_default()
                    .entry(field)
                    .or_default()
                    .entry(name)
                    .or_default()
                    .insert(c, annotation);
            }
            AnnotationScope::Part => {
                let c = count.unwrap_or(0);
                let p = part.unwrap_or_default();
                self.part
                    .entry(citekey)
                    .or_default()
                    .entry(field)
                    .or_default()
                    .entry(name)
                    .or_default()
                    .entry(c)
                    .or_default()
                    .insert(p, annotation);
            }
        }
    }

    /// Get all field-scope annotation names for a (citekey, field).
    pub fn get_field_annotation_names(&self, citekey: &str, field: &str) -> Vec<&str> {
        self.field
            .get(citekey)
            .and_then(|f| f.get(field))
            .map(|m| m.keys().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Get all item-scope annotation names for a (citekey, field).
    pub fn get_item_annotation_names(&self, citekey: &str, field: &str) -> Vec<&str> {
        self.item
            .get(citekey)
            .and_then(|f| f.get(field))
            .map(|m| m.keys().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Get all part-scope annotation names for a (citekey, field).
    pub fn get_part_annotation_names(&self, citekey: &str, field: &str) -> Vec<&str> {
        self.part
            .get(citekey)
            .and_then(|f| f.get(field))
            .map(|m| m.keys().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Get a field-scope annotation value by (citekey, field, name).
    pub fn get_field_annotation(
        &self,
        citekey: &str,
        field: &str,
        name: &str,
    ) -> Option<&Annotation> {
        self.field
            .get(citekey)
            .and_then(|f| f.get(field))
            .and_then(|m| m.get(name))
    }

    /// Get item-scope annotation counts for (citekey, field, name).
    pub fn get_item_counts(&self, citekey: &str, field: &str, name: &str) -> Vec<u32> {
        self.item
            .get(citekey)
            .and_then(|f| f.get(field))
            .and_then(|m| m.get(name))
            .map(|m| m.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Get an item-scope annotation by (citekey, field, name, count).
    pub fn get_item_annotation(
        &self,
        citekey: &str,
        field: &str,
        name: &str,
        count: u32,
    ) -> Option<&Annotation> {
        self.item
            .get(citekey)
            .and_then(|f| f.get(field))
            .and_then(|m| m.get(name))
            .and_then(|m| m.get(&count))
    }

    /// Get part names for (citekey, field, name, count) in part scope.
    pub fn get_part_names(&self, citekey: &str, field: &str, name: &str, count: u32) -> Vec<&str> {
        self.part
            .get(citekey)
            .and_then(|f| f.get(field))
            .and_then(|m| m.get(name))
            .and_then(|m| m.get(&count))
            .map(|m| m.keys().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Get a part-scope annotation by (citekey, field, name, count, part).
    pub fn get_part_annotation(
        &self,
        citekey: &str,
        field: &str,
        name: &str,
        count: u32,
        part: &str,
    ) -> Option<&Annotation> {
        self.part
            .get(citekey)
            .and_then(|f| f.get(field))
            .and_then(|m| m.get(name))
            .and_then(|m| m.get(&count))
            .and_then(|m| m.get(part))
    }

    /// Check if a citekey has any annotations at all.
    pub fn has_annotations(&self, citekey: &str) -> bool {
        self.annotated_fields.contains_key(citekey)
    }

    /// Check if a (citekey, field) has annotations.
    pub fn is_annotated_field(&self, citekey: &str, field: &str) -> bool {
        self.annotated_fields
            .get(citekey)
            .is_some_and(|s| s.contains(field))
    }

    /// Get all annotated field names for a citekey.
    pub fn get_annotated_fields(&self, citekey: &str) -> Vec<&str> {
        self.annotated_fields
            .get(citekey)
            .map(|s| s.iter().map(|f| f.as_str()).collect())
            .unwrap_or_default()
    }

    /// Get all annotation names for a (citekey, field).
    pub fn get_annotation_names(&self, citekey: &str, field: &str) -> Vec<&str> {
        self.annotation_names
            .get(citekey)
            .and_then(|f| f.get(field))
            .map(|s| s.iter().map(|n| n.as_str()).collect())
            .unwrap_or_default()
    }

    /// Copy all annotations from one citekey to another.
    /// Used during related-entry cloning.
    pub fn copy_annotations(&mut self, source: &str, target: &str) {
        // Field scope
        if let Some(fields) = self.field.get(source).cloned() {
            for (field, names) in fields {
                for (name, ann) in names {
                    self.set_annotation(
                        AnnotationScope::Field,
                        target,
                        &field,
                        &name,
                        &ann.value,
                        ann.literal,
                        None,
                        None,
                    );
                }
            }
        }

        // Item scope
        if let Some(fields) = self.item.get(source).cloned() {
            for (field, names) in fields {
                for (name, counts) in names {
                    for (count, ann) in counts {
                        self.set_annotation(
                            AnnotationScope::Item,
                            target,
                            &field,
                            &name,
                            &ann.value,
                            ann.literal,
                            Some(count),
                            None,
                        );
                    }
                }
            }
        }

        // Part scope
        if let Some(fields) = self.part.get(source).cloned() {
            for (field, names) in fields {
                for (name, counts) in names {
                    for (count, parts) in counts {
                        for (part, ann) in parts {
                            self.set_annotation(
                                AnnotationScope::Part,
                                target,
                                &field,
                                &name,
                                &ann.value,
                                ann.literal,
                                Some(count),
                                Some(part.clone()),
                            );
                        }
                    }
                }
            }
        }
    }

    /// Remove all annotations for a citekey.
    pub fn remove_citekey(&mut self, citekey: &str) {
        self.field.remove(citekey);
        self.item.remove(citekey);
        self.part.remove(citekey);
        self.annotated_fields.remove(citekey);
        self.annotation_names.remove(citekey);
    }

    /// Iterate over all field-scope annotations: (citekey, field, name, ann).
    pub fn iter_field(&self) -> impl Iterator<Item = (&str, &str, &str, &Annotation)> {
        self.field.iter().flat_map(|(ck, fields)| {
            fields.iter().flat_map(move |(f, names)| {
                names
                    .iter()
                    .map(move |(n, a)| (ck.as_str(), f.as_str(), n.as_str(), a))
            })
        })
    }

    /// Iterate over all item-scope annotations: (citekey, field, name, count, ann).
    pub fn iter_item(&self) -> impl Iterator<Item = (&str, &str, &str, u32, &Annotation)> {
        self.item.iter().flat_map(|(ck, fields)| {
            fields.iter().flat_map(move |(f, names)| {
                names.iter().flat_map(move |(n, counts)| {
                    counts
                        .iter()
                        .map(move |(c, a)| (ck.as_str(), f.as_str(), n.as_str(), *c, a))
                })
            })
        })
    }

    /// Iterate over all part-scope annotations: (citekey, field, name, count, part, ann).
    pub fn iter_part(&self) -> impl Iterator<Item = (&str, &str, &str, u32, &str, &Annotation)> {
        self.part.iter().flat_map(|(ck, fields)| {
            fields.iter().flat_map(move |(f, names)| {
                names.iter().flat_map(move |(n, counts)| {
                    counts.iter().flat_map(move |(c, parts)| {
                        parts.iter().map(move |(p, a)| {
                            (ck.as_str(), f.as_str(), n.as_str(), *c, p.as_str(), a)
                        })
                    })
                })
            })
        })
    }
}

/// Parsed result of a BibTeX annotation field.
#[derive(Debug, Clone)]
pub struct BibAnnotation {
    /// The base field name (with annotation marker stripped).
    pub field: String,
    /// The annotation name (from optional `:name` suffix, or "default").
    pub name: String,
    /// Parsed annotation entries.
    pub entries: Vec<AnnotationEntry>,
}

/// A single parsed entry from an annotation field value.
#[derive(Debug, Clone)]
pub struct AnnotationEntry {
    /// Scope (deduced from presence of count and part).
    pub scope: AnnotationScope,
    /// Item index (1-based), for item and part scopes.
    pub count: Option<u32>,
    /// Name part name, for part scope.
    pub part: Option<String>,
    /// The annotation value.
    pub value: String,
    /// Whether the value was quoted (literal).
    pub literal: bool,
}

/// Parse a BibTeX annotation field name and value.
///
/// Detects the `+an` (or custom annotation_marker) suffix and optional
/// `:name` named-annotation suffix. If the field name does NOT match the
/// annotation pattern, returns `None`.
///
/// # Syntax
///
/// Field name: `BASEFIELD` `+an` [`:` `NAME`]
///
/// Value: `ENTRY1; ENTRY2; ...` where each entry:
///
/// - `=VALUE` → field-scope annotation
/// - `N=VALUE` → item-scope annotation at index N
/// - `N:PART=VALUE` → part-scope annotation
///
/// Quoted values (`"..."`) are treated as literal.
pub fn parse_annotation_field(
    field_name: &str,
    value: &str,
    annotation_marker: &str,
    named_marker: &str,
) -> Option<BibAnnotation> {
    let lower = field_name.to_lowercase();

    // Check if the field ends with the annotation marker
    let marker_pos = lower.rfind(annotation_marker)?;

    // The marker must be at the end or followed by the named marker
    let after_marker = &lower[marker_pos + annotation_marker.len()..];
    let name = if after_marker.is_empty() {
        "default"
    } else {
        let named = after_marker.strip_prefix(named_marker)?;
        if named.is_empty() {
            "default"
        } else {
            named
        }
    };

    let base_field = &field_name[..marker_pos];

    // Parse semicolon-separated entries
    let entries = parse_annotation_value(value);

    Some(BibAnnotation {
        field: base_field.to_string(),
        name: name.to_string(),
        entries,
    })
}

/// Parse the value part of an annotation field.
///
/// Splits on `;` and parses each entry as:
/// - `=VALUE` → field scope
/// - `N=VALUE` → item scope
/// - `N:PART=VALUE` → part scope
fn parse_annotation_value(value: &str) -> Vec<AnnotationEntry> {
    let mut entries = Vec::new();

    // Split on semicolons, respecting quotes
    let parts = split_semicolon_quoted(value);

    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if let Some(eq_pos) = part.find('=') {
            let lhs = part[..eq_pos].trim();
            let rhs = &part[eq_pos + 1..];

            let (literal, val) = if rhs.starts_with('"') && rhs.ends_with('"') && rhs.len() >= 2 {
                (true, rhs[1..rhs.len() - 1].to_string())
            } else {
                (false, rhs.to_string())
            };

            if lhs.is_empty() {
                // Field scope: `=VALUE`
                entries.push(AnnotationEntry {
                    scope: AnnotationScope::Field,
                    count: None,
                    part: None,
                    value: val,
                    literal,
                });
            } else if let Some(colon_pos) = lhs.find(':') {
                // Part scope: `N:PART=VALUE`
                let count_str = lhs[..colon_pos].trim();
                let part_name = lhs[colon_pos + 1..].trim();
                let count: u32 = count_str.parse().unwrap_or(0);
                entries.push(AnnotationEntry {
                    scope: AnnotationScope::Part,
                    count: Some(count),
                    part: Some(part_name.to_string()),
                    value: val,
                    literal,
                });
            } else {
                // Item scope: `N=VALUE`
                let count: u32 = lhs.parse().unwrap_or(0);
                entries.push(AnnotationEntry {
                    scope: AnnotationScope::Item,
                    count: Some(count),
                    part: None,
                    value: val,
                    literal,
                });
            }
        }
    }

    entries
}

/// Split a string on semicolons, respecting double-quoted strings.
fn split_semicolon_quoted(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for c in s.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                current.push(c);
            }
            ';' if !in_quotes => {
                parts.push(current);
                current = String::new();
            }
            _ => {
                current.push(c);
            }
        }
    }

    if !current.trim().is_empty() {
        parts.push(current);
    }

    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_field_annotation() {
        let result = parse_annotation_field("title+an", "=one, two", "+an", ":").unwrap();
        assert_eq!(result.field, "title");
        assert_eq!(result.name, "default");
        assert_eq!(result.entries.len(), 1);
        assert_eq!(
            result.entries[0].scope as i32,
            AnnotationScope::Field as i32
        );
        assert_eq!(result.entries[0].value, "one, two");
        assert!(!result.entries[0].literal);
    }

    #[test]
    fn parse_item_annotation() {
        let result =
            parse_annotation_field("language+an", "1=ann1; 2=ann2, ann3; =ann4", "+an", ":")
                .unwrap();
        assert_eq!(result.field, "language");
        assert_eq!(result.entries.len(), 3);

        // Entry 1: item scope
        assert_eq!(result.entries[0].scope as i32, AnnotationScope::Item as i32);
        assert_eq!(result.entries[0].count, Some(1));
        assert_eq!(result.entries[0].value, "ann1");
        assert!(!result.entries[0].literal);

        // Entry 2: item scope
        assert_eq!(result.entries[1].scope as i32, AnnotationScope::Item as i32);
        assert_eq!(result.entries[1].count, Some(2));
        assert_eq!(result.entries[1].value, "ann2, ann3");
        assert!(!result.entries[1].literal);

        // Entry 3: field scope
        assert_eq!(
            result.entries[2].scope as i32,
            AnnotationScope::Field as i32
        );
        assert_eq!(result.entries[2].value, "ann4");
    }

    #[test]
    fn parse_part_annotation() {
        let result =
            parse_annotation_field("author+an", "1:family=student;2=corresponding", "+an", ":")
                .unwrap();
        assert_eq!(result.field, "author");
        assert_eq!(result.entries.len(), 2);

        // Entry 1: part scope
        assert_eq!(result.entries[0].scope as i32, AnnotationScope::Part as i32);
        assert_eq!(result.entries[0].count, Some(1));
        assert_eq!(result.entries[0].part.as_deref(), Some("family"));
        assert_eq!(result.entries[0].value, "student");
        assert!(!result.entries[0].literal);

        // Entry 2: item scope (no part)
        assert_eq!(result.entries[1].scope as i32, AnnotationScope::Item as i32);
        assert_eq!(result.entries[1].count, Some(2));
        assert_eq!(result.entries[1].value, "corresponding");
    }

    #[test]
    fn parse_named_annotation() {
        let result = parse_annotation_field("title+an:default", "=\"one\"", "+an", ":").unwrap();
        assert_eq!(result.field, "title");
        assert_eq!(result.name, "default");
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].value, "one");
        assert!(result.entries[0].literal);

        let result = parse_annotation_field("title+an:french", "=\"un\"", "+an", ":").unwrap();
        assert_eq!(result.name, "french");
    }

    #[test]
    fn parse_literal_annotation() {
        let result = parse_annotation_field(
            "author+an",
            "1:family=\"student\";2=corresponding",
            "+an",
            ":",
        )
        .unwrap();
        assert_eq!(result.entries.len(), 2);
        assert!(result.entries[0].literal);
        assert_eq!(result.entries[0].value, "student");
        assert!(!result.entries[1].literal);
        assert_eq!(result.entries[1].value, "corresponding");
    }

    #[test]
    fn non_annotation_field_returns_none() {
        let result = parse_annotation_field("author", "Doe", "+an", ":");
        assert!(result.is_none());

        let result = parse_annotation_field("author+annotated", "x", "+an", ":");
        assert!(result.is_none());
    }

    #[test]
    fn annotation_store_basic() {
        let mut store = AnnotationStore::new();
        store.set_annotation(
            AnnotationScope::Field,
            "key1",
            "title",
            "default",
            "one, two",
            false,
            None,
            None,
        );

        assert!(store.has_annotations("key1"));
        assert_eq!(store.get_annotated_fields("key1"), vec!["title"]);
        let ann = store
            .get_field_annotation("key1", "title", "default")
            .unwrap();
        assert_eq!(ann.value, "one, two");
    }

    #[test]
    fn annotation_store_item_scope() {
        let mut store = AnnotationStore::new();
        store.set_annotation(
            AnnotationScope::Item,
            "key1",
            "language",
            "default",
            "ann1",
            false,
            Some(1),
            None,
        );

        let counts = store.get_item_counts("key1", "language", "default");
        assert_eq!(counts, vec![1]);
        let ann = store
            .get_item_annotation("key1", "language", "default", 1)
            .unwrap();
        assert_eq!(ann.value, "ann1");
    }

    #[test]
    fn annotation_store_part_scope() {
        let mut store = AnnotationStore::new();
        store.set_annotation(
            AnnotationScope::Part,
            "key1",
            "author",
            "default",
            "student",
            false,
            Some(1),
            Some("family".into()),
        );

        let parts = store.get_part_names("key1", "author", "default", 1);
        assert_eq!(parts, vec!["family"]);
        let ann = store
            .get_part_annotation("key1", "author", "default", 1, "family")
            .unwrap();
        assert_eq!(ann.value, "student");
    }

    #[test]
    fn annotation_store_copy() {
        let mut store = AnnotationStore::new();
        store.set_annotation(
            AnnotationScope::Field,
            "src",
            "title",
            "default",
            "ann_val",
            false,
            None,
            None,
        );
        store.copy_annotations("src", "dst");
        assert!(store.has_annotations("dst"));
        let ann = store
            .get_field_annotation("dst", "title", "default")
            .unwrap();
        assert_eq!(ann.value, "ann_val");
    }

    #[test]
    fn annotation_store_remove() {
        let mut store = AnnotationStore::new();
        store.set_annotation(
            AnnotationScope::Field,
            "key1",
            "title",
            "default",
            "val",
            false,
            None,
            None,
        );
        assert!(store.has_annotations("key1"));
        store.remove_citekey("key1");
        assert!(!store.has_annotations("key1"));
    }

    #[test]
    fn roundtrip_full_bibtex_annotation() {
        let field = "author+an:default";
        let value = "1:family=student;2=corresponding";
        let ann = parse_annotation_field(field, value, "+an", ":").unwrap();
        assert_eq!(ann.field, "author");
        assert_eq!(ann.name, "default");

        let mut store = AnnotationStore::new();
        for entry in &ann.entries {
            store.set_annotation(
                entry.scope,
                "key1",
                &ann.field,
                &ann.name,
                &entry.value,
                entry.literal,
                entry.count,
                entry.part.clone(),
            );
        }

        assert!(store.is_annotated_field("key1", "author"));
        assert_eq!(
            store.get_annotation_names("key1", "author"),
            vec!["default"]
        );
        assert_eq!(
            store.get_part_names("key1", "author", "default", 1),
            vec!["family"]
        );
        let ann = store
            .get_part_annotation("key1", "author", "default", 1, "family")
            .unwrap();
        assert_eq!(ann.value, "student");
    }
}
