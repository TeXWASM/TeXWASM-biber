//! Data list model — output lists with sorting and filtering.
//!
//! Ported from `lib/Biber/DataList.pm` and `lib/Biber/DataLists.pm`.

use std::collections::HashMap;

/// Label disambiguation cache for "v" mode (per-name variable width).
///
/// Maps a raw field string to its disambiguation data: a list of substrings
/// of increasing lengths and the index of the minimal disambiguating length.
#[derive(Debug, Clone, Default)]
pub struct LabelCacheV {
    /// Per-string disambiguation data: raw_string → { data, index, nameindex }.
    pub entries: HashMap<String, LabelCacheVEntry>,
    /// Global index overrides (for "f" mode).
    pub global_indices: HashMap<String, usize>,
}

/// A single entry in the "v" mode label cache.
#[derive(Debug, Clone, Default)]
pub struct LabelCacheVEntry {
    /// Substrings of increasing length for this string.
    pub data: Vec<String>,
    /// Index into `data` for the minimal disambiguating length.
    pub index: usize,
    /// The name index (for name fields).
    pub nameindex: u32,
}

/// Label disambiguation cache for "l" mode (list-wide).
///
/// A 2D array: data[entry_index][name_index] = disambiguated substring.
#[derive(Debug, Clone, Default)]
pub struct LabelCacheL {
    /// Disambiguated substrings: data[entry_idx][name_idx].
    pub data: Vec<Vec<String>>,
}

/// A filter on a data list (from `<bcf:filter>` or `<bcf:filteror>`).
#[derive(Debug, Clone)]
pub struct ListFilter {
    /// Filter type (e.g. "type", "category", "keyword").
    pub r#type: String,
    /// Filter value.
    pub value: String,
}

/// A disjunctive filter group (from `<bcf:filteror>`).
#[derive(Debug, Clone)]
pub struct ListFilterOr {
    /// The disjunctive filters.
    pub filters: Vec<ListFilter>,
}

/// A data list definition (from `<bcf:datalist>`).
///
/// Each data list specifies a sorting template, name key template, etc.
/// and holds the entries that pass its filters, sorted accordingly.
#[derive(Debug, Clone)]
pub struct DataList {
    /// Section number this list belongs to.
    pub section: u32,
    /// Sorting template name.
    pub sortingtemplatename: String,
    /// Sorting name key template name.
    pub sortingnamekeytemplatename: String,
    /// Unique name template name.
    pub uniquenametemplatename: String,
    /// Label alpha name template name.
    pub labelalphanametemplatename: String,
    /// Name hash template name.
    pub namehashtemplatename: String,
    /// Label prefix.
    pub labelprefix: String,
    /// List name.
    pub name: String,
    /// List type ("entry", "shorthand", etc.).
    pub r#type: String,
    /// Filters on this list.
    pub filters: Vec<ListFilter>,
    /// Disjunctive filter groups.
    pub filterors: Vec<ListFilterOr>,
    /// Per-list state (populated during processing, not from BCF).
    pub state: DataListState,
}

impl DataList {
    /// Create a new data list with the given attributes.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        section: u32,
        sortingtemplatename: impl Into<String>,
        sortingnamekeytemplatename: impl Into<String>,
        uniquenametemplatename: impl Into<String>,
        labelalphanametemplatename: impl Into<String>,
        namehashtemplatename: impl Into<String>,
        labelprefix: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            section,
            sortingtemplatename: sortingtemplatename.into(),
            sortingnamekeytemplatename: sortingnamekeytemplatename.into(),
            uniquenametemplatename: uniquenametemplatename.into(),
            labelalphanametemplatename: labelalphanametemplatename.into(),
            namehashtemplatename: namehashtemplatename.into(),
            labelprefix: labelprefix.into(),
            name: name.into(),
            r#type: "entry".to_string(),
            filters: Vec::new(),
            filterors: Vec::new(),
            state: DataListState::default(),
        }
    }

    /// Set the list type.
    pub fn set_type(&mut self, t: impl Into<String>) {
        self.r#type = t.into();
    }

    /// Set the list name.
    pub fn set_name(&mut self, n: impl Into<String>) {
        self.name = n.into();
    }

    /// Add a filter.
    pub fn add_filter(&mut self, filter: ListFilter) {
        self.filters.push(filter);
    }

    /// Add a disjunctive filter group.
    pub fn add_filteror(&mut self, filteror: ListFilterOr) {
        self.filterors.push(filteror);
    }
}

/// Per-list processing state (not from BCF).
#[derive(Debug, Clone, Default)]
pub struct DataListState {
    /// Entries in this list (citekeys in sort order).
    pub entries: Vec<String>,
    /// Seen primary-author counts.
    pub seenpa: HashMap<String, HashMap<String, bool>>,
    /// Per-entry sortinit values (first character of sort key).
    pub sortinit: HashMap<String, String>,
    /// Sortinit hashes.
    pub sortinithash: HashMap<String, String>,
    /// Per-entry presort values.
    pub presort: HashMap<String, String>,
    /// Seen base-name strings for name disambiguation.
    /// Maps labelname_source -> (citekey -> Vec<base_string_per_name_index>)
    pub seen_namedis_bases: HashMap<String, HashMap<String, Vec<String>>>,
    /// Extradate tracking: citekey -> tracking_string.
    pub nametitledateparts: HashMap<String, String>,
    /// Extradate tracking: tracking_string -> seen count.
    pub seen_nametitledateparts: HashMap<String, u32>,
    /// Extradate per-group letter counter: tracking_string -> counter.
    pub seen_extradate: HashMap<String, u32>,
    /// Extradate letter counter: citekey -> letter index (1='a', 2='b', ...).
    pub extradatedata: HashMap<String, u32>,

    // ---- Label alpha / extraalpha tracking ----
    /// Count of entries sharing each labelalpha string (disambiguation).
    pub ladisambiguation: HashMap<String, u32>,
    /// Per-labelalpha counter for extraalpha assignment.
    pub seen_extraalpha: HashMap<String, u32>,
    /// Stored labelalpha per citekey (final output).
    pub labelalphadata: HashMap<String, String>,
    /// Stored sortlabelalpha per citekey (for sorting, no markup).
    pub sortlabelalphadata: HashMap<String, String>,
    /// Stored extraalpha per citekey (final output).
    pub extraalphadata: HashMap<String, String>,
    /// Label disambiguation cache for "v" mode (per-name variable width): field → cache.
    pub labelcache_v: HashMap<String, LabelCacheV>,
    /// Label disambiguation cache for "l" mode (list-wide): field → cache.
    pub labelcache_l: HashMap<String, LabelCacheL>,
    /// Visible alpha name count per namelist ID (e.g. "author" → 3).
    pub visible_alpha: HashMap<String, u32>,
    /// Whether a namelist has more names than visible ("et al." marker).
    pub morenames: HashMap<String, bool>,

    // ---- Extra name / title tracking ----
    /// Count of entries sharing a labelnamehash (for extraname).
    pub seen_labelname: HashMap<String, u32>,
    /// Count of entries sharing a (namehash,title) combo (for extratitle).
    pub seen_nametitle: HashMap<String, u32>,
    /// Count of entries sharing a (title,year) combo (for extratitleyear).
    pub seen_titleyear: HashMap<String, u32>,
    /// Stored extraname per citekey.
    pub extranamedata: HashMap<String, String>,
    /// Stored extratitle per citekey.
    pub extratitledata: HashMap<String, String>,
    /// Stored extratitleyear per citekey.
    pub extratitleyeardata: HashMap<String, String>,
    /// Per-entry labelnamehash (stored on datalist state during processing).
    pub labelnamehash: HashMap<String, String>,
    /// Per-entry nametitle string (stored on datalist state during processing).
    pub nametitle: HashMap<String, String>,
    /// Per-entry titleyear string (stored on datalist state during processing).
    pub titleyear: HashMap<String, String>,

    // ---- Work uniqueness tracking ----
    /// Count of entries sharing a fullhash of labelname (for singletitle).
    pub seenname: HashMap<String, u32>,
    /// Count of entries sharing a labeltitle value (for uniquetitle).
    pub seentitle: HashMap<String, u32>,
    /// Count of entries sharing a labeltitle when no labelname (for uniquebaretitle).
    pub seenbaretitle: HashMap<String, u32>,
    /// Count of entries sharing fullhash+labeltitle (for uniquework).
    pub seenwork: HashMap<String, u32>,
    /// Per-entry resolved labelprefix (from shorthand or datalist attribute).
    pub labelprefix_data: HashMap<String, String>,
}

/// A collection of data lists.
///
/// Ported from `lib/Biber/DataLists.pm`.
#[derive(Debug, Clone, Default)]
pub struct DataLists {
    lists: Vec<DataList>,
}

impl DataLists {
    /// Create an empty collection.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a data list.
    pub fn add_list(&mut self, list: DataList) {
        self.lists.push(list);
    }

    /// Get all lists.
    pub fn get_lists(&self) -> &[DataList] {
        &self.lists
    }

    /// Get lists for a specific section.
    pub fn get_lists_for_section(&self, section: u32) -> Vec<&DataList> {
        self.lists.iter().filter(|l| l.section == section).collect()
    }

    /// Get mutable lists for a specific section.
    pub fn get_lists_for_section_mut(&mut self, section: u32) -> Vec<&mut DataList> {
        self.lists
            .iter_mut()
            .filter(|l| l.section == section)
            .collect()
    }

    /// Check if a list with the given attributes already exists.
    pub fn has_list(
        &self,
        section: u32,
        name: &str,
        list_type: &str,
        sortingtemplatename: &str,
    ) -> bool {
        self.lists.iter().any(|l| {
            l.section == section
                && l.name == name
                && l.r#type == list_type
                && l.sortingtemplatename == sortingtemplatename
        })
    }

    /// Number of lists.
    pub fn len(&self) -> usize {
        self.lists.len()
    }

    /// Is the collection empty?
    pub fn is_empty(&self) -> bool {
        self.lists.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datalist_creation() {
        let dl = DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        assert_eq!(dl.section, 0);
        assert_eq!(dl.sortingtemplatename, "nty");
        assert_eq!(dl.r#type, "entry");
    }

    #[test]
    fn datalists_collection() {
        let mut dls = DataLists::new();
        assert!(dls.is_empty());

        dls.add_list(DataList::new(
            0, "nty", "global", "global", "global", "global", "", "list1",
        ));
        dls.add_list(DataList::new(
            1, "nyt", "global", "global", "global", "global", "", "list2",
        ));
        assert_eq!(dls.len(), 2);

        let s0 = dls.get_lists_for_section(0);
        assert_eq!(s0.len(), 1);
    }
}
