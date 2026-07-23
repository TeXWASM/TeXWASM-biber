//! Browser-facing WASM bindings for TeXWASM Biber.
//!
//! Exposes [`process_biber`]: given a `.bcf` string and an array of
//! `[name, contents]` `.bib` datasource pairs, parses the BCF, loads
//! entries from the `.bib` sources, runs the processing pipeline, and
//! returns the `.bbl` output as a string.
//!
//! The host (browser JS) is responsible for resolving files/URLs and
//! passing UTF-8 strings. No filesystem or network access is needed.

#![forbid(unsafe_code)]

use biber_core::entry::Entry;
use wasm_bindgen::prelude::*;

/// Run the biber pipeline in the browser.
///
/// `bibs` is a JS array of `[name, contents]` pairs. Example:
///
/// ```js
/// const bbl = process_biber(
///   bcfString,
///   [["refs.bib", bibContents]],
///   {} // options (currently ignored)
/// );
/// ```
///
/// Returns the `.bbl` output as a string, or throws on error.
#[wasm_bindgen]
pub fn process_biber(bcf: String, bibs: JsValue, _opts: JsValue) -> Result<String, JsValue> {
    // Parse the BCF control file
    let mut biber = biber_input_bcf::parse_bcf(&bcf)
        .map_err(|e| JsValue::from_str(&format!("BCF parse error: {e}")))?;

    // Parse the .bib datasources passed from JS
    let bib_entries = parse_bib_js(&bibs)?;

    // Load entries into sections (mirrors the CLI flow)
    for (ds_name, bib_map, key_order) in &bib_entries {
        for section in biber.sections.get_sections_mut() {
            // Extract IDS aliases
            for key in key_order {
                if let Some(bib_entry) = bib_map.get(key) {
                    if let Some(ids) = bib_entry.get("ids") {
                        for alias in ids.split(',').map(|s| s.trim().to_string()) {
                            if !alias.is_empty() && alias != *key {
                                section.set_citekey_alias(&alias, key);
                                if section.get_citekeys().contains(&alias) {
                                    section.add_citekeys(std::iter::once(key.clone()));
                                }
                            }
                        }
                    }
                }
            }

            // Add entries
            for key in key_order {
                let is_cited = section.get_citekeys().contains(key);
                let is_alias_target = section
                    .get_citekeys()
                    .iter()
                    .any(|ck| section.get_citekey_alias(ck) == Some(key.as_str()));
                if (is_cited || is_alias_target || section.is_allkeys())
                    && !section.bibentries.has_entry(key)
                {
                    if let Some(bib_entry) = bib_map.get(key) {
                        let mut entry = Entry::new(key.clone(), bib_entry.typ.clone());
                        entry.datasource = ds_name.clone();
                        entry.set_field_str("citekey", key);
                        entry.set_field_str("entrytype", &bib_entry.typ);
                        for (field, value) in &bib_entry.fields {
                            entry.set_field_str(field, value);
                        }
                        entry.set_field_str("datatype", "bibtex");
                        section.bibentries.add_entry(entry);
                    }
                }
            }

            // Mark undefined citekeys
            let cited: Vec<String> = section.get_citekeys().to_vec();
            for ck in &cited {
                if !section.bibentries.has_entry(ck) && section.get_citekey_alias(ck).is_none() {
                    section.add_undef_citekey(ck);
                }
            }
        }
    }

    // Run the processing pipeline
    biber_core::pipeline::prepare(&mut biber);

    // Generate .bbl output
    let bbl = biber_output_bbl::write_bbl(&biber);

    Ok(bbl)
}

/// Parsed bib datasource: (name, entry_map, key_order).
type BibDatasource = (
    String,
    std::collections::HashMap<String, biber_input_bib::BibEntry>,
    Vec<String>,
);

/// Parse the JS `bibs` value (array of `[name, contents]` pairs) into
/// a list of (datasource_name, entry_map, key_order).
fn parse_bib_js(bibs: &JsValue) -> Result<Vec<BibDatasource>, JsValue> {
    let mut result = Vec::new();

    if bibs.is_undefined() || bibs.is_null() {
        return Ok(result);
    }

    let bibs_arr = js_sys::Array::from(bibs);
    for i in 0..bibs_arr.length() {
        let pair = bibs_arr.get(i);
        let pair_arr = js_sys::Array::from(&pair);
        if pair_arr.length() < 2 {
            continue;
        }
        let name = pair_arr.get(0).as_string().unwrap_or_default();
        let contents = pair_arr.get(1).as_string().unwrap_or_default();

        let (map, order, _preambles) = biber_input_bib::parse_bib_into_map(&contents)
            .map_err(|e| JsValue::from_str(&format!("BibTeX parse error in {}: {e}", name)))?;

        result.push((name, map, order));
    }

    Ok(result)
}

#[wasm_bindgen]
pub fn version() -> String {
    format!(
        "TeXWASM Biber - A Rust port of the Biber bibliography processor {} — BBL format v{}",
        env!("CARGO_PKG_VERSION"),
        biber_core::BBL_VERSION
    )
}

#[cfg(test)]
mod wasm_tests {
    use super::*;
    use wasm_bindgen_test::*;

    #[wasm_bindgen_test]
    fn version_returns_string() {
        let v = version();
        assert!(v.contains("biber"));
        assert!(v.contains("Rust port"));
    }

    #[wasm_bindgen_test]
    fn process_biber_returns_bbl_header() {
        let bcf = r#"<?xml version="1.0" encoding="UTF-8"?>
<bcf:controlfile version="3.11" bltxversion="3.21" xmlns:bcf="https://sourceforge.net/projects/biblatex">
  <bcf:bibdata section="0">
    <bcf:datasource type="file" datatype="bibtex">test.bib</bcf:datasource>
  </bcf:bibdata>
  <bcf:section number="0">
    <bcf:citekey order="1" intorder="1">key1</bcf:citekey>
  </bcf:section>
  <bcf:datalist sortingnamekeytemplatename="global" section="0" name="nty/global//global/global/global" sortingtemplatename="nty" type="entry" labelprefix="" uniquenametemplatename="global" labelalphanametemplatename="global" namehashtemplatename="global"/>
</bcf:controlfile>"#;

        let bib_content = r#"@book{key1, author = {John Doe}, title = {A Book}, year = {2020}}"#;

        // Build the JS bibs array
        let bibs = js_sys::Array::new();
        let pair = js_sys::Array::new();
        pair.push(&JsValue::from_str("test.bib"));
        pair.push(&JsValue::from_str(bib_content));
        bibs.push(&pair);

        let result = process_biber(bcf.to_string(), bibs.into(), JsValue::undefined());
        assert!(result.is_ok(), "process_biber should succeed");
        let bbl = result.unwrap();
        assert!(bbl.contains("biblatex bbl format version"));
        assert!(bbl.contains("\\refsection{0}"));
        assert!(bbl.contains("\\entry{key1}{book}"));
        assert!(bbl.contains("\\field{title}{A Book}"));
        assert!(bbl.contains("\\field{year}{2020}"));
        assert!(bbl.contains("\\endinput"));
    }

    #[wasm_bindgen_test]
    fn process_biber_handles_empty_bibs() {
        let bcf = r#"<?xml version="1.0" encoding="UTF-8"?>
<bcf:controlfile version="3.11" bltxversion="3.21" xmlns:bcf="https://sourceforge.net/projects/biblatex">
  <bcf:bibdata section="0">
    <bcf:datasource type="file" datatype="bibtex">test.bib</bcf:datasource>
  </bcf:bibdata>
  <bcf:section number="0">
    <bcf:citekey order="1" intorder="1">missing</bcf:citekey>
  </bcf:section>
</bcf:controlfile>"#;

        let result = process_biber(bcf.to_string(), JsValue::undefined(), JsValue::undefined());
        assert!(result.is_ok());
        let bbl = result.unwrap();
        assert!(bbl.contains("\\refsection{0}"));
        // missing key should appear as \missing{missing}
        assert!(bbl.contains("\\missing{missing}"));
    }
}
