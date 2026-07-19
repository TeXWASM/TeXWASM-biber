//! Configuration option storage.
//!
//! Ported from `lib/Biber/Config.pm`. Stores biber options (from CLI,
//! config file, and `.bcf`) and biblatex options (from `.bcf` and
//! `biber-tool.conf`). Options are scoped: GLOBAL, ENTRYTYPE, ENTRY,
//! NAMELIST, NAME.
//!
//! Defer `kpsewhich`/`File::Spec` path resolution — inputs come
//! pre-resolved in the WASM port.

use std::collections::{BTreeMap, HashMap};

use crate::constants::{default_biber_options, default_biblatex_options, OptionScope};

/// A configuration value. Mirrors the loosely-typed Perl option values.
#[derive(Debug, Clone)]
pub enum ConfigValue {
    /// A string value.
    Str(String),
    /// A list of values (for multivalued options).
    List(Vec<ConfigValue>),
    /// A nested map (for complex options like sorting templates).
    Map(BTreeMap<String, ConfigValue>),
    /// Raw XML subtree (for options like `inheritance`, `datamodel` that
    /// are stored as opaque XML structures in Perl).
    Raw(String),
}

impl ConfigValue {
    /// Get as a string, if possible.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Get as a list, if possible.
    pub fn as_list(&self) -> Option<&[ConfigValue]> {
        match self {
            Self::List(v) => Some(v),
            _ => None,
        }
    }
}

impl From<&str> for ConfigValue {
    fn from(s: &str) -> Self {
        Self::Str(s.to_string())
    }
}

impl From<String> for ConfigValue {
    fn from(s: String) -> Self {
        Self::Str(s)
    }
}

/// Metadata about a biblatex option (from `<bcf:optionscope>`).
#[derive(Debug, Clone, Default)]
pub struct OptionMeta {
    /// Whether the option is output to the `.bbl` (`backendout` attribute).
    pub output: bool,
    /// Input mapping (`backendin` attribute), if any.
    pub input: Option<String>,
}

/// The main configuration store.
///
/// In Perl, this is the `$CONFIG` package variable in `Biber::Config.pm`,
/// a global mutable singleton. Here it's a plain struct owned by the
/// `Biber` processor.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Biber options: name → value.
    biber_opts: HashMap<String, ConfigValue>,

    /// Biber options that were explicitly set (not defaults).
    explicit_biber_opts: std::collections::HashSet<String>,

    /// Biblatex options at GLOBAL scope: name → value.
    biblatex_global: HashMap<String, ConfigValue>,

    /// Biblatex options per entrytype: entrytype → (name → value).
    biblatex_per_entrytype: HashMap<String, HashMap<String, ConfigValue>>,

    /// Biblatex option metadata from `<bcf:optionscope>`:
    /// scope → (option_name → OptionMeta).
    pub biblatex_option_meta: HashMap<OptionScope, HashMap<String, OptionMeta>>,

    /// Maps option name → set of scopes where it's valid
    /// (`%CONFIG_OPTSCOPE_BIBLATEX`).
    pub optscope: HashMap<String, std::collections::HashSet<OptionScope>>,

    /// Maps scope → set of option names valid at that scope
    /// (`%CONFIG_SCOPEOPT_BIBLATEX`).
    pub scopeopt: HashMap<OptionScope, std::collections::HashSet<String>>,

    /// Maps option name → datatype string (`%CONFIG_OPTTYPE_BIBLATEX`).
    pub opttype: HashMap<String, String>,

    /// Datafield sets (`%DATAFIELD_SETS`): set name → list of fields/types.
    pub datafield_sets: HashMap<String, Vec<DatafieldSetMember>>,

    /// Key order: section → (key → order).
    /// Used for `\citeorder` sorting.
    pub keyorder: HashMap<u32, HashMap<String, u32>>,

    /// Internal key order: section → (key → intorder).
    pub internal_keyorder: HashMap<u32, HashMap<String, u32>>,

    /// Path to the control file.
    pub ctrlfile_path: Option<String>,

    /// Inheritance edges for circular reference detection:
    /// type ("crossref" | "xdata") → list of (source, target) pairs.
    pub inheritance_edges: HashMap<String, Vec<(String, String)>>,
}

/// A member of a datafield set.
#[derive(Debug, Clone)]
pub struct DatafieldSetMember {
    /// Field name (if this is a field reference).
    pub field: Option<String>,
    /// Field type (if this is a type-based member).
    pub fieldtype: Option<String>,
    /// Data type (if this is a type-based member).
    pub datatype: Option<String>,
}

impl Config {
    /// Create a new config, initialised with default biber options.
    pub fn new() -> Self {
        let mut cfg = Self::default();
        for (name, default) in default_biber_options() {
            cfg.biber_opts
                .insert(name.to_string(), ConfigValue::Str(default.as_str()));
        }
        for (name, default) in default_biblatex_options() {
            cfg.biblatex_global
                .insert(name.to_string(), ConfigValue::Str(default.as_str()));
        }
        cfg
    }

    // ---- Biber options ----

    /// Get a biber option value.
    pub fn getoption(&self, name: &str) -> Option<&ConfigValue> {
        self.biber_opts.get(name)
    }

    /// Get a biber option as a string.
    pub fn getoption_str(&self, name: &str) -> Option<&str> {
        self.biber_opts.get(name).and_then(|v| v.as_str())
    }

    /// Set a biber option.
    pub fn setoption<S: Into<String>>(&mut self, name: S, value: ConfigValue) {
        self.biber_opts.insert(name.into(), value);
    }

    /// Set a biber option from a string value (convenience).
    pub fn setoption_str<S: Into<String>, V: Into<String>>(&mut self, name: S, value: V) {
        self.biber_opts
            .insert(name.into(), ConfigValue::Str(value.into()));
    }

    /// Mark a biber option as explicitly set (e.g. from CLI or config file).
    pub fn mark_explicit<S: Into<String>>(&mut self, name: S) {
        self.explicit_biber_opts.insert(name.into());
    }

    /// Check if a biber option was explicitly set.
    pub fn isexplicitoption(&self, name: &str) -> bool {
        self.explicit_biber_opts.contains(name)
    }

    // ---- Biblatex options ----

    /// Get a global biblatex option.
    pub fn getblxoption(&self, _secnum: Option<u32>, name: &str) -> Option<&ConfigValue> {
        self.biblatex_global.get(name)
    }

    /// Get a biblatex option as a string.
    pub fn getblxoption_str(&self, name: &str) -> Option<&str> {
        self.biblatex_global.get(name).and_then(|v| v.as_str())
    }

    /// Get a per-entrytype biblatex option.
    pub fn getblxoption_entrytype(&self, entrytype: &str, name: &str) -> Option<&ConfigValue> {
        self.biblatex_per_entrytype
            .get(entrytype)
            .and_then(|m| m.get(name))
    }

    /// Get a biblatex option with the full fallback chain:
    /// entrytype → global.
    ///
    /// This mirrors Perl's `Biber::Config->getblxoption($secnum, $opt, $entrytype)`.
    pub fn getblxoption_for_entry(&self, entrytype: &str, name: &str) -> Option<&ConfigValue> {
        self.biblatex_per_entrytype
            .get(entrytype)
            .and_then(|m| m.get(name))
            .or_else(|| self.biblatex_global.get(name))
    }

    /// Get a biblatex option as a string with the full fallback chain.
    pub fn getblxoption_for_entry_str(&self, entrytype: &str, name: &str) -> Option<&str> {
        self.getblxoption_for_entry(entrytype, name)
            .and_then(|v| v.as_str())
    }

    /// Set a global biblatex option.
    pub fn setblxoption(&mut self, _secnum: Option<u32>, name: &str, value: ConfigValue) {
        self.biblatex_global.insert(name.to_string(), value);
    }

    /// Set a per-entrytype biblatex option.
    pub fn setblxoption_entrytype(
        &mut self,
        _secnum: Option<u32>,
        name: &str,
        value: ConfigValue,
        entrytype: &str,
    ) {
        self.biblatex_per_entrytype
            .entry(entrytype.to_string())
            .or_default()
            .insert(name.to_string(), value);
    }

    // ---- Option scope metadata ----

    /// Record an option scope definition from `<bcf:optionscope>`.
    pub fn add_optionscope(
        &mut self,
        scope: OptionScope,
        opt_name: &str,
        datatype: &str,
        output: bool,
        input: Option<String>,
    ) {
        self.optscope
            .entry(opt_name.to_string())
            .or_default()
            .insert(scope);
        self.scopeopt
            .entry(scope)
            .or_default()
            .insert(opt_name.to_string());
        self.opttype
            .insert(opt_name.to_string(), datatype.to_lowercase());
        self.biblatex_option_meta
            .entry(scope)
            .or_default()
            .insert(opt_name.to_string(), OptionMeta { output, input });
    }

    // ---- Logging / diagnostics helpers ----

    /// Returns `true` if `--trace` is active.
    pub fn is_trace(&self) -> bool {
        self.getoption_str("trace") == Some("1")
    }

    /// Returns `true` if `--debug` is active (or `--trace`, which implies debug).
    pub fn is_debug(&self) -> bool {
        self.getoption_str("debug") == Some("1") || self.is_trace()
    }

    /// Returns `true` if `--quiet` is active.
    pub fn is_quiet(&self) -> bool {
        self.getoption_str("quiet") == Some("1")
    }

    /// Returns `true` if `--nolog` is active (suppress .blg output).
    pub fn is_nolog(&self) -> bool {
        self.getoption_str("nolog") == Some("1")
    }

    /// Returns the `--logfile` value, if set.
    pub fn logfile_name(&self) -> Option<&str> {
        self.getoption_str("logfile")
    }

    /// Returns the `--output-directory` value, if set.
    pub fn output_directory(&self) -> Option<&str> {
        self.getoption_str("output-directory")
    }

    // ---- Key ordering ----

    /// Set the citation order of a key in a section.
    pub fn set_keyorder(&mut self, secnum: u32, key: &str, order: u32) {
        self.keyorder
            .entry(secnum)
            .or_default()
            .insert(key.to_string(), order);
    }

    /// Set the internal citation order of a key in a section.
    pub fn set_internal_keyorder(&mut self, secnum: u32, key: &str, order: u32) {
        self.internal_keyorder
            .entry(secnum)
            .or_default()
            .insert(key.to_string(), order);
    }

    /// Get the citation order of a key in a section.
    pub fn get_keyorder(&self, secnum: u32, key: &str) -> Option<u32> {
        self.keyorder.get(&secnum).and_then(|m| m.get(key)).copied()
    }

    // ---- Datafield sets ----

    /// Add a member to a datafield set.
    pub fn add_datafield_set_member(&mut self, set_name: &str, member: DatafieldSetMember) {
        self.datafield_sets
            .entry(set_name.to_lowercase())
            .or_default()
            .push(member);
    }

    // ---- Control file path ----

    /// Set the control file path.
    pub fn set_ctrlfile_path<S: Into<String>>(&mut self, path: S) {
        self.ctrlfile_path = Some(path.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_populated() {
        let cfg = Config::new();
        assert_eq!(cfg.getoption_str("input_encoding"), Some("UTF-8"));
        assert_eq!(cfg.getoption_str("output_format"), Some("bbl"));
        assert_eq!(cfg.getblxoption_str("sortingtemplatename"), Some("tool"));
    }

    #[test]
    fn option_set_get() {
        let mut cfg = Config::new();
        cfg.setoption_str("output_format", "bbl");
        assert_eq!(cfg.getoption_str("output_format"), Some("bbl"));
    }

    #[test]
    fn explicit_option_tracking() {
        let mut cfg = Config::new();
        assert!(!cfg.isexplicitoption("debug"));
        cfg.mark_explicit("debug");
        assert!(cfg.isexplicitoption("debug"));
    }

    #[test]
    fn per_entrytype_options() {
        let mut cfg = Config::new();
        cfg.setblxoption_entrytype(None, "maxcitenames", "5".into(), "article");
        assert_eq!(
            cfg.getblxoption_entrytype("article", "maxcitenames")
                .and_then(|v| v.as_str()),
            Some("5")
        );
    }

    #[test]
    fn optionscope_tracking() {
        let mut cfg = Config::new();
        cfg.add_optionscope(
            OptionScope::Global,
            "sortingtemplatename",
            "string",
            false,
            None,
        );
        assert!(cfg
            .optscope
            .get("sortingtemplatename")
            .unwrap()
            .contains(&OptionScope::Global));
    }
}
