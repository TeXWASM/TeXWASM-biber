//! Dump Rust pipeline output for `full-bbl.bcf` to a file for comparison.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("CARGO_MANIFEST_DIR should be crates/biber-core")
        .to_path_buf()
}

fn tdata_dir() -> PathBuf {
    repo_root().join("t").join("tdata")
}

fn run_rust_biber(bcf: &Path) -> Result<String, String> {
    let bcf_text = std::fs::read_to_string(bcf).map_err(|e| format!("cannot read bcf: {e}"))?;
    let mut biber =
        biber_input_bcf::parse_bcf(&bcf_text).map_err(|e| format!("BCF parse error: {e}"))?;
    let bcf_dir = bcf.parent().unwrap_or(Path::new("."));

    for section in biber.sections.get_sections_mut() {
        let datasources: Vec<_> = section.get_datasources().to_vec();
        for ds_ref in &datasources {
            if ds_ref.r#type != "file" || ds_ref.name.is_empty() {
                continue;
            }
            let ds_path = if Path::new(&ds_ref.name).is_absolute() {
                PathBuf::from(&ds_ref.name)
            } else {
                bcf_dir.join(&ds_ref.name)
            };

            let bib_text = match std::fs::read_to_string(&ds_path) {
                Ok(s) => s,
                Err(_) => {
                    let sibling = bcf.with_extension("bib");
                    match std::fs::read_to_string(&sibling) {
                        Ok(s) => s,
                        Err(_) => continue,
                    }
                }
            };

            let (map, order, _preambles) = biber_input_bib::parse_bib_into_map(&bib_text)
                .map_err(|e| format!("BibTeX parse error: {e}"))?;

            for key in order.iter() {
                if let Some(bib_entry) = map.get(key) {
                    let ids_val = bib_entry
                        .fields
                        .iter()
                        .find(|(f, _)| f.eq_ignore_ascii_case("ids"))
                        .map(|(_, v)| v.as_str());
                    if let Some(ids) = ids_val {
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

            for key in &order {
                let is_cited = section.get_citekeys().contains(key);
                let is_alias_target = section
                    .get_citekeys()
                    .iter()
                    .any(|ck| section.get_citekey_alias(ck) == Some(key.as_str()));
                if (is_cited || is_alias_target || section.is_allkeys())
                    && !section.bibentries.has_entry(key)
                {
                    let bib_entry = &map[key];
                    let mut entry = biber_core::Entry::new(key.clone(), bib_entry.typ.clone());
                    entry.datasource = ds_ref.name.clone();
                    entry.set_field_str("citekey", key);
                    entry.set_field_str("entrytype", &bib_entry.typ);
                    for (field, value) in &bib_entry.fields {
                        entry.set_field_str(field, value);
                    }
                    entry.set_field_str("datatype", "bibtex");
                    section.bibentries.add_entry(entry);
                }
            }

            let cited_keys: Vec<String> = section.get_citekeys().to_vec();
            for ck in &cited_keys {
                if !section.bibentries.has_entry(ck) && section.get_citekey_alias(ck).is_none() {
                    section.add_undef_citekey(ck);
                }
            }
        }
    }

    biber_core::pipeline::prepare(&mut biber);
    Ok(biber_output_bbl::write_bbl(&biber))
}

#[test]
fn dump_bbl() {
    let bcf = tdata_dir().join("full-bbl.bcf");
    let actual = run_rust_biber(&bcf).expect("Rust pipeline should succeed");
    let out_path = repo_root().join("target").join("full-bbl-actual.bbl");
    std::fs::write(&out_path, &actual).expect("write output");
    eprintln!("Wrote actual output to: {}", out_path.display());
}
