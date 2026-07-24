# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]



## [1.0.0] - 2026-07-24

### Added

- Full Rust rewrite of the biber bibliography backend for biblatex.
- `.bcf` control-file parsing (BCF v3.11).
- BibTeX (`.bib`) reader with entry scanner, macros, `#` concatenation, and both standard and extended name parsing.
- LaTeX ↔ Unicode recode (`latex_decode` / `latex_encode`) with Base and Full character sets.
- ISO 8601-2 / EDTF date parser with uncertain/approximate and Julian calendar support.
- BCP47 language tag parser.
- Processing pipeline: alias/xdata/sets/interentry/datamodel-validation, label generation (labelname, labeldate, labeltitle, labelalpha), name hashing/disambiguation, extradate/extraalpha/extraname/extratitle, work uniqueness.
- ICU4X locale-aware collation for sort/filter lists.
- `.bbl` writer (BBL v3.3).
- `bblxml` output (`--output-format=bblxml`).
- `dot` output (`--output-format=dot`, Graphviz digraph).
- Sourcemap (`\DeclareSourcemap`) application with field rename, type rename, match/replace, field set, entry clone/null, and filters.
- CrossRef/XDATA inheritance with circular reference detection and datepart blocking.
- CLI (normal mode and tool mode with multiple output formats).
- Logging with `BlgLayer` and dual tracing subscriber; `.blg` file output.
- BiblateXML (`.bltxml`) input reader and output writer.
- WASM browser bindings (`wasm32-unknown-unknown`) — `process_biber()` API.
- WASI/Node CLI (`wasm32-wasip1`) with full filesystem access.
- ISBN/ISSN/ISMN validation.
- Config file parsing (`biber.conf`).
- Output safechars (`--output-safechars` / `--output-safecharsset`).
- Line wrapping (`--wraplines`).
- Annotations (`\annotation` in `.bbl`).
- RELAX NG schema generation and validation (`--validate-config`, `--validate-control`, `--validate-datamodel`).
- Transliteration (3 pairs via pure Rust).
- Cross-language parity test harness comparing Rust output byte-for-byte with Perl `bin/biber`.

### Deferred (from original Perl biber)

- Remote `.bib` fetching (`LWP::UserAgent`) — dropped for WASM; host resolves URLs.
- `kpsewhich` path resolution — dropped for WASM; inputs come pre-resolved.
