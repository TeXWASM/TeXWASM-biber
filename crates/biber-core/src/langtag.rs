//! BCP47 language tag parser.

/// A parsed BCP47 language tag.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LangTag {
    /// Language subtag (2-3 or 4 or 5-8 alpha).
    pub language: Option<String>,
    /// Extended language subtags (up to 3 × 3 alpha, separated by `-`).
    pub extlang: Vec<String>,
    /// Script subtag (4 alpha, e.g. "Hans", "Latn").
    pub script: Option<String>,
    /// Region subtag (2 alpha or 3 digit, e.g. "US", "419").
    pub region: Option<String>,
    /// Variant subtags (5-8 alphanum, e.g. "1996", "fonupa").
    pub variant: Vec<String>,
    /// Extension subtags (singleton + 2-8 alphanum, repeated).
    pub extension: Vec<(String, Vec<String>)>,
    /// Private use subtag (`x-...`).
    pub privateuse: Vec<String>,
    /// Grandfathered tag (irregular or regular).
    pub grandfathered: Option<String>,
}

impl LangTag {
    /// Parse a BCP47 language tag string.
    ///
    /// Returns `None` if the tag is not a valid BCP47 tag.
    pub fn parse(tag: &str) -> Option<Self> {
        let tag = tag.trim();
        if tag.is_empty() {
            return None;
        }

        // Check grandfathered tags first
        if let Some(gf) = check_grandfathered(tag) {
            return Some(Self {
                grandfathered: Some(gf.to_string()),
                ..Default::default()
            });
        }

        // Check private use only (starts with "x-")
        if tag.starts_with("x-") || tag == "x" {
            // The "x" is the singleton; remaining parts are the values
            let parts: Vec<&str> = tag.split('-').collect();
            let mut lt = Self::default();
            for (i, part) in parts.iter().enumerate() {
                if i == 0 {
                    // Skip "x" — it's the privateuse singleton
                } else {
                    lt.privateuse.push(part.to_string());
                }
            }
            return Some(lt);
        }

        // Parse as a normal langtag
        Self::parse_langtag(tag)
    }

    fn parse_langtag(tag: &str) -> Option<Self> {
        let parts: Vec<&str> = tag.split('-').collect();
        if parts.is_empty() {
            return None;
        }

        let mut lt = Self::default();
        let mut idx = 0;

        // language (2-3 or 4 or 5-8 alpha)
        let lang = parts[0];
        if !is_alpha(lang) || !(2..=8).contains(&lang.len()) {
            return None;
        }
        lt.language = Some(lang.to_string());
        idx += 1;

        // extlang (up to 3 × 3-alpha, each preceded by '-')
        while idx < parts.len() && parts[idx].len() == 3 && is_alpha(parts[idx]) {
            lt.extlang.push(parts[idx].to_string());
            idx += 1;
        }

        // script (4 alpha)
        if idx < parts.len() && parts[idx].len() == 4 && is_alpha(parts[idx]) {
            lt.script = Some(parts[idx].to_string());
            idx += 1;
        }

        // region (2 alpha or 3 digit)
        if idx < parts.len()
            && ((parts[idx].len() == 2 && is_alpha(parts[idx]))
                || (parts[idx].len() == 3 && is_digit(parts[idx])))
        {
            lt.region = Some(parts[idx].to_string());
            idx += 1;
        }

        // variant (5-8 alphanum, or 4-char starting with digit)
        while idx < parts.len() && is_variant(parts[idx]) {
            lt.variant.push(parts[idx].to_string());
            idx += 1;
        }

        // extension (singleton + 2-8 alphanum, repeated)
        while idx < parts.len() && is_singleton(parts[idx]) {
            let singleton = parts[idx].to_string();
            idx += 1;
            let mut ext_values: Vec<String> = Vec::new();
            while idx < parts.len() && is_alphanum(parts[idx]) && parts[idx].len() >= 2 {
                ext_values.push(parts[idx].to_string());
                idx += 1;
            }
            if !ext_values.is_empty() {
                lt.extension.push((singleton, ext_values));
            }
        }

        // private use (x-...)
        if idx < parts.len() && parts[idx].eq_ignore_ascii_case("x") {
            idx += 1;
            while idx < parts.len() && is_alphanum(parts[idx]) {
                lt.privateuse.push(parts[idx].to_string());
                idx += 1;
            }
        }

        // Should have consumed all parts
        if idx != parts.len() {
            return None;
        }

        Some(lt)
    }

    /// Get the primary language subtag.
    pub fn language(&self) -> Option<&str> {
        self.language.as_deref()
    }

    /// Get the script subtag.
    pub fn script(&self) -> Option<&str> {
        self.script.as_deref()
    }

    /// Get the region subtag.
    pub fn region(&self) -> Option<&str> {
        self.region.as_deref()
    }

    /// Serialize back to a canonical string.
    pub fn to_canonical(&self) -> String {
        if let Some(ref gf) = self.grandfathered {
            return gf.clone();
        }

        let mut parts: Vec<String> = Vec::new();

        if let Some(ref lang) = self.language {
            parts.push(lang.to_lowercase());
        }
        for ext in &self.extlang {
            parts.push(ext.to_lowercase());
        }
        if let Some(ref script) = self.script {
            // Capitalize first letter
            let mut s = script.to_lowercase();
            if let Some(c) = s.get_mut(..1) {
                c.make_ascii_uppercase();
            }
            parts.push(s);
        }
        if let Some(ref region) = self.region {
            parts.push(region.to_uppercase());
        }
        for v in &self.variant {
            parts.push(v.to_lowercase());
        }
        for (singleton, values) in &self.extension {
            parts.push(singleton.to_lowercase());
            for v in values {
                parts.push(v.to_lowercase());
            }
        }
        if !self.privateuse.is_empty() {
            parts.push("x".to_string());
            for v in &self.privateuse {
                parts.push(v.to_lowercase());
            }
        }

        parts.join("-")
    }

    /// Convert to a BCP47 string suitable for biblatex (lowercase language,
    /// capitalized script, uppercase region).
    pub fn to_biblatex_string(&self) -> String {
        self.to_canonical()
    }
}

/// Check if a string is all alphabetic characters.
fn is_alpha(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphabetic())
}

/// Check if a string is all digits.
fn is_digit(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

/// Check if a string is alphanumeric.
fn is_alphanum(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric())
}

/// Check if a string is a valid variant subtag.
fn is_variant(s: &str) -> bool {
    let len = s.len();
    ((5..=8).contains(&len) && is_alphanum(s))
        || (len == 4 && s.starts_with(|c: char| c.is_ascii_digit()) && is_alphanum(s))
}

/// Check if a string is a valid singleton (for extensions).
fn is_singleton(s: &str) -> bool {
    s.len() == 1
        && (s.chars().all(|c| c.is_ascii_digit())
            || s.chars()
                .all(|c| c.is_ascii_alphabetic() && c != 'x' && c != 'X'))
}

/// Check if a tag is grandfathered (irregular or regular).
fn check_grandfathered(tag: &str) -> Option<&'static str> {
    let irregular = [
        "en-GB-oed",
        "i-ami",
        "i-bnn",
        "i-default",
        "i-enochian",
        "i-hak",
        "i-klingon",
        "i-lux",
        "i-mingo",
        "i-navajo",
        "i-pwn",
        "i-tao",
        "i-tay",
        "i-tsu",
        "sgn-BE-FR",
        "sgn-BE-NL",
        "sgn-CH-DE",
    ];
    let regular = [
        "art-lojban",
        "cel-gaulish",
        "no-bok",
        "no-nyn",
        "zh-guoyu",
        "zh-hakka",
        "zh-min",
        "zh-min-nan",
        "zh-xiang",
    ];

    irregular
        .iter()
        .chain(regular.iter())
        .find(|&&gf| gf.eq_ignore_ascii_case(tag))
        .copied()
}

/// Parse a BCP47 tag and return a `LangTag`, or `None` if invalid.
pub fn parse_langtag(tag: &str) -> Option<LangTag> {
    LangTag::parse(tag)
}

/// Map a babel/polyglossia language name to a BCP47 locale tag.
///
/// Uses the `%LOCALE_MAP` from Constants.pm.
pub fn language_to_bcp47(lang: &str) -> Option<String> {
    let map = crate::constants::locale_map();
    map.get(lang).map(|s| (*s).to_string())
}

/// Map a BCP47 locale tag to a babel/polyglossia language name.
///
/// Uses the reverse of `%LOCALE_MAP`.
pub fn bcp47_to_language(locale: &str) -> Option<String> {
    let map = crate::constants::locale_map();
    // Reverse lookup (first match wins, like the Perl `%LOCALE_MAP_R`)
    for (name, tag) in &map {
        if tag.eq_ignore_ascii_case(locale) {
            return Some((*name).to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_language() {
        let lt = parse_langtag("en").unwrap();
        assert_eq!(lt.language, Some("en".to_string()));
        assert!(lt.script.is_none());
        assert!(lt.region.is_none());
    }

    #[test]
    fn parse_language_region() {
        let lt = parse_langtag("en-US").unwrap();
        assert_eq!(lt.language, Some("en".to_string()));
        assert_eq!(lt.region, Some("US".to_string()));
    }

    #[test]
    fn parse_language_script_region() {
        let lt = parse_langtag("zh-Hans-CN").unwrap();
        assert_eq!(lt.language, Some("zh".to_string()));
        assert_eq!(lt.script, Some("Hans".to_string()));
        assert_eq!(lt.region, Some("CN".to_string()));
    }

    #[test]
    fn parse_with_variant() {
        let lt = parse_langtag("de-DE-1996").unwrap();
        assert_eq!(lt.language, Some("de".to_string()));
        assert_eq!(lt.region, Some("DE".to_string()));
        assert_eq!(lt.variant, vec!["1996".to_string()]);
    }

    #[test]
    fn parse_with_extlang() {
        let lt = parse_langtag("zh-cmn-Hans-CN").unwrap();
        assert_eq!(lt.language, Some("zh".to_string()));
        assert_eq!(lt.extlang, vec!["cmn".to_string()]);
        assert_eq!(lt.script, Some("Hans".to_string()));
        assert_eq!(lt.region, Some("CN".to_string()));
    }

    #[test]
    fn parse_private_use() {
        let lt = parse_langtag("x-private-use").unwrap();
        assert!(lt.language.is_none());
        assert_eq!(
            lt.privateuse,
            vec!["private".to_string(), "use".to_string()]
        );
    }

    #[test]
    fn parse_grandfathered() {
        let lt = parse_langtag("en-GB-oed").unwrap();
        assert_eq!(lt.grandfathered, Some("en-GB-oed".to_string()));
    }

    #[test]
    fn parse_grandfathered_case_insensitive() {
        let lt = parse_langtag("I-KLINGON").unwrap();
        assert_eq!(lt.grandfathered, Some("i-klingon".to_string()));
    }

    #[test]
    fn roundtrip_canonical() {
        let lt = parse_langtag("en-US").unwrap();
        assert_eq!(lt.to_canonical(), "en-US");

        let lt = parse_langtag("zh-Hans-CN").unwrap();
        assert_eq!(lt.to_canonical(), "zh-Hans-CN");
    }

    #[test]
    fn invalid_tag() {
        assert!(parse_langtag("").is_none());
        assert!(parse_langtag("1").is_none());
        assert!(parse_langtag("en-1234-foo-bar-baz-extra-stuff-that-is-too-long").is_none());
    }

    #[test]
    fn language_to_bcp47_map() {
        assert_eq!(language_to_bcp47("english"), Some("en-US".to_string()));
        assert_eq!(language_to_bcp47("german"), Some("de-DE".to_string()));
        assert_eq!(language_to_bcp47("french"), Some("fr-FR".to_string()));
    }

    #[test]
    fn bcp47_to_language_map() {
        // "en-US" maps to both "american" and "english" in the locale map
        let result = bcp47_to_language("en-US");
        assert!(result.is_some());
        // "de-DE" maps to both "german" and "ngerman"
        let result = bcp47_to_language("de-DE");
        assert!(result.is_some());
    }
}
