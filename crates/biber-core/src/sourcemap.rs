//! Sourcemap application — `\DeclareSourcemap` logic.
//!
//! Sourcemaps define transformations applied to entries at the datasource
//! level: field renaming, entry-type mapping, regex match/replace, field
//! setting, entry cloning, and entry/field nullification.
//!
//! Sourcemaps are parsed from `<bcf:sourcemap>` in `.bcf` files or from
//! `<sourcemap>` in `biber.conf`, stored as `ConfigValue::Raw` XML, and
//! re-parsed here at application time.

use regex::Regex;
use roxmltree::Document;
use tracing::debug;

use crate::config::ConfigValue;
use crate::processor::Biber;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A parsed sourcemap (`<sourcemap>` → `<maps>` → `<map>` + `<map_step>`).
#[derive(Debug, Clone, Default)]
pub struct Sourcemap {
    /// The ordered list of `<map>` elements.
    pub maps: Vec<SourcemapMap>,
}

/// A `<map>` within a sourcemap `<maps>` block.
#[derive(Debug, Clone)]
pub struct SourcemapMap {
    /// `map_overwrite` — whether this map can overwrite existing fields.
    pub overwrite: bool,
    /// `<per_type>` — restrict to entries of this type (case-insensitive).
    pub per_type: Option<String>,
    /// `<per_datasource>` — restrict to entries from this datasource.
    pub per_datasource: Option<String>,
    /// The ordered list of `<map_step>` elements.
    pub steps: Vec<SourcemapStep>,
}

/// A single `<map_step>` within a `<map>`.
#[derive(Debug, Clone, Default)]
pub struct SourcemapStep {
    // ---- Field rename ----
    /// `map_field_source` — source field name for renaming.
    pub field_source: Option<String>,
    /// `map_field_target` — target field name for renaming.
    pub field_target: Option<String>,

    // ---- Entry type rename ----
    /// `map_type_source` — source entry type for type renaming.
    pub type_source: Option<String>,
    /// `map_type_target` — target entry type for type renaming.
    pub type_target: Option<String>,

    // ---- Field set ----
    /// `map_field_set` — field name to set a value on.
    pub field_set: Option<String>,
    /// `map_field_value` — value to assign to `field_set`.
    pub field_value: Option<String>,

    // ---- Regex match / replace ----
    /// `map_match` or `map_matchi` — regex pattern to match against a field value.
    pub match_pattern: Option<String>,
    /// Whether the match is case-insensitive (i.e. `map_matchi` was used).
    pub match_case_insensitive: bool,
    /// `map_replace` — replacement string for regex substitution.
    pub replace: Option<String>,

    // ---- Entry operations ----
    /// `map_entry_null` — if true, remove the entire entry.
    pub entry_null: bool,
    /// `map_entry_clone` — template for the new citekey when cloning an entry.
    pub entry_clone: Option<String>,
    /// `map_entry_new` — create a new entry with this citekey.
    pub entry_new: Option<String>,
    /// `map_entry_newtype` — new entry type when creating a new entry.
    pub entry_newtype: Option<String>,

    // ---- Field operations ----
    /// `map_null` — if true, remove the field named by `field_source`.
    pub field_null: bool,
    /// `map_notfield` — skip this step if the named field exists.
    pub notfield: Option<String>,

    // ---- Entry key filters ----
    /// `map_entrykey_cited` — only apply if the entry key is cited.
    pub key_cited: bool,
    /// `map_entrykey_nocited` — only apply if the entry key is not cited.
    pub key_nocited: bool,
    /// `map_entrykey_allnocited` — only apply if all entry keys are not cited.
    pub key_allnocited: bool,
    /// `map_entrykey_citedornocited` — apply regardless of cited status.
    pub key_citedornocited: bool,

    // ---- Control ----
    /// `map_final` — if true, stop processing further steps after this one.
    pub final_: bool,
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse a `<sourcemap>` XML string into a `Sourcemap` value.
pub fn parse_sourcemap_xml(raw: &str) -> Sourcemap {
    let mut sm = Sourcemap::default();
    let doc = match Document::parse(raw) {
        Ok(d) => d,
        Err(_) => return sm,
    };
    let root = doc.root();
    // Accept either bare `<maps>` or `<sourcemap>` wrapping
    let maps_nodes: Vec<_> = if raw.trim().starts_with("<sourcemap") {
        root.descendants()
            .filter(|n| n.has_tag_name("maps"))
            .collect()
    } else {
        root.descendants()
            .filter(|n| n.has_tag_name("maps"))
            .collect()
    };

    for maps_node in &maps_nodes {
        let overwrite = maps_node
            .attribute("map_overwrite")
            .map(|v| v == "1")
            .unwrap_or(true);

        for map_node in maps_node.descendants().filter(|n| n.has_tag_name("map")) {
            let mm = parse_single_map(map_node, overwrite);
            sm.maps.push(mm);
        }
    }

    sm
}

fn parse_single_map(map_node: roxmltree::Node, default_overwrite: bool) -> SourcemapMap {
    let per_type = map_node
        .descendants()
        .find(|n| n.has_tag_name("per_type"))
        .and_then(|n| n.text())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let per_datasource = map_node
        .descendants()
        .find(|n| n.has_tag_name("per_datasource"))
        .and_then(|n| n.text())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let steps: Vec<SourcemapStep> = map_node
        .descendants()
        .filter(|n| n.has_tag_name("map_step"))
        .map(parse_step)
        .collect();

    SourcemapMap {
        overwrite: map_node
            .attribute("map_overwrite")
            .map(|v| v == "1")
            .unwrap_or(default_overwrite),
        per_type,
        per_datasource,
        steps,
    }
}

fn parse_step(step_node: roxmltree::Node) -> SourcemapStep {
    let a = |name: &str| step_node.attribute(name).map(|s| s.to_string());
    let ab = |name: &str| step_node.attribute(name) == Some("1");

    SourcemapStep {
        field_source: a("map_field_source"),
        field_target: a("map_field_target"),
        type_source: a("map_type_source"),
        type_target: a("map_type_target"),
        field_set: a("map_field_set"),
        field_value: a("map_field_value"),
        match_pattern: a("map_match").or_else(|| {
            if step_node.attribute("map_matchi").is_some() {
                a("map_matchi")
            } else {
                None
            }
        }),
        match_case_insensitive: step_node.attribute("map_matchi").is_some(),
        replace: a("map_replace"),
        entry_null: ab("map_entry_null"),
        entry_clone: a("map_entry_clone"),
        entry_new: a("map_entry_new"),
        entry_newtype: a("map_entry_newtype"),
        field_null: ab("map_null"),
        notfield: a("map_notfield"),
        key_cited: ab("map_entrykey_cited"),
        key_nocited: ab("map_entrykey_nocited"),
        key_allnocited: ab("map_entrykey_allnocited"),
        key_citedornocited: ab("map_entrykey_citedornocited"),
        final_: ab("map_final"),
    }
}

// ---------------------------------------------------------------------------
// Application
// ---------------------------------------------------------------------------

/// Retrieve and parse the sourcemap from the biber config, then apply it
/// to all entries in the given section.
pub fn apply_sourcemap(biber: &mut Biber, secnum: u32) {
    let raw = match biber.config.getoption("sourcemap") {
        Some(ConfigValue::Raw(xml)) => xml.clone(),
        _ => return,
    };

    if raw.trim().is_empty() {
        return;
    }

    debug!("Applying sourcemap for section {secnum}");
    let sm = parse_sourcemap_xml(&raw);
    apply_sourcemap_to_section(biber, secnum, &sm);
}

/// Apply a parsed sourcemap to all entries in a section.
fn apply_sourcemap_to_section(biber: &mut Biber, secnum: u32, sm: &Sourcemap) {
    for mm in &sm.maps {
        apply_map(biber, secnum, mm);
    }
}

/// Apply a single `<map>` to all matching entries in the section.
fn apply_map(biber: &mut Biber, secnum: u32, mm: &SourcemapMap) {
    let keys: Vec<String> = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        section.get_citekeys().to_vec()
    };

    for key in &keys {
        let (entry_type, datasource) = {
            let section = match biber.sections.get_section(secnum) {
                Some(s) => s,
                None => return,
            };
            match section.bibentries.get_entry(key) {
                Some(e) => (e.entrytype.clone(), e.datasource.clone()),
                None => continue,
            }
        };

        // Filter: per_type
        if let Some(ref pt) = mm.per_type {
            if !entry_type.eq_ignore_ascii_case(pt) {
                continue;
            }
        }

        // Filter: per_datasource
        if let Some(ref pd) = mm.per_datasource {
            if datasource != *pd {
                // Accept if datasource ends with the filter (filename match)
                if !datasource.ends_with(pd) {
                    continue;
                }
            }
        }

        apply_map_entry(biber, secnum, key, mm);
    }
}

/// Apply a single `<map>` to one entry.
fn apply_map_entry(biber: &mut Biber, secnum: u32, key: &str, mm: &SourcemapMap) {
    for step in &mm.steps {
        if apply_step(biber, secnum, key, step, mm.overwrite) {
            // map_final = 1 means stop processing further steps
            if step.final_ {
                break;
            }
        }
    }
}

/// Apply a single `<map_step>` to an entry.
///
/// Returns `true` if the step was applied (matched), `false` if it was
/// skipped due to filters not matching. This matters for `map_final`.
fn apply_step(
    biber: &mut Biber,
    secnum: u32,
    key: &str,
    step: &SourcemapStep,
    overwrite: bool,
) -> bool {
    let section = match biber.sections.get_section(secnum) {
        Some(s) => s,
        None => return false,
    };
    let entry = match section.bibentries.get_entry(key) {
        Some(e) => e,
        None => return false,
    };

    // ---- Entry key filters ----
    let is_cited = section.is_cited(key);
    if step.key_cited && !is_cited {
        return false;
    }
    if step.key_nocited && is_cited {
        return false;
    }
    if step.key_allnocited && is_cited {
        return false;
    }
    if step.key_citedornocited {
        // Always matches (cited OR nocited)
    }

    // ---- map_notfield: skip if the named field EXISTS ----
    if let Some(ref nf) = step.notfield {
        if entry.get_field_str(nf).is_some() {
            return false;
        }
    }

    // ---- map_type_source / map_type_target ----
    if let (Some(ref ts), Some(ref tt)) = (&step.type_source, &step.type_target) {
        if entry.entrytype.eq_ignore_ascii_case(ts) {
            if let Some(section) = biber.sections.get_section_mut(secnum) {
                if let Some(be) = section.bibentries.get_entry_mut(key) {
                    be.entrytype = tt.clone();
                }
            }
        }
    }

    // ---- map_field_source / map_field_target (field rename) ----
    if let (Some(ref fs), Some(ref ft)) = (&step.field_source, &step.field_target) {
        if let Some(section) = biber.sections.get_section_mut(secnum) {
            if let Some(be) = section.bibentries.get_entry_mut(key) {
                if let Some(val) = be.del_field(fs) {
                    if overwrite || !be.has_field(ft) {
                        be.set_field(ft.to_string(), val);
                    } else {
                        // Target exists and overwrite is false — put the original back
                        be.set_field(fs.to_string(), val);
                    }
                }
            }
        }
    }

    // ---- map_match / map_replace ----
    if let (Some(ref mp), Some(ref rp)) = (&step.match_pattern, &step.replace) {
        // Determine which field to match against
        let target_field = step.field_source.as_deref().or(step.field_set.as_deref());
        if let Some(fname) = target_field {
            if let Some(section) = biber.sections.get_section_mut(secnum) {
                if let Some(be) = section.bibentries.get_entry_mut(key) {
                    if let Some(val) = be.get_field_str(fname).map(|s| s.to_string()) {
                        let re_flags = if step.match_case_insensitive {
                            "(?i)"
                        } else {
                            ""
                        };
                        let re_str = format!("{}{}", re_flags, mp);
                        if let Ok(re) = Regex::new(&re_str) {
                            let new_val = re.replace_all(&val, rp.as_str()).to_string();
                            if new_val != val {
                                let actual_target = step.field_set.as_deref().unwrap_or(fname);
                                be.set_field_str(actual_target, &new_val);
                            }
                        }
                    }
                }
            }
        }
    }

    // ---- map_field_set / map_field_value ----
    if let (Some(ref fs), Some(ref fv)) = (&step.field_set, &step.field_value) {
        if let Some(section) = biber.sections.get_section_mut(secnum) {
            if let Some(be) = section.bibentries.get_entry_mut(key) {
                if overwrite || be.get_field_str(fs).is_none() {
                    be.set_field_str(fs, fv);
                }
            }
        }
    }

    // ---- map_null (remove field) ----
    if step.field_null {
        if let Some(ref fs) = step.field_source {
            if let Some(section) = biber.sections.get_section_mut(secnum) {
                if let Some(be) = section.bibentries.get_entry_mut(key) {
                    be.del_field(fs);
                }
            }
        }
    }

    // ---- map_entry_null (remove entry) ----
    if step.entry_null {
        if let Some(section) = biber.sections.get_section_mut(secnum) {
            section.bibentries.remove_entry(key);
            section.del_citekey(key);
        }
        return true;
    }

    // ---- map_entry_clone ----
    if let Some(ref clone_tmpl) = step.entry_clone {
        // Evaluate template (simple $1 substitution from match groups)
        let new_key = evaluate_template(clone_tmpl, step.match_pattern.as_deref(), key);
        if !new_key.is_empty() && new_key != key {
            if let Some(section) = biber.sections.get_section_mut(secnum) {
                if let Some(original) = section.bibentries.get_entry(key) {
                    let mut cloned = original.clone();
                    cloned.citekey = new_key.clone();
                    cloned.clone = true;
                    section.bibentries.add_entry(cloned);
                    section.add_citekeys(vec![new_key.clone()]);
                }
            }
        }
    }

    true
}

/// Simple template evaluation: `$1`, `$2`, etc. from capture groups,
/// `$ENTRYKEY` → original citekey, and `$MAPLOOP` as a passthrough.
fn evaluate_template(template: &str, match_pattern: Option<&str>, original_key: &str) -> String {
    // Replace $ENTRYKEY with the original citekey (the $ is part of the variable name)
    let template = template.replace("$ENTRYKEY", original_key);

    if !template.contains('$') {
        return template;
    }

    match match_pattern {
        Some(pat) => {
            let full_pat = format!("^{}$", pat);
            if let Ok(re) = Regex::new(&full_pat) {
                if let Some(caps) = re.captures(original_key) {
                    let mut result = template.to_string();
                    for i in (1..caps.len()).rev() {
                        let placeholder = format!("${}", i);
                        if let Some(val) = caps.get(i) {
                            result = result.replace(&placeholder, val.as_str());
                        }
                    }
                    return result;
                }
            }
            template
        }
        None => template,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datalist::DataList;
    use crate::entry::Entry;
    use crate::section::Section;

    fn make_biber() -> Biber {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.set_allkeys(true);

        let mut entry = Entry::new("test1", "article");
        entry.set_field_str("author", "Smith");
        entry.set_field_str("title", "A Great Paper");
        entry.set_field_str("journal", "Some Journal");
        entry.datasource = "refs.bib".to_string();
        section.bibentries.add_entry(entry);

        let mut entry2 = Entry::new("test2", "book");
        entry2.set_field_str("author", "Jones");
        entry2.set_field_str("title", "A Book");
        entry2.datasource = "refs.bib".to_string();
        section.bibentries.add_entry(entry2);

        section.add_citekeys(vec!["test1".to_string(), "test2".to_string()]);
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
        dl.state.entries = vec!["test1".to_string(), "test2".to_string()];
        biber.datalists.add_list(dl);
        biber
    }

    #[test]
    fn empty_sourcemap_does_nothing() {
        let sm = parse_sourcemap_xml("");
        assert!(sm.maps.is_empty());
    }

    #[test]
    fn parse_sourcemap_with_maps_tag() {
        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_field_source="usera" map_field_target="userd"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        assert_eq!(sm.maps.len(), 1);
        assert_eq!(sm.maps[0].steps.len(), 1);
    }

    #[test]
    fn parse_sourcemap_with_sourcemap_wrapper() {
        let xml = r#"<sourcemap>
            <maps datatype="bibtex">
                <map>
                    <map_step map_field_source="usera" map_field_target="userd"/>
                </map>
            </maps>
        </sourcemap>"#;
        let sm = parse_sourcemap_xml(xml);
        assert_eq!(sm.maps.len(), 1);
    }

    #[test]
    fn parse_per_type_and_per_datasource() {
        let xml = r#"<maps datatype="bibtex">
            <map>
                <per_type>ARTICLE</per_type>
                <per_datasource>test.bib</per_datasource>
                <map_step map_field_source="author" map_field_target="editor"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        let m = &sm.maps[0];
        assert_eq!(m.per_type.as_deref(), Some("ARTICLE"));
        assert_eq!(m.per_datasource.as_deref(), Some("test.bib"));
    }

    #[test]
    fn parse_map_overwrite_defaults_true() {
        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_field_source="a" map_field_target="b"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        assert!(sm.maps[0].overwrite);
    }

    #[test]
    fn parse_map_overwrite_false() {
        let xml = r#"<maps datatype="bibtex" map_overwrite="1">
            <map map_overwrite="0">
                <map_step map_field_source="a" map_field_target="b"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        assert!(!sm.maps[0].overwrite);
    }

    #[test]
    fn parse_step_various_attributes() {
        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_field_source="author" map_field_target="editor"
                    map_type_source="article" map_type_target="misc"
                    map_match="Smith" map_replace="Jones"
                    map_final="1" map_null="1"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        let step = &sm.maps[0].steps[0];
        assert_eq!(step.field_source.as_deref(), Some("author"));
        assert_eq!(step.field_target.as_deref(), Some("editor"));
        assert_eq!(step.type_source.as_deref(), Some("article"));
        assert_eq!(step.type_target.as_deref(), Some("misc"));
        assert_eq!(step.match_pattern.as_deref(), Some("Smith"));
        assert_eq!(step.replace.as_deref(), Some("Jones"));
        assert!(step.final_);
        assert!(step.field_null);
    }

    #[test]
    fn parse_map_entry_null() {
        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_entry_null="1"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        assert!(sm.maps[0].steps[0].entry_null);
    }

    #[test]
    fn parse_map_entry_clone() {
        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_entry_clone="clone-$1"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        assert_eq!(sm.maps[0].steps[0].entry_clone.as_deref(), Some("clone-$1"));
    }

    // ---- Application tests ----

    #[test]
    fn field_rename() {
        let mut biber = make_biber();
        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_field_source="author" map_field_target="editor"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        apply_sourcemap_to_section(&mut biber, 0, &sm);
        let section = biber.sections.get_section(0).unwrap();
        let entry = section.bibentries.get_entry("test1").unwrap();
        assert!(entry.get_field_str("author").is_none());
        assert_eq!(entry.get_field_str("editor"), Some("Smith"));
    }

    #[test]
    fn entry_type_rename() {
        let mut biber = make_biber();
        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_type_source="article" map_type_target="misc"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        apply_sourcemap_to_section(&mut biber, 0, &sm);
        let section = biber.sections.get_section(0).unwrap();
        let entry = section.bibentries.get_entry("test1").unwrap();
        assert_eq!(entry.entrytype, "misc");
        // test2 (book) unaffected
        let entry2 = section.bibentries.get_entry("test2").unwrap();
        assert_eq!(entry2.entrytype, "book");
    }

    #[test]
    fn field_set_value() {
        let mut biber = make_biber();
        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_field_set="note" map_field_value="A note"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        apply_sourcemap_to_section(&mut biber, 0, &sm);
        let section = biber.sections.get_section(0).unwrap();
        let entry = section.bibentries.get_entry("test1").unwrap();
        assert_eq!(entry.get_field_str("note"), Some("A note"));
    }

    #[test]
    fn per_type_filter() {
        let mut biber = make_biber();
        let xml = r#"<maps datatype="bibtex">
            <map>
                <per_type>ARTICLE</per_type>
                <map_step map_field_source="author" map_field_target="editor"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        apply_sourcemap_to_section(&mut biber, 0, &sm);
        let section = biber.sections.get_section(0).unwrap();
        // test1 is article → renamed
        assert!(section
            .bibentries
            .get_entry("test1")
            .unwrap()
            .get_field_str("author")
            .is_none());
        // test2 is book → not renamed
        assert_eq!(
            section
                .bibentries
                .get_entry("test2")
                .unwrap()
                .get_field_str("author"),
            Some("Jones")
        );
    }

    #[test]
    fn regex_replace() {
        let mut biber = make_biber();
        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_field_source="title" map_match="Great" map_replace="Wonderful"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        apply_sourcemap_to_section(&mut biber, 0, &sm);
        let section = biber.sections.get_section(0).unwrap();
        let entry = section.bibentries.get_entry("test1").unwrap();
        assert_eq!(entry.get_field_str("title"), Some("A Wonderful Paper"));
    }

    #[test]
    fn map_field_null_removes_field() {
        let mut biber = make_biber();
        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_field_source="title" map_null="1"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        apply_sourcemap_to_section(&mut biber, 0, &sm);
        let section = biber.sections.get_section(0).unwrap();
        let entry = section.bibentries.get_entry("test1").unwrap();
        assert!(entry.get_field_str("title").is_none());
    }

    #[test]
    fn map_entry_null_removes_entry() {
        let mut biber = make_biber();
        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_entry_null="1"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        apply_sourcemap_to_section(&mut biber, 0, &sm);
        let section = biber.sections.get_section(0).unwrap();
        assert!(section.bibentries.get_entry("test1").is_none());
        assert!(!section.get_citekeys().contains(&"test1".to_string()));
    }

    #[test]
    fn map_entry_clone_creates_duplicate() {
        let mut biber = make_biber();
        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_entry_clone="copy-$ENTRYKEY"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        apply_sourcemap_to_section(&mut biber, 0, &sm);
        let section = biber.sections.get_section(0).unwrap();
        let cloned = section.bibentries.get_entry("copy-test1");
        assert!(cloned.is_some());
        assert_eq!(cloned.unwrap().get_field_str("author"), Some("Smith"));
        assert!(section.get_citekeys().contains(&"copy-test1".to_string()));
    }

    #[test]
    fn map_final_stops_processing() {
        let mut biber = make_biber();
        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_field_set="note" map_field_value="first" map_final="1"/>
                <map_step map_field_set="note" map_field_value="second"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        apply_sourcemap_to_section(&mut biber, 0, &sm);
        let section = biber.sections.get_section(0).unwrap();
        let entry = section.bibentries.get_entry("test1").unwrap();
        // Should be "first" because final stops the second step
        assert_eq!(entry.get_field_str("note"), Some("first"));
    }

    #[test]
    fn map_entrykey_cited_filter() {
        let mut biber = make_biber();
        let section = biber.sections.get_section_mut(0).unwrap();
        section.add_cite("test1");
        let _ = section;

        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_field_set="note" map_field_value="cited" map_entrykey_cited="1"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        apply_sourcemap_to_section(&mut biber, 0, &sm);
        let section = biber.sections.get_section(0).unwrap();
        assert_eq!(
            section
                .bibentries
                .get_entry("test1")
                .unwrap()
                .get_field_str("note"),
            Some("cited")
        );
        // test2 not cited → note not set
        assert!(section
            .bibentries
            .get_entry("test2")
            .unwrap()
            .get_field_str("note")
            .is_none());
    }

    #[test]
    fn map_overwrite_false_protects_existing() {
        let mut biber = make_biber();
        let xml = r#"<maps datatype="bibtex">
            <map map_overwrite="0">
                <map_step map_field_source="author" map_field_target="title"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        apply_sourcemap_to_section(&mut biber, 0, &sm);
        let section = biber.sections.get_section(0).unwrap();
        let entry = section.bibentries.get_entry("test1").unwrap();
        // title already exists, overwrite is false, so author should not be moved
        assert_eq!(entry.get_field_str("author"), Some("Smith"));
        assert_eq!(entry.get_field_str("title"), Some("A Great Paper"));
    }

    #[test]
    fn map_notfield_skips_when_field_exists() {
        let mut biber = make_biber();
        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_field_set="note" map_field_value="skip" map_notfield="title"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        apply_sourcemap_to_section(&mut biber, 0, &sm);
        let section = biber.sections.get_section(0).unwrap();
        let entry = section.bibentries.get_entry("test1").unwrap();
        // test1 has 'title' field, so notfield triggers → skip
        assert!(entry.get_field_str("note").is_none());
    }

    #[test]
    fn map_notfield_applies_when_field_absent() {
        let mut biber = make_biber();
        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_field_set="note" map_field_value="applied" map_notfield="missingfield"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        apply_sourcemap_to_section(&mut biber, 0, &sm);
        let section = biber.sections.get_section(0).unwrap();
        let entry = section.bibentries.get_entry("test1").unwrap();
        assert_eq!(entry.get_field_str("note"), Some("applied"));
    }

    #[test]
    fn parse_key_cited_flags() {
        let xml = r#"<maps datatype="bibtex">
            <map>
                <map_step map_entrykey_cited="1" map_entrykey_nocited="0" map_entrykey_allnocited="0" map_entrykey_citedornocited="1"/>
            </map>
        </maps>"#;
        let sm = parse_sourcemap_xml(xml);
        let step = &sm.maps[0].steps[0];
        assert!(step.key_cited);
        assert!(!step.key_nocited);
        assert!(!step.key_allnocited);
        assert!(step.key_citedornocited);
    }
}
