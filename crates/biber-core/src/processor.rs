//! The main biber processor — holds all state for a single run.
//!
//! Ported from `lib/Biber.pm` (the `Biber` class). Holds the parsed BCF
//! state (sections, datalists, config, datamodel). The processing
//! pipeline is in `biber_core::pipeline`.

use crate::config::Config;
use crate::data_model::DataModel;
use crate::datalist::DataLists;
use crate::section::Sections;

/// The main biber processor.
///
/// In Perl, this is the `Biber` object (`$biber` in `bin/biber`).
/// It owns the configuration, sections, datalists, and the output object.
#[derive(Debug, Clone, Default)]
pub struct Biber {
    /// Configuration (biber + biblatex options).
    pub config: Config,
    /// Bibliography sections.
    pub sections: Sections,
    /// Data lists (sorted/filtered output lists).
    pub datalists: DataLists,
    /// Current section being processed.
    pub current_section: Option<u32>,
    /// Parsed data model (from BCF or biber.conf).
    pub datamodel: DataModel,
}

impl Biber {
    /// Create a new biber processor with default config.
    pub fn new() -> Self {
        Self {
            config: Config::new(),
            ..Default::default()
        }
    }

    /// Set the current section number.
    pub fn set_current_section(&mut self, secnum: u32) {
        self.current_section = Some(secnum);
    }

    /// Get the current section number.
    pub fn get_current_section(&self) -> Option<u32> {
        self.current_section
    }
}
