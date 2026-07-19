//! Vendored static data files.
//!
//! These are `include_str!`'d at compile time from the repo's `data/` and
//! `lib/Biber/LaTeX/` directories so the crate is self-contained. Mirrors
//! the `data_files` copy step in `Build.PL` for the Perl distribution.

/// `data/biber-tool.conf` — global config defaults, datamodel, label/sort
/// templates. ~1756 lines of XML.
pub const BIBER_TOOL_CONF: &str = include_str!("../../../data/biber-tool.conf");
/// `data/recode_data.xml` — LaTeX ↔ Unicode recode tables.
/// ~62 KB; used by `latex_recode`.
pub const RECODE_DATA_XML: &str = include_str!("../../../data/recode_data.xml");

/// `data/schemata/bcf.rnc` — RelaxNG compact schema for the `.bcf` format.
pub const BCF_RNC: &str = include_str!("../../../data/schemata/bcf.rnc");

/// `data/schemata/config.rnc` — RelaxNG compact schema for `biber.conf`.
pub const CONFIG_RNC: &str = include_str!("../../../data/schemata/config.rnc");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vendored_files_are_nonempty() {
        assert!(!BIBER_TOOL_CONF.is_empty());
        assert!(BIBER_TOOL_CONF.contains("<config>"));
        assert!(!RECODE_DATA_XML.is_empty());
        assert!(RECODE_DATA_XML.contains("<texmap>"));
        assert!(BCF_RNC.starts_with("namespace bcf"));
        assert!(CONFIG_RNC.starts_with("start ="));
    }
}
