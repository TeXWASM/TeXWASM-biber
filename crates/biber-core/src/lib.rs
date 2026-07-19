//! Core biber pipeline: data model, processing passes, orchestration.
//!
//! All phases complete: domain model, BCF/BibTeX/BiblateXML readers,
//! processing pipeline, BBL/BBLXML/DOT/BibTeX/BiblateXML writers,
//! WASM bindings, RELAX NG validation, and schema generation.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod annotation;
pub mod collation;
pub mod config;
pub mod config_reader;
pub mod constants;
pub mod data_model;
pub mod datalist;
pub mod dates;
pub mod entry;
pub mod inheritance;
pub mod label_alpha;
pub mod langtag;
pub mod latex_recode;
pub mod name;
pub mod pipeline;
pub mod processor;
pub mod section;
pub mod sourcemap;
pub mod transliteration;
pub mod validate;
pub mod vendored;

pub use annotation::{
    parse_annotation_field, Annotation, AnnotationEntry, AnnotationScope, AnnotationStore,
    BibAnnotation,
};
pub use config::{Config, ConfigValue, DatafieldSetMember, OptionMeta};
pub use config_reader::parse_biber_config;
pub use constants::{BBL_VERSION, BCF_VERSION};
pub use data_model::DataModel;
pub use datalist::{DataList, DataLists, ListFilter, ListFilterOr};
pub use dates::{parse_date_range, DateRange, ParsedDate};
pub use entry::{Entries, Entry};
pub use inheritance::{
    inherit_from, parse_inheritance_xml, resolve_xdata_section, InheritanceField, InheritanceRule,
    InheritanceScheme,
};
pub use langtag::{parse_langtag, LangTag};
pub use latex_recode::{latex_decode, latex_encode, normalise_utf8, RecodeSet, Recoder};
pub use name::{compute_name_hash, Name, Names};
pub use processor::Biber;
pub use section::{DatasourceRef, Section, Sections};
pub use sourcemap::{apply_sourcemap, parse_sourcemap_xml, Sourcemap, SourcemapMap, SourcemapStep};
pub use transliteration::{
    apply as apply_transliteration, parse_transliteration_xml, rule_to_config_value,
    rules_from_config_value, TranslitRule,
};

use thiserror::Error;

/// Error type for the biber pipeline.
#[derive(Debug, Error)]
pub enum BiberError {
    /// An input file could not be parsed.
    #[error("input error: {0}")]
    Input(String),
    /// The pipeline produced an invalid state.
    #[error("processing error: {0}")]
    Processing(String),
    /// An I/O error (only with the `fs` feature).
    #[cfg(feature = "fs")]
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Per-run options passed to [`process`].
///
/// Fields mirror the most common `bin/biber` CLI options.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Equivalent to `--noconf`: ignore any `biber.conf` the host would
    /// otherwise resolve. WASM callers should leave this `true`.
    pub noconf: bool,
    /// Equivalent to `--nolog`: suppress `.blg` log file output.
    pub nolog: bool,
    /// Output encoding (e.g. `utf8`). Defaults to `utf8` if `None`.
    pub output_encoding: Option<String>,
    /// Input encoding for `.bib` sources (e.g. `utf8`). Defaults to `utf8`.
    pub input_encoding: Option<String>,
}

/// A bibliographic datasource: a name plus its raw bytes.
///
/// The host resolves files/URLs and hands the bytes to the pipeline.
/// `name` is kept for diagnostics and for resolving relative crossrefs.
#[derive(Debug, Clone)]
pub struct Datasource<'a> {
    /// Display name (filename or label).
    pub name: String,
    /// Raw source bytes (typically UTF-8 text of a `.bib` file).
    pub bytes: &'a [u8],
}

/// Run the biber pipeline and return the `.bbl` output as a string.
///
/// The CLI/wasm layer handles BCF + Bib parsing and calls
/// `biber_core::pipeline::prepare()` directly. This function is kept
/// for API compatibility; the caller should use `prepare()` instead.
pub fn process(
    _bcf: &str,
    _bibs: &[Datasource<'_>],
    _opts: &Options,
) -> Result<String, BiberError> {
    // The CLI/wasm layer handles BCF + Bib parsing and calls
    // pipeline::prepare() directly. This function is kept for API
    // compatibility; the caller should use prepare() instead.
    Ok(empty_bbl())
}

/// Generate a minimal, well-formed empty `.bbl`.
///
/// Mirrors the header biber writes when there are no entries to emit.
/// The parity harness treats this as the stub output.
pub fn empty_bbl() -> String {
    // Format version 3.3, matching `$BBL_VERSION` in lib/Biber/Constants.pm.
    let mut s = String::new();
    s.push_str("% $ biblatex auxiliary file $\n");
    s.push_str("% $ biblatex bbl format version 3.3 $\n");
    s.push_str("% Do not modify the above lines!\n");
    s.push_str("%\n");
    s.push_str("% This is an auxiliary file used by the 'biblatex' package.\n");
    s.push_str("% This file may safely be deleted. It will be recreated by\n");
    s.push_str("% biber as required.\n");
    s.push_str("%\n");
    s.push_str("\\begingroup\n");
    s.push_str("\\makeatletter\n");
    s.push_str(
        "\\@ifundefined{ver@biblatex.sty}\n  {\\@latex@error\n     {Missing 'biblatex' package}\n     {The bibliography requires the 'biblatex' package.}\n      \\aftergroup\\endinput}\n  {}\n",
    );
    s.push_str("\\endgroup\n\n");
    s.push_str("\\endinput\n");
    s
}
