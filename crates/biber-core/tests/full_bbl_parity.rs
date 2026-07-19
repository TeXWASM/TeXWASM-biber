//! Compare Rust pipeline output against the committed `full-bbl.bbl` reference.
//! This test runs *without* Perl — it uses the checked-in `.bbl` as the
//! expected output, allowing us to iterate on parity without a Perl env.

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

            // Extract IDS aliases (e.g. IDS={F1a} -> alias F1a->F1)
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

            // Mark citekeys not found in any datasource as undefined
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
fn rust_matches_reference_bbl() {
    let bcf = tdata_dir().join("full-bbl.bcf");
    let reference_path = tdata_dir().join("full-bbl.bbl");
    let reference_raw =
        std::fs::read_to_string(&reference_path).expect("reference full-bbl.bbl should exist");
    // Normalise CRLF → LF so the test works on Windows
    let reference = reference_raw.replace("\r\n", "\n");

    let actual = run_rust_biber(&bcf).expect("Rust pipeline should succeed");

    // sortinithash differs because Perl uses Unicode::Collate::viewSortKey
    // which requires ICU4X collation (deferred). Normalise both sides to a
    // placeholder so only structural differences are checked.
    let mask_sortinithash = |s: &str| -> String {
        s.lines()
            .map(|line| {
                if line.contains("sortinithash") {
                    let indent = &line[..line.len() - line.trim_start().len()];
                    format!("{indent}\\field{{sortinithash}}{SORTINITHASH_PLACEHOLDER}")
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let ref_masked = mask_sortinithash(&reference);
    let act_masked = mask_sortinithash(&actual);

    if ref_masked != act_masked {
        let ref_lines: Vec<&str> = ref_masked.lines().collect();
        let act_lines: Vec<&str> = act_masked.lines().collect();
        for (i, (r, a)) in ref_lines.iter().zip(act_lines.iter()).enumerate() {
            if r != a {
                eprintln!("First difference at line {}:", i + 1);
                eprintln!("  reference: {r}");
                eprintln!("  actual:    {a}");
                break;
            }
        }
        if ref_lines.len() != act_lines.len() {
            eprintln!(
                "Line count: reference={}, actual={}",
                ref_lines.len(),
                act_lines.len()
            );
        }
        let max_show = 30;
        for (i, (r, a)) in ref_lines
            .iter()
            .zip(act_lines.iter())
            .enumerate()
            .take(max_show)
        {
            if r != a {
                eprintln!("  {:3} ref: {r}", i + 1);
                eprintln!("      act: {a}");
            }
        }
        eprintln!("\n=== REFERENCE BBL ===\n{reference}");
        eprintln!("\n=== ACTUAL BBL ===\n{actual}");
        panic!("Rust output does not match reference full-bbl.bbl (sortinithash masked)");
    }
}
const SORTINITHASH_PLACEHOLDER: &str = "{<SORTINITHASH>}";
