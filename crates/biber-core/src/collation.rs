//! Locale-aware collation helpers using ICU4X.
//!
//! Provides locale resolution and collator creation for sorting
//! bibliography entries according to language-specific rules.

use icu_collator::{AlternateHandling, CaseLevel, Collator, CollatorOptions, Strength};
use icu_locid::Locale;
use icu_provider::DataLocale;

use crate::config::Config;
use crate::constants::locale_map;

/// Resolve a babel/polyglossia locale name or BCP47 tag to an ICU4X `Locale`.
///
/// Tries the `locale_map` first (babel names → BCP47), then attempts
/// to parse the string directly as a BCP47 locale.
pub fn resolve_locale(name: &str) -> Locale {
    let map = locale_map();
    if let Some(&bcp47) = map.get(name) {
        bcp47.parse().unwrap_or(Locale::UND)
    } else {
        name.parse().unwrap_or(Locale::UND)
    }
}

/// Create an ICU4X `Collator` from resolved locale and config options.
///
/// Applies `sortcase`, `sortupper`, and `collate_options` from config.
pub fn create_collator(locale: &Locale, config: &Config) -> Collator {
    let mut options = CollatorOptions::new();

    // Parse collate_options (default: "level=4,variable=non-ignorable,...")
    if let Some(collate_str) = config.getoption_str("collate_options") {
        apply_collate_options(&mut options, collate_str);
    }

    // Apply sortcase: when sortcase=0 (case-insensitive), ensure strength
    // is at most Secondary (ignores case differences)
    if let Some(sortcase) = config.getoption_str("sortcase") {
        if sortcase == "0" && options.strength.is_none() {
            options.strength = Some(Strength::Secondary);
        }
    }

    let data_locale: DataLocale = locale.into();
    Collator::try_new(&data_locale, options).unwrap_or_else(|_| {
        let root: DataLocale = Locale::UND.into();
        Collator::try_new(&root, CollatorOptions::new())
            .expect("ICU4X fallback collator should not fail")
    })
}

/// Parse a `collate_options` string and apply to `CollatorOptions`.
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
            "normalization" => {
                // ICU4X handles normalization internally; this is a hint.
            }
            _ => {}
        }
    }
}

/// Convert a locale name to an ICU4X `Collator` using config options.
///
/// This is a convenience wrapper for the common case.
pub fn locale_to_collator(locale_str: &str, config: &Config) -> Collator {
    let locale = resolve_locale(locale_str);
    create_collator(&locale, config)
}
