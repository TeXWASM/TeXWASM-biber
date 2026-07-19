//! Bibliographic entry model.
//!
//! Ported from `lib/Biber/Entry.pm`. An `Entry` holds the field data for a
//! single bibliography item (e.g. a `@book` or `@article`).

use std::collections::{BTreeMap, HashMap};

use crate::config::ConfigValue;
use crate::name::Names;

/// A bibliography entry.
///
/// In Perl, `Biber::Entry` is a blessed hash with dynamic fields. Here we
/// use a struct with named fields for the common case, plus a generic
/// `fields` map for data fields from the `.bib` source.
#[derive(Debug, Clone, Default)]
pub struct Entry {
    /// The citekey for this entry.
    pub citekey: String,
    /// The entry type (e.g. "book", "article").
    pub entrytype: String,
    /// The datasource name this entry came from.
    pub datasource: String,
    /// Data fields: field name → value.
    pub fields: BTreeMap<String, ConfigValue>,
    /// Whether this entry is a clone of another (for related entries).
    pub clone: bool,
    /// Whether this entry is a set member.
    pub set_member: bool,
    /// Parsed name lists: field name → Names object.
    pub names: HashMap<String, Names>,
    /// The citekey of the source entry this was cloned from (if clone=true).
    pub clonesourcekey: Option<String>,
}

impl Entry {
    /// Create a new empty entry.
    pub fn new(citekey: impl Into<String>, entrytype: impl Into<String>) -> Self {
        Self {
            citekey: citekey.into(),
            entrytype: entrytype.into(),
            ..Default::default()
        }
    }

    /// Get a field value.
    pub fn get_field(&self, name: &str) -> Option<&ConfigValue> {
        self.fields.get(name)
    }

    /// Get a field as a string.
    pub fn get_field_str(&self, name: &str) -> Option<&str> {
        self.fields.get(name).and_then(|v| v.as_str())
    }

    /// Set a field value.
    pub fn set_field(&mut self, name: impl Into<String>, value: ConfigValue) {
        self.fields.insert(name.into(), value);
    }

    /// Set a field to a string value.
    pub fn set_field_str(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.fields
            .insert(name.into(), ConfigValue::Str(value.into()));
    }

    /// Check if a field exists.
    pub fn has_field(&self, name: &str) -> bool {
        self.fields.contains_key(name)
    }

    /// Remove a field.
    pub fn del_field(&mut self, name: &str) -> Option<ConfigValue> {
        self.fields.remove(name)
    }

    /// Get all field names.
    pub fn field_names(&self) -> impl Iterator<Item = &str> {
        self.fields.keys().map(|s| s.as_str())
    }

    /// Deep-copy this entry with a new citekey, for related-entry cloning.
    ///
    /// This mirrors Perl's `Biber::Entry::clone()`: copies all fields,
    /// names, and sets `clone=true` and `clonesourcekey`.
    pub fn clone_with_key(&self, new_key: impl Into<String>) -> Self {
        let new_key = new_key.into();
        Self {
            citekey: new_key,
            entrytype: self.entrytype.clone(),
            datasource: self.datasource.clone(),
            fields: self.fields.clone(),
            clone: true,
            set_member: self.set_member,
            names: self.names.clone(),
            clonesourcekey: Some(self.citekey.clone()),
        }
    }
}

/// A collection of entries keyed by citekey.
///
/// Ported from `lib/Biber/Entries.pm`.
#[derive(Debug, Clone, Default)]
pub struct Entries {
    entries: BTreeMap<String, Entry>,
}

impl Entries {
    /// Create an empty collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an entry.
    pub fn add_entry(&mut self, entry: Entry) {
        self.entries.insert(entry.citekey.clone(), entry);
    }

    /// Get an entry by citekey.
    pub fn get_entry(&self, citekey: &str) -> Option<&Entry> {
        self.entries.get(citekey)
    }

    /// Get a mutable entry by citekey.
    pub fn get_entry_mut(&mut self, citekey: &str) -> Option<&mut Entry> {
        self.entries.get_mut(citekey)
    }

    /// Check if an entry exists.
    pub fn has_entry(&self, citekey: &str) -> bool {
        self.entries.contains_key(citekey)
    }

    /// Get all entries.
    pub fn entries(&self) -> impl Iterator<Item = (&str, &Entry)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Get all entries (mutable).
    pub fn entries_mut(&mut self) -> impl Iterator<Item = (&str, &mut Entry)> {
        self.entries.iter_mut().map(|(k, v)| (k.as_str(), v))
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Is the collection empty?
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get all citekeys.
    pub fn citekeys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(|s| s.as_str())
    }

    /// Remove an entry by citekey, returning it if it existed.
    pub fn remove_entry(&mut self, citekey: &str) -> Option<Entry> {
        self.entries.remove(citekey)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_field_access() {
        let mut e = Entry::new("smith2020", "book");
        e.set_field_str("title", "A Book");
        e.set_field_str("year", "2020");

        assert_eq!(e.get_field_str("title"), Some("A Book"));
        assert_eq!(e.get_field_str("year"), Some("2020"));
        assert!(!e.has_field("author"));
        assert_eq!(e.field_names().count(), 2);
    }

    #[test]
    fn entries_collection() {
        let mut entries = Entries::new();
        assert!(entries.is_empty());

        entries.add_entry(Entry::new("key1", "article"));
        entries.add_entry(Entry::new("key2", "book"));
        assert_eq!(entries.len(), 2);

        assert!(entries.has_entry("key1"));
        assert!(!entries.has_entry("key3"));

        let e = entries.get_entry("key1").unwrap();
        assert_eq!(e.entrytype, "article");
    }
}
