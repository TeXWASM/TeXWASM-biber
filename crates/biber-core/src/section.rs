//! Section model — groups of cited keys and their data sources.
//!
//! Ported from `lib/Biber/Section.pm` and `lib/Biber/Sections.pm`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::annotation::AnnotationStore;
use crate::entry::Entries;

/// A datasource reference (from `<bcf:bibdata>`/`<bcf:datasource>`).
#[derive(Debug, Clone)]
pub struct DatasourceRef {
    /// Source type (usually "file").
    pub r#type: String,
    /// Source name/path.
    pub name: String,
    /// Data type ("bibtex" or "biblatexml").
    pub datatype: String,
    /// Text encoding (e.g. "UTF-8").
    pub encoding: Option<String>,
    /// Whether to glob-expand the datasource name.
    pub glob: bool,
}

/// A bibliography section: a set of cited keys and associated data.
///
/// In the non-tool-mode pipeline, sections correspond to `\refsection{}`
/// blocks. Section 0 is the default.
#[derive(Debug, Clone)]
pub struct Section {
    /// Section number.
    pub number: u32,
    /// The bibliography entries for this section.
    pub bibentries: Entries,
    /// Datasources for this section.
    datasources: Vec<DatasourceRef>,
    /// Ordered list of citekeys.
    citekeys: Vec<String>,
    /// Set of citekeys (for O(1) lookup).
    citekeys_h: BTreeSet<String>,
    /// Keys that are `\nocite`d.
    nocite_keys: BTreeSet<String>,
    /// Keys that are `\cite`d.
    cite_keys: BTreeSet<String>,
    /// Undefined keys (not found in any datasource).
    undef_citekeys: Vec<String>,
    /// Dynamic set definitions: set_key → member keys.
    dynamic_sets: HashMap<String, Vec<String>>,
    /// Citekey aliases: alias → real key.
    citekey_aliases: HashMap<String, String>,
    /// Seen-key counts (for duplicate detection).
    seenkeys: HashMap<String, u32>,
    /// Cite counts from biblatex.
    citecount: HashMap<String, u32>,
    /// Whether to use all keys (`\nocite{*}`).
    allkeys: bool,
    /// Whether allkeys was set via `\nocite{*}`.
    allkeys_nocite: bool,
    /// Original order of citekeys as they appear in the .bcf.
    pub orig_order_citekeys: Vec<String>,
    /// Keys that are used as related entries (for dependency tracking).
    related_keys: HashSet<String>,
    /// Maps original key → clone key (for related-entry cloning).
    keytorelclone: HashMap<String, String>,
    /// Maps clone key → original key (for related-entry cloning).
    relclonetokey: HashMap<String, String>,
    /// Annotations (metadata on fields/nameparts).
    pub annotations: AnnotationStore,
}

impl Section {
    /// Create a new section with the given number.
    pub fn new(number: u32) -> Self {
        Self {
            number,
            bibentries: Entries::new(),
            datasources: Vec::new(),
            citekeys: Vec::new(),
            citekeys_h: BTreeSet::new(),
            nocite_keys: BTreeSet::new(),
            cite_keys: BTreeSet::new(),
            undef_citekeys: Vec::new(),
            dynamic_sets: HashMap::new(),
            citekey_aliases: HashMap::new(),
            seenkeys: HashMap::new(),
            citecount: HashMap::new(),
            allkeys: false,
            allkeys_nocite: false,
            orig_order_citekeys: Vec::new(),
            related_keys: HashSet::new(),
            keytorelclone: HashMap::new(),
            relclonetokey: HashMap::new(),
            annotations: AnnotationStore::new(),
        }
    }

    /// Get the section number.
    pub fn number(&self) -> u32 {
        self.number
    }

    // ---- Datasources ----

    /// Get the datasources for this section.
    pub fn get_datasources(&self) -> &[DatasourceRef] {
        &self.datasources
    }

    /// Set the datasources for this section.
    pub fn set_datasources(&mut self, ds: Vec<DatasourceRef>) {
        self.datasources = ds;
    }

    /// Add a datasource.
    pub fn add_datasource(&mut self, ds: DatasourceRef) {
        self.datasources.push(ds);
    }

    // ---- Citekeys ----

    /// Get the citekeys.
    pub fn get_citekeys(&self) -> &[String] {
        &self.citekeys
    }

    /// Add citekeys.
    pub fn add_citekeys(&mut self, keys: impl IntoIterator<Item = String>) {
        for key in keys {
            if !self.citekeys_h.contains(&key) {
                self.citekeys.push(key.clone());
                self.citekeys_h.insert(key);
            }
        }
    }

    /// Add a single citekey.
    pub fn add_cite(&mut self, key: impl Into<String>) {
        let key = key.into();
        self.cite_keys.insert(key.clone());
        if !self.citekeys_h.contains(&key) {
            self.citekeys.push(key.clone());
            self.citekeys_h.insert(key);
        }
    }

    /// Add a `\nocite` key.
    pub fn add_nocite(&mut self, key: impl Into<String>) {
        let key = key.into();
        self.nocite_keys.insert(key.clone());
        if !self.citekeys_h.contains(&key) {
            self.citekeys.push(key.clone());
            self.citekeys_h.insert(key);
        }
    }

    /// Delete all citekeys (used when `allkeys` is set).
    pub fn del_citekeys(&mut self) {
        self.citekeys.clear();
        self.citekeys_h.clear();
    }

    /// Delete a citekey.
    pub fn del_citekey(&mut self, key: &str) {
        self.citekeys.retain(|k| k != key);
        self.citekeys_h.remove(key);
    }

    /// Add an undefined citekey.
    pub fn add_undef_citekey(&mut self, key: impl Into<String>) {
        self.undef_citekeys.push(key.into());
    }

    /// Get undefined citekeys.
    pub fn get_undef_citekeys(&self) -> &[String] {
        &self.undef_citekeys
    }

    // ---- Allkeys ----

    /// Check if this section uses all keys.
    pub fn is_allkeys(&self) -> bool {
        self.allkeys
    }

    /// Set the allkeys flag.
    pub fn set_allkeys(&mut self, v: bool) {
        self.allkeys = v;
    }

    /// Set the allkeys-nocite flag.
    pub fn set_allkeys_nocite(&mut self, v: bool) {
        self.allkeys_nocite = v;
    }

    /// Check if a key was `\nocite`'d.
    pub fn contains_nocite(&self, key: &str) -> bool {
        self.nocite_keys.contains(key)
    }

    /// Check if all keys were nocite'd (`\nocite{*}`).
    pub fn is_allkeys_nocite(&self) -> bool {
        self.allkeys_nocite
    }

    // ---- Seen keys ----

    /// Get the seen count of a key.
    pub fn get_seenkey(&self, key: &str) -> u32 {
        self.seenkeys.get(key).copied().unwrap_or(0)
    }

    /// Increment the seen count of a key.
    pub fn incr_seenkey(&mut self, key: impl Into<String>) {
        *self.seenkeys.entry(key.into()).or_insert(0) += 1;
    }

    // ---- Cite counts ----

    /// Set the cite count for a key.
    pub fn set_citecount(&mut self, key: impl Into<String>, count: u32) {
        self.citecount.insert(key.into(), count);
    }

    /// Get the cite count for a key (returns -1 if unset, matching Perl).
    pub fn get_citecount(&self, key: &str) -> i32 {
        self.citecount.get(key).map(|&c| c as i32).unwrap_or(-1)
    }

    // ---- Dynamic sets ----

    /// Set a dynamic set definition.
    pub fn set_dynamic_set(&mut self, key: impl Into<String>, members: Vec<String>) {
        self.dynamic_sets.insert(key.into(), members);
    }

    /// Get a dynamic set definition.
    pub fn get_dynamic_set(&self, key: &str) -> Option<&Vec<String>> {
        self.dynamic_sets.get(key)
    }

    // ---- Citekey aliases ----

    /// Set a citekey alias.
    pub fn set_citekey_alias(&mut self, alias: impl Into<String>, real: impl Into<String>) {
        self.citekey_aliases.insert(alias.into(), real.into());
    }

    /// Get a citekey alias.
    pub fn get_citekey_alias(&self, alias: &str) -> Option<&str> {
        self.citekey_aliases.get(alias).map(|s| s.as_str())
    }

    /// Get all citekey aliases (alias, real_key) pairs.
    pub fn get_citekey_aliases(&self) -> impl Iterator<Item = (&str, &str)> {
        self.citekey_aliases
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Check if a key is cited (in the `\cite` or `\nocite` list).
    pub fn is_cited(&self, key: &str) -> bool {
        self.cite_keys.contains(key) || self.nocite_keys.contains(key)
    }

    // ---- Related entry tracking ----

    /// Record a key as a related entry.
    pub fn add_related(&mut self, key: impl Into<String>) {
        self.related_keys.insert(key.into());
    }

    /// Check if a key is a related entry.
    pub fn is_related(&self, key: &str) -> bool {
        self.related_keys.contains(key)
    }

    /// Set a key → clone key mapping.
    pub fn set_keytorelclone(&mut self, key: impl Into<String>, clonekey: impl Into<String>) {
        let k = key.into();
        let ck = clonekey.into();
        self.keytorelclone.insert(k.clone(), ck.clone());
        self.relclonetokey.insert(ck, k);
    }

    /// Get the clone key for an original key.
    pub fn get_keytorelclone(&self, key: &str) -> Option<&str> {
        self.keytorelclone.get(key).map(|s| s.as_str())
    }

    /// Get the original key for a clone key.
    pub fn get_relclonetokey(&self, clonekey: &str) -> Option<&str> {
        self.relclonetokey.get(clonekey).map(|s| s.as_str())
    }

    /// Check if an original key has a clone.
    pub fn has_keytorelclone(&self, key: &str) -> bool {
        self.keytorelclone.contains_key(key)
    }

    /// Check if a clone key has an original.
    pub fn has_relclonetokey(&self, clonekey: &str) -> bool {
        self.relclonetokey.contains_key(clonekey)
    }

    // ---- Static citekeys (non-dynamic) ----

    /// Get citekeys that are not dynamic set definitions.
    pub fn get_static_citekeys(&self) -> Vec<&str> {
        self.citekeys
            .iter()
            .filter(|k| !self.dynamic_sets.contains_key(*k))
            .map(|s| s.as_str())
            .collect()
    }

    // ---- Entry access ----

    /// Get a bib entry by citekey.
    pub fn bibentry(&self, citekey: &str) -> Option<&crate::entry::Entry> {
        self.bibentries.get_entry(citekey)
    }

    /// Get a mutable bib entry by citekey.
    pub fn bibentry_mut(&mut self, citekey: &str) -> Option<&mut crate::entry::Entry> {
        self.bibentries.get_entry_mut(citekey)
    }

    /// Get the number of cited keys.
    pub fn num_citekeys(&self) -> usize {
        self.citekeys.len()
    }
}

/// A collection of sections keyed by section number.
///
/// Ported from `lib/Biber/Sections.pm`.
#[derive(Debug, Clone, Default)]
pub struct Sections {
    sections: BTreeMap<u32, Section>,
}

impl Sections {
    /// Create an empty collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a section.
    pub fn add_section(&mut self, section: Section) {
        self.sections.insert(section.number, section);
    }

    /// Get a section by number.
    pub fn get_section(&self, number: u32) -> Option<&Section> {
        self.sections.get(&number)
    }

    /// Get a mutable section by number.
    pub fn get_section_mut(&mut self, number: u32) -> Option<&mut Section> {
        self.sections.get_mut(&number)
    }

    /// Get all sections, sorted by number.
    pub fn get_sections(&self) -> Vec<&Section> {
        self.sections.values().collect()
    }

    /// Get all sections (mutable), sorted by number.
    pub fn get_sections_mut(&mut self) -> Vec<&mut Section> {
        self.sections.values_mut().collect()
    }

    /// Number of sections.
    pub fn len(&self) -> usize {
        self.sections.len()
    }

    /// Is the collection empty?
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    /// Delete a section.
    pub fn delete_section(&mut self, number: u32) {
        self.sections.remove(&number);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn section_citekeys() {
        let mut s = Section::new(0);
        s.add_cite("key1");
        s.add_cite("key2");
        s.add_cite("key1"); // duplicate
        assert_eq!(s.get_citekeys().len(), 2);
        assert_eq!(s.get_citekeys()[0], "key1");
    }

    #[test]
    fn section_allkeys() {
        let mut s = Section::new(0);
        s.add_cite("key1");
        s.set_allkeys(true);
        s.del_citekeys();
        assert!(s.is_allkeys());
        assert_eq!(s.get_citekeys().len(), 0);
    }

    #[test]
    fn sections_collection() {
        let mut ss = Sections::new();
        ss.add_section(Section::new(0));
        ss.add_section(Section::new(1));
        assert_eq!(ss.len(), 2);
        assert!(ss.get_section(0).is_some());
        assert!(ss.get_section(2).is_none());
    }
}
