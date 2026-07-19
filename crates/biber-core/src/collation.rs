//! Locale-aware collation helpers using ICU4X.
//!
//! Provides locale resolution and collator creation for sorting
//! bibliography entries according to language-specific rules.

use icu_collator::{
    options::{AlternateHandling, CaseLevel, CollatorOptions, Strength},
    Collator, CollatorBorrowed, CollatorPreferences,
};
use icu_locale_core::Locale;

use crate::config::Config;
use crate::constants::locale_map;

/// Resolve a babel/polyglossia locale name or BCP47 tag to an ICU4X `Locale`.
///
/// Tries the `locale_map` first (babel names → BCP47), then attempts
/// to parse the string directly as a BCP47 locale.
pub fn resolve_locale(name: &str) -> Locale {
    let map = locale_map();
    if let Some(&bcp47) = map.get(name) {
        bcp47.parse().unwrap_or(Locale::UNKNOWN)
    } else {
        name.parse().unwrap_or(Locale::UNKNOWN)
    }
}

/// Create an ICU4X `CollatorBorrowed` from resolved locale and config options.
///
/// Applies `sortcase`, `sortupper`, and `collate_options` from config.
pub fn create_collator(locale: &Locale, config: &Config) -> CollatorBorrowed<'static> {
    let mut options = CollatorOptions::default();

    if let Some(collate_str) = config.getoption_str("collate_options") {
        apply_collate_options(&mut options, collate_str);
    }

    if let Some(sortcase) = config.getoption_str("sortcase") {
        if sortcase == "0" && options.strength.is_none() {
            options.strength = Some(Strength::Secondary);
        }
    }

    Collator::try_new(CollatorPreferences::from(locale), options).unwrap_or_else(|_| {
        Collator::try_new(Default::default(), CollatorOptions::default())
            .expect("ICU4X fallback collator should not fail")
    })
}

fn apply_collate_options(options: &mut CollatorOptions, s: &str) {
    for part in s.split(',') {
        let kv: Vec<&str> = part.splitn(2, '=').collect();
        if kv.len() != 2 {
            continue;
        }
        match kv[0] {
            "level" => {
                options.strength = Some(match kv[1] {
                    "1" => Strength::Primary,
                    "2" => Strength::Secondary,
                    "3" => Strength::Tertiary,
                    "4" => Strength::Quaternary,
                    _ => Strength::Identical,
                });
            }
            "variable" => {
                options.alternate_handling = Some(match kv[1] {
                    "non-ignorable" | "non_ignorable" => AlternateHandling::NonIgnorable,
                    "shifted" => AlternateHandling::Shifted,
                    _ => AlternateHandling::NonIgnorable,
                });
            }
            "case_level" | "caselevel" => {
                if kv[1] == "true" || kv[1] == "1" {
                    options.case_level = Some(CaseLevel::On);
                } else {
                    options.case_level = Some(CaseLevel::Off);
                }
            }
            "normalization" => {}
            _ => {}
        }
    }
}

/// Convert a locale name to an ICU4X `CollatorBorrowed` using config options.
///
/// This is a convenience wrapper for the common case.
pub fn locale_to_collator(locale_str: &str, config: &Config) -> CollatorBorrowed<'static> {
    let locale = resolve_locale(locale_str);
    create_collator(&locale, config)
}
