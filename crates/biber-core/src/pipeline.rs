//! Processing pipeline — the ~25 `process_*` passes from `Biber.pm`.
//!
//! Ported from `Biber::prepare` (`lib/Biber.pm:4474`). This module
//! orchestrates the full pipeline: for each section, it runs the passes
//! in order, then generates output via the output backend.
//!
//! Pass order (from `prepare()`):
//! 1. process_setup
//! 2. [per-section] preprocess_options → fetch_data → process_citekey_aliases
//!    → instantiate_dynamic → resolve_alias_refs → resolve_xdata
//!    → cite_setmembers → preprocess_sets → calculate_interentry
//!    → process_interentry → validate_datamodel → postprocess_sets
//!    → process_entries_static → process_lists → output
//! 3. output misc

use std::collections::{BTreeMap, HashMap};

use md5::{Digest, Md5};
use tracing::{debug, error, info, trace, warn};

use crate::annotation::{parse_annotation_field, AnnotationScope};
use crate::collation;
use crate::config::ConfigValue;
use crate::entry::Entry;
use crate::inheritance::{clear_inheritance, inherit_from, resolve_xdata_section};
use crate::name::{gen_initials, parse_entry_names};
use crate::processor::Biber;
use crate::sourcemap::apply_sourcemap;
use crate::validate::validate_entry_fields;

/// Run the biber processing pipeline in tool mode.
///
/// Tool mode is datasource-centric: reads a `.bib` file (no `.bcf`),
/// applies transformations, and writes a new `.bib`. This is a subset
/// of the full `prepare()` pipeline: passes that depend on BCF metadata
/// (xrefs, sets, dynamic entries, uniqueness) are skipped.
pub fn prepare_tool(biber: &mut Biber) {
    process_setup(biber);

    let section_nums: Vec<u32> = biber
        .sections
        .get_sections()
        .iter()
        .map(|s| s.number)
        .collect();

    for secnum in section_nums {
        let has_keys = {
            let section = biber.sections.get_section(secnum);
            match section {
                Some(s) => !s.get_citekeys().is_empty() || s.is_allkeys(),
                None => false,
            }
        };
        if !has_keys {
            continue;
        }

        info!("Processing section {secnum} (tool mode)");

        biber.set_current_section(secnum);
        preprocess_options(biber, secnum);

        // Sourcemap applies in tool mode too
        apply_sourcemap(biber, secnum);

        process_annotations(biber, secnum);

        // Tool mode skips alias, xdata, set, and interentry passes.
        // Only label/date/title extraction, sorting, and static entry
        // processing runs.

        // Skip: process_citekey_aliases (no dynamic aliases in tool mode)
        // Skip: instantiate_dynamic (no dynamic sets in tool mode)
        // Skip: resolve_alias_refs (tool mode has no xref/crossref resolution)
        // Skip: resolve_xdata (tool mode has no xdata)
        // Skip: cite_setmembers (no set member promotion)
        // Skip: preprocess_sets (no set tracking)
        // Skip: calculate_interentry (no crossref inclusion)
        // Skip: process_interentry (no crossref inheritance)
        // Skip: validate_datamodel (only if --validate-datamodel is set)
        validate_datamodel(biber, secnum);
        validate_entry_fields(biber, secnum);
        // Skip: postprocess_sets (no set post-processing)

        // Static entry processing runs (labelname, labeldate, labeltitle,
        // presort, name hashes, labelalpha, extradate, namedis, extraname)
        // but skips work-uniqueness passes (singletitle, uniquetitle, etc.)
        process_entries_static_tool(biber, secnum);

        process_lists(biber, secnum);
    }
}

/// Static entry processing for tool mode — same as process_entries_static
/// but skips work-uniqueness passes.
fn process_entries_static_tool(biber: &mut Biber, secnum: u32) {
    let citekeys: Vec<String> = biber
        .sections
        .get_section(secnum)
        .map(|s| s.get_citekeys().to_vec())
        .unwrap_or_default();

    for citekey in &citekeys {
        trace!("process_entries_static_tool: entry '{citekey}'");
        process_nocite(biber, secnum, citekey);
        process_labelname(biber, secnum, citekey);
        process_labeldate(biber, secnum, citekey);
        process_labeltitle(biber, secnum, citekey);
        process_presort(biber, secnum, citekey);

        process_namehash(biber, secnum, citekey);
        process_fullhash(biber, secnum, citekey);
        process_pername_hashes(biber, secnum, citekey);

        process_labelalpha(biber, secnum, citekey);
        process_extraalpha(biber, secnum, citekey);

        process_extradate(biber, secnum, citekey);
        process_namedis(biber, secnum, citekey);
        process_extraname(biber, secnum, citekey);

        // Tool mode skips: extratitle, extratitleyear,
        // process_workuniqueness, singletitle, uniquetitle,
        // uniquebaretitle, uniquework, uniqueprimaryauthor
    }

    assign_extradate_letters(biber, secnum);
    assign_extraalpha_letters(biber, secnum);
    assign_extraname_extratitle_letters(biber, secnum);
}

/// Run the full biber processing pipeline.
///
/// This is the Rust equivalent of `Biber::prepare()`. It modifies the
/// `Biber` processor in place: populates entries, runs all processing
/// passes, and prepares the output (which is written separately).
pub fn prepare(biber: &mut Biber) {
    process_setup(biber);

    let section_nums: Vec<u32> = biber
        .sections
        .get_sections()
        .iter()
        .map(|s| s.number)
        .collect();

    for secnum in section_nums {
        let has_keys = {
            let section = biber.sections.get_section(secnum);
            match section {
                Some(s) => !s.get_citekeys().is_empty() || s.is_allkeys(),
                None => false,
            }
        };
        if !has_keys {
            continue;
        }

        info!("Processing section {secnum}");
        trace!(
            "prepare: section {secnum} -- citekeys={}",
            biber
                .sections
                .get_section(secnum)
                .map(|s| s.get_citekeys().len())
                .unwrap_or(0)
        );

        biber.set_current_section(secnum);
        preprocess_options(biber, secnum);
        // Sourcemap: transforms entries at the datasource level
        apply_sourcemap(biber, secnum);
        // fetch_data is handled by the CLI layer (reads .bib files)
        process_annotations(biber, secnum);
        process_citekey_aliases(biber, secnum);
        instantiate_dynamic(biber, secnum);
        process_related(biber, secnum);
        resolve_alias_refs(biber, secnum);
        resolve_xdata(biber, secnum);
        cite_setmembers(biber, secnum);
        preprocess_sets(biber, secnum);
        calculate_interentry(biber, secnum);
        process_interentry(biber, secnum);
        validate_datamodel(biber, secnum);
        validate_entry_fields(biber, secnum);
        postprocess_sets(biber, secnum);
        process_entries_static(biber, secnum);
        process_lists(biber, secnum);
    }
}

// ---- Setup / options ----

/// Global pre-processing setup.
fn process_setup(biber: &mut Biber) {
    debug!("process_setup");

    // For bibtex output format, delete all sections except 99999
    // (not relevant for v1 — bbl output only)
    let output_format = biber
        .config
        .getoption_str("output_format")
        .unwrap_or("bbl")
        .to_string();
    if output_format == "bibtex" {
        let nums: Vec<u32> = biber
            .sections
            .get_sections()
            .iter()
            .map(|s| s.number)
            .collect();
        for n in nums {
            if n != 99999 {
                biber.sections.delete_section(n);
            }
        }
    }

    // Ensure each section has a default entry datalist with global sorting
    let globalss = biber
        .config
        .getblxoption_str("sortingtemplatename")
        .unwrap_or("nty")
        .to_string();

    for section in biber.sections.get_sections() {
        let secnum = section.number;
        let has_entry_list = biber
            .datalists
            .get_lists_for_section(secnum)
            .iter()
            .any(|l| l.r#type == "entry");
        if !has_entry_list {
            let mut dl = crate::datalist::DataList::new(
                secnum,
                &globalss,
                "global",
                "global",
                "global",
                "global",
                "",
                format!("{}/global//global/global/global", globalss),
            );
            dl.set_type("entry");
            biber.datalists.add_list(dl);
        }
    }
}

/// Extract annotation metadata from entry fields into the AnnotationStore.
///
/// Scans every entry's fields for annotation-pattern names (e.g.
/// `author+an`, `title+an:french`), parses their values, stores them
/// in the section's `AnnotationStore`, and removes the annotation
/// fields from the entry.
fn process_annotations(biber: &mut Biber, secnum: u32) {
    debug!("process_annotations for section {secnum}");

    let section = match biber.sections.get_section_mut(secnum) {
        Some(s) => s,
        None => return,
    };

    let ann_marker = biber
        .config
        .getoption_str("annotation_marker")
        .unwrap_or("+an")
        .to_string();
    let named_marker = biber
        .config
        .getoption_str("named_annotation_marker")
        .unwrap_or(":")
        .to_string();

    // First pass: collect annotations and field names to remove
    // Iterate over all bibentries (not just cited keys) to catch
    // annotation fields on any loaded entry.
    #[allow(clippy::type_complexity)]
    let mut to_store: Vec<(
        i32,
        String,
        String,
        String,
        String,
        bool,
        Option<u32>,
        Option<String>,
    )> = Vec::new();
    let mut ann_fields_per_key: Vec<(String, Vec<String>)> = Vec::new();

    let all_keys: Vec<String> = section
        .bibentries
        .citekeys()
        .map(|s| s.to_string())
        .collect();

    for citekey in &all_keys {
        let entry = match section.bibentry(citekey) {
            Some(e) => e,
            None => continue,
        };

        let mut fields_to_remove = Vec::new();

        for f in entry.field_names() {
            let val = match entry.get_field_str(f) {
                Some(v) => v,
                None => continue,
            };
            let Some(parsed) = parse_annotation_field(f, val, &ann_marker, &named_marker) else {
                continue;
            };

            fields_to_remove.push(f.to_string());

            for ann_entry in &parsed.entries {
                to_store.push((
                    ann_entry.scope as i32,
                    citekey.clone(),
                    parsed.field.clone(),
                    parsed.name.clone(),
                    ann_entry.value.clone(),
                    ann_entry.literal,
                    ann_entry.count,
                    ann_entry.part.clone(),
                ));
            }
        }

        if !fields_to_remove.is_empty() {
            ann_fields_per_key.push((citekey.clone(), fields_to_remove));
        }
    }

    // Second pass: store annotations in the AnnotationStore (mutable borrow)
    for (scope_val, citekey, field, name, value, literal, count, part) in &to_store {
        let scope = match scope_val {
            0 => AnnotationScope::Field,
            1 => AnnotationScope::Item,
            _ => AnnotationScope::Part,
        };
        section.annotations.set_annotation(
            scope,
            citekey,
            field,
            name,
            value,
            *literal,
            *count,
            part.clone(),
        );
    }

    // Third pass: remove annotation fields from entries (mutable borrow)
    for (citekey, fields) in &ann_fields_per_key {
        if let Some(be) = section.bibentry_mut(citekey) {
            for f in fields {
                be.del_field(f);
            }
        }
    }
}

/// Preprocess options for the current section.
fn preprocess_options(biber: &Biber, secnum: u32) {
    debug!("preprocess_options for section {secnum}");
    // Most option preprocessing is done during BCF parsing.
    // Additional per-section option resolution would go here.
    let _ = biber;
    let _ = secnum;
}

// ---- Alias / xdata / set passes ----

/// Remove citekey aliases from citekeys (they don't point to real entries).
fn process_citekey_aliases(biber: &mut Biber, secnum: u32) {
    debug!("process_citekey_aliases for section {secnum}");
    let section = match biber.sections.get_section_mut(secnum) {
        Some(s) => s,
        None => return,
    };

    // Collect keys that are aliases
    let alias_keys: Vec<String> = section
        .get_citekeys()
        .iter()
        .filter(|k| section.get_citekey_alias(k).is_some())
        .cloned()
        .collect();

    for k in alias_keys {
        debug!("Pruning citekey alias '{k}' from citekeys");
        section.del_citekey(&k);
    }
}

/// Instantiate dynamic set entries.
fn instantiate_dynamic(biber: &mut Biber, secnum: u32) {
    debug!("instantiate_dynamic for section {secnum}");

    // Collect dynamic set definitions
    let dynamic_sets: Vec<(String, Vec<String>)> = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        // We need to collect from the section, but get_dynamic_set requires &self
        // so we collect the keys first
        let citekeys: Vec<String> = section.get_citekeys().to_vec();
        let mut sets = Vec::new();
        for k in &citekeys {
            if let Some(members) = section.get_dynamic_set(k) {
                sets.push((k.clone(), members.clone()));
            }
        }
        sets
    };

    for (dset, members) in &dynamic_sets {
        // Resolve aliases in members
        let realmems: Vec<String> = members
            .iter()
            .map(|m| {
                biber
                    .sections
                    .get_section(secnum)
                    .and_then(|s| s.get_citekey_alias(m))
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| m.clone())
            })
            .collect();

        // Update the dynamic set with resolved members
        if let Some(section) = biber.sections.get_section_mut(secnum) {
            section.set_dynamic_set(dset.clone(), realmems.clone());
        }

        // Create the dynamic set entry
        let mut be = crate::entry::Entry::new(dset.clone(), "set");
        be.set_field_str("entrytype", "set");
        be.set_field(
            "entryset",
            ConfigValue::List(
                realmems
                    .iter()
                    .map(|m| ConfigValue::Str(m.clone()))
                    .collect(),
            ),
        );
        be.set_field_str("citekey", dset);
        be.set_field_str("datatype", "dynamic");

        if let Some(section) = biber.sections.get_section_mut(secnum) {
            section.bibentries.add_entry(be);
        }
        debug!("Created dynamic set entry '{dset}' in section {secnum}");
    }
}

// ---- Related entry cloning ----

/// Process related entries: clone each related entry as a visible
/// bibliography entry with modified options.
///
/// This implements the Perl `relclone()` logic (`Biber/Entry.pm:57`).
fn process_related(biber: &mut Biber, secnum: u32) {
    debug!("process_related for section {secnum}");

    let citekeys: Vec<String> = biber
        .sections
        .get_section(secnum)
        .map(|s| s.get_citekeys().to_vec())
        .unwrap_or_default();

    // First pass: discover dependencies — ensure related entries exist
    // and record them; remove references to missing entries
    for citekey in &citekeys {
        let related_str = {
            let section = biber.sections.get_section(secnum);
            section
                .and_then(|s| s.bibentry(citekey))
                .and_then(|be| be.get_field_str("related"))
                .map(|s| s.to_string())
        };

        let Some(related_val) = related_str else {
            continue;
        };
        let relkeys: Vec<String> = related_val
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        for relkey in &relkeys {
            // Resolve alias
            let nrelkey = biber
                .sections
                .get_section(secnum)
                .and_then(|s| s.get_citekey_alias(relkey))
                .map(|s| s.to_string())
                .unwrap_or_else(|| relkey.clone());

            let exists = biber
                .sections
                .get_section(secnum)
                .is_some_and(|s| s.bibentry(&nrelkey).is_some());

            if exists {
                biber
                    .sections
                    .get_section_mut(secnum)
                    .unwrap()
                    .add_related(nrelkey);
            } else {
                // Remove missing related key from parent's field
                warn!("Related entry '{nrelkey}' not found (referenced by '{citekey}')");
                let section = biber.sections.get_section_mut(secnum).unwrap();
                if let Some(be) = section.bibentries.get_entry_mut(citekey) {
                    if let Some(old_val) = be.get_field_str("related") {
                        let remaining: Vec<&str> = old_val
                            .split(',')
                            .map(|s| s.trim())
                            .filter(|&s| !s.is_empty() && s != relkey)
                            .collect();
                        if remaining.is_empty() {
                            be.del_field("related");
                            be.del_field("relatedtype");
                            be.del_field("relatedstring");
                        } else {
                            be.set_field_str("related", remaining.join(", "));
                        }
                    }
                }
            }
        }
    }

    // Second pass: actually clone the related entries
    for citekey in &citekeys {
        relclone_entry(biber, secnum, citekey);
    }
}

/// Clone a single entry's related entries (recursive for cascading).
///
/// This implements Perl's `Biber::Entry::relclone()` for one entry.
fn relclone_entry(biber: &mut Biber, secnum: u32, citekey: &str) {
    let related_str = {
        let section = biber.sections.get_section(secnum);
        section
            .and_then(|s| s.bibentry(citekey))
            .and_then(|be| be.get_field_str("related"))
            .map(|s| s.to_string())
    };

    let Some(related_val) = related_str else {
        return;
    };

    let relkeys: Vec<String> = related_val
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if relkeys.is_empty() {
        return;
    }

    let mut clone_keys: Vec<String> = Vec::new();

    for relkey in &relkeys {
        // Resolve alias
        let nrelkey = biber
            .sections
            .get_section(secnum)
            .and_then(|s| s.get_citekey_alias(relkey))
            .map(|s| s.to_string())
            .unwrap_or_else(|| relkey.clone());

        // Loop avoidance: check if clone already exists
        let clonekey = {
            let section = biber.sections.get_section(secnum);
            section
                .and_then(|s| s.get_keytorelclone(&nrelkey))
                .map(|s| s.to_string())
        };

        if let Some(ck) = clonekey {
            clone_keys.push(ck);
            continue;
        }

        // Generate clone key = md5(relkey)
        let ck = hex::encode(Md5::digest(nrelkey.as_bytes()));
        let ck_for_map = ck.clone();
        clone_keys.push(ck.clone());

        // Get the related entry and clone it
        let relentry = match biber
            .sections
            .get_section(secnum)
            .and_then(|s| s.bibentry(&nrelkey))
        {
            Some(e) => e,
            None => {
                warn!("Related entry '{nrelkey}' not found for cloning");
                continue;
            }
        };

        let mut relclone = relentry.clone_with_key(&ck);

        // Handle options
        let parent_opts = biber
            .sections
            .get_section(secnum)
            .and_then(|s| s.bibentry(citekey))
            .and_then(|be| be.get_field_str("relatedoptions"))
            .map(|s| s.to_string());

        let merged_opts = if let Some(ref ropts) = parent_opts {
            // Use relatedoptions from parent
            let mut opts: Vec<String> = ropts
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            // If the related key is already cited, add skipbib/skipbiblist
            let is_cited = biber
                .sections
                .get_section(secnum)
                .is_some_and(|s| s.get_citekeys().contains(&nrelkey));
            if is_cited {
                if !opts.contains(&"skipbib".to_string()) {
                    opts.push("skipbib".to_string());
                }
                if !opts.contains(&"skipbiblist".to_string()) {
                    opts.push("skipbiblist".to_string());
                }
            }
            opts
        } else {
            // Default options for clones
            let defaults = vec![
                "skipbib".to_string(),
                "skiplab".to_string(),
                "skipbiblist".to_string(),
                "uniquename=false".to_string(),
                "uniquelist=false".to_string(),
            ];
            // Merge with original entry's options
            let orig_opts: Vec<String> = relentry
                .get_field_str("options")
                .map(|o| {
                    o.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();

            let mut merged = defaults.clone();
            for o in &orig_opts {
                if !merged.contains(o) {
                    merged.push(o.clone());
                }
            }
            merged
        };

        // Apply options to the clone
        relclone.set_field_str("options", merged_opts.join(", "));

        // Register the clone
        let section = biber.sections.get_section_mut(secnum).unwrap();
        section.bibentries.add_entry(relclone);
        section.set_keytorelclone(nrelkey.clone(), ck_for_map.clone());
        section.annotations.copy_annotations(&nrelkey, &ck_for_map);

        // Recurse for cascading related entries
        relclone_entry(biber, secnum, &ck_for_map);
    }

    // Add clone keys to citekeys and update the parent's related field
    let section = biber.sections.get_section_mut(secnum).unwrap();
    section.add_citekeys(clone_keys.clone());
    if let Some(be) = section.bibentries.get_entry_mut(citekey) {
        // Don't overwrite if already set — just add clone keys
        if !clone_keys.is_empty() {
            be.set_field_str("related", clone_keys.join(", "));
        }
    }
}

/// Resolve xref/crossref/xdata aliases to real keys.
fn resolve_alias_refs(biber: &mut Biber, secnum: u32) {
    debug!("resolve_alias_refs for section {secnum}");
    let citekeys: Vec<String> = biber
        .sections
        .get_section(secnum)
        .map(|s| s.get_citekeys().to_vec())
        .unwrap_or_default();

    for citekey in &citekeys {
        // Collect alias resolutions first (immutable borrow)
        let resolutions = {
            let section = biber.sections.get_section(secnum).unwrap();
            let be = section.bibentry(citekey);
            match be {
                Some(be) => {
                    let xref = be.get_field_str("xref");
                    let xref_resolved =
                        xref.and_then(|r| section.get_citekey_alias(r).map(|s| s.to_string()));
                    let crossref = be.get_field_str("crossref");
                    let crossref_resolved =
                        crossref.and_then(|r| section.get_citekey_alias(r).map(|s| s.to_string()));
                    let xdata = be.get_field_str("xdata");
                    let xdata_resolved = xdata.map(|s| {
                        // Resolve each comma-separated key
                        s.split(',')
                            .map(|part| {
                                let trimmed = part.trim();
                                section
                                    .get_citekey_alias(trimmed)
                                    .map(|s| s.to_string())
                                    .unwrap_or_else(|| trimmed.to_string())
                            })
                            .collect::<Vec<_>>()
                            .join(", ")
                    });
                    (xref_resolved, crossref_resolved, xdata_resolved)
                }
                None => (None, None, None),
            }
        };

        // Apply resolutions (mutable borrow)
        let (xref_resolved, crossref_resolved, xdata_resolved) = resolutions;
        let section = biber.sections.get_section_mut(secnum).unwrap();
        if let Some(be) = section.bibentries.get_entry_mut(citekey) {
            if let Some(real) = xref_resolved {
                be.set_field_str("xref", &real);
            }
            if let Some(real) = crossref_resolved {
                be.set_field_str("crossref", &real);
            }
            if let Some(real) = xdata_resolved {
                be.set_field_str("xdata", &real);
            }
        }
    }
}

/// Resolve xdata references.
fn resolve_xdata(biber: &mut Biber, secnum: u32) {
    debug!("resolve_xdata for section {secnum}");
    clear_inheritance(biber);
    resolve_xdata_section(biber, secnum);
}

/// Promote set members to cited status.
fn cite_setmembers(biber: &mut Biber, secnum: u32) {
    debug!("cite_setmembers for section {secnum}");

    let citekeys: Vec<String> = biber
        .sections
        .get_section(secnum)
        .map(|s| s.get_citekeys().to_vec())
        .unwrap_or_default();

    for citekey in &citekeys {
        // Check if this is a set entry
        let (entryset, is_set) = {
            let section = biber.sections.get_section(secnum).unwrap();
            let be = section.bibentry(citekey);
            match be {
                Some(be) => {
                    let et = be.get_field_str("entrytype").unwrap_or("");
                    let es = be.get_field("entryset");
                    (es.cloned(), et == "set")
                }
                None => (None, false),
            }
        };

        if is_set {
            let entryset = match entryset {
                Some(ConfigValue::List(v)) => v,
                _ => continue,
            };

            // Resolve aliases in members
            let realmems: Vec<String> = entryset
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .map(|m| {
                    biber
                        .sections
                        .get_section(secnum)
                        .and_then(|s| s.get_citekey_alias(&m))
                        .map(|s| s.to_string())
                        .unwrap_or(m)
                })
                .collect();

            // Update the entryset field
            {
                let section = biber.sections.get_section_mut(secnum).unwrap();
                if let Some(be) = section.bibentries.get_entry_mut(citekey) {
                    be.set_field(
                        "entryset",
                        ConfigValue::List(
                            realmems
                                .iter()
                                .map(|m| ConfigValue::Str(m.clone()))
                                .collect(),
                        ),
                    );
                }
            }

            // Add set members to citekeys
            for mem in &realmems {
                debug!("Adding set member '{mem}' to the citekeys (section {secnum})");
                let section = biber.sections.get_section_mut(secnum).unwrap();
                section.add_citekeys(std::iter::once(mem.clone()));
            }
        }
    }
}

/// Record set information for use later.
fn preprocess_sets(biber: &mut Biber, secnum: u32) {
    debug!("preprocess_sets for section {secnum}");
    // Record set parent→child and child→parent mappings.
    // This is a no-op — the full set tracking will be
    // implemented when the parity harness exercises it.
    let _ = biber;
    let _ = secnum;
}

// ---- Interentry passes ----

/// Ensure crossrefs/xrefs meeting mincrossrefs/minxrefs are included.
fn calculate_interentry(biber: &mut Biber, secnum: u32) {
    debug!("calculate_interentry for section {secnum}");

    type RefPairs = Vec<(String, String)>;
    let (crossref_keys, xref_keys): (RefPairs, RefPairs) = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        let mut crs = Vec::new();
        let mut xrs = Vec::new();
        for k in section.get_citekeys() {
            if let Some(be) = section.bibentry(k) {
                if let Some(cr) = be.get_field_str("crossref") {
                    crs.push((k.clone(), cr.to_string()));
                }
                if let Some(xr) = be.get_field_str("xref") {
                    xrs.push((k.clone(), xr.to_string()));
                }
            }
        }
        (crs, xrs)
    };

    // Count crossref occurrences
    let mut crossref_counts: HashMap<String, u32> = HashMap::new();
    for (_, refkey) in &crossref_keys {
        let exists = biber
            .sections
            .get_section(secnum)
            .is_some_and(|s| s.bibentry(refkey).is_some());
        if exists {
            *crossref_counts.entry(refkey.clone()).or_insert(0) += 1;
        }
    }

    // Count xref occurrences
    let mut xref_counts: HashMap<String, u32> = HashMap::new();
    for (_, refkey) in &xref_keys {
        let exists = biber
            .sections
            .get_section(secnum)
            .is_some_and(|s| s.bibentry(refkey).is_some());
        if exists {
            *xref_counts.entry(refkey.clone()).or_insert(0) += 1;
        }
    }

    let mincrossrefs: u32 = biber
        .config
        .getoption_str("mincrossrefs")
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);

    let minxrefs: u32 = biber
        .config
        .getoption_str("minxrefs")
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);

    // Add crossref targets that meet the threshold
    let to_add: Vec<String> = crossref_counts
        .iter()
        .filter(|(_, &count)| count >= mincrossrefs)
        .map(|(k, _)| k.clone())
        .chain(
            xref_counts
                .iter()
                .filter(|(_, &count)| count >= minxrefs)
                .map(|(k, _)| k.clone()),
        )
        .collect();

    for k in to_add {
        let already_cited = biber
            .sections
            .get_section(secnum)
            .is_some_and(|s| s.get_citekeys().contains(&k));

        if !already_cited {
            let section = biber.sections.get_section_mut(secnum).unwrap();
            if let Some(be) = section.bibentries.get_entry_mut(&k) {
                be.set_field_str("crossrefsource", "1");
            }
            section.add_citekeys(std::iter::once(k));
        }
    }
}

/// Process crossref inheritance.
fn process_interentry(biber: &mut Biber, secnum: u32) {
    debug!("process_interentry for section {secnum}");
    clear_inheritance(biber);

    let citekeys: Vec<String> = biber
        .sections
        .get_section(secnum)
        .map(|s| s.get_citekeys().to_vec())
        .unwrap_or_default();

    for citekey in &citekeys {
        let parent_key = {
            let section = match biber.sections.get_section(secnum) {
                Some(s) => s,
                None => continue,
            };
            let be = match section.bibentry(citekey) {
                Some(be) => be,
                None => continue,
            };
            match be.get_field_str("crossref") {
                Some(cr) => {
                    // Resolve alias
                    let resolved = section
                        .get_citekey_alias(cr)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| cr.to_string());
                    resolved
                }
                None => continue,
            }
        };

        // Check if the parent exists
        let parent_exists = biber
            .sections
            .get_section(secnum)
            .is_some_and(|s| s.bibentry(&parent_key).is_some());

        if !parent_exists {
            warn!("Cannot inherit from crossref key '{parent_key}' - does it exist?");
            continue;
        }

        inherit_from(biber, secnum, citekey, &parent_key);
    }
}

// ---- Validation ----

/// Validate entries against the data model.
fn validate_datamodel(biber: &mut Biber, secnum: u32) {
    debug!("validate_datamodel for section {secnum}");

    let validate = biber
        .config
        .getoption_str("validate_datamodel")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if !validate {
        return;
    }

    info!("Datamodel validation starting for section {secnum}");

    let dieondm = biber
        .config
        .getoption_str("dieondatamodel")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let citekeys: Vec<String> = biber
        .sections
        .get_section(secnum)
        .map(|s| s.get_citekeys().to_vec())
        .unwrap_or_default();

    for k in &citekeys {
        // Entry type validity
        let et_warning = {
            let section = match biber.sections.get_section(secnum) {
                Some(s) => s,
                None => continue,
            };
            let be = match section.bibentry(k) {
                Some(be) => be,
                None => continue,
            };
            biber.datamodel.validate_entrytype(be)
        };

        if let Some(w) = et_warning {
            if dieondm {
                error!("Datamodel: entry '{k}': {w}");
            } else {
                warn!("Datamodel: entry '{k}': {w}");
            }
            // Default to misc for invalid/empty entrytype
            if let Some(section) = biber.sections.get_section_mut(secnum) {
                if let Some(be) = section.bibentries.get_entry_mut(k) {
                    be.set_field_str("entrytype", "misc");
                }
            }
        }

        // Field validity per entrytype
        let field_warnings = {
            let section = match biber.sections.get_section(secnum) {
                Some(s) => s,
                None => continue,
            };
            let be = match section.bibentry(k) {
                Some(be) => be,
                None => continue,
            };
            biber.datamodel.validate_fields(be)
        };

        for w in &field_warnings {
            if dieondm {
                error!("Datamodel: {w}");
            } else {
                warn!("Datamodel: {w}");
            }
        }

        // Mandatory constraints
        let mandatory_warnings = {
            let section = match biber.sections.get_section(secnum) {
                Some(s) => s,
                None => continue,
            };
            let be = match section.bibentry(k) {
                Some(be) => be,
                None => continue,
            };
            biber.datamodel.check_mandatory_constraints(be)
        };

        for w in &mandatory_warnings {
            if dieondm {
                error!("Datamodel: {w}");
            } else {
                warn!("Datamodel: {w}");
            }
        }

        // Conditional constraints
        let (cond_warnings, to_delete) = {
            let section = match biber.sections.get_section(secnum) {
                Some(s) => s,
                None => continue,
            };
            let be = match section.bibentry(k) {
                Some(be) => be,
                None => continue,
            };
            biber.datamodel.check_conditional_constraints(be)
        };

        // Delete fields that violate conditional constraints (consequent quant=none)
        if !to_delete.is_empty() {
            if let Some(section) = biber.sections.get_section_mut(secnum) {
                if let Some(be) = section.bibentries.get_entry_mut(k) {
                    for f in &to_delete {
                        be.del_field(f);
                        debug!("Datamodel: deleted field '{f}' from entry '{k}' (conditional constraint)");
                    }
                }
            }
        }

        for w in &cond_warnings {
            if dieondm {
                error!("Datamodel: {w}");
            } else {
                warn!("Datamodel: {w}");
            }
        }

        // Datatype checking
        let (datatype_warnings, type_to_delete) = {
            let section = match biber.sections.get_section(secnum) {
                Some(s) => s,
                None => continue,
            };
            let be = match section.bibentry(k) {
                Some(be) => be,
                None => continue,
            };
            biber.datamodel.check_datatypes(be)
        };

        // Delete fields with wrong datatypes
        if !type_to_delete.is_empty() {
            if let Some(section) = biber.sections.get_section_mut(secnum) {
                if let Some(be) = section.bibentries.get_entry_mut(k) {
                    for f in &type_to_delete {
                        be.del_field(f);
                        debug!("Datamodel: deleted field '{f}' from entry '{k}' (wrong datatype)");
                    }
                }
            }
        }

        for w in &datatype_warnings {
            if dieondm {
                error!("Datamodel: {w}");
            } else {
                warn!("Datamodel: {w}");
            }
        }

        // Data constraints (isbn/issn/ismn, ranges, patterns)
        let data_warnings = {
            let section = match biber.sections.get_section(secnum) {
                Some(s) => s,
                None => continue,
            };
            let be = match section.bibentry(k) {
                Some(be) => be,
                None => continue,
            };
            biber.datamodel.check_data_constraints(be)
        };

        for w in &data_warnings {
            if dieondm {
                error!("Datamodel: {w}");
            } else {
                warn!("Datamodel: {w}");
            }
        }
    }

    info!("Datamodel validation complete for section {secnum}");
}

/// Post-process sets (add options to set members etc.).
fn postprocess_sets(biber: &mut Biber, secnum: u32) {
    debug!("postprocess_sets for section {secnum}");
    let _ = biber;
    let _ = secnum;
}

// ---- Static entry processing ----

/// Generate static entry data not dependent on lists.
fn process_entries_static(biber: &mut Biber, secnum: u32) {
    debug!("process_entries_static for section {secnum}");

    let citekeys: Vec<String> = biber
        .sections
        .get_section(secnum)
        .map(|s| s.get_citekeys().to_vec())
        .unwrap_or_default();

    for citekey in &citekeys {
        trace!("process_entries_static: entry '{citekey}'");
        process_nocite(biber, secnum, citekey);
        process_labelname(biber, secnum, citekey);
        process_labeldate(biber, secnum, citekey);
        process_labeltitle(biber, secnum, citekey);
        process_presort(biber, secnum, citekey);

        // Name hashing
        process_namehash(biber, secnum, citekey);
        process_fullhash(biber, secnum, citekey);
        process_pername_hashes(biber, secnum, citekey);

        // Label alpha
        process_labelalpha(biber, secnum, citekey);
        process_extraalpha(biber, secnum, citekey);

        // Extradate
        process_extradate(biber, secnum, citekey);

        // Name disambiguation
        process_namedis(biber, secnum, citekey);
        process_extraname(biber, secnum, citekey);

        // Extra title
        process_extratitle(biber, secnum, citekey);
        process_extratitleyear(biber, secnum, citekey);

        // Work uniqueness
        process_workuniqueness(biber, secnum, citekey);
        generate_singletitle(biber, secnum, citekey);
        generate_uniquetitle(biber, secnum, citekey);
        generate_uniquebaretitle(biber, secnum, citekey);
        generate_uniquework(biber, secnum, citekey);

        // Primary author uniqueness
        process_uniqueprimaryauthor(biber, secnum, citekey);
        generate_uniquepa(biber, secnum, citekey);
    }

    // Assign extradate letters (needs all entries processed first)
    trace!(
        "process_entries_static: per-entry done for {} entries, starting cross-entry assignments",
        citekeys.len()
    );
    assign_extradate_letters(biber, secnum);

    // Assign extraalpha letters (needs all entries processed first)
    assign_extraalpha_letters(biber, secnum);

    // Assign extra name/title/year letters (needs all entries processed first)
    assign_extraname_extratitle_letters(biber, secnum);
}

/// Generate nocite information.
fn process_nocite(biber: &mut Biber, secnum: u32, citekey: &str) {
    let section = match biber.sections.get_section(secnum) {
        Some(s) => s,
        None => return,
    };
    // Check if this key was nocite'd (either explicitly or via \nocite{*})
    let is_nocite = section.contains_nocite(citekey) || section.is_allkeys_nocite();
    if is_nocite {
        if let Some(be) = biber
            .sections
            .get_section_mut(secnum)
            .and_then(|s| s.bibentries.get_entry_mut(citekey))
        {
            be.set_field_str("nocite", "1");
        }
    }
}

/// Generate labelname information.
///
/// Sets the `labelname_info` field on the entry, indicating which name
/// field should be used as the label name.
fn process_labelname(biber: &mut Biber, secnum: u32, citekey: &str) {
    debug!("process_labelname for '{citekey}'");
    // The labelnamespec is a list of candidate name fields from the BCF.
    // We pick the first one that exists on the entry.
    // Common candidates: shortauthor, author, shorteditor, editor, translator
    let candidates = [
        "shortauthor",
        "author",
        "shorteditor",
        "editor",
        "translator",
    ];

    if let Some(section) = biber.sections.get_section_mut(secnum) {
        if let Some(be) = section.bibentries.get_entry_mut(citekey) {
            for ln in &candidates {
                if be.has_field(ln) {
                    be.set_field_str("labelname", *ln);
                    debug!("Set labelname for '{citekey}' to '{ln}'");
                    break;
                }
            }
        }
    }
}

/// Generate labeldate information.
fn process_labeldate(biber: &mut Biber, secnum: u32, citekey: &str) {
    debug!("process_labeldate for '{citekey}'");
    // The labeldatespec from the BCF specifies candidate date fields.
    // Common: date, year. We check for the year field derived from date.
    if let Some(section) = biber.sections.get_section_mut(secnum) {
        if let Some(be) = section.bibentries.get_entry_mut(citekey) {
            // Check for year field
            if be.has_field("year") {
                let year = be.get_field_str("year").map(|s| s.to_string());
                if let Some(year) = year {
                    if !year.is_empty() {
                        be.set_field_str("labelyear", &year);
                        be.set_field_str("labeldatesource", "year");
                        debug!("Set labelyear for '{citekey}' to '{year}'");
                    }
                }
            }
        }
    }
}

/// Generate labeltitle information.
fn process_labeltitle(biber: &mut Biber, secnum: u32, citekey: &str) {
    debug!("process_labeltitle for '{citekey}'");
    // The labeltitlespec from the BCF specifies candidate title fields.
    // Common: shorttitle, title
    let candidates = ["shorttitle", "title"];

    if let Some(section) = biber.sections.get_section_mut(secnum) {
        if let Some(be) = section.bibentries.get_entry_mut(citekey) {
            for lt in &candidates {
                if be.has_field(lt) {
                    be.set_field_str("labeltitle", *lt);
                    debug!("Set labeltitle for '{citekey}' to '{lt}'");
                    break;
                }
            }
        }
    }
}

/// Push entry-specific presort fields.
fn process_presort(biber: &mut Biber, secnum: u32, citekey: &str) {
    let section = match biber.sections.get_section(secnum) {
        Some(s) => s,
        None => return,
    };
    let presort = section
        .bibentry(citekey)
        .and_then(|be| be.get_field_str("presort"))
        .map(|s| s.to_string())
        .unwrap_or_else(|| "mm".to_string());

    // Store presort in the first datalist's state for later sorting
    for list in biber.datalists.get_lists_for_section_mut(secnum) {
        list.state
            .presort
            .insert(citekey.to_string(), presort.clone());
    }
}

// ---- List processing (sort + filter) ----

/// Process the output lists (sort and filter).
fn process_lists(biber: &mut Biber, secnum: u32) {
    debug!("process_lists for section {secnum}");

    // For each datalist in this section, populate it with citekeys
    // and apply sorting.
    let list_names: Vec<(String, String)> = {
        biber
            .datalists
            .get_lists_for_section(secnum)
            .iter()
            .map(|l| (l.name.clone(), l.sortingtemplatename.clone()))
            .collect()
    };

    for (list_name, _sorting_template) in &list_names {
        debug!("Processing datalist '{list_name}' for section {secnum}");

        // Get all citekeys for this section
        let citekeys: Vec<String> = biber
            .sections
            .get_section(secnum)
            .map(|s| s.get_citekeys().to_vec())
            .unwrap_or_default();

        trace!(
            "process_lists: datalist '{list_name}' -- {} citekeys",
            citekeys.len()
        );

        // Sort entries using locale-aware collation
        let sorted = sort_entries_for_list(biber, secnum, list_name, &citekeys);

        for list in biber.datalists.get_lists_for_section_mut(secnum) {
            if list.name == *list_name {
                list.state.entries = sorted;
                break;
            }
        }

        // Per-list passes: sortinit, sortinithash, labelprefix
        process_sortinit(biber, secnum, list_name);
        process_sortinithash(biber, secnum, list_name);
        process_labelprefix(biber, secnum, list_name);
    }
}

/// Sort entries in a datalist using locale-aware ICU4X collation.
///
/// Resolves the sorting template and locale, builds sort keys for each
/// entry, and sorts them using an ICU4X `Collator` configured from
/// `sortcase`, `sortupper`, and `collate_options` settings.
fn sort_entries_for_list(
    biber: &Biber,
    secnum: u32,
    list_name: &str,
    citekeys: &[String],
) -> Vec<String> {
    if citekeys.is_empty() {
        return Vec::new();
    }

    // Resolve sorting template
    let tmpl_map = match biber.config.getblxoption(None, "sortingtemplate") {
        Some(ConfigValue::Map(m)) => m,
        _ => return citekeys.to_vec(),
    };

    // Find the sorting template name for this list
    let sortingtemplatename = {
        let lists = biber.datalists.get_lists_for_section(secnum);
        match lists.iter().find(|l| l.name == *list_name) {
            Some(l) => l.sortingtemplatename.clone(),
            None => return citekeys.to_vec(),
        }
    };

    let template = match tmpl_map.get(&sortingtemplatename) {
        Some(ConfigValue::Map(m)) => m,
        _ => return citekeys.to_vec(),
    };

    let specs = match template.get("spec") {
        Some(ConfigValue::List(s)) => s,
        _ => return citekeys.to_vec(),
    };

    // Resolve locale: template-level > sortlocale biber option > default
    let locale_str = template
        .get("locale")
        .and_then(|v| v.as_str())
        .or_else(|| biber.config.getoption_str("sortlocale"))
        .unwrap_or("en-US");

    let locale = collation::resolve_locale(locale_str);
    let collator = collation::create_collator(&locale, &biber.config);

    // Build sort keys for all entries
    let section = match biber.sections.get_section(secnum) {
        Some(s) => s,
        None => return citekeys.to_vec(),
    };

    let presort_opt = biber
        .config
        .getblxoption_str("presort")
        .unwrap_or("mm")
        .to_string();

    // Per-entrytype transliteration rule cache
    let mut translit_cache: HashMap<String, Vec<crate::transliteration::TranslitRule>> =
        HashMap::new();

    let mut entries_with_keys: Vec<(&str, String)> = Vec::with_capacity(citekeys.len());

    for citekey in citekeys {
        let be = match section.bibentry(citekey) {
            Some(be) => be,
            None => {
                // Entry not found; keep citekey but with empty sort key
                entries_with_keys.push((citekey.as_str(), String::new()));
                continue;
            }
        };

        let translit_rules: &Vec<crate::transliteration::TranslitRule> = translit_cache
            .entry(be.entrytype.clone())
            .or_insert_with(|| {
                let mut rules = Vec::new();
                if let Some(cv) = biber
                    .config
                    .getblxoption_for_entry(&be.entrytype, "translit")
                {
                    rules.extend(crate::transliteration::rules_from_config_value(cv));
                }
                rules
            });

        let cite_index = citekeys.iter().position(|k| k == citekey).unwrap_or(0);
        let mut sort_key = build_sort_string(specs, be, cite_index, translit_rules);

        // Strip presort prefix + following separator(s), matching process_sortinit
        let rest = if sort_key.starts_with(&presort_opt) {
            let after = &sort_key[presort_opt.len()..];
            after.trim_start_matches(SORT_SEP)
        } else {
            &sort_key
        };
        sort_key = rest.to_string();

        entries_with_keys.push((citekey.as_str(), sort_key));
    }

    // Sort using ICU4X collator
    entries_with_keys.sort_by(|a, b| {
        let a_empty = a.1.is_empty();
        let b_empty = b.1.is_empty();
        match (a_empty, b_empty) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => {
                let mut ordering = collator.compare(&a.1, &b.1);
                // Check per-sort-group sort_direction in the template spec
                if let Some(ConfigValue::List(spec_list)) = template.get("spec") {
                    if let Some(ConfigValue::Map(group_map)) = spec_list.first() {
                        if let Some(dir) = group_map.get("sort_direction").and_then(|v| v.as_str())
                        {
                            if dir == "descending" || dir == "desc" {
                                ordering = ordering.reverse();
                            }
                        }
                    }
                }
                ordering
            }
        }
    });

    entries_with_keys
        .into_iter()
        .map(|(k, _)| k.to_string())
        .collect()
}

// ---- Processing passes (all implemented) ----

/// Process name disambiguation.
///
/// Uses the `uniquenametemplate` config to build base-name strings for each
/// name in the labelname field. When two entries share the same labelname
/// field and have the same base name at the same name index, marks the
/// second+ occurrence with `un=1`, `uniquepart=base`.
fn process_namedis(biber: &mut Biber, secnum: u32, citekey: &str) {
    // --- Collect read-only data first ---
    let lnsource = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        let be = match section.bibentry(citekey) {
            Some(be) => be,
            None => return,
        };
        match be.get_field_str("labelname") {
            Some(s) => s.to_string(),
            None => return,
        }
    };

    // Parse names and build bases
    let (num_names, bases) = {
        let section = match biber.sections.get_section_mut(secnum) {
            Some(s) => s,
            None => return,
        };
        let be = match section.bibentries.get_entry_mut(citekey) {
            Some(be) => be,
            None => return,
        };
        parse_entry_names(be);

        let num_names = match be.names.get(&lnsource) {
            Some(n) => n.count(),
            None => return,
        };
        if num_names == 0 {
            return;
        }

        // Get uniquenametemplate
        let unt = match biber.config.getblxoption(None, "uniquenametemplate") {
            Some(ConfigValue::Map(m)) => m
                .get("global")
                .and_then(|v| v.as_list())
                .map(|v| v.to_vec()),
            _ => None,
        };

        let base_parts: Vec<String> = match &unt {
            Some(list) => list
                .iter()
                .filter_map(|item| {
                    if let ConfigValue::Map(m) = item {
                        let is_base = m
                            .get("base")
                            .and_then(|v| v.as_str())
                            .map(|s| s == "1")
                            .unwrap_or(false);
                        if is_base {
                            m.get("namepart")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect(),
            None => return,
        };

        if base_parts.is_empty() {
            return;
        }

        let bases: Vec<String> = be.names.get(&lnsource).map_or(Vec::new(), |names| {
            names
                .iter()
                .map(|name| {
                    let mut s = String::new();
                    for part in &base_parts {
                        if let Some(val) = name.get_namepart(part) {
                            s.push_str(val);
                        }
                    }
                    s
                })
                .collect()
        });

        (num_names, bases)
    };

    // --- Cross-entry tracking (no concurrent borrows) ---
    let has_lists = !biber.datalists.get_lists_for_section(secnum).is_empty();
    if !has_lists {
        return;
    }
    let (need_disambig, triggered_by) = {
        let mut lists = biber.datalists.get_lists_for_section_mut(secnum);
        let state = &mut lists[0].state;
        let tracking = state
            .seen_namedis_bases
            .entry(lnsource.clone())
            .or_default();

        let mut need = vec![false; num_names];
        let mut triggered: Vec<Vec<String>> = vec![Vec::new(); num_names];

        for (idx, base) in bases.iter().enumerate() {
            if base.is_empty() {
                continue;
            }
            for (other_key, other_bases) in tracking.iter() {
                if idx < other_bases.len() && &other_bases[idx] == base {
                    need[idx] = true;
                    triggered[idx].push(other_key.clone());
                }
            }
        }

        let tracking_key = citekey.to_string();
        tracking.insert(tracking_key, bases);

        (need, triggered)
    };

    // --- Apply disambiguation state to own entry ---
    if need_disambig.iter().any(|&v| v) {
        let section = match biber.sections.get_section_mut(secnum) {
            Some(s) => s,
            None => return,
        };
        let be = match section.bibentries.get_entry_mut(citekey) {
            Some(be) => be,
            None => return,
        };

        let nlist = match be.names.get_mut(&lnsource) {
            Some(n) => n,
            None => return,
        };

        for (idx, &dis) in need_disambig.iter().enumerate() {
            if dis {
                if let Some(name) = nlist.names.get_mut(idx) {
                    name.un = 1;
                    name.uniquepart = "base".to_string();
                    if !name.nameparts.contains_key("familyi") {
                        if let Some(f) = name.get_namepart("family") {
                            name.nameparts
                                .insert("familyi".into(), gen_initials(f).join(""));
                        }
                    }
                    if !name.nameparts.contains_key("giveni") {
                        if let Some(g) = name.get_namepart("given") {
                            name.nameparts
                                .insert("giveni".into(), gen_initials(g).join(""));
                        }
                    }
                }
            }
        }

        // Also mark previously-seen entries
        for (idx, others) in triggered_by.iter().enumerate() {
            if !need_disambig[idx] {
                continue;
            }
            for other_citekey in others {
                if let Some(other_be) = section.bibentries.get_entry_mut(other_citekey) {
                    let other_nlist = match other_be.names.get_mut(&lnsource) {
                        Some(n) => n,
                        None => continue,
                    };
                    if let Some(other_name) = other_nlist.names.get_mut(idx) {
                        other_name.un = 1;
                        other_name.uniquepart = "base".to_string();
                        if !other_name.nameparts.contains_key("familyi") {
                            if let Some(f) = other_name.get_namepart("family") {
                                other_name
                                    .nameparts
                                    .insert("familyi".into(), gen_initials(f).join(""));
                            }
                        }
                        if !other_name.nameparts.contains_key("giveni") {
                            if let Some(g) = other_name.get_namepart("given") {
                                other_name
                                    .nameparts
                                    .insert("giveni".into(), gen_initials(g).join(""));
                            }
                        }
                    }
                }
            }
        }
    }

    debug!("namedis for '{citekey}' (ln={lnsource})");
}

/// Generate fullhash, fullhashraw, and bibnamehash.
///
/// `fullhash` uses the namehashtemplate (like namehash) but without
/// list truncation. `fullhashraw` uses all nameparts without template
/// filtering (raw bib values).
fn process_fullhash(biber: &mut Biber, secnum: u32, citekey: &str) {
    let section = match biber.sections.get_section_mut(secnum) {
        Some(s) => s,
        None => return,
    };
    let be = match section.bibentries.get_entry_mut(citekey) {
        Some(be) => be,
        None => return,
    };

    parse_entry_names(be);

    let template = match biber.config.getblxoption(None, "namehashtemplate") {
        Some(ConfigValue::Map(m)) => m.get("global").and_then(|v| v.as_list()),
        _ => None,
    };

    let name_fields = ["author", "editor", "translator", "bookauthor"];
    let mut hashkey_template = String::new();
    let mut hashkey_raw = String::new();
    let namepart_order = ["family", "given", "prefix", "suffix"];

    for field in &name_fields {
        if let Some(names) = be.names.get(*field) {
            for name in names.iter() {
                // fullhash: respect namehashtemplate (like namehash)
                if let Some(template) = template {
                    for item in template {
                        if let ConfigValue::Map(m) = item {
                            let np = m.get("namepart").and_then(|v| v.as_str()).unwrap_or("");
                            if let Some(val) = name.get_namepart(np) {
                                hashkey_template.push_str(val);
                            }
                        }
                    }
                }
                // fullhashraw: all nameparts, no template filtering
                for np in &namepart_order {
                    if let Some(val) = name.get_namepart(np) {
                        hashkey_raw.push_str(val);
                    }
                }
            }
        }
    }

    let hash_template = hex::encode(Md5::digest(hashkey_template.as_bytes()));
    be.set_field_str("fullhash", &hash_template);

    let hash_raw = hex::encode(Md5::digest(hashkey_raw.as_bytes()));
    be.set_field_str("fullhashraw", &hash_raw);

    // bibnamehash uses _getnamehash with bib visibility; for now same as fullhash
    be.set_field_str("bibnamehash", &hash_template);

    debug!("fullhash for '{citekey}' = {hash_template}");
}

/// Generate labelalpha and sortlabelalpha using the template engine.
///
/// This replaces the earlier simplified version with the full
/// `labelalphatemplate` support from `Biber::Internals::_genlabel`.
fn process_labelalpha(biber: &mut Biber, secnum: u32, citekey: &str) {
    debug!("process_labelalpha for '{citekey}'");

    // Check labelalpha option (default true)
    let enabled = biber
        .config
        .getblxoption_str("labelalpha")
        .map(|s| s == "1" || s == "true")
        .unwrap_or(true);
    if !enabled {
        return;
    }

    // Check skiplab option per entry
    let skiplab = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        let be = match section.bibentry(citekey) {
            Some(be) => be,
            None => return,
        };
        let entrytype = &be.entrytype;
        biber
            .config
            .getblxoption_for_entry_str(entrytype, "skiplab")
            .map(|s| s == "1" || s == "true")
            .unwrap_or(false)
    };
    if skiplab {
        return;
    }

    // Parse labelalphatemplate from config
    let templates = {
        let raw = biber
            .config
            .getblxoption(None, "labelalphatemplate")
            .cloned();
        match raw {
            Some(cv) => crate::label_alpha::parse_labelalphatemplate_config(&cv),
            None => HashMap::new(),
        }
    };

    // Parse labelalphanametemplate from config
    let name_templates = {
        let raw = biber
            .config
            .getblxoption(None, "labelalphanametemplate")
            .cloned();
        match raw {
            Some(cv) => crate::label_alpha::parse_labelalphanametemplate_config(&cv),
            None => HashMap::new(),
        }
    };

    // Generate the label
    let (label, slabel) = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        let be = match section.bibentry(citekey) {
            Some(be) => be,
            None => return,
        };

        // Build state with section citekeys
        let section_citekeys = section.get_citekeys().to_vec();
        let mut state = crate::label_alpha::LabelAlphaState {
            section_citekeys,
            config: &biber.config,
            labelcache_v: HashMap::new(),
            labelcache_l: HashMap::new(),
            visible_alpha: HashMap::new(),
            morenames: HashMap::new(),
            label_final: false,
        };

        crate::label_alpha::gen_label(
            citekey,
            be,
            &biber.config,
            &templates,
            &name_templates,
            &mut state,
        )
    };

    if label.is_empty() {
        return;
    }

    trace!("labelalpha for '{citekey}': label='{label}', sortlabel='{slabel}'");

    // Store in all datalists for this section
    for dl in biber.datalists.get_lists_for_section_mut(secnum) {
        dl.state
            .labelalphadata
            .insert(citekey.to_string(), label.clone());
        dl.state
            .sortlabelalphadata
            .insert(citekey.to_string(), slabel.clone());
    }
}

/// Track extraalpha information (per-entry: count labelalpha duplicates).
fn process_extraalpha(biber: &mut Biber, secnum: u32, citekey: &str) {
    debug!("process_extraalpha for '{citekey}'");

    let enabled = biber
        .config
        .getblxoption_str("labelalpha")
        .map(|s| s == "1" || s == "true")
        .unwrap_or(true);
    if !enabled {
        return;
    }

    for dl in biber.datalists.get_lists_for_section_mut(secnum) {
        if let Some(la) = dl.state.labelalphadata.get(citekey) {
            let key = la.clone();
            *dl.state.ladisambiguation.entry(key).or_default() += 1;
        }
    }
}

/// Assign extraalpha letters for entries whose labelalpha has duplicates.
/// Called after all entries have been processed by `process_labelalpha` / `process_extraalpha`.
fn assign_extraalpha_letters(biber: &mut Biber, secnum: u32) {
    debug!("assign_extraalpha_letters for section {secnum}");

    let enabled = biber
        .config
        .getblxoption_str("labelalpha")
        .map(|s| s == "1" || s == "true")
        .unwrap_or(true);
    if !enabled {
        return;
    }

    let citekeys: Vec<String> = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s.get_citekeys().to_vec(),
            None => return,
        };
        section
    };

    for dl in biber.datalists.get_lists_for_section_mut(secnum) {
        for citekey in &citekeys {
            if let Some(la) = dl.state.labelalphadata.get(citekey) {
                if dl.state.ladisambiguation.get(la).copied().unwrap_or(0) > 1 {
                    let counter = dl.state.seen_extraalpha.entry(la.clone()).or_insert(0);
                    *counter += 1;
                    dl.state
                        .extraalphadata
                        .insert(citekey.clone(), counter.to_string());
                }
            }
        }
    }
}

/// Track extradate information.
///
/// Builds a tracking key from `extradatespec` fields and context, stores
/// per-entry tracking info in the datalist state. Actual letter assignment
/// happens in `assign_extradate_letters` after all entries are processed.
fn process_extradate(biber: &mut Biber, secnum: u32, citekey: &str) {
    // Check labeldateparts
    let labeldateparts = biber
        .config
        .getblxoption_str("labeldateparts")
        .map(|s| s == "1" || s == "true")
        .unwrap_or(true);
    if !labeldateparts {
        return;
    }

    // Get extradatespec from config (list of scopes, each a list of fields)
    let edspec: Vec<Vec<String>> = {
        match biber.config.getblxoption(None, "extradatespec") {
            Some(ConfigValue::List(scopes)) => scopes
                .iter()
                .filter_map(|s| match s {
                    ConfigValue::List(fields) => Some(
                        fields
                            .iter()
                            .filter_map(|f| f.as_str().map(|s| s.to_string()))
                            .collect(),
                    ),
                    _ => None,
                })
                .collect(),
            _ => return,
        }
    };

    if edspec.is_empty() {
        return;
    }

    // Build date string and extradatescope
    let (datestring, edscope) = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        let be = match section.bibentry(citekey) {
            Some(be) => be,
            None => return,
        };

        let mut ds = String::new();
        let mut scope = String::new();
        for scope_fields in &edspec {
            for field_name in scope_fields {
                if let Some(val) = be.get_field_str(field_name) {
                    ds.push_str(val);
                    if scope.is_empty() {
                        scope = field_name.clone();
                    }
                    break;
                }
            }
        }
        (ds, scope)
    };

    // Build context from labelname source field value
    let context = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        let be = match section.bibentry(citekey) {
            Some(be) => be,
            None => return,
        };
        be.get_field_str("labelname")
            .and_then(|ln| be.get_field_str(ln))
            .unwrap_or("")
            .to_string()
    };

    let tracking_string = if context.is_empty() {
        datestring.clone()
    } else {
        format!("{},{}", context, datestring)
    };

    // Update entry fields and tracking state
    {
        let section = match biber.sections.get_section_mut(secnum) {
            Some(s) => s,
            None => return,
        };
        let be = match section.bibentries.get_entry_mut(citekey) {
            Some(be) => be,
            None => return,
        };
        if !edscope.is_empty() {
            be.set_field_str("extradatescope", &edscope);
        }
    }

    let has_lists = !biber.datalists.get_lists_for_section(secnum).is_empty();
    if !has_lists {
        return;
    }
    let mut lists = biber.datalists.get_lists_for_section_mut(secnum);
    let state = &mut lists[0].state;
    state
        .nametitledateparts
        .insert(citekey.to_string(), tracking_string.clone());
    *state
        .seen_nametitledateparts
        .entry(tracking_string)
        .or_insert(0) += 1;
}

/// Assign extradate letters based on tracking data.
///
/// For each entry whose `nametitledateparts` tracking key was seen more
/// than once, assigns a counter (1='a', 2='b', ...) as the `extradate` field.
fn assign_extradate_letters(biber: &mut Biber, secnum: u32) {
    let citekeys: Vec<String> = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s.get_citekeys().to_vec(),
            None => return,
        };
        section
    };

    let has_lists = !biber.datalists.get_lists_for_section(secnum).is_empty();
    if !has_lists {
        return;
    }

    let mut to_assign: Vec<(String, String)> = Vec::new();
    {
        let mut lists = biber.datalists.get_lists_for_section_mut(secnum);
        let state = &mut lists[0].state;

        for citekey in &citekeys {
            if let Some(ts) = state.nametitledateparts.get(citekey) {
                let count = state.seen_nametitledateparts.get(ts).copied().unwrap_or(0);
                if count > 1 {
                    let entry = state.seen_extradate.entry(ts.clone()).or_insert(0);
                    *entry += 1;
                    state.extradatedata.insert(citekey.clone(), *entry);
                    // Store the raw counter value (biblatex converts 1→'a', 2→'b', ...)
                    to_assign.push((citekey.clone(), entry.to_string()));
                }
            }
        }
    }

    for (citekey, val) in &to_assign {
        if let Some(section) = biber.sections.get_section_mut(secnum) {
            if let Some(be) = section.bibentries.get_entry_mut(citekey) {
                be.set_field_str("extradate", val);
            }
        }
    }
}

/// Track extraname information.
///
/// Builds a labelnamehash from the labelname field's name parts and tracks
/// how many entries share it. Letters are assigned post-loop.
fn process_extraname(biber: &mut Biber, secnum: u32, citekey: &str) {
    debug!("process_extraname for '{citekey}'");

    let ln_field = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        let be = match section.bibentry(citekey) {
            Some(be) => be,
            None => return,
        };
        match be.get_field_str("labelname") {
            Some(s) => s.to_string(),
            None => return,
        }
    };

    let hashkey = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        let be = match section.bibentry(citekey) {
            Some(be) => be,
            None => return,
        };
        match be.names.get(&ln_field) {
            Some(names) if !names.is_empty() => {
                let mut s = String::new();
                for name in names.iter() {
                    if let Some(family) = name.family() {
                        s.push_str(family);
                    }
                    if let Some(given) = name.given() {
                        s.push_str(given);
                    }
                    if let Some(prefix) = name.get_namepart("prefix") {
                        s.push_str(prefix);
                    }
                    if let Some(suffix) = name.get_namepart("suffix") {
                        s.push_str(suffix);
                    }
                }
                if s.is_empty() {
                    return;
                }
                s
            }
            _ => return,
        }
    };

    for dl in biber.datalists.get_lists_for_section_mut(secnum) {
        dl.state
            .labelnamehash
            .insert(citekey.to_string(), hashkey.clone());
        *dl.state.seen_labelname.entry(hashkey.clone()).or_default() += 1;
    }
}

/// Track extratitle information.
///
/// Builds a nametitle string from (labelnamehash, labeltitle) and tracks
/// how many entries share it.
fn process_extratitle(biber: &mut Biber, secnum: u32, citekey: &str) {
    debug!("process_extratitle for '{citekey}'");

    let enabled = biber
        .config
        .getblxoption_str("labeltitle")
        .map(|s| s == "1" || s == "true")
        .unwrap_or(false);
    if !enabled {
        return;
    }

    let (namehash, title_string) = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        let be = match section.bibentry(citekey) {
            Some(be) => be,
            None => return,
        };

        let namehash = {
            let ln_field: &str = be.get_field_str("labelname").unwrap_or_default();
            if ln_field.is_empty() {
                String::new()
            } else {
                match be.names.get(ln_field) {
                    Some(names) if !names.is_empty() => {
                        let mut s = String::new();
                        for name in names.iter() {
                            if let Some(family) = name.family() {
                                s.push_str(family);
                            }
                            if let Some(given) = name.given() {
                                s.push_str(given);
                            }
                        }
                        s
                    }
                    _ => String::new(),
                }
            }
        };

        let title_field: &str = be.get_field_str("labeltitle").unwrap_or_default();
        let title_string = if title_field.is_empty() {
            String::new()
        } else {
            be.get_field_str(title_field).unwrap_or("").to_string()
        };

        (namehash, title_string)
    };

    let nametitle_string = if namehash.is_empty() {
        format!(",{title_string}")
    } else {
        format!("{namehash},{title_string}")
    };

    for dl in biber.datalists.get_lists_for_section_mut(secnum) {
        dl.state
            .nametitle
            .insert(citekey.to_string(), nametitle_string.clone());

        let entry = dl
            .state
            .seen_nametitle
            .entry(nametitle_string.clone())
            .or_insert(0);
        if *entry == 0 || !title_string.is_empty() {
            *entry += 1;
        }
    }
}

/// Track extratitleyear information.
///
/// Builds a titleyear string from (labeltitle, labelyear) and tracks how
/// many entries share it.
fn process_extratitleyear(biber: &mut Biber, secnum: u32, citekey: &str) {
    debug!("process_extratitleyear for '{citekey}'");

    let enabled = biber
        .config
        .getblxoption_str("labeltitleyear")
        .map(|s| s == "1" || s == "true")
        .unwrap_or(false);
    if !enabled {
        return;
    }

    let (title_string, year_string) = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        let be = match section.bibentry(citekey) {
            Some(be) => be,
            None => return,
        };

        let title_field: &str = be.get_field_str("labeltitle").unwrap_or_default();
        let title_string = if title_field.is_empty() {
            String::new()
        } else {
            be.get_field_str(title_field).unwrap_or("").to_string()
        };

        let year_string = be
            .get_field_str("labelyear")
            .or_else(|| be.get_field_str("year"))
            .map(|s| s.to_string())
            .unwrap_or_default();

        (title_string, year_string)
    };

    let titleyear_string = format!("{title_string},{year_string}");

    for dl in biber.datalists.get_lists_for_section_mut(secnum) {
        dl.state
            .titleyear
            .insert(citekey.to_string(), titleyear_string.clone());

        let entry = dl
            .state
            .seen_titleyear
            .entry(titleyear_string.clone())
            .or_insert(0);
        if *entry == 0 || !title_string.is_empty() {
            *entry += 1;
        }
    }
}

/// Assign extra name/title/year letters for entries whose tracking data
/// has duplicates. Called after all entries processed.
fn assign_extraname_extratitle_letters(biber: &mut Biber, secnum: u32) {
    debug!("assign_extraname_extratitle_letters for section {secnum}");

    let citekeys: Vec<String> = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s.get_citekeys().to_vec(),
            None => return,
        };
        section
    };

    let labeltitle_enabled = biber
        .config
        .getblxoption_str("labeltitle")
        .map(|s| s == "1" || s == "true")
        .unwrap_or(false);
    let labeltitleyear_enabled = biber
        .config
        .getblxoption_str("labeltitleyear")
        .map(|s| s == "1" || s == "true")
        .unwrap_or(false);

    for dl in biber.datalists.get_lists_for_section_mut(secnum) {
        // extraname
        for citekey in &citekeys {
            if let Some(lh) = dl.state.labelnamehash.get(citekey).cloned() {
                let count = dl.state.seen_labelname.get(&lh).copied().unwrap_or(0);
                if count > 1 {
                    // Use seen_extraalpha as per-labelnamehash counter for extraname
                    let counter = dl.state.seen_extraalpha.entry(lh).or_insert(0);
                    *counter += 1;
                    dl.state
                        .extranamedata
                        .insert(citekey.clone(), counter.to_string());
                }
            }
        }

        // extratitle
        if labeltitle_enabled {
            for citekey in &citekeys {
                if let Some(nt) = dl.state.nametitle.get(citekey).cloned() {
                    let count = dl.state.seen_nametitle.get(&nt).copied().unwrap_or(0);
                    if count > 1 {
                        // Use ladisambiguation as per-nametitle counter for extratitle
                        let counter = dl.state.ladisambiguation.entry(nt).or_insert(0);
                        *counter += 1;
                        dl.state
                            .extratitledata
                            .insert(citekey.clone(), counter.to_string());
                    }
                }
            }
        }

        // extratitleyear
        if labeltitleyear_enabled {
            for citekey in &citekeys {
                if let Some(ty) = dl.state.titleyear.get(citekey).cloned() {
                    let count = dl.state.seen_titleyear.get(&ty).copied().unwrap_or(0);
                    if count > 1 {
                        let counter = dl.state.ladisambiguation.entry(ty).or_insert(0);
                        *counter += 1;
                        dl.state
                            .extratitleyeardata
                            .insert(citekey.clone(), counter.to_string());
                    }
                }
            }
        }
    }
}

/// Compute a hash from a specific namelist (like Perl's _getfullhash for one field).
fn compute_namelist_hash(biber: &Biber, citekey: &str, field: &str, secnum: u32) -> Option<String> {
    let section = biber.sections.get_section(secnum)?;
    let be = section.bibentry(citekey)?;
    let names = be.names.get(field)?;

    let mut hashkey = String::new();
    let namepart_order = ["family", "given", "prefix", "suffix"];

    for name in names.iter() {
        for np in &namepart_order {
            if let Some(val) = name.get_namepart(np) {
                hashkey.push_str(val);
            }
        }
    }

    if hashkey.is_empty() {
        return None;
    }

    Some(hex::encode(Md5::digest(hashkey.as_bytes())))
}

/// Track seen work combination for singletitle etc.
///
/// For each entry, builds identifiers that are stored as entry fields
/// (`seenname`, `seentitle`, `seenbaretitle`, `seenwork`) and increments
/// corresponding counters in datalist state. The generators then read
/// these counters to set `singletitle`, `uniquetitle`, etc.
fn process_workuniqueness(biber: &mut Biber, secnum: u32, citekey: &str) {
    let lni;
    let lti;
    let has_options_skipbib;
    {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        let be = match section.bibentry(citekey) {
            Some(be) => be,
            None => return,
        };
        lni = be.get_field_str("labelname").map(|s| s.to_string());
        lti = be.get_field_str("labeltitle").map(|s| s.to_string());

        // If this is a data clone with skipbib set, don't record uniqueness info
        has_options_skipbib = be
            .get_field("options")
            .and_then(|v| v.as_list())
            .map(|list| {
                list.iter()
                    .any(|opt| matches!(opt, ConfigValue::Str(s) if s == "skipbib"))
            })
            .unwrap_or(false);
    }

    if has_options_skipbib {
        return;
    }

    let lists = biber.datalists.get_lists_for_section_mut(secnum);
    if lists.is_empty() {
        return;
    }

    // We must borrow biber again for compute_namelist_hash. Use a read-only borrow.
    // singletitle: identifier = fullhash of labelname field
    if let Some(ref lni) = lni {
        if biber.config.getblxoption_str("singletitle") == Some("1") {
            if let Some(hash) = compute_namelist_hash(biber, citekey, lni, secnum) {
                for dl in &mut *biber.datalists.get_lists_for_section_mut(secnum) {
                    *dl.state.seenname.entry(hash.clone()).or_insert(0) += 1;
                }
                if let Some(be) = biber
                    .sections
                    .get_section_mut(secnum)
                    .and_then(|s| s.bibentries.get_entry_mut(citekey))
                {
                    be.set_field_str("seenname", &hash);
                }
            }
        }
    }

    // uniquetitle: identifier = labeltitle field value
    if let Some(ref lti) = lti {
        if biber.config.getblxoption_str("uniquetitle") == Some("1") {
            let identifier = biber
                .sections
                .get_section(secnum)
                .and_then(|s| s.bibentry(citekey))
                .and_then(|be| be.get_field_str(lti))
                .map(|s| s.to_string());
            if let Some(ref identifier) = identifier {
                if !identifier.is_empty() {
                    for dl in &mut *biber.datalists.get_lists_for_section_mut(secnum) {
                        *dl.state.seentitle.entry(identifier.clone()).or_insert(0) += 1;
                    }
                    if let Some(be) = biber
                        .sections
                        .get_section_mut(secnum)
                        .and_then(|s| s.bibentries.get_entry_mut(citekey))
                    {
                        be.set_field_str("seentitle", identifier);
                    }
                }
            }
        }
    }

    // uniquebaretitle: identifier = labeltitle value, but only when labelname is absent
    if lni.is_none() {
        if let Some(ref lti) = lti {
            if biber.config.getblxoption_str("uniquebaretitle") == Some("1") {
                let identifier = biber
                    .sections
                    .get_section(secnum)
                    .and_then(|s| s.bibentry(citekey))
                    .and_then(|be| be.get_field_str(lti))
                    .map(|s| s.to_string());
                if let Some(ref identifier) = identifier {
                    if !identifier.is_empty() {
                        for dl in &mut *biber.datalists.get_lists_for_section_mut(secnum) {
                            *dl.state
                                .seenbaretitle
                                .entry(identifier.clone())
                                .or_insert(0) += 1;
                        }
                        if let Some(be) = biber
                            .sections
                            .get_section_mut(secnum)
                            .and_then(|s| s.bibentries.get_entry_mut(citekey))
                        {
                            be.set_field_str("seenbaretitle", identifier);
                        }
                    }
                }
            }
        }
    }

    // uniquework: identifier = fullhash(labelname) + labeltitle value
    if let (Some(ref lni), Some(ref lti)) = (lni, lti) {
        if biber.config.getblxoption_str("uniquework") == Some("1") {
            let name_hash = compute_namelist_hash(biber, citekey, lni, secnum);
            let title_val = biber
                .sections
                .get_section(secnum)
                .and_then(|s| s.bibentry(citekey))
                .and_then(|be| be.get_field_str(lti))
                .unwrap_or("")
                .to_string();
            if let Some(ref nh) = name_hash {
                let identifier = format!("{nh}{title_val}");
                for dl in &mut *biber.datalists.get_lists_for_section_mut(secnum) {
                    *dl.state.seenwork.entry(identifier.clone()).or_insert(0) += 1;
                }
                if let Some(be) = biber
                    .sections
                    .get_section_mut(secnum)
                    .and_then(|s| s.bibentries.get_entry_mut(citekey))
                {
                    be.set_field_str("seenwork", &identifier);
                }
            }
        }
    }
}

/// Generate namehash.
fn process_namehash(biber: &mut Biber, secnum: u32, citekey: &str) {
    let section = match biber.sections.get_section_mut(secnum) {
        Some(s) => s,
        None => return,
    };
    let be = match section.bibentries.get_entry_mut(citekey) {
        Some(be) => be,
        None => return,
    };

    // Parse names if not already done
    parse_entry_names(be);

    // Get namehashtemplate from config
    let template = match biber.config.getblxoption(None, "namehashtemplate") {
        Some(ConfigValue::Map(m)) => m.get("global").and_then(|v| v.as_list()),
        _ => None,
    };

    let name_fields = ["author", "editor", "translator", "bookauthor"];
    let mut input = String::new();

    for field in &name_fields {
        let names = match be.names.get(*field) {
            Some(n) => n,
            None => continue,
        };
        for name in names.iter() {
            if let Some(template) = template {
                for item in template {
                    if let ConfigValue::Map(m) = item {
                        let namepart = m.get("namepart").and_then(|v| v.as_str()).unwrap_or("");
                        if let Some(val) = name.get_namepart(namepart) {
                            input.push_str(val);
                        }
                    }
                }
            }
        }
    }

    let hash = hex::encode(Md5::digest(input.as_bytes()));
    be.set_field_str("namehash", &hash);
    debug!("namehash for '{citekey}' = {hash}");
}

/// Generate per-name hashes for each name field.
///
/// For each name field, computes `{field}namehash`, `{field}fullhash`,
/// `{field}fullhashraw`, `{field}bibnamehash`.
fn process_pername_hashes(biber: &mut Biber, secnum: u32, citekey: &str) {
    let section = match biber.sections.get_section_mut(secnum) {
        Some(s) => s,
        None => return,
    };
    let be = match section.bibentries.get_entry_mut(citekey) {
        Some(be) => be,
        None => return,
    };

    parse_entry_names(be);

    let template = match biber.config.getblxoption(None, "namehashtemplate") {
        Some(ConfigValue::Map(m)) => m.get("global").and_then(|v| v.as_list()),
        _ => None,
    };

    let name_fields = ["author", "editor", "translator", "bookauthor"];
    let namepart_order = ["family", "given", "prefix", "suffix"];

    for field in &name_fields {
        let mut hashkey_name = String::new();
        let mut hashkey_full = String::new();

        if let Some(names) = be.names.get(*field) {
            for name in names.iter() {
                // namehash: respect namehashtemplate
                if let Some(template) = template {
                    for item in template {
                        if let ConfigValue::Map(m) = item {
                            let np = m.get("namepart").and_then(|v| v.as_str()).unwrap_or("");
                            if let Some(val) = name.get_namepart(np) {
                                hashkey_name.push_str(val);
                            }
                        }
                    }
                }
                // fullhash: all nameparts, no template filtering
                for np in &namepart_order {
                    if let Some(val) = name.get_namepart(np) {
                        hashkey_full.push_str(val);
                    }
                }
            }
        }

        if !hashkey_name.is_empty() {
            let hash = hex::encode(Md5::digest(hashkey_name.as_bytes()));
            be.set_field_str(format!("{field}namehash"), &hash);
            be.set_field_str(format!("{field}bibnamehash"), &hash);
        }
        if !hashkey_full.is_empty() {
            let hash = hex::encode(Md5::digest(hashkey_full.as_bytes()));
            be.set_field_str(format!("{field}fullhash"), &hash);
            be.set_field_str(format!("{field}fullhashraw"), &hash);
        }
    }

    debug!("pername_hashes for '{citekey}'");
}

/// Track seen primary author base names.
/// Track seen primary author base names for uniqueprimaryauthor.
fn process_uniqueprimaryauthor(biber: &mut Biber, secnum: u32, citekey: &str) {
    let lni = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        let be = match section.bibentry(citekey) {
            Some(be) => be,
            None => return,
        };
        match be.get_field_str("labelname") {
            Some(s) => s.to_string(),
            None => return,
        }
    };

    if biber.config.getblxoption_str("uniqueprimaryauthor") != Some("1") {
        return;
    }

    // Parse names and get the base name from name disambiguation template
    let pabase = {
        let section = match biber.sections.get_section_mut(secnum) {
            Some(s) => s,
            None => return,
        };
        let be = match section.bibentries.get_entry_mut(citekey) {
            Some(be) => be,
            None => return,
        };
        parse_entry_names(be);

        let names = match be.names.get(&lni) {
            Some(n) => n,
            None => return,
        };
        if names.count() == 0 {
            return;
        }

        // Get first name's base from uniquenametemplate
        let unt = match biber.config.getblxoption(None, "uniquenametemplate") {
            Some(ConfigValue::Map(m)) => m
                .get("global")
                .and_then(|v| v.as_list())
                .map(|v| v.to_vec()),
            _ => None,
        };

        let base_parts: Vec<String> = match &unt {
            Some(list) => list
                .iter()
                .filter_map(|item| {
                    if let ConfigValue::Map(m) = item {
                        let is_base = m
                            .get("base")
                            .and_then(|v| v.as_str())
                            .map(|s| s == "1")
                            .unwrap_or(false);
                        if is_base {
                            m.get("namepart")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                })
                .collect(),
            None => return,
        };

        if base_parts.is_empty() {
            return;
        }

        let first_name = match names.iter().next() {
            Some(n) => n,
            None => return,
        };

        let mut base = String::new();
        for part in &base_parts {
            if let Some(val) = first_name.get_namepart(part) {
                base.push_str(val);
            }
        }

        base
    };

    if pabase.is_empty() {
        return;
    }

    let first_hash = {
        let section = match biber.sections.get_section(secnum) {
            Some(s) => s,
            None => return,
        };
        let be = match section.bibentry(citekey) {
            Some(be) => be,
            None => return,
        };
        let names = match be.names.get(&lni) {
            Some(n) => n,
            None => return,
        };
        let first_name = match names.iter().next() {
            Some(n) => n,
            None => return,
        };
        let mut hashkey = String::new();
        for part in ["family", "given", "prefix", "suffix"] {
            if let Some(val) = first_name.get_namepart(part) {
                hashkey.push_str(val);
            }
        }
        if hashkey.is_empty() {
            String::new()
        } else {
            hex::encode(Md5::digest(hashkey.as_bytes()))
        }
    };

    for dl in biber.datalists.get_lists_for_section_mut(secnum) {
        dl.state
            .seenpa
            .entry(pabase.clone())
            .or_default()
            .entry(first_hash.clone())
            .or_insert(true);
    }

    if let Some(be) = biber
        .sections
        .get_section_mut(secnum)
        .and_then(|s| s.bibentries.get_entry_mut(citekey))
    {
        be.set_field_str("seenprimaryauthor", &pabase);
    }
}

/// Generate singletitle field.
fn generate_singletitle(biber: &mut Biber, secnum: u32, citekey: &str) {
    if biber.config.getblxoption_str("singletitle") != Some("1") {
        return;
    }
    let count = biber
        .sections
        .get_section(secnum)
        .and_then(|s| s.bibentry(citekey))
        .and_then(|be| be.get_field_str("seenname"))
        .and_then(|sn| {
            biber
                .datalists
                .get_lists_for_section(secnum)
                .first()
                .and_then(|dl| dl.state.seenname.get(sn).copied())
        })
        .unwrap_or(0);
    if count < 2 {
        trace!("singletitle for '{citekey}': unique (count={count})");
        if let Some(be) = biber
            .sections
            .get_section_mut(secnum)
            .and_then(|s| s.bibentries.get_entry_mut(citekey))
        {
            be.set_field_str("singletitle", "1");
        }
    }
}

/// Generate uniquetitle field.
fn generate_uniquetitle(biber: &mut Biber, secnum: u32, citekey: &str) {
    if biber.config.getblxoption_str("uniquetitle") != Some("1") {
        return;
    }
    let count = biber
        .sections
        .get_section(secnum)
        .and_then(|s| s.bibentry(citekey))
        .and_then(|be| be.get_field_str("seentitle"))
        .and_then(|ut| {
            biber
                .datalists
                .get_lists_for_section(secnum)
                .first()
                .and_then(|dl| dl.state.seentitle.get(ut).copied())
        })
        .unwrap_or(0);
    if count < 2 {
        if let Some(be) = biber
            .sections
            .get_section_mut(secnum)
            .and_then(|s| s.bibentries.get_entry_mut(citekey))
        {
            be.set_field_str("uniquetitle", "1");
        }
    }
}

/// Generate uniquebaretitle field.
fn generate_uniquebaretitle(biber: &mut Biber, secnum: u32, citekey: &str) {
    if biber.config.getblxoption_str("uniquebaretitle") != Some("1") {
        return;
    }
    let count = biber
        .sections
        .get_section(secnum)
        .and_then(|s| s.bibentry(citekey))
        .and_then(|be| be.get_field_str("seenbaretitle"))
        .and_then(|ubt| {
            biber
                .datalists
                .get_lists_for_section(secnum)
                .first()
                .and_then(|dl| dl.state.seenbaretitle.get(ubt).copied())
        })
        .unwrap_or(0);
    if count < 2 {
        if let Some(be) = biber
            .sections
            .get_section_mut(secnum)
            .and_then(|s| s.bibentries.get_entry_mut(citekey))
        {
            be.set_field_str("uniquebaretitle", "1");
        }
    }
}

/// Generate uniquework field.
fn generate_uniquework(biber: &mut Biber, secnum: u32, citekey: &str) {
    if biber.config.getblxoption_str("uniquework") != Some("1") {
        return;
    }
    let count = biber
        .sections
        .get_section(secnum)
        .and_then(|s| s.bibentry(citekey))
        .and_then(|be| be.get_field_str("seenwork"))
        .and_then(|sw| {
            biber
                .datalists
                .get_lists_for_section(secnum)
                .first()
                .and_then(|dl| dl.state.seenwork.get(sw).copied())
        })
        .unwrap_or(0);
    if count < 2 {
        if let Some(be) = biber
            .sections
            .get_section_mut(secnum)
            .and_then(|s| s.bibentries.get_entry_mut(citekey))
        {
            be.set_field_str("uniquework", "1");
        }
    }
}

/// Generate uniqueprimaryauthor field.
fn generate_uniquepa(biber: &mut Biber, secnum: u32, citekey: &str) {
    if biber.config.getblxoption_str("uniqueprimaryauthor") != Some("1") {
        return;
    }
    let seenpa_count = biber
        .sections
        .get_section(secnum)
        .and_then(|s| s.bibentry(citekey))
        .and_then(|be| be.get_field_str("seenprimaryauthor"))
        .and_then(|spa| {
            biber
                .datalists
                .get_lists_for_section(secnum)
                .first()
                .and_then(|dl| dl.state.seenpa.get(spa).map(|hashes| hashes.len()))
        })
        .unwrap_or(0);
    if seenpa_count < 2 {
        if let Some(be) = biber
            .sections
            .get_section_mut(secnum)
            .and_then(|s| s.bibentries.get_entry_mut(citekey))
        {
            be.set_field_str("uniqueprimaryauthor", "1");
        }
    }
}

// ---- Per-list passes (sortinit, sortinithash, labelprefix) ----

/// Sort field separator used between sort group values.
const SORT_SEP: &str = ",";

/// Apply substring truncation to a sort value.
fn apply_substring(val: &str, attrs: &BTreeMap<String, ConfigValue>) -> String {
    let width = attrs
        .get("substring_width")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<usize>().ok());
    let side = attrs.get("substring_side").and_then(|v| v.as_str());

    match (width, side) {
        (Some(w), Some(s)) if !s.is_empty() && w > 0 && w < val.chars().count() => {
            if s == "left" {
                val.chars().take(w).collect()
            } else {
                val.chars()
                    .rev()
                    .take(w)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect()
            }
        }
        _ => val.to_string(),
    }
}

/// Apply padding to a sort value.
fn apply_padding(val: &str, attrs: &BTreeMap<String, ConfigValue>) -> String {
    let width = attrs
        .get("pad_width")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<usize>().ok());
    let default_char = " ".to_string();
    let pad_char = attrs
        .get("pad_char")
        .and_then(|v| v.as_str())
        .unwrap_or(&default_char);
    let pad_side = attrs
        .get("pad_side")
        .and_then(|v| v.as_str())
        .unwrap_or("right");

    match width {
        Some(w) if w > val.len() => {
            let padding = pad_char.repeat(w - val.len());
            if pad_side == "left" {
                format!("{}{}", padding, val)
            } else {
                format!("{}{}", val, padding)
            }
        }
        _ => val.to_string(),
    }
}

/// Get the family name of the first name in a name field.
fn get_first_name_family(be: &Entry, field_name: &str) -> Option<String> {
    // Try parsed names first
    if let Some(names) = be.names.get(field_name) {
        if let Some(name) = names.iter().next() {
            if let Some(family) = name.get_namepart("family") {
                if !family.is_empty() {
                    return Some(family.to_string());
                }
            }
        }
    }
    // Fallback to raw text, take last word as family name
    if let Some(raw) = be.get_field_str(field_name) {
        let parts: Vec<&str> = raw.split_whitespace().collect();
        if let Some(last) = parts.last() {
            if !last.is_empty() {
                return Some(last.to_string());
            }
        }
        return Some(raw.to_string());
    }
    None
}

/// Get the sort value for a given field name from an entry.
fn get_sort_field_raw(field_name: &str, be: &Entry, cite_index: usize) -> Option<String> {
    match field_name {
        "presort" => Some(be.get_field_str("presort").unwrap_or("mm").to_string()),
        "sortkey" => be.get_field_str("sortkey").map(|s| s.to_string()),
        "sortname" => {
            let lnsource = be.get_field_str("labelname")?;
            get_first_name_family(be, lnsource)
        }
        "sorttitle" => be
            .get_field_str("sorttitle")
            .or_else(|| be.get_field_str("title"))
            .map(|s| s.to_string()),
        "sortyear" => be
            .get_field_str("sortyear")
            .or_else(|| be.get_field_str("year"))
            .or_else(|| be.get_field_str("labelyear"))
            .map(|s| s.to_string()),
        "citeorder" => Some((cite_index + 1).to_string()),
        "entrykey" => Some(be.citekey.clone()),
        "entrytype" => Some(be.entrytype.clone()),
        "shorthand" | "sortshorthand" => be.get_field_str("shorthand").map(|s| s.to_string()),
        "labelalpha" => be.get_field_str("labelalpha").map(|s| s.to_string()),
        "labelname" => be.get_field_str("labelname").map(|s| s.to_string()),
        "labeltitle" => be.get_field_str("labeltitle").map(|s| s.to_string()),
        "labelyear" => be
            .get_field_str("labelyear")
            .or_else(|| be.get_field_str("year"))
            .map(|s| s.to_string()),
        "labelmonth" => be
            .get_field_str("labelmonth")
            .or_else(|| be.get_field_str("month"))
            .map(|s| s.to_string()),
        "labelday" => be
            .get_field_str("labelday")
            .or_else(|| be.get_field_str("day"))
            .map(|s| s.to_string()),
        "volume" => be.get_field_str("volume").map(|s| s.to_string()),
        author if ["author", "editor", "translator", "bookauthor"].contains(&author) => {
            get_first_name_family(be, author)
        }
        _ => be.get_field_str(field_name).map(|s| s.to_string()),
    }
}

/// Build a sort key string from a sorting template spec for a given entry.
fn build_sort_string(
    specs: &[ConfigValue],
    be: &Entry,
    cite_index: usize,
    translit_rules: &[crate::transliteration::TranslitRule],
) -> String {
    let mut parts: Vec<String> = Vec::new();
    let entrytype = &be.entrytype;
    let langid = be.get_field_str("langid");

    for spec in specs {
        let group = match spec {
            ConfigValue::Map(m) => m,
            _ => continue,
        };

        let is_final = group
            .get("final")
            .and_then(|v| v.as_str())
            .map(|s| s == "1")
            .unwrap_or(false);

        let items = match group.get("items") {
            Some(ConfigValue::List(items)) => items,
            _ => continue,
        };

        let mut group_val: Option<String> = None;
        for item in items {
            let item_map = match item {
                ConfigValue::Map(m) => m,
                _ => continue,
            };

            let (field_name, field_attrs_val) = match item_map.iter().next() {
                Some((k, v)) => (k, v),
                None => continue,
            };

            let field_attrs = match field_attrs_val {
                ConfigValue::Map(m) => m,
                _ => continue,
            };

            let is_literal = field_attrs
                .get("literal")
                .and_then(|v| v.as_str())
                .map(|s| s == "1")
                .unwrap_or(false);

            let raw = if is_literal {
                Some(field_name.to_string())
            } else {
                get_sort_field_raw(field_name, be, cite_index)
            };

            if let Some(val) = raw {
                let val = apply_substring(&val, field_attrs);
                let val = apply_padding(&val, field_attrs);
                let val = crate::transliteration::apply(
                    translit_rules,
                    entrytype,
                    langid,
                    field_name,
                    &val,
                );
                group_val = Some(val);
                break;
            }
        }

        let val = group_val.as_deref().unwrap_or("").to_string();
        if is_final {
            if let Some(ref v) = group_val {
                parts.push(v.clone());
                let remaining = specs.len() - parts.len();
                for _ in 0..remaining {
                    parts.push(v.clone());
                }
                break;
            }
            // Empty final group: fall through to next sort level
            continue;
        }
        parts.push(val);
    }

    let s = parts.join(SORT_SEP);
    if s.is_empty() {
        String::new()
    } else {
        s
    }
}

/// Compute sortinit for all entries in a datalist.
fn process_sortinit(biber: &mut Biber, secnum: u32, list_name: &str) {
    // Resolve sorting template
    let tmpl_map = match biber.config.getblxoption(None, "sortingtemplate") {
        Some(ConfigValue::Map(m)) => m,
        _ => return,
    };

    let (sortingtemplatename, citekeys) = {
        let lists = biber.datalists.get_lists_for_section(secnum);
        let list = match lists.iter().find(|l| l.name == *list_name) {
            Some(l) => l,
            None => return,
        };
        (list.sortingtemplatename.clone(), list.state.entries.clone())
    };

    let template = match tmpl_map.get(&sortingtemplatename) {
        Some(ConfigValue::Map(m)) => m,
        _ => return,
    };

    let specs = match template.get("spec") {
        Some(ConfigValue::List(s)) => s.clone(),
        _ => return,
    };

    let section = match biber.sections.get_section(secnum) {
        Some(s) => s.clone(),
        None => return,
    };

    let presort_opt = biber
        .config
        .getblxoption_str("presort")
        .unwrap_or("mm")
        .to_string();

    // Per-entrytype transliteration rule cache
    let mut translit_cache: HashMap<String, Vec<crate::transliteration::TranslitRule>> =
        HashMap::new();

    for citekey in &citekeys {
        let be = match section.bibentry(citekey) {
            Some(be) => be,
            None => continue,
        };

        let translit_rules: &Vec<crate::transliteration::TranslitRule> = translit_cache
            .entry(be.entrytype.clone())
            .or_insert_with(|| {
                let mut rules = Vec::new();
                if let Some(cv) = biber
                    .config
                    .getblxoption_for_entry(&be.entrytype, "translit")
                {
                    rules.extend(crate::transliteration::rules_from_config_value(cv));
                }
                rules
            });

        let cite_index = citekeys.iter().position(|k| k == citekey).unwrap_or(0);
        let sort_string = build_sort_string(&specs, be, cite_index, translit_rules);
        if sort_string.is_empty() {
            continue;
        }

        // Strip presort prefix + following separator(s)
        let rest = if sort_string.starts_with(&presort_opt) {
            let after = &sort_string[presort_opt.len()..];
            after.trim_start_matches(SORT_SEP)
        } else {
            &sort_string
        };

        // Remove LaTeX macros and non-letter characters (matching Perl's
        // normalise_string_common / normalise_string_sort), then take first char
        // Normalise: remove LaTeX commands + punctuation/symbols/controls
        // (matching Perl's normalise_string_common)
        #[allow(clippy::incompatible_msrv)]
        static RE_LATEX_CMD: std::sync::LazyLock<regex::Regex> =
            std::sync::LazyLock::new(|| regex::Regex::new(r"\\[A-Za-z]+").unwrap());
        let no_cmds = RE_LATEX_CMD.replace_all(rest, "");
        let normalised: String = no_cmds.chars().filter(|&c| c.is_alphanumeric()).collect();
        let init = normalised
            .chars()
            .next()
            .map(|c| c.to_string())
            .unwrap_or_default();
        if !init.is_empty() {
            for list_mut in biber.datalists.get_lists_for_section_mut(secnum) {
                if list_mut.name == *list_name {
                    list_mut
                        .state
                        .sortinit
                        .insert(citekey.clone(), init.clone());
                    break;
                }
            }
        }
    }
}

/// Compute sortinithash for all entries in a datalist.
fn process_sortinithash(biber: &mut Biber, secnum: u32, list_name: &str) {
    let sortinit_map: HashMap<String, String> = {
        let lists = biber.datalists.get_lists_for_section(secnum);
        let list = match lists.iter().find(|l| l.name == *list_name) {
            Some(l) => l,
            None => return,
        };
        list.state.sortinit.clone()
    };

    for (citekey, init) in &sortinit_map {
        let hash = format!("{:x}", Md5::digest(init.as_bytes()));
        for list_mut in biber.datalists.get_lists_for_section_mut(secnum) {
            if list_mut.name == *list_name {
                list_mut.state.sortinithash.insert(citekey.clone(), hash);
                break;
            }
        }
    }
}

/// Compute labelprefix for all entries in a datalist.
fn process_labelprefix(biber: &mut Biber, secnum: u32, list_name: &str) {
    let section = match biber.sections.get_section(secnum) {
        Some(s) => s.clone(),
        None => return,
    };

    let (list_prefix, citekeys) = {
        let lists = biber.datalists.get_lists_for_section(secnum);
        let list = match lists.iter().find(|l| l.name == *list_name) {
            Some(l) => l,
            None => return,
        };
        (list.labelprefix.clone(), list.state.entries.clone())
    };

    for citekey in &citekeys {
        let be = match section.bibentry(citekey) {
            Some(be) => be,
            None => continue,
        };

        // Labelprefix: use shorthand if present, else datalist labelprefix
        let lp = be
            .get_field_str("shorthand")
            .map(|s| s.to_string())
            .or_else(|| {
                if list_prefix.is_empty() {
                    None
                } else {
                    Some(list_prefix.clone())
                }
            });

        if let Some(val) = lp {
            for list_mut in biber.datalists.get_lists_for_section_mut(secnum) {
                if list_mut.name == *list_name {
                    list_mut.state.labelprefix_data.insert(citekey.clone(), val);
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::Entry;
    use crate::section::Section;
    use std::collections::BTreeMap;

    #[test]
    fn prepare_runs_on_empty_biber() {
        let mut biber = Biber::new();
        biber.sections.add_section(Section::new(0));
        // Should not panic
        prepare(&mut biber);
    }

    #[test]
    fn process_citekey_aliases_removes_aliases() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("realkey");
        section.set_citekey_alias("aliaskey", "realkey");
        // Add the alias to citekeys
        section.add_cite("aliaskey");
        biber.sections.add_section(section);
        biber.set_current_section(0);

        process_citekey_aliases(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();
        assert!(section.get_citekeys().contains(&"realkey".to_string()));
        assert!(!section.get_citekeys().contains(&"aliaskey".to_string()));
    }

    #[test]
    fn instantiate_dynamic_creates_set_entry() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("set1");
        section.set_dynamic_set("set1", vec!["member1".to_string(), "member2".to_string()]);
        // Add member entries
        section
            .bibentries
            .add_entry(Entry::new("member1", "article"));
        section
            .bibentries
            .add_entry(Entry::new("member2", "article"));
        biber.sections.add_section(section);
        biber.set_current_section(0);

        instantiate_dynamic(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();
        let set_entry = section.bibentry("set1").unwrap();
        assert_eq!(set_entry.get_field_str("entrytype"), Some("set"));
        assert_eq!(set_entry.get_field_str("datatype"), Some("dynamic"));
    }

    #[test]
    fn process_nocite_sets_nocite_flag_for_explicit_nocite() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("key1");
        section.add_nocite("key1");
        let entry = Entry::new("key1", "book");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);
        biber.set_current_section(0);

        process_nocite(&mut biber, 0, "key1");

        let section = biber.sections.get_section(0).unwrap();
        let be = section.bibentry("key1").unwrap();
        assert_eq!(be.get_field_str("nocite"), Some("1"));
    }

    #[test]
    fn process_nocite_does_not_set_flag_for_cited_only() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("key1");
        let entry = Entry::new("key1", "book");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);
        biber.set_current_section(0);

        process_nocite(&mut biber, 0, "key1");

        let section = biber.sections.get_section(0).unwrap();
        let be = section.bibentry("key1").unwrap();
        assert_eq!(be.get_field_str("nocite"), None);
    }

    #[test]
    fn process_nocite_sets_flag_for_allkeys_nocite() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("key1");
        section.set_allkeys_nocite(true);
        let entry = Entry::new("key1", "book");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);
        biber.set_current_section(0);

        process_nocite(&mut biber, 0, "key1");

        let section = biber.sections.get_section(0).unwrap();
        let be = section.bibentry("key1").unwrap();
        assert_eq!(be.get_field_str("nocite"), Some("1"));
    }

    #[test]
    fn process_labelname_sets_labelname() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("key1");
        let mut entry = Entry::new("key1", "book");
        entry.set_field_str("author", "John Doe");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);
        biber.set_current_section(0);

        process_labelname(&mut biber, 0, "key1");

        let section = biber.sections.get_section(0).unwrap();
        let be = section.bibentry("key1").unwrap();
        assert_eq!(be.get_field_str("labelname"), Some("author"));
    }

    #[test]
    fn process_labeldate_sets_labelyear() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("key1");
        let mut entry = Entry::new("key1", "book");
        entry.set_field_str("year", "2020");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);
        biber.set_current_section(0);

        process_labeldate(&mut biber, 0, "key1");

        let section = biber.sections.get_section(0).unwrap();
        let be = section.bibentry("key1").unwrap();
        assert_eq!(be.get_field_str("labelyear"), Some("2020"));
    }

    #[test]
    fn process_labeltitle_sets_labeltitle() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("key1");
        let mut entry = Entry::new("key1", "book");
        entry.set_field_str("title", "A Book Title");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);
        biber.set_current_section(0);

        process_labeltitle(&mut biber, 0, "key1");

        let section = biber.sections.get_section(0).unwrap();
        let be = section.bibentry("key1").unwrap();
        assert_eq!(be.get_field_str("labeltitle"), Some("title"));
    }

    #[test]
    fn process_presort_stores_presort_in_datalist_state() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("key1");
        let mut entry = Entry::new("key1", "book");
        entry.set_field_str("presort", "abc");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);

        // Add a datalist so presort is stored into it
        let dl = crate::datalist::DataList::new(
            0, "nty", "global", "global", "global", "global", "", "test",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_presort(&mut biber, 0, "key1");

        let lists = biber.datalists.get_lists_for_section(0);
        assert!(!lists.is_empty(), "expected at least one datalist");
        assert_eq!(lists[0].state.presort.get("key1"), Some(&"abc".to_string()));
    }

    #[test]
    fn process_presort_defaults_to_mm_when_no_field() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("key1");
        let entry = Entry::new("key1", "book");
        // No presort field set — should default to "mm"
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0, "nty", "global", "global", "global", "global", "", "test",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_presort(&mut biber, 0, "key1");

        let lists = biber.datalists.get_lists_for_section(0);
        assert!(!lists.is_empty());
        assert_eq!(lists[0].state.presort.get("key1"), Some(&"mm".to_string()));
    }

    #[test]
    fn calculate_interentry_adds_crossref_above_threshold() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        // Add entries that crossref "parent"
        for i in 1..=3 {
            let key = format!("child{i}");
            section.add_cite(key.clone());
            let mut entry = Entry::new(key, "inbook");
            entry.set_field_str("crossref", "parent");
            section.bibentries.add_entry(entry);
        }
        // Add the parent entry (not cited)
        section.bibentries.add_entry(Entry::new("parent", "book"));
        biber.sections.add_section(section);
        biber.set_current_section(0);

        calculate_interentry(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();
        // parent should now be in citekeys (3 crossrefs >= mincrossrefs=2)
        assert!(section.get_citekeys().contains(&"parent".to_string()));
    }

    #[test]
    fn process_namehash_matches_expected_hash() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("F1");
        let mut entry = Entry::new("F1", "book");
        entry.set_field_str("author", "John Doe");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);

        // Set up namehashtemplate in config like biber-tool.conf does
        let mut template_parts = Vec::new();
        for (namepart, order) in &[
            ("family", "1"),
            ("given", "2"),
            ("prefix", "3"),
            ("suffix", "4"),
        ] {
            let mut m = BTreeMap::new();
            m.insert("namepart".into(), ConfigValue::Str(namepart.to_string()));
            template_parts.push((order.parse::<u32>().unwrap(), ConfigValue::Map(m)));
        }
        template_parts.sort_by_key(|(o, _)| *o);
        let parts: Vec<ConfigValue> = template_parts.into_iter().map(|(_, v)| v).collect();
        let mut template = BTreeMap::new();
        template.insert("global".into(), ConfigValue::List(parts));
        biber
            .config
            .setblxoption(None, "namehashtemplate", ConfigValue::Map(template));

        biber.set_current_section(0);
        process_namehash(&mut biber, 0, "F1");
        process_fullhash(&mut biber, 0, "F1");
        process_pername_hashes(&mut biber, 0, "F1");

        let section = biber.sections.get_section(0).unwrap();
        let be = section.bibentry("F1").unwrap();
        assert_eq!(
            be.get_field_str("namehash"),
            Some("bd051a2f7a5f377e3a62581b0e0f8577")
        );
        assert_eq!(
            be.get_field_str("fullhash"),
            Some("bd051a2f7a5f377e3a62581b0e0f8577")
        );
        assert_eq!(
            be.get_field_str("fullhashraw"),
            Some("bd051a2f7a5f377e3a62581b0e0f8577")
        );
        assert_eq!(
            be.get_field_str("bibnamehash"),
            Some("bd051a2f7a5f377e3a62581b0e0f8577")
        );
        assert_eq!(
            be.get_field_str("authornamehash"),
            Some("bd051a2f7a5f377e3a62581b0e0f8577")
        );
        assert_eq!(
            be.get_field_str("authorfullhash"),
            Some("bd051a2f7a5f377e3a62581b0e0f8577")
        );
        assert_eq!(
            be.get_field_str("authorfullhashraw"),
            Some("bd051a2f7a5f377e3a62581b0e0f8577")
        );
        assert_eq!(
            be.get_field_str("authorbibnamehash"),
            Some("bd051a2f7a5f377e3a62581b0e0f8577")
        );
    }

    #[test]
    fn process_namedis_disambiguates_duplicate_bases() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);

        // Two entries with same author family name
        let mut entry1 = Entry::new("smith2020", "book");
        entry1.set_field_str("author", "John Smith");
        section.bibentries.add_entry(entry1);

        let mut entry2 = Entry::new("smith2021", "book");
        entry2.set_field_str("author", "Jane Smith");
        section.bibentries.add_entry(entry2);

        section.add_cite("smith2020");
        section.add_cite("smith2021");
        biber.sections.add_section(section);

        // Set up uniquenametemplate: family is base part
        let items = vec![ConfigValue::Map(BTreeMap::from([
            ("namepart".into(), ConfigValue::Str("family".into())),
            ("base".into(), ConfigValue::Str("1".into())),
        ]))];
        let mut template = BTreeMap::new();
        template.insert("global".into(), ConfigValue::List(items));
        biber
            .config
            .setblxoption(None, "uniquenametemplate", ConfigValue::Map(template));

        // Also set up namehashtemplate like the other test
        let mut tmpl_parts = Vec::new();
        for (np, order) in &[("family", "1"), ("given", "2")] {
            let mut m = BTreeMap::new();
            m.insert("namepart".into(), ConfigValue::Str(np.to_string()));
            tmpl_parts.push((order.parse::<u32>().unwrap(), ConfigValue::Map(m)));
        }
        tmpl_parts.sort_by_key(|(o, _)| *o);
        let parts: Vec<ConfigValue> = tmpl_parts.into_iter().map(|(_, v)| v).collect();
        let mut template = BTreeMap::new();
        template.insert("global".into(), ConfigValue::List(parts));
        biber
            .config
            .setblxoption(None, "namehashtemplate", ConfigValue::Map(template));

        // Run labelname and namedis passes
        biber.set_current_section(0);
        process_labelname(&mut biber, 0, "smith2020");
        process_labelname(&mut biber, 0, "smith2021");

        // Set up datalist (needed for cross-entry tracking)
        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);

        process_namedis(&mut biber, 0, "smith2020");
        process_namedis(&mut biber, 0, "smith2021");

        let section = biber.sections.get_section(0).unwrap();

        // Both entries share the same base "Smith", so both should be disambiguated
        {
            let be = section.bibentry("smith2020").unwrap();
            let names = be.names.get("author").unwrap();
            let name = names.get(0).unwrap();
            assert_eq!(
                name.un, 1,
                "first entry should be disambiguated (retroactive)"
            );
            assert_eq!(name.uniquepart, "base");
            assert_eq!(name.nameparts.get("familyi").unwrap(), "S");
            assert_eq!(name.nameparts.get("giveni").unwrap(), "J");
        }

        {
            let be = section.bibentry("smith2021").unwrap();
            let names = be.names.get("author").unwrap();
            let name = names.get(0).unwrap();
            assert_eq!(name.un, 1, "second entry should be disambiguated");
            assert_eq!(name.uniquepart, "base");
            assert_eq!(name.nameparts.get("familyi").unwrap(), "S");
            assert_eq!(name.nameparts.get("giveni").unwrap(), "J");
        }
    }

    #[test]
    fn process_extradate_assigns_letters_for_duplicate_years() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);

        // Three entries with same year, different authors
        let mut e1 = Entry::new("smith2020", "book");
        e1.set_field_str("author", "John Smith");
        e1.set_field_str("year", "2020");
        section.bibentries.add_entry(e1);

        let mut e2 = Entry::new("jones2020", "article");
        e2.set_field_str("author", "Alice Jones");
        e2.set_field_str("year", "2020");
        section.bibentries.add_entry(e2);

        let mut e3 = Entry::new("lee2021", "book");
        e3.set_field_str("author", "Bob Lee");
        e3.set_field_str("year", "2021");
        section.bibentries.add_entry(e3);

        section.add_cite("smith2020");
        section.add_cite("jones2020");
        section.add_cite("lee2021");
        biber.sections.add_section(section);

        // Set extradatespec: one scope with labelyear and year as fallback
        let edspec = ConfigValue::List(vec![ConfigValue::List(vec![
            ConfigValue::Str("labelyear".into()),
            ConfigValue::Str("year".into()),
        ])]);
        biber
            .config
            .setblxoption(None, "extradatespec", edspec.clone());

        // labeldateparts default is true (already set in defaults)

        // Set up labelname for context
        let mut tmpl_parts = Vec::new();
        for (np, order) in &[("family", "1"), ("given", "2")] {
            let mut m = BTreeMap::new();
            m.insert("namepart".into(), ConfigValue::Str(np.to_string()));
            tmpl_parts.push((order.parse::<u32>().unwrap(), ConfigValue::Map(m)));
        }
        tmpl_parts.sort_by_key(|(o, _)| *o);
        let parts: Vec<ConfigValue> = tmpl_parts.into_iter().map(|(_, v)| v).collect();
        let mut template = BTreeMap::new();
        template.insert("global".into(), ConfigValue::List(parts));
        biber
            .config
            .setblxoption(None, "namehashtemplate", ConfigValue::Map(template));
        biber.config.setblxoption(
            None,
            "uniquenametemplate",
            ConfigValue::Map({
                let items = vec![ConfigValue::Map(BTreeMap::from([
                    ("namepart".into(), ConfigValue::Str("family".into())),
                    ("base".into(), ConfigValue::Str("1".into())),
                ]))];
                let mut t = BTreeMap::new();
                t.insert("global".into(), ConfigValue::List(items));
                t
            }),
        );

        // Add datalist for tracking
        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);

        biber.set_current_section(0);

        // Run necessary passes
        process_labelname(&mut biber, 0, "smith2020");
        process_labelname(&mut biber, 0, "jones2020");
        process_labelname(&mut biber, 0, "lee2021");

        process_labeldate(&mut biber, 0, "smith2020");
        process_labeldate(&mut biber, 0, "jones2020");
        process_labeldate(&mut biber, 0, "lee2021");

        // extradate needs to run per-entry, then assign letters after
        process_extradate(&mut biber, 0, "smith2020");
        process_extradate(&mut biber, 0, "jones2020");
        process_extradate(&mut biber, 0, "lee2021");

        assign_extradate_letters(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();

        // smith2020 and jones2020 share year 2020 but different authors → no extradate needed
        // (each author+year combo is unique)
        assert_eq!(
            section
                .bibentry("smith2020")
                .unwrap()
                .get_field_str("extradate"),
            None
        );
        assert_eq!(
            section
                .bibentry("jones2020")
                .unwrap()
                .get_field_str("extradate"),
            None
        );
        assert_eq!(
            section
                .bibentry("lee2021")
                .unwrap()
                .get_field_str("extradate"),
            None
        );

        // Now test with same author AND same year - should get extradate letters
        let mut biber2 = Biber::new();
        let mut section2 = Section::new(0);

        let mut e1 = Entry::new("smith2020a", "book");
        e1.set_field_str("author", "John Smith");
        e1.set_field_str("year", "2020");
        section2.bibentries.add_entry(e1);

        let mut e2 = Entry::new("smith2020b", "book");
        e2.set_field_str("author", "John Smith");
        e2.set_field_str("year", "2020");
        section2.bibentries.add_entry(e2);

        section2.add_cite("smith2020a");
        section2.add_cite("smith2020b");
        biber2.sections.add_section(section2);

        biber2
            .config
            .setblxoption(None, "extradatespec", edspec.clone());
        let dl2 = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber2.datalists.add_list(dl2);
        biber2.set_current_section(0);

        process_labelname(&mut biber2, 0, "smith2020a");
        process_labelname(&mut biber2, 0, "smith2020b");
        process_labeldate(&mut biber2, 0, "smith2020a");
        process_labeldate(&mut biber2, 0, "smith2020b");
        process_extradate(&mut biber2, 0, "smith2020a");
        process_extradate(&mut biber2, 0, "smith2020b");
        assign_extradate_letters(&mut biber2, 0);

        let section2 = biber2.sections.get_section(0).unwrap();
        assert_eq!(
            section2
                .bibentry("smith2020a")
                .unwrap()
                .get_field_str("extradate"),
            Some("1")
        );
        assert_eq!(
            section2
                .bibentry("smith2020b")
                .unwrap()
                .get_field_str("extradate"),
            Some("2")
        );
    }

    #[test]
    fn process_labelalpha_computes_simple_labelalpha() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);

        // Entry with shorthand
        let mut e1 = Entry::new("abc2020", "book");
        e1.set_field_str("shorthand", "ABC");
        e1.set_field_str("year", "2020");
        section.bibentries.add_entry(e1);

        // Entry with labelname (author) but no shorthand
        let mut e2 = Entry::new("smith2020", "book");
        e2.set_field_str("author", "Smith, John");
        e2.set_field_str("year", "2020");
        // Parse entry names so that names are populated
        crate::name::parse_entry_names(&mut e2);
        section.bibentries.add_entry(e2);

        // Entry with no label source -> no labelalpha
        let mut e3 = Entry::new("noalpha", "misc");
        e3.set_field_str("year", "2020");
        section.bibentries.add_entry(e3);

        section.add_cite("abc2020");
        section.add_cite("smith2020");
        section.add_cite("noalpha");
        biber.sections.add_section(section);

        // Add datalist
        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);

        biber.set_current_section(0);

        // Run label passes
        process_labelname(&mut biber, 0, "abc2020");
        process_labelname(&mut biber, 0, "smith2020");
        process_labelname(&mut biber, 0, "noalpha");
        process_labeldate(&mut biber, 0, "abc2020");
        process_labeldate(&mut biber, 0, "smith2020");
        process_labeldate(&mut biber, 0, "noalpha");

        process_labelalpha(&mut biber, 0, "abc2020");
        process_labelalpha(&mut biber, 0, "smith2020");
        process_labelalpha(&mut biber, 0, "noalpha");

        let lists = biber.datalists.get_lists_for_section(0);
        assert!(!lists.is_empty());

        // abc2020: shorthand "ABC" with final="1" → stops at shorthand, no year
        assert_eq!(
            lists[0]
                .state
                .labelalphadata
                .get("abc2020")
                .map(|s| s.as_str()),
            Some("ABC")
        );
        // smith2020: no shorthand, author family "Smith" -> "Smi" + "20"
        assert_eq!(
            lists[0]
                .state
                .labelalphadata
                .get("smith2020")
                .map(|s| s.as_str()),
            Some("Smi20")
        );
        // noalpha: no labelname, but year part in labelelement 2 still produces "20"
        assert_eq!(
            lists[0]
                .state
                .labelalphadata
                .get("noalpha")
                .map(|s| s.as_str()),
            Some("20")
        );
    }

    #[test]
    fn process_extraalpha_assigns_letters_for_duplicates() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);

        // Two entries with same author and year -> duplicate labelalpha
        let mut e1 = Entry::new("smith2020a", "book");
        e1.set_field_str("author", "Smith, John");
        e1.set_field_str("year", "2020");
        crate::name::parse_entry_names(&mut e1);
        section.bibentries.add_entry(e1);

        let mut e2 = Entry::new("smith2020b", "book");
        e2.set_field_str("author", "Smith, Jane");
        e2.set_field_str("year", "2020");
        crate::name::parse_entry_names(&mut e2);
        section.bibentries.add_entry(e2);

        // A third entry with different author -> unique labelalpha
        let mut e3 = Entry::new("jones2020", "book");
        e3.set_field_str("author", "Jones, Bob");
        e3.set_field_str("year", "2020");
        crate::name::parse_entry_names(&mut e3);
        section.bibentries.add_entry(e3);

        section.add_cite("smith2020a");
        section.add_cite("smith2020b");
        section.add_cite("jones2020");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_labelname(&mut biber, 0, "smith2020a");
        process_labelname(&mut biber, 0, "smith2020b");
        process_labelname(&mut biber, 0, "jones2020");
        process_labeldate(&mut biber, 0, "smith2020a");
        process_labeldate(&mut biber, 0, "smith2020b");
        process_labeldate(&mut biber, 0, "jones2020");

        process_labelalpha(&mut biber, 0, "smith2020a");
        process_labelalpha(&mut biber, 0, "smith2020b");
        process_labelalpha(&mut biber, 0, "jones2020");
        process_extraalpha(&mut biber, 0, "smith2020a");
        process_extraalpha(&mut biber, 0, "smith2020b");
        process_extraalpha(&mut biber, 0, "jones2020");

        assign_extraalpha_letters(&mut biber, 0);

        let lists = biber.datalists.get_lists_for_section(0);
        // Both Smith entries have labelalpha "Smi20" (collision)
        assert_eq!(
            lists[0]
                .state
                .extraalphadata
                .get("smith2020a")
                .map(|s| s.as_str()),
            Some("1")
        );
        assert_eq!(
            lists[0]
                .state
                .extraalphadata
                .get("smith2020b")
                .map(|s| s.as_str()),
            Some("2")
        );
        // Jones entry has unique labelalpha "Jon20" -> no extraalpha
        assert!(!lists[0].state.extraalphadata.contains_key("jones2020"));
    }

    #[test]
    fn process_extraname_assigns_letters_for_duplicate_authors() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);

        // Two entries with same author -> same labelnamehash -> extraname needed
        let mut e1 = Entry::new("dup1", "book");
        e1.set_field_str("author", "Smith, John");
        e1.set_field_str("year", "2020");
        e1.set_field_str("title", "Alpha");
        crate::name::parse_entry_names(&mut e1);
        section.bibentries.add_entry(e1);

        let mut e2 = Entry::new("dup2", "book");
        e2.set_field_str("author", "Smith, John");
        e2.set_field_str("year", "2021");
        e2.set_field_str("title", "Beta");
        crate::name::parse_entry_names(&mut e2);
        section.bibentries.add_entry(e2);

        // Third entry with different author -> unique labelnamehash -> no extraname
        let mut e3 = Entry::new("unique", "book");
        e3.set_field_str("author", "Jones, Bob");
        e3.set_field_str("year", "2020");
        e3.set_field_str("title", "Gamma");
        crate::name::parse_entry_names(&mut e3);
        section.bibentries.add_entry(e3);

        section.add_cite("dup1");
        section.add_cite("dup2");
        section.add_cite("unique");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_labelname(&mut biber, 0, "dup1");
        process_labelname(&mut biber, 0, "dup2");
        process_labelname(&mut biber, 0, "unique");
        process_extraname(&mut biber, 0, "dup1");
        process_extraname(&mut biber, 0, "dup2");
        process_extraname(&mut biber, 0, "unique");

        assign_extraname_extratitle_letters(&mut biber, 0);

        let lists = biber.datalists.get_lists_for_section(0);
        // dup1 and dup2 share labelnamehash -> both get extraname
        assert!(lists[0].state.extranamedata.contains_key("dup1"));
        assert!(lists[0].state.extranamedata.contains_key("dup2"));
        // unique has different author -> no extraname
        assert!(!lists[0].state.extranamedata.contains_key("unique"));
    }

    #[test]
    fn process_extratitle_assigns_letters_for_duplicate_titles() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);

        biber
            .config
            .setblxoption(None, "labeltitle", ConfigValue::Str("1".into()));

        // Two entries with same author and same title -> extratitle needed
        let mut e1 = Entry::new("dup1", "book");
        e1.set_field_str("author", "Smith, John");
        e1.set_field_str("title", "Same Title");
        e1.set_field_str("year", "2020");
        crate::name::parse_entry_names(&mut e1);
        section.bibentries.add_entry(e1);

        let mut e2 = Entry::new("dup2", "book");
        e2.set_field_str("author", "Smith, John");
        e2.set_field_str("title", "Same Title");
        e2.set_field_str("year", "2021");
        crate::name::parse_entry_names(&mut e2);
        section.bibentries.add_entry(e2);

        section.add_cite("dup1");
        section.add_cite("dup2");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_labelname(&mut biber, 0, "dup1");
        process_labelname(&mut biber, 0, "dup2");
        process_labeltitle(&mut biber, 0, "dup1");
        process_labeltitle(&mut biber, 0, "dup2");

        process_extraname(&mut biber, 0, "dup1");
        process_extraname(&mut biber, 0, "dup2");
        process_extratitle(&mut biber, 0, "dup1");
        process_extratitle(&mut biber, 0, "dup2");

        assign_extraname_extratitle_letters(&mut biber, 0);

        let lists = biber.datalists.get_lists_for_section(0);
        // Both entries share (SmithJohn, Same Title) -> extratitle
        assert!(lists[0].state.extratitledata.contains_key("dup1"));
        assert!(lists[0].state.extratitledata.contains_key("dup2"));
    }

    #[test]
    fn process_extratitleyear_assigns_letters_for_duplicate_title_year() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);

        biber
            .config
            .setblxoption(None, "labeltitleyear", ConfigValue::Str("1".into()));

        // Two entries with same title and same year -> extratitleyear needed
        let mut e1 = Entry::new("dup1", "book");
        e1.set_field_str("author", "Different1");
        e1.set_field_str("title", "Same Title");
        e1.set_field_str("year", "2020");
        crate::name::parse_entry_names(&mut e1);
        section.bibentries.add_entry(e1);

        let mut e2 = Entry::new("dup2", "book");
        e2.set_field_str("author", "Different2");
        e2.set_field_str("title", "Same Title");
        e2.set_field_str("year", "2020");
        crate::name::parse_entry_names(&mut e2);
        section.bibentries.add_entry(e2);

        section.add_cite("dup1");
        section.add_cite("dup2");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_labeltitle(&mut biber, 0, "dup1");
        process_labeltitle(&mut biber, 0, "dup2");
        process_labeldate(&mut biber, 0, "dup1");
        process_labeldate(&mut biber, 0, "dup2");

        process_extratitleyear(&mut biber, 0, "dup1");
        process_extratitleyear(&mut biber, 0, "dup2");

        assign_extraname_extratitle_letters(&mut biber, 0);

        let lists = biber.datalists.get_lists_for_section(0);
        // Both entries share (Same Title, 2020) -> extratitleyear
        assert!(lists[0].state.extratitleyeardata.contains_key("dup1"));
        assert!(lists[0].state.extratitleyeardata.contains_key("dup2"));
    }

    #[test]
    fn process_lists_populates_entries() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("key1");
        section.add_cite("key2");
        section.bibentries.add_entry(Entry::new("key1", "book"));
        section.bibentries.add_entry(Entry::new("key2", "article"));
        biber.sections.add_section(section);

        // Add a datalist
        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);

        process_lists(&mut biber, 0);

        let lists = biber.datalists.get_lists_for_section(0);
        assert!(!lists.is_empty());
        assert_eq!(lists[0].state.entries.len(), 2);
    }

    // ---- Work uniqueness tests ----

    #[test]
    fn process_workuniqueness_tracks_singletitle_seenname() {
        let mut biber = Biber::new();
        biber
            .config
            .setblxoption(Some(0), "singletitle", ConfigValue::Str("1".into()));
        let mut section = Section::new(0);
        // Two entries with same author -> share seenname identifier
        let mut e1 = Entry::new("key1", "book");
        e1.set_field_str("author", "John Smith");
        e1.set_field_str("title", "Book A");
        crate::name::parse_entry_names(&mut e1);
        e1.set_field_str("labelname", "author");
        section.bibentries.add_entry(e1);

        let mut e2 = Entry::new("key2", "book");
        e2.set_field_str("author", "John Smith");
        e2.set_field_str("title", "Book B");
        crate::name::parse_entry_names(&mut e2);
        e2.set_field_str("labelname", "author");
        section.bibentries.add_entry(e2);

        section.add_cite("key1");
        section.add_cite("key2");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_workuniqueness(&mut biber, 0, "key1");
        process_workuniqueness(&mut biber, 0, "key2");

        let lists = biber.datalists.get_lists_for_section(0);
        assert_eq!(lists[0].state.seenname.len(), 1);
        let count: u32 = lists[0].state.seenname.values().sum();
        assert_eq!(count, 2);
    }

    #[test]
    fn process_workuniqueness_tracks_uniquetitle_seentitle() {
        let mut biber = Biber::new();
        biber
            .config
            .setblxoption(Some(0), "uniquetitle", ConfigValue::Str("1".into()));
        let mut section = Section::new(0);
        // Two entries with same title -> share seentitle identifier
        let mut e1 = Entry::new("key1", "book");
        e1.set_field_str("title", "Same Title");
        e1.set_field_str("labeltitle", "title");
        section.bibentries.add_entry(e1);

        let mut e2 = Entry::new("key2", "book");
        e2.set_field_str("title", "Same Title");
        e2.set_field_str("labeltitle", "title");
        section.bibentries.add_entry(e2);

        section.add_cite("key1");
        section.add_cite("key2");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_workuniqueness(&mut biber, 0, "key1");
        process_workuniqueness(&mut biber, 0, "key2");

        let lists = biber.datalists.get_lists_for_section(0);
        assert_eq!(lists[0].state.seentitle.len(), 1);
        let count: u32 = lists[0].state.seentitle.values().sum();
        assert_eq!(count, 2);
    }

    #[test]
    fn process_workuniqueness_tracks_uniquebaretitle_seenbaretitle() {
        let mut biber = Biber::new();
        biber
            .config
            .setblxoption(Some(0), "uniquebaretitle", ConfigValue::Str("1".into()));
        let mut section = Section::new(0);
        // Two entries with no labelname but same title -> share seenbaretitle
        let mut e1 = Entry::new("key1", "book");
        e1.set_field_str("title", "Shared Title");
        e1.set_field_str("labeltitle", "title");
        section.bibentries.add_entry(e1);

        let mut e2 = Entry::new("key2", "book");
        e2.set_field_str("title", "Shared Title");
        e2.set_field_str("labeltitle", "title");
        section.bibentries.add_entry(e2);

        section.add_cite("key1");
        section.add_cite("key2");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_workuniqueness(&mut biber, 0, "key1");
        process_workuniqueness(&mut biber, 0, "key2");

        let lists = biber.datalists.get_lists_for_section(0);
        assert_eq!(lists[0].state.seenbaretitle.len(), 1);
        let count: u32 = lists[0].state.seenbaretitle.values().sum();
        assert_eq!(count, 2);
    }

    #[test]
    fn process_workuniqueness_tracks_uniquework_seenwork() {
        let mut biber = Biber::new();
        biber
            .config
            .setblxoption(Some(0), "uniquework", ConfigValue::Str("1".into()));
        let mut section = Section::new(0);
        // Two entries with same author + same title -> share seenwork
        let mut e1 = Entry::new("key1", "book");
        e1.set_field_str("author", "John Smith");
        e1.set_field_str("title", "Shared Book");
        crate::name::parse_entry_names(&mut e1);
        e1.set_field_str("labelname", "author");
        e1.set_field_str("labeltitle", "title");
        section.bibentries.add_entry(e1);

        let mut e2 = Entry::new("key2", "book");
        e2.set_field_str("author", "John Smith");
        e2.set_field_str("title", "Shared Book");
        crate::name::parse_entry_names(&mut e2);
        e2.set_field_str("labelname", "author");
        e2.set_field_str("labeltitle", "title");
        section.bibentries.add_entry(e2);

        section.add_cite("key1");
        section.add_cite("key2");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_workuniqueness(&mut biber, 0, "key1");
        process_workuniqueness(&mut biber, 0, "key2");

        let lists = biber.datalists.get_lists_for_section(0);
        assert_eq!(lists[0].state.seenwork.len(), 1);
        // Both entries share the same work, so sum of counters is 2
        let count: u32 = lists[0].state.seenwork.values().sum();
        assert_eq!(count, 2);
    }

    // ---- Per-list pass tests ----

    #[test]
    fn build_sort_string_concatenates_field_values() {
        let mut section = Section::new(0);
        let mut e1 = Entry::new("test1", "book");
        e1.set_field_str("author", "John Doe");
        e1.set_field_str("year", "2020");
        e1.set_field_str("title", "A Book");
        crate::name::parse_entry_names(&mut e1);
        e1.set_field_str("presort", "mm");
        section.bibentries.add_entry(e1);

        // Template: presort → author → year → title
        let item_presort = ConfigValue::Map(BTreeMap::from([(
            "presort".to_string(),
            ConfigValue::Map(BTreeMap::new()),
        )]));
        let item_author = ConfigValue::Map(BTreeMap::from([(
            "author".to_string(),
            ConfigValue::Map(BTreeMap::new()),
        )]));
        let item_year = ConfigValue::Map(BTreeMap::from([(
            "year".to_string(),
            ConfigValue::Map(BTreeMap::new()),
        )]));
        let item_title = ConfigValue::Map(BTreeMap::from([(
            "title".to_string(),
            ConfigValue::Map(BTreeMap::new()),
        )]));

        let group1 = ConfigValue::Map(BTreeMap::from([(
            "items".to_string(),
            ConfigValue::List(vec![item_presort]),
        )]));
        let group2 = ConfigValue::Map(BTreeMap::from([(
            "items".to_string(),
            ConfigValue::List(vec![item_author]),
        )]));
        let group3 = ConfigValue::Map(BTreeMap::from([(
            "items".to_string(),
            ConfigValue::List(vec![item_year]),
        )]));
        let group4 = ConfigValue::Map(BTreeMap::from([(
            "items".to_string(),
            ConfigValue::List(vec![item_title]),
        )]));

        let specs = vec![group1, group2, group3, group4];
        let be = section.bibentry("test1").unwrap();
        let result = build_sort_string(&specs, be, 0, &[]);
        // presort="mm", author family="Doe", year="2020", title="A Book"
        assert_eq!(result, "mm,Doe,2020,A Book");
    }

    #[test]
    fn build_sort_string_with_substring_and_padding() {
        let mut section = Section::new(0);
        let mut e1 = Entry::new("test1", "book");
        e1.set_field_str("volume", "5");
        section.bibentries.add_entry(e1);

        // volume with substring_width=1 (left) and pad_width=3, pad_char=0, pad_side=left
        let item_vol_attrs = ConfigValue::Map(BTreeMap::from([
            ("substring_width".to_string(), ConfigValue::Str("1".into())),
            (
                "substring_side".to_string(),
                ConfigValue::Str("left".into()),
            ),
            ("pad_width".to_string(), ConfigValue::Str("3".into())),
            ("pad_char".to_string(), ConfigValue::Str("0".into())),
            ("pad_side".to_string(), ConfigValue::Str("left".into())),
        ]));
        let item_vol = ConfigValue::Map(BTreeMap::from([("volume".to_string(), item_vol_attrs)]));

        let group = ConfigValue::Map(BTreeMap::from([(
            "items".to_string(),
            ConfigValue::List(vec![item_vol]),
        )]));

        let specs = vec![group];
        let be = section.bibentry("test1").unwrap();
        // substring left 1 of "5" = "5", then pad to 3 with "0" left = "005"
        let result = build_sort_string(&specs, be, 0, &[]);
        assert_eq!(result, "005");
    }

    #[test]
    fn build_sort_string_literal_item() {
        let mut section = Section::new(0);
        let e1 = Entry::new("test1", "book");
        section.bibentries.add_entry(e1);

        // Sort with a literal value "zzz"
        let item_literal = ConfigValue::Map(BTreeMap::from([(
            "zzz".to_string(),
            ConfigValue::Map(BTreeMap::from([(
                "literal".to_string(),
                ConfigValue::Str("1".into()),
            )])),
        )]));
        let group = ConfigValue::Map(BTreeMap::from([(
            "items".to_string(),
            ConfigValue::List(vec![item_literal]),
        )]));
        let specs = vec![group];
        let be = section.bibentry("test1").unwrap();
        let result = build_sort_string(&specs, be, 0, &[]);
        assert_eq!(result, "zzz");
    }

    #[test]
    fn process_sortinit_computes_first_char_after_presort_strip() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);

        let mut e1 = Entry::new("doe2020", "book");
        e1.set_field_str("author", "John Doe");
        e1.set_field_str("year", "2020");
        crate::name::parse_entry_names(&mut e1);
        e1.set_field_str("labelname", "author");
        e1.set_field_str("presort", "mm");
        section.bibentries.add_entry(e1);

        section.add_cite("doe2020");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);

        // Manually populate list entries (as process_lists does)
        for list in biber.datalists.get_lists_for_section_mut(0) {
            if list.name == "nty/global//global/global/global" {
                list.state.entries = vec!["doe2020".to_string()];
                break;
            }
        }

        // Set up a sorting template in config
        let item_presort = ConfigValue::Map(BTreeMap::from([(
            "presort".to_string(),
            ConfigValue::Map(BTreeMap::new()),
        )]));
        let item_author = ConfigValue::Map(BTreeMap::from([(
            "author".to_string(),
            ConfigValue::Map(BTreeMap::new()),
        )]));
        let item_year = ConfigValue::Map(BTreeMap::from([(
            "year".to_string(),
            ConfigValue::Map(BTreeMap::new()),
        )]));

        let group1 = ConfigValue::Map(BTreeMap::from([(
            "items".to_string(),
            ConfigValue::List(vec![item_presort]),
        )]));
        let group2 = ConfigValue::Map(BTreeMap::from([(
            "items".to_string(),
            ConfigValue::List(vec![item_author]),
        )]));
        let group3 = ConfigValue::Map(BTreeMap::from([(
            "items".to_string(),
            ConfigValue::List(vec![item_year]),
        )]));

        let nty_template = ConfigValue::Map(BTreeMap::from([(
            "spec".to_string(),
            ConfigValue::List(vec![group1, group2, group3]),
        )]));
        let templates = ConfigValue::Map(BTreeMap::from([("nty".to_string(), nty_template)]));
        biber
            .config
            .setblxoption(None, "sortingtemplate", templates);
        biber.set_current_section(0);

        process_sortinit(&mut biber, 0, "nty/global//global/global/global");

        let lists = biber.datalists.get_lists_for_section(0);
        assert_eq!(
            lists[0].state.sortinit.get("doe2020").map(|s| s.as_str()),
            Some("D")
        );
    }

    #[test]
    fn process_sortinithash_hashes_sortinit() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        let e1 = Entry::new("doe2020", "book");
        section.bibentries.add_entry(e1);
        section.add_cite("doe2020");

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);

        // Manually set sortinit
        for list in biber.datalists.get_lists_for_section_mut(0) {
            if list.name == "nty/global//global/global/global" {
                list.state
                    .sortinit
                    .insert("doe2020".to_string(), "D".to_string());
                break;
            }
        }

        biber.set_current_section(0);
        process_sortinithash(&mut biber, 0, "nty/global//global/global/global");

        let lists = biber.datalists.get_lists_for_section(0);
        let hash = lists[0].state.sortinithash.get("doe2020");
        assert!(hash.is_some());
        // MD5 of "D" is 826e0f... (input depends on encoding; just check 32 hex chars)
        let h = hash.unwrap();
        assert_eq!(h.len(), 32);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn process_labelprefix_uses_shorthand_when_present() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        let mut e1 = Entry::new("abc2020", "book");
        e1.set_field_str("shorthand", "ABC");
        section.bibentries.add_entry(e1);
        section.add_cite("abc2020");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);

        for list in biber.datalists.get_lists_for_section_mut(0) {
            if list.name == "nty/global//global/global/global" {
                list.state.entries = vec!["abc2020".to_string()];
                break;
            }
        }

        biber.set_current_section(0);
        process_labelprefix(&mut biber, 0, "nty/global//global/global/global");

        let lists = biber.datalists.get_lists_for_section(0);
        assert_eq!(
            lists[0]
                .state
                .labelprefix_data
                .get("abc2020")
                .map(|s| s.as_str()),
            Some("ABC")
        );
    }

    #[test]
    fn process_labelprefix_uses_datalist_prefix_when_no_shorthand() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        let mut e1 = Entry::new("doe2020", "book");
        e1.set_field_str("author", "John Doe");
        section.bibentries.add_entry(e1);
        section.add_cite("doe2020");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "P",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);

        for list in biber.datalists.get_lists_for_section_mut(0) {
            if list.name == "nty/global//global/global/global" {
                list.state.entries = vec!["doe2020".to_string()];
                break;
            }
        }

        biber.set_current_section(0);
        process_labelprefix(&mut biber, 0, "nty/global//global/global/global");

        let lists = biber.datalists.get_lists_for_section(0);
        assert_eq!(
            lists[0]
                .state
                .labelprefix_data
                .get("doe2020")
                .map(|s| s.as_str()),
            Some("P")
        );
    }

    #[test]
    fn process_labelprefix_skips_when_shorthand_and_datalist_prefix_empty() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        let e1 = Entry::new("abc2020", "book");
        section.bibentries.add_entry(e1);
        section.add_cite("abc2020");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);

        for list in biber.datalists.get_lists_for_section_mut(0) {
            if list.name == "nty/global//global/global/global" {
                list.state.entries = vec!["abc2020".to_string()];
                break;
            }
        }

        biber.set_current_section(0);
        process_labelprefix(&mut biber, 0, "nty/global//global/global/global");

        let lists = biber.datalists.get_lists_for_section(0);
        assert!(!lists[0].state.labelprefix_data.contains_key("abc2020"));
    }

    #[test]
    fn process_sortinit_returns_none_when_no_template() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("test1");
        biber.sections.add_section(section);
        biber.set_current_section(0);

        // No sortingtemplate in config
        process_sortinit(&mut biber, 0, "nonexistent");

        // Should not panic; no state should be set
        let lists = biber.datalists.get_lists_for_section(0);
        assert!(lists.is_empty());
    }

    #[test]
    fn process_sortinit_and_sortinithash_set_state_correctly() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);

        let mut e1 = Entry::new("doe2020", "book");
        e1.set_field_str("author", "John Doe");
        e1.set_field_str("year", "2020");
        crate::name::parse_entry_names(&mut e1);
        e1.set_field_str("labelname", "author");
        e1.set_field_str("presort", "mm");
        section.bibentries.add_entry(e1);

        section.add_cite("doe2020");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);

        for list in biber.datalists.get_lists_for_section_mut(0) {
            if list.name == "nty/global//global/global/global" {
                list.state.entries = vec!["doe2020".to_string()];
                break;
            }
        }

        // Set up a sorting template
        let item_presort = ConfigValue::Map(BTreeMap::from([(
            "presort".to_string(),
            ConfigValue::Map(BTreeMap::new()),
        )]));
        let item_author = ConfigValue::Map(BTreeMap::from([(
            "author".to_string(),
            ConfigValue::Map(BTreeMap::new()),
        )]));
        let group1 = ConfigValue::Map(BTreeMap::from([(
            "items".to_string(),
            ConfigValue::List(vec![item_presort]),
        )]));
        let group2 = ConfigValue::Map(BTreeMap::from([(
            "items".to_string(),
            ConfigValue::List(vec![item_author]),
        )]));
        let nty_template = ConfigValue::Map(BTreeMap::from([(
            "spec".to_string(),
            ConfigValue::List(vec![group1, group2]),
        )]));
        let templates = ConfigValue::Map(BTreeMap::from([("nty".to_string(), nty_template)]));
        biber
            .config
            .setblxoption(None, "sortingtemplate", templates);
        biber.set_current_section(0);

        process_sortinit(&mut biber, 0, "nty/global//global/global/global");
        process_sortinithash(&mut biber, 0, "nty/global//global/global/global");

        let lists = biber.datalists.get_lists_for_section(0);
        let sortinit = lists[0].state.sortinit.get("doe2020");
        let sortinithash = lists[0].state.sortinithash.get("doe2020");
        assert_eq!(sortinit.map(|s| s.as_str()), Some("D"));
        assert!(sortinithash.is_some());
        assert_eq!(sortinithash.unwrap().len(), 32);
    }

    #[test]
    fn generate_singletitle_sets_field_when_unique() {
        let mut biber = Biber::new();
        biber
            .config
            .setblxoption(Some(0), "singletitle", ConfigValue::Str("1".into()));
        let mut section = Section::new(0);
        let mut e1 = Entry::new("key1", "book");
        e1.set_field_str("author", "Unique Author");
        e1.set_field_str("title", "Only Book");
        crate::name::parse_entry_names(&mut e1);
        e1.set_field_str("labelname", "author");
        section.bibentries.add_entry(e1);
        section.add_cite("key1");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_workuniqueness(&mut biber, 0, "key1");
        generate_singletitle(&mut biber, 0, "key1");

        let be = biber
            .sections
            .get_section(0)
            .unwrap()
            .bibentry("key1")
            .unwrap();
        assert_eq!(be.get_field_str("singletitle"), Some("1"));
    }

    #[test]
    fn generate_singletitle_does_not_set_when_duplicate() {
        let mut biber = Biber::new();
        biber
            .config
            .setblxoption(Some(0), "singletitle", ConfigValue::Str("1".into()));
        let mut section = Section::new(0);
        let mut e1 = Entry::new("key1", "book");
        e1.set_field_str("author", "John Smith");
        crate::name::parse_entry_names(&mut e1);
        e1.set_field_str("labelname", "author");
        section.bibentries.add_entry(e1);

        let mut e2 = Entry::new("key2", "book");
        e2.set_field_str("author", "John Smith");
        crate::name::parse_entry_names(&mut e2);
        e2.set_field_str("labelname", "author");
        section.bibentries.add_entry(e2);

        section.add_cite("key1");
        section.add_cite("key2");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_workuniqueness(&mut biber, 0, "key1");
        process_workuniqueness(&mut biber, 0, "key2");
        generate_singletitle(&mut biber, 0, "key1");
        generate_singletitle(&mut biber, 0, "key2");

        let be1 = biber
            .sections
            .get_section(0)
            .unwrap()
            .bibentry("key1")
            .unwrap();
        let be2 = biber
            .sections
            .get_section(0)
            .unwrap()
            .bibentry("key2")
            .unwrap();
        // Neither should have singletitle since they share a hash
        assert_eq!(be1.get_field_str("singletitle"), None);
        assert_eq!(be2.get_field_str("singletitle"), None);
    }

    #[test]
    fn generate_uniquetitle_sets_field_when_unique() {
        let mut biber = Biber::new();
        biber
            .config
            .setblxoption(Some(0), "uniquetitle", ConfigValue::Str("1".into()));
        let mut section = Section::new(0);
        let mut e1 = Entry::new("key1", "book");
        e1.set_field_str("title", "Unique Title");
        e1.set_field_str("labeltitle", "title");
        section.bibentries.add_entry(e1);
        section.add_cite("key1");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_workuniqueness(&mut biber, 0, "key1");
        generate_uniquetitle(&mut biber, 0, "key1");

        let be = biber
            .sections
            .get_section(0)
            .unwrap()
            .bibentry("key1")
            .unwrap();
        assert_eq!(be.get_field_str("uniquetitle"), Some("1"));
    }

    #[test]
    fn generate_uniquebaretitle_sets_field_when_unique() {
        let mut biber = Biber::new();
        biber
            .config
            .setblxoption(Some(0), "uniquebaretitle", ConfigValue::Str("1".into()));
        let mut section = Section::new(0);
        // No labelname set, only labeltitle
        let mut e1 = Entry::new("key1", "book");
        e1.set_field_str("title", "Bare Unique");
        e1.set_field_str("labeltitle", "title");
        section.bibentries.add_entry(e1);
        section.add_cite("key1");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_workuniqueness(&mut biber, 0, "key1");
        generate_uniquebaretitle(&mut biber, 0, "key1");

        let be = biber
            .sections
            .get_section(0)
            .unwrap()
            .bibentry("key1")
            .unwrap();
        assert_eq!(be.get_field_str("uniquebaretitle"), Some("1"));
    }

    #[test]
    fn generate_uniquework_sets_field_when_unique() {
        let mut biber = Biber::new();
        biber
            .config
            .setblxoption(Some(0), "uniquework", ConfigValue::Str("1".into()));
        let mut section = Section::new(0);
        let mut e1 = Entry::new("key1", "book");
        e1.set_field_str("author", "Unique Author");
        e1.set_field_str("title", "Unique Book");
        crate::name::parse_entry_names(&mut e1);
        e1.set_field_str("labelname", "author");
        e1.set_field_str("labeltitle", "title");
        section.bibentries.add_entry(e1);
        section.add_cite("key1");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_workuniqueness(&mut biber, 0, "key1");
        generate_uniquework(&mut biber, 0, "key1");

        let be = biber
            .sections
            .get_section(0)
            .unwrap()
            .bibentry("key1")
            .unwrap();
        assert_eq!(be.get_field_str("uniquework"), Some("1"));
    }

    #[test]
    fn generate_uniquepa_sets_field_when_unique() {
        let mut biber = Biber::new();
        biber
            .config
            .setblxoption(Some(0), "uniqueprimaryauthor", ConfigValue::Str("1".into()));

        // Need a uniquenametemplate for base-part extraction
        let mut global_map = std::collections::BTreeMap::new();
        global_map.insert(
            "global".to_string(),
            ConfigValue::List(vec![
                ConfigValue::Map({
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("namepart".to_string(), ConfigValue::Str("family".into()));
                    m.insert("base".to_string(), ConfigValue::Str("1".into()));
                    m
                }),
                ConfigValue::Map({
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("namepart".to_string(), ConfigValue::Str("given".into()));
                    m
                }),
            ]),
        );
        let template = ConfigValue::Map(global_map);
        biber
            .config
            .setblxoption(Some(0), "uniquenametemplate", template);

        let mut section = Section::new(0);
        let mut e1 = Entry::new("key1", "book");
        e1.set_field_str("author", "Smith");
        crate::name::parse_entry_names(&mut e1);
        e1.set_field_str("labelname", "author");
        section.bibentries.add_entry(e1);
        section.add_cite("key1");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_namedis(&mut biber, 0, "key1");
        process_uniqueprimaryauthor(&mut biber, 0, "key1");
        generate_uniquepa(&mut biber, 0, "key1");

        let be = biber
            .sections
            .get_section(0)
            .unwrap()
            .bibentry("key1")
            .unwrap();
        assert_eq!(be.get_field_str("uniqueprimaryauthor"), Some("1"));
    }

    #[test]
    fn process_workuniqueness_skips_skipbib_entries() {
        let mut biber = Biber::new();
        biber
            .config
            .setblxoption(Some(0), "singletitle", ConfigValue::Str("1".into()));
        let mut section = Section::new(0);

        let mut e1 = Entry::new("key1", "book");
        e1.set_field_str("author", "John Smith");
        crate::name::parse_entry_names(&mut e1);
        e1.set_field_str("labelname", "author");
        e1.set_field(
            "options",
            ConfigValue::List(vec![ConfigValue::Str("skipbib".into())]),
        );
        section.bibentries.add_entry(e1);

        let mut e2 = Entry::new("key2", "book");
        e2.set_field_str("author", "John Smith");
        crate::name::parse_entry_names(&mut e2);
        e2.set_field_str("labelname", "author");
        section.bibentries.add_entry(e2);

        section.add_cite("key1");
        section.add_cite("key2");
        biber.sections.add_section(section);

        let dl = crate::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);
        biber.set_current_section(0);

        process_workuniqueness(&mut biber, 0, "key1");
        process_workuniqueness(&mut biber, 0, "key2");

        let lists = biber.datalists.get_lists_for_section(0);
        // key2 should still be tracked, but key1 was skipped
        assert_eq!(lists[0].state.seenname.len(), 1);
        let count: u32 = lists[0].state.seenname.values().sum();
        assert_eq!(count, 1);
    }

    // ---- process_related tests ----

    #[test]
    fn relclone_basic_creates_clone_entry() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("parent");

        let mut parent = Entry::new("parent", "book");
        parent.set_field_str("author", "Parent Author");
        parent.set_field_str("related", "child");
        section.bibentries.add_entry(parent);

        let mut child = Entry::new("child", "book");
        child.set_field_str("author", "Child Author");
        child.set_field_str("title", "Child Title");
        child.set_field_str("year", "2020");
        section.bibentries.add_entry(child);

        biber.sections.add_section(section);
        biber.set_current_section(0);

        process_related(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();
        // Should have a clone key for "child"
        let clonekey = section.get_keytorelclone("child");
        assert!(clonekey.is_some(), "Should have a clone key for 'child'");

        let clonekey = clonekey.unwrap().to_string();
        // The clone entry should exist
        let clone_entry = section.bibentry(&clonekey);
        assert!(
            clone_entry.is_some(),
            "Clone entry should exist with key '{clonekey}'"
        );

        let clone = clone_entry.unwrap();
        assert!(clone.clone, "Clone entry should have clone=true");
        assert_eq!(
            clone.clonesourcekey,
            Some("child".to_string()),
            "Clone should track source key"
        );
        assert_eq!(
            clone.get_field_str("author"),
            Some("Child Author"),
            "Clone should inherit author from original"
        );
        assert_eq!(
            clone.get_field_str("title"),
            Some("Child Title"),
            "Clone should inherit title from original"
        );

        // The parent's related field should now reference the clone key
        let parent_entry = section.bibentry("parent").unwrap();
        assert_eq!(
            parent_entry.get_field_str("related"),
            Some(clonekey.as_str())
        );

        // The clone key should be in the citekeys
        assert!(
            section.get_citekeys().contains(&clonekey),
            "Clone key should be in citekeys"
        );
    }

    #[test]
    fn relclone_multiple_related_entries() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("parent");

        let mut parent = Entry::new("parent", "book");
        parent.set_field_str("related", "rel1, rel2");
        section.bibentries.add_entry(parent);

        let mut rel1 = Entry::new("rel1", "article");
        rel1.set_field_str("author", "Author One");
        section.bibentries.add_entry(rel1);

        let mut rel2 = Entry::new("rel2", "article");
        rel2.set_field_str("author", "Author Two");
        section.bibentries.add_entry(rel2);

        biber.sections.add_section(section);
        biber.set_current_section(0);

        process_related(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();

        let ck1 = section.get_keytorelclone("rel1").unwrap().to_string();
        let ck2 = section.get_keytorelclone("rel2").unwrap().to_string();

        assert!(
            section.bibentry(&ck1).is_some(),
            "Clone of rel1 should exist"
        );
        assert!(
            section.bibentry(&ck2).is_some(),
            "Clone of rel2 should exist"
        );

        let parent_entry = section.bibentry("parent").unwrap();
        let related_str = parent_entry.get_field_str("related").unwrap();
        assert!(
            related_str.contains(&ck1),
            "Parent related should contain ck1"
        );
        assert!(
            related_str.contains(&ck2),
            "Parent related should contain ck2"
        );

        // Both clone keys should be in citekeys
        assert!(section.get_citekeys().contains(&ck1));
        assert!(section.get_citekeys().contains(&ck2));
    }

    #[test]
    fn relclone_missing_related_key_removed() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("parent");

        let mut parent = Entry::new("parent", "book");
        parent.set_field_str("related", "existing, missing1");
        parent.set_field_str("relatedtype", "reprintas");
        section.bibentries.add_entry(parent);

        let existing = Entry::new("existing", "article");
        section.bibentries.add_entry(existing);

        biber.sections.add_section(section);
        biber.set_current_section(0);

        process_related(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();
        let parent_entry = section.bibentry("parent").unwrap();
        // "missing1" should have been removed
        let related = parent_entry.get_field_str("related").unwrap();
        assert!(
            !related.contains("missing1"),
            "Missing related key should be removed"
        );
        // "existing" should be replaced with its clone key
        assert!(!related.contains("existing"));
    }

    #[test]
    fn relclone_all_missing_removes_related_fields() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("parent");

        let mut parent = Entry::new("parent", "book");
        parent.set_field_str("related", "missing1");
        parent.set_field_str("relatedtype", "reprintas");
        parent.set_field_str("relatedstring", "Reprint of");
        section.bibentries.add_entry(parent);

        biber.sections.add_section(section);
        biber.set_current_section(0);

        process_related(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();
        let parent_entry = section.bibentry("parent").unwrap();
        // All related fields should be removed
        assert_eq!(parent_entry.get_field_str("related"), None);
        assert_eq!(parent_entry.get_field_str("relatedtype"), None);
        assert_eq!(parent_entry.get_field_str("relatedstring"), None);
    }

    #[test]
    fn relclone_duplicate_avoidance() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("parent1");
        section.add_cite("parent2");

        let mut p1 = Entry::new("parent1", "book");
        p1.set_field_str("related", "shared");
        section.bibentries.add_entry(p1);

        let mut p2 = Entry::new("parent2", "book");
        p2.set_field_str("related", "shared");
        section.bibentries.add_entry(p2);

        let shared = Entry::new("shared", "book");
        section.bibentries.add_entry(shared);

        biber.sections.add_section(section);
        biber.set_current_section(0);

        process_related(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();
        let ck = section.get_keytorelclone("shared").unwrap().to_string();

        // Both parents should reference the same clone key
        let p1_entry = section.bibentry("parent1").unwrap();
        let p2_entry = section.bibentry("parent2").unwrap();
        assert_eq!(p1_entry.get_field_str("related"), Some(ck.as_str()));
        assert_eq!(p2_entry.get_field_str("related"), Some(ck.as_str()));

        // Clone key should only be in citekeys once
        let count = section.get_citekeys().iter().filter(|k| *k == &ck).count();
        assert_eq!(count, 1, "Clone key should only appear once in citekeys");
    }

    #[test]
    fn relclone_cascading_related_entries() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("a");

        let mut a = Entry::new("a", "book");
        a.set_field_str("related", "b");
        section.bibentries.add_entry(a);

        let mut b = Entry::new("b", "book");
        b.set_field_str("related", "c");
        b.set_field_str("title", "B Title");
        section.bibentries.add_entry(b);

        let mut c = Entry::new("c", "book");
        c.set_field_str("title", "C Title");
        section.bibentries.add_entry(c);

        biber.sections.add_section(section);
        biber.set_current_section(0);

        process_related(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();

        // Both b and c should have clones
        let ck_b = section.get_keytorelclone("b").unwrap().to_string();
        let ck_c = section.get_keytorelclone("c").unwrap().to_string();

        assert!(section.bibentry(&ck_b).is_some(), "Clone of b should exist");
        assert!(section.bibentry(&ck_c).is_some(), "Clone of c should exist");

        // The clone of b should have its related field pointing to c's clone
        let b_clone = section.bibentry(&ck_b).unwrap();
        assert_eq!(
            b_clone.get_field_str("related"),
            Some(ck_c.as_str()),
            "B's clone should reference C's clone"
        );

        // Both clone keys should be in citekeys
        assert!(section.get_citekeys().contains(&ck_b));
        assert!(section.get_citekeys().contains(&ck_c));
    }

    #[test]
    fn entry_clone_with_key_preserves_fields() {
        let mut original = Entry::new("original", "book");
        original.set_field_str("author", "Original Author");
        original.set_field_str("title", "Original Title");
        original.set_field_str("year", "2023");
        original.set_member = true;

        let clone = original.clone_with_key("clonekey");

        assert_eq!(clone.citekey, "clonekey");
        assert_eq!(clone.entrytype, "book");
        assert!(clone.clone, "cloned entry should have clone=true");
        assert_eq!(clone.clonesourcekey, Some("original".to_string()));
        assert_eq!(clone.get_field_str("author"), Some("Original Author"));
        assert_eq!(clone.get_field_str("title"), Some("Original Title"));
        assert_eq!(clone.get_field_str("year"), Some("2023"));
        assert!(clone.set_member, "clone should preserve set_member");
    }

    // ---- Annotation tests ----

    #[test]
    fn process_annotations_extracts_field_scope() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("ann1");

        let mut entry = Entry::new("ann1", "misc");
        entry.set_field_str("title", "The Title");
        entry.set_field_str("title+an", "=one, two");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);

        process_annotations(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();
        let entry = section.bibentry("ann1").unwrap();

        // Annotation field should be removed from entry
        assert!(!entry.has_field("title+an"));
        // Title field should remain
        assert_eq!(entry.get_field_str("title"), Some("The Title"));

        // Annotation should be stored
        let ann = section
            .annotations
            .get_field_annotation("ann1", "title", "default");
        assert!(ann.is_some());
        assert_eq!(ann.unwrap().value, "one, two");
        assert!(!ann.unwrap().literal);
    }

    #[test]
    fn process_annotations_extracts_item_scope() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("ann1");

        let mut entry = Entry::new("ann1", "misc");
        entry.set_field_str("language", "english and french");
        entry.set_field_str("language+an", "1=ann1; =ann4");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);

        process_annotations(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();

        // Item-scope annotation
        let ann = section
            .annotations
            .get_item_annotation("ann1", "language", "default", 1);
        assert!(ann.is_some());
        assert_eq!(ann.unwrap().value, "ann1");

        // Field-scope annotation (no count)
        let ann = section
            .annotations
            .get_field_annotation("ann1", "language", "default");
        assert!(ann.is_some());
        assert_eq!(ann.unwrap().value, "ann4");
    }

    #[test]
    fn process_annotations_extracts_part_scope() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("ann1");

        let mut entry = Entry::new("ann1", "misc");
        entry.set_field_str("author", "Last1, First1 and Last2, First2");
        entry.set_field_str("author+an", "1:family=student;2=corresponding");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);

        process_annotations(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();

        // Part-scope annotation
        let ann = section
            .annotations
            .get_part_annotation("ann1", "author", "default", 1, "family");
        assert!(ann.is_some());
        assert_eq!(ann.unwrap().value, "student");

        // Item-scope annotation (no part)
        let ann = section
            .annotations
            .get_item_annotation("ann1", "author", "default", 2);
        assert!(ann.is_some());
        assert_eq!(ann.unwrap().value, "corresponding");
    }

    #[test]
    fn process_annotations_named_annotation() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("ann2");

        let mut entry = Entry::new("ann2", "misc");
        entry.set_field_str("title", "The Title");
        entry.set_field_str("title+an:default", "=\"one\"");
        entry.set_field_str("title+an:french", "=\"un\"");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);

        process_annotations(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();

        let ann = section
            .annotations
            .get_field_annotation("ann2", "title", "default");
        assert!(ann.is_some());
        assert_eq!(ann.unwrap().value, "one");
        assert!(ann.unwrap().literal);

        let ann = section
            .annotations
            .get_field_annotation("ann2", "title", "french");
        assert!(ann.is_some());
        assert_eq!(ann.unwrap().value, "un");
        assert!(ann.unwrap().literal);
    }

    #[test]
    fn process_annotations_removes_fields() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("ann1");

        let mut entry = Entry::new("ann1", "misc");
        entry.set_field_str("title", "Real Title");
        entry.set_field_str("title+an", "=ann_val");
        entry.set_field_str("language+an", "1=item_ann");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);

        process_annotations(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();
        let entry = section.bibentry("ann1").unwrap();

        // Annotation fields should be removed
        assert!(!entry.has_field("title+an"));
        assert!(!entry.has_field("language+an"));
        // Regular fields should remain
        assert_eq!(entry.get_field_str("title"), Some("Real Title"));
    }

    #[test]
    fn process_annotations_skips_regular_fields() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("plain");

        let mut entry = Entry::new("plain", "book");
        entry.set_field_str("author", "John Doe");
        entry.set_field_str("title", "A Plain Book");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);

        process_annotations(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();
        // No annotations should be stored
        assert!(!section.annotations.has_annotations("plain"));
        // Fields should be unchanged
        let entry = section.bibentry("plain").unwrap();
        assert_eq!(entry.get_field_str("author"), Some("John Doe"));
        assert_eq!(entry.get_field_str("title"), Some("A Plain Book"));
    }

    #[test]
    fn annotations_copied_during_relclone() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.add_cite("parent");

        let mut parent = Entry::new("parent", "book");
        parent.set_field_str("related", "child");
        section.bibentries.add_entry(parent);

        let mut child = Entry::new("child", "book");
        child.set_field_str("title", "Child Title");
        child.set_field_str("title+an", "=child_ann");
        section.bibentries.add_entry(child);

        biber.sections.add_section(section);

        // First process annotations to extract them
        process_annotations(&mut biber, 0);
        // Then clone related entries (which copies annotations)
        process_related(&mut biber, 0);

        let section = biber.sections.get_section(0).unwrap();

        // Get the clone key for child
        let ck = section.get_keytorelclone("child").unwrap().to_string();

        // Original entry should have annotations
        let ann = section
            .annotations
            .get_field_annotation("child", "title", "default");
        assert!(ann.is_some());

        // Clone should have copied annotations
        let cloned_ann = section
            .annotations
            .get_field_annotation(&ck, "title", "default");
        assert!(cloned_ann.is_some(), "Clone should have copied annotations");
        assert_eq!(cloned_ann.unwrap().value, "child_ann");
    }
}
