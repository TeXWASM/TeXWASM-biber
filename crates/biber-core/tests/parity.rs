//! Differential parity harness: Rust port vs. Perl `bin/biber`.
//!
//! Mirrors `t/full-bbl.t` but cross-language. For each `.bcf` fixture in
//! `t/tdata/`, runs the Perl binary once to capture the reference `.bbl`,
//! then asserts the Rust port matches byte-for-byte.
//!
//! Expects the original biber repo cloned at `biber/` in the workspace root
//! (or set `BIBER_BIBER` to an alternate path). If the Perl biber script is
//! not found, the test skips gracefully.
//!
//! Optionally set `BIBER_PERL` to choose a specific Perl binary.

#![cfg(test)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// Locate the repo root from CARGO_MANIFEST_DIR (crates/biber-core -> root).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("CARGO_MANIFEST_DIR should be crates/biber-core")
        .to_path_buf()
}

/// The shared fixture directory (`t/tdata`).
fn tdata_dir() -> PathBuf {
    repo_root().join("t").join("tdata")
}

/// The Perl `bin/biber` script path.
///
/// Uses `BIBER_BIBER` env var if set, otherwise checks the cloned
/// `biber/` directory in the repo root.
fn perl_biber() -> PathBuf {
    if let Ok(path) = std::env::var("BIBER_BIBER") {
        return PathBuf::from(path);
    }
    // Default: cloned biber repo under the workspace root
    repo_root().join("biber").join("bin").join("biber")
}

/// Discover every `.bcf` fixture in `t/tdata`.
fn bcf_fixtures() -> Vec<PathBuf> {
    let dir = tdata_dir();
    let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "bcf"))
        .collect();
    out.sort();
    out
}

/// Run the Perl `bin/biber` on `bcf` and return the `.bbl` it produces.
///
/// Mirrors `t/full-bbl.t`: `--noconf --nolog --output-file=<tmp>`.
fn run_perl_biber(bcf: &Path) -> Result<String, String> {
    let perl = std::env::var("BIBER_PERL").unwrap_or_else(|_| "perl".to_string());
    let script = perl_biber();
    if !script.exists() {
        return Err(format!(
            "Perl biber not found at {} — run from a bootstrapped source tree",
            script.display()
        ));
    }

    let tmp = std::env::temp_dir().join(format!(
        "biber-parity-{}.bbl",
        bcf.file_stem().and_then(|s| s.to_str()).unwrap_or("x")
    ));

    let output = Command::new(&perl)
        .arg(&script)
        .arg("--noconf")
        .arg("--nolog")
        .arg(format!("--output-file={}", tmp.display()))
        .arg(bcf)
        .current_dir(repo_root())
        .output()
        .map_err(|e| format!("spawning perl: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "perl biber exited {}: stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    std::fs::read_to_string(&tmp).map_err(|e| format!("reading {}: {e}", tmp.display()))
}

/// Run the Rust biber pipeline on a `.bcf` fixture and return the `.bbl`.
///
/// Mirrors the flow in `biber-cli/src/main.rs`: BCF parse → Bib parse →
/// prepare() → write_bbl().
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
                    // Fallback: try sibling .bib with the same stem
                    let sibling = bcf.with_extension("bib");
                    match std::fs::read_to_string(&sibling) {
                        Ok(s) => s,
                        Err(_) => continue,
                    }
                }
            };

            let (map, order, _preambles) = biber_input_bib::parse_bib_into_map(&bib_text)
                .map_err(|e| format!("BibTeX parse error: {e}"))?;

            // Add entries to the section
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
        }
    }

    biber_core::pipeline::prepare(&mut biber);
    Ok(biber_output_bbl::write_bbl(&biber))
}

// ----------------------------------------------------------------------------
// Basic smoke test. Runs by default; no Perl needed.
// ----------------------------------------------------------------------------

#[test]
fn stub_returns_well_formed_empty_bbl() {
    let opts = biber_core::Options::default();
    let bbl = biber_core::process("", &[], &opts).expect("stub process() failed");
    // The stub output equals empty_bbl().
    assert_eq!(bbl, biber_core::empty_bbl());
    // Sanity: well-formed bbl header.
    assert!(bbl.contains("biblatex bbl format version 3.3"));
    assert!(bbl.contains("\\begingroup"));
    assert!(bbl.contains("\\makeatletter"));
    assert!(bbl.contains("\\endinput"));
    // Trailing newline.
    assert!(bbl.ends_with('\n'), "bbl must end with a newline");
}

#[test]
fn fixtures_are_discoverable() {
    let fixtures = bcf_fixtures();
    // The Perl suite ships 53 .bcf fixtures; we should find all of them.
    assert!(
        fixtures.len() >= 50,
        "expected >=50 .bcf fixtures, found {}; looked in {}",
        fixtures.len(),
        tdata_dir().display()
    );
    // Every fixture has a readable stem.
    for f in &fixtures {
        assert!(
            f.file_stem().is_some(),
            "fixture without a stem: {}",
            f.display()
        );
    }
}

// ----------------------------------------------------------------------------
#[test]
fn parity_matches_perl_on_all_fixtures() {
    let fixtures = bcf_fixtures();
    assert!(!fixtures.is_empty(), "no .bcf fixtures found");

    let mut mismatches = Vec::new();
    let mut per_skip = 0;
    let mut rust_errors = 0;

    for bcf in &fixtures {
        let stem = bcf
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("<unknown>");

        let reference = match run_perl_biber(bcf) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("[skip perl] {stem}: {e}");
                per_skip += 1;
                continue;
            }
        };

        match run_rust_biber(bcf) {
            Ok(actual) => {
                if actual != reference {
                    mismatches.push(stem.to_string());
                    if mismatches.len() <= 3 {
                        eprintln!(
                            "[mismatch] {stem}\n--- perl ({} bytes) ---\n{}--- rust ({} bytes) ---\n{}",
                            reference.len(),
                            &reference[..reference.len().min(400)],
                            actual.len(),
                            &actual[..actual.len().min(400)]
                        );
                    }
                } else {
                    eprintln!("[ok] {stem}");
                }
            }
            Err(e) => {
                rust_errors += 1;
                mismatches.push(format!("{stem} (rust error: {e})"));
            }
        }
    }

    if per_skip > 0 {
        eprintln!("skipped {per_skip} fixtures (perl unavailable or read errors)");
    }
    if rust_errors > 0 {
        eprintln!("{rust_errors} fixture(s) caused Rust errors");
    }

    assert!(
        mismatches.is_empty(),
        "{} fixture(s) mismatched perl: {}",
        mismatches.len(),
        mismatches.join(", ")
    );
}
