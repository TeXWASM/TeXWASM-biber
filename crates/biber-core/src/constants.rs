//! Version strings and configuration constants.
//!
//! Ported from `lib/Biber/Constants.pm`. Only the data needed by the BCF
//! reader and config initialisation is included here; processing-pass
//! constants will be added as the pipeline matures.

use std::collections::HashMap;

/// Version of the biblatex control file (`$BCF_VERSION`).
pub const BCF_VERSION: &str = "3.11";

/// Format version of the `.bbl` (`$BBL_VERSION`).
pub const BBL_VERSION: &str = "3.3";

/// BibTeX month macros (`%MONTHS`).
pub fn months() -> HashMap<&'static str, &'static str> {
    [
        ("jan", "1"),
        ("feb", "2"),
        ("mar", "3"),
        ("apr", "4"),
        ("may", "5"),
        ("jun", "6"),
        ("jul", "7"),
        ("aug", "8"),
        ("sep", "9"),
        ("oct", "10"),
        ("nov", "11"),
        ("dec", "12"),
    ]
    .into_iter()
    .collect()
}

/// ISO 8601-2 year divisions (`%YEARDIVISIONS`).
pub fn year_divisions() -> HashMap<u32, &'static str> {
    [
        (21u32, "spring"),
        (22, "summer"),
        (23, "autumn"),
        (24, "winter"),
        (25, "springN"),
        (26, "summerN"),
        (27, "autumnN"),
        (28, "winterN"),
        (29, "springS"),
        (30, "summerS"),
        (31, "autumnS"),
        (32, "winterS"),
        (33, "Q1"),
        (34, "Q2"),
        (35, "Q3"),
        (36, "Q4"),
        (37, "QD1"),
        (38, "QD2"),
        (39, "QD3"),
        (40, "S1"),
        (41, "S2"),
    ]
    .into_iter()
    .collect()
}

/// Option scope types. Mirrors the `<bcf:optionscope type="...">` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptionScope {
    /// Global scope.
    Global,
    /// Per-entrytype scope.
    Entrytype,
    /// Per-entry scope.
    Entry,
    /// Per-namelist scope.
    Namelist,
    /// Per-name scope.
    Name,
}

impl OptionScope {
    /// Parse from a BCF attribute string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_bcf_str(s: &str) -> Option<Self> {
        match s {
            "GLOBAL" => Some(Self::Global),
            "ENTRYTYPE" => Some(Self::Entrytype),
            "ENTRY" => Some(Self::Entry),
            "NAMELIST" => Some(Self::Namelist),
            "NAME" => Some(Self::Name),
            _ => None,
        }
    }
}

/// Uniquename value mapping (`%UNIQUENAME_VALUES`).
pub fn uniquename_values() -> HashMap<&'static str, u32> {
    [("none", 0u32), ("init", 1), ("full", 2)]
        .into_iter()
        .collect()
}

/// Datasource extension mapping (`%DS_EXTENSIONS`).
pub fn ds_extensions() -> HashMap<&'static str, &'static str> {
    [("bibtex", "bib"), ("biblatexml", "bltxml")]
        .into_iter()
        .collect()
}

/// Default biber options (`$CONFIG_DEFAULT_BIBER`).
///
/// Returns a map of option name → default value (as a string).
/// Only the options that the BCF reader and pipeline need for non-tool
/// mode are included.
pub fn default_biber_options() -> HashMap<&'static str, BiberOptionDefault> {
    let mut m = HashMap::new();
    m.insert("input_encoding", BiberOptionDefault::Str("UTF-8"));
    m.insert("input_format", BiberOptionDefault::Str("bibtex"));
    m.insert("output_encoding", BiberOptionDefault::Str("UTF-8"));
    m.insert("output_format", BiberOptionDefault::Str("bbl"));
    m.insert("output_fieldcase", BiberOptionDefault::Str("upper"));
    m.insert(
        "output_field_order",
        BiberOptionDefault::Str("options,abstract,names,lists,dates"),
    );
    m.insert("output_indent", BiberOptionDefault::Str("2"));
    m.insert("output_listsep", BiberOptionDefault::Str("and"));
    m.insert("output_namesep", BiberOptionDefault::Str("and"));
    m.insert("output_annotation_marker", BiberOptionDefault::Str("+an"));
    m.insert(
        "output_named_annotation_marker",
        BiberOptionDefault::Str(":"),
    );
    m.insert("output_xdatamarker", BiberOptionDefault::Str("xdata"));
    m.insert("output_xdatasep", BiberOptionDefault::Str("-"));
    m.insert("output_xnamesep", BiberOptionDefault::Str("="));
    m.insert("annotation_marker", BiberOptionDefault::Str("+an"));
    m.insert("named_annotation_marker", BiberOptionDefault::Str(":"));
    m.insert("xdatamarker", BiberOptionDefault::Str("xdata"));
    m.insert("xdatasep", BiberOptionDefault::Str("-"));
    m.insert("xnamesep", BiberOptionDefault::Str("="));
    m.insert("xsvsep", BiberOptionDefault::Str(r"\s*,\s*"));
    m.insert("listsep", BiberOptionDefault::Str("and"));
    m.insert("namesep", BiberOptionDefault::Str("and"));
    m.insert("others_string", BiberOptionDefault::Str("others"));
    m.insert("decodecharsset", BiberOptionDefault::Str("base"));
    m.insert("output_safechars", BiberOptionDefault::Bool(false));
    m.insert("output_safecharsset", BiberOptionDefault::Str("base"));
    m.insert("mincrossrefs", BiberOptionDefault::Str("2"));
    m.insert("minxrefs", BiberOptionDefault::Str("2"));
    m.insert(
        "collate_options",
        BiberOptionDefault::Str("level=4,variable=non-ignorable,normalization=prenormalized"),
    );
    m.insert("debug", BiberOptionDefault::Bool(false));
    m.insert("trace", BiberOptionDefault::Bool(false));
    m.insert("nolog", BiberOptionDefault::Bool(false));
    m.insert("quiet", BiberOptionDefault::Bool(false));
    m.insert("sortcase", BiberOptionDefault::Bool(true));
    m.insert("sortupper", BiberOptionDefault::Bool(true));
    m.insert("tool", BiberOptionDefault::Bool(false));
    m.insert("clrmacros", BiberOptionDefault::Bool(false));
    m.insert("glob_datasources", BiberOptionDefault::Bool(false));
    m.insert("nostdmacros", BiberOptionDefault::Bool(false));
    m.insert("strip_comments", BiberOptionDefault::Bool(false));
    m.insert("wraplines", BiberOptionDefault::Str("0"));
    m.insert("dieondatamodel", BiberOptionDefault::Bool(false));
    m.insert("nodieonerror", BiberOptionDefault::Bool(false));
    m.insert("noskipduplicates", BiberOptionDefault::Bool(false));
    m.insert("no_default_datamodel", BiberOptionDefault::Bool(false));
    m.insert("validate_datamodel", BiberOptionDefault::Bool(false));
    m.insert("validate_control", BiberOptionDefault::Bool(false));
    m.insert("validate_config", BiberOptionDefault::Bool(false));
    m.insert("validate_bblxml", BiberOptionDefault::Bool(false));
    m.insert("validate_bltxml", BiberOptionDefault::Bool(false));
    m.insert("no_bblxml_schema", BiberOptionDefault::Bool(false));
    m.insert("no_bltxml_schema", BiberOptionDefault::Bool(false));
    m
}

/// Default biblatex options (`%CONFIG_DEFAULT_BIBLATEX`).
pub fn default_biblatex_options() -> HashMap<&'static str, BiberOptionDefault> {
    let mut m = HashMap::new();
    m.insert("sortingtemplatename", BiberOptionDefault::Str("tool"));
    m.insert("useauthor", BiberOptionDefault::Bool(true));
    m.insert("useeditor", BiberOptionDefault::Bool(true));
    m.insert("usetranslator", BiberOptionDefault::Bool(true));
    m.insert("maxbibnames", BiberOptionDefault::Str("100"));
    m.insert("maxitems", BiberOptionDefault::Str("100"));
    m.insert("minbibnames", BiberOptionDefault::Str("100"));
    m.insert("maxalphanames", BiberOptionDefault::Str("100"));
    m.insert("maxcitenames", BiberOptionDefault::Str("100"));
    m.insert("maxsortnames", BiberOptionDefault::Str("100"));
    m.insert("minalphanames", BiberOptionDefault::Str("100"));
    m.insert("mincitenames", BiberOptionDefault::Str("100"));
    m.insert("minsortnames", BiberOptionDefault::Str("100"));
    m.insert("minitems", BiberOptionDefault::Str("100"));
    m.insert("useprefix", BiberOptionDefault::Bool(false));
    m.insert("labeldateparts", BiberOptionDefault::Bool(true));
    m.insert("labelalpha", BiberOptionDefault::Bool(true));
    m.insert("singletitle", BiberOptionDefault::Bool(false));
    m.insert("uniquetitle", BiberOptionDefault::Bool(false));
    m.insert("uniquebaretitle", BiberOptionDefault::Bool(false));
    m.insert("uniquework", BiberOptionDefault::Bool(false));
    m.insert("uniqueprimaryauthor", BiberOptionDefault::Bool(false));
    m
}

/// A simplified representation of a biber/biblatex option default.
#[derive(Debug, Clone)]
pub enum BiberOptionDefault {
    /// String-valued option.
    Str(&'static str),
    /// Boolean option.
    Bool(bool),
}

impl BiberOptionDefault {
    /// Get the value as a string.
    pub fn as_str(&self) -> String {
        match self {
            Self::Str(s) => (*s).to_string(),
            Self::Bool(true) => "1".to_string(),
            Self::Bool(false) => "0".to_string(),
        }
    }
}

/// Maps babel/polyglossia language names to BCP47 locale tags (`%LOCALE_MAP`).
pub fn locale_map() -> HashMap<&'static str, &'static str> {
    [
        ("american", "en-US"),
        ("british", "en-GB"),
        ("english", "en-US"),
        ("USenglish", "en-US"),
        ("UKenglish", "en-UK"),
        ("german", "de-DE"),
        ("ngerman", "de-DE"),
        ("austrian", "de-AT"),
        ("naustrian", "de-AT"),
        ("french", "fr-FR"),
        ("francais", "fr-FR"),
        ("spanish", "es-ES"),
        ("italian", "it-IT"),
        ("portuguese", "pt-PT"),
        ("brazilian", "pt-BR"),
        ("russian", "ru-RU"),
        ("japanese", "ja-JP"),
        ("chinese", "zh-CN"),
        ("dutch", "nl-NL"),
        ("swedish", "sv-SE"),
        ("finnish", "fi-FI"),
        ("polish", "pl-PL"),
        ("czech", "cs-CZ"),
        ("slovak", "sk-SK"),
        ("hungarian", "hu-HU"),
        ("romanian", "ro-RO"),
        ("bulgarian", "bg-BG"),
        ("serbian", "sr-Latn"),
        ("croatian", "hr-HR"),
        ("slovenian", "sl-SI"),
        ("danish", "da-DK"),
        ("norwegian", "nn-NO"),
        ("norsk", "nb-NO"),
        ("nynorsk", "nn-NO"),
        ("greek", "el-GR"),
        ("turkish", "tr-TR"),
        ("ukrainian", "uk-UA"),
        ("thai", "th-TH"),
        ("vietnamese", "vi-VN"),
        ("arabic", "ar-001"),
        ("hebrew", "he-IL"),
        ("hindi", "hi-IN"),
        ("bengali", "bn-BD"),
        ("persian", "fa-IR"),
        ("korean", "ko-KR"),
        ("lithuanian", "lt-LT"),
        ("latvian", "lv-LV"),
        ("estonian", "et-EE"),
        ("irish", "ga-IE"),
        ("galician", "gl-ES"),
        ("basque", "eu-ES"),
        ("catalan", "ca-AD"),
        ("welsh", "cy-GB"),
        ("icelandic", "is-IS"),
        ("albanian", "sq-AL"),
        ("belarusian", "be-BY"),
        ("macedonian", "mk-MK"),
        ("latin", "la-Latn"),
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_set() {
        assert_eq!(BCF_VERSION, "3.11");
        assert_eq!(BBL_VERSION, "3.3");
    }

    #[test]
    fn months_have_12_entries() {
        assert_eq!(months().len(), 12);
        assert_eq!(months().get("jan"), Some(&"1"));
        assert_eq!(months().get("dec"), Some(&"12"));
    }

    #[test]
    fn option_scope_parses() {
        assert_eq!(
            OptionScope::from_bcf_str("GLOBAL"),
            Some(OptionScope::Global)
        );
        assert_eq!(
            OptionScope::from_bcf_str("ENTRYTYPE"),
            Some(OptionScope::Entrytype)
        );
        assert_eq!(OptionScope::from_bcf_str("ENTRY"), Some(OptionScope::Entry));
        assert_eq!(
            OptionScope::from_bcf_str("NAMELIST"),
            Some(OptionScope::Namelist)
        );
        assert_eq!(OptionScope::from_bcf_str("NAME"), Some(OptionScope::Name));
        assert_eq!(OptionScope::from_bcf_str("UNKNOWN"), None);
    }

    #[test]
    fn default_options_have_entries() {
        let biber = default_biber_options();
        assert_eq!(biber.get("input_encoding").unwrap().as_str(), "UTF-8");
        assert_eq!(biber.get("output_format").unwrap().as_str(), "bbl");

        let biblatex = default_biblatex_options();
        assert_eq!(
            biblatex.get("sortingtemplatename").unwrap().as_str(),
            "tool"
        );
        assert_eq!(biblatex.get("maxcitenames").unwrap().as_str(), "100");
    }

    #[test]
    fn locale_map_covers_common_locales() {
        let lm = locale_map();
        assert_eq!(lm.get("english"), Some(&"en-US"));
        assert_eq!(lm.get("german"), Some(&"de-DE"));
        assert_eq!(lm.get("japanese"), Some(&"ja-JP"));
    }
}
