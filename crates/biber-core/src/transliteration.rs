//! Lingua::Translit transliteration support.
//!
//! Implements the three transliteration pairs supported by the original
//! Perl `Lingua::Translit` module:
//!
//! - IAST → Devanagari (Sanskrit)
//! - Russian Cyrillic → ALA-LC
//! - Russian Cyrillic → BGN/PCGN
//!
//! Also parses `<transliteration>` / `<bcf:transliteration>` XML config
//! blocks into structured rules for use during sort-key generation.

use unicode_normalization::UnicodeNormalization;

/// A single transliteration rule from config/BCF.
#[derive(Debug, Clone)]
pub struct TranslitRule {
    /// Entry type this rule applies to (`"*"` means all).
    pub entrytype: String,
    /// Comma-separated language IDs (None = any).
    pub langids: Option<Vec<String>>,
    /// Target field name (`"*"` = any, or a specific field name).
    pub target: String,
    /// Source script/encoding name (lowercased).
    pub from: String,
    /// Target script/encoding name (lowercased).
    pub to: String,
}

/// Parse `<bcf:transliteration>` or `<transliteration>` XML into rules.
///
/// Handles both the `.bcf` format:
/// ```xml
/// <bcf:transliteration entrytype="*">
///   <bcf:translit langids="sanskrit" target="title" from="iast" to="devanagari"/>
/// </bcf:transliteration>
/// ```
///
/// and the `.conf` format (same structure, no namespace).
pub fn parse_transliteration_xml(xml: &str) -> Vec<TranslitRule> {
    let doc = match roxmltree::Document::parse(xml) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let root = doc.root_element();
    parse_transliteration_element(root)
}

fn parse_transliteration_element(node: roxmltree::Node) -> Vec<TranslitRule> {
    let entrytype = node.attribute("entrytype").unwrap_or("*").to_string();

    let mut rules = Vec::new();
    for child in node.children() {
        if !child.is_element() {
            continue;
        }
        let name = child.tag_name().name();
        if name != "translit" && !name.ends_with(":translit") {
            continue;
        }
        let langids = child.attribute("langids").map(|s| {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        });
        let target = child.attribute("target").unwrap_or("").to_string();
        let from = child.attribute("from").unwrap_or("").to_lowercase();
        let to = child.attribute("to").unwrap_or("").to_lowercase();
        if target.is_empty() || from.is_empty() || to.is_empty() {
            continue;
        }
        rules.push(TranslitRule {
            entrytype: entrytype.clone(),
            langids,
            target,
            from,
            to,
        });
    }
    rules
}

/// Encode a single [`TranslitRule`] as a [`ConfigValue::Map`] so it can be
/// stored in `Config` and later retrieved by the pipeline.
pub fn rule_to_config_value(rule: &TranslitRule) -> crate::config::ConfigValue {
    use std::collections::BTreeMap;
    let mut m = BTreeMap::new();
    m.insert(
        "entrytype".into(),
        crate::config::ConfigValue::Str(rule.entrytype.clone()),
    );
    if let Some(ref langids) = rule.langids {
        m.insert(
            "langids".into(),
            crate::config::ConfigValue::Str(langids.join(",")),
        );
    }
    m.insert(
        "target".into(),
        crate::config::ConfigValue::Str(rule.target.clone()),
    );
    m.insert(
        "from".into(),
        crate::config::ConfigValue::Str(rule.from.clone()),
    );
    m.insert(
        "to".into(),
        crate::config::ConfigValue::Str(rule.to.clone()),
    );
    crate::config::ConfigValue::Map(m)
}

/// Decode a [`ConfigValue`] (from `Config` storage) back into a list of
/// [`TranslitRule`].
pub fn rules_from_config_value(value: &crate::config::ConfigValue) -> Vec<TranslitRule> {
    match value {
        crate::config::ConfigValue::List(list) => list.iter().filter_map(map_to_rule).collect(),
        crate::config::ConfigValue::Map(m) => {
            // Single rule stored as a map
            vec![map_to_rule(&crate::config::ConfigValue::Map(m.clone()))]
                .into_iter()
                .flatten()
                .collect()
        }
        crate::config::ConfigValue::Raw(xml) => parse_transliteration_xml(xml),
        _ => Vec::new(),
    }
}

fn map_to_rule(value: &crate::config::ConfigValue) -> Option<TranslitRule> {
    match value {
        crate::config::ConfigValue::Map(m) => {
            let entrytype = m
                .get("entrytype")
                .and_then(|v| v.as_str())
                .unwrap_or("*")
                .to_string();
            let langids = m.get("langids").and_then(|v| v.as_str()).map(|s| {
                s.split(',')
                    .map(|p| p.trim().to_string())
                    .filter(|p| !p.is_empty())
                    .collect()
            });
            let target = m.get("target")?.as_str()?.to_string();
            let from = m.get("from")?.as_str()?.to_lowercase();
            let to = m.get("to")?.as_str()?.to_lowercase();
            Some(TranslitRule {
                entrytype,
                langids,
                target,
                from,
                to,
            })
        }
        _ => None,
    }
}

/// Apply transliteration rules to a string value during sort-key generation.
///
/// Iterates rules in order and returns the first matching transliteration.
/// If no rule matches, returns the original text unchanged.
pub fn apply(
    rules: &[TranslitRule],
    entrytype: &str,
    langid: Option<&str>,
    target: &str,
    text: &str,
) -> String {
    for rule in rules {
        if rule.entrytype != "*" && rule.entrytype != entrytype {
            continue;
        }
        if let Some(ref langids) = rule.langids {
            match langid {
                Some(lid) => {
                    let matched = langids.iter().any(|l| l.eq_ignore_ascii_case(lid));
                    if !matched {
                        continue;
                    }
                }
                None => continue,
            }
        }
        if rule.target != "*" && rule.target != target {
            continue;
        }
        return transliterate(text, &rule.from, &rule.to);
    }
    text.to_string()
}

/// Perform the actual transliteration for a known from/to pair.
fn transliterate(text: &str, from: &str, to: &str) -> String {
    match (from, to) {
        ("iast", "devanagari") => iast_to_devanagari(text),
        ("russian", "ala-lc") => russian_to_ala_lc(text),
        ("russian", "bgn/pcgn-standard") => russian_to_bgn_pcgn(text),
        _ => text.to_string(),
    }
}

// ---------------------------------------------------------------------------
// IAST → Devanagari
// ---------------------------------------------------------------------------

/// IAST vowel → independent Devanagari.
const IAST_VOWEL_INDEP: &[(&str, &str)] = &[
    ("a", "\u{0905}"),  // अ
    ("ā", "\u{0906}"),  // आ
    ("i", "\u{0907}"),  // इ
    ("ī", "\u{0908}"),  // ई
    ("u", "\u{0909}"),  // उ
    ("ū", "\u{090A}"),  // ऊ
    ("ṛ", "\u{090B}"),  // ऋ
    ("ṝ", "\u{0960}"),  // ॠ
    ("ḷ", "\u{090C}"),  // ऌ
    ("e", "\u{090F}"),  // ए
    ("ai", "\u{0910}"), // ऐ
    ("o", "\u{0913}"),  // ओ
    ("au", "\u{0914}"), // औ
];

/// IAST vowel → dependent Devanagari vowel sign (mātrā).
const IAST_VOWEL_DEP: &[(&str, &str)] = &[
    ("a", ""),          // inherent "a" has no sign
    ("ā", "\u{093E}"),  // ा
    ("i", "\u{093F}"),  // ि
    ("ī", "\u{0940}"),  // ी
    ("u", "\u{0941}"),  // ु
    ("ū", "\u{0942}"),  // ू
    ("ṛ", "\u{0943}"),  // ृ
    ("ṝ", "\u{0944}"),  // ॄ
    ("ḷ", "\u{0962}"),  // ॢ
    ("e", "\u{0947}"),  // े
    ("ai", "\u{0948}"), // ै
    ("o", "\u{094B}"),  // ो
    ("au", "\u{094C}"), // ौ
];

/// IAST consonant → Devanagari (consonant + inherent vowel "a" implied).
const IAST_CONSONANT: &[(&str, &str)] = &[
    ("k", "\u{0915}"),  // क
    ("kh", "\u{0916}"), // ख
    ("g", "\u{0917}"),  // ग
    ("gh", "\u{0918}"), // घ
    ("ṅ", "\u{0919}"),  // ङ
    ("c", "\u{091A}"),  // च
    ("ch", "\u{091B}"), // छ
    ("j", "\u{091C}"),  // ज
    ("jh", "\u{091D}"), // झ
    ("ñ", "\u{091E}"),  // ञ
    ("ṭ", "\u{091F}"),  // ट
    ("ṭh", "\u{0920}"), // ठ
    ("ḍ", "\u{0921}"),  // ड
    ("ḍh", "\u{0922}"), // ढ
    ("ṇ", "\u{0923}"),  // ण
    ("t", "\u{0924}"),  // त
    ("th", "\u{0925}"), // थ
    ("d", "\u{0926}"),  // द
    ("dh", "\u{0927}"), // ध
    ("n", "\u{0928}"),  // न
    ("p", "\u{092A}"),  // प
    ("ph", "\u{092B}"), // फ
    ("b", "\u{092C}"),  // ब
    ("bh", "\u{092D}"), // भ
    ("m", "\u{092E}"),  // म
    ("y", "\u{092F}"),  // य
    ("r", "\u{0930}"),  // र
    ("l", "\u{0932}"),  // ल
    ("v", "\u{0935}"),  // व
    ("ś", "\u{0936}"),  // श
    ("ṣ", "\u{0937}"),  // ष
    ("s", "\u{0938}"),  // स
    ("h", "\u{0939}"),  // ह
];

const HALANT: &str = "\u{094D}"; // ्
const ANUSVARA: &str = "\u{0902}"; // ं
const VISARGA: &str = "\u{0903}"; // ः
const AVAGRAHA: &str = "\u{093D}"; // ऽ

/// Check whether a character is a (single-char) IAST vowel letter.
fn is_iast_vowel(c: char) -> bool {
    matches!(
        c,
        'a' | 'ā'
            | 'i'
            | 'ī'
            | 'u'
            | 'ū'
            | 'ṛ'
            | 'ṝ'
            | 'ḷ'
            | 'e'
            | 'o'
            | 'A'
            | 'Ā'
            | 'I'
            | 'Ī'
            | 'U'
            | 'Ū'
            | 'Ṛ'
            | 'Ṝ'
            | 'Ḷ'
            | 'E'
            | 'O'
    )
}

/// Check if a Devanagari character is a consonant (with implicit inherent a).
fn is_devanagari_consonant(c: char) -> bool {
    matches!(c, '\u{0915}'..='\u{0939}' | '\u{0958}'..='\u{095F}')
}

/// Look up the dependent Devanagari vowel sign for an IAST vowel string.
fn find_dependent(vowel: &str) -> Option<&'static str> {
    for (pat, dev) in IAST_VOWEL_DEP {
        if *pat == vowel {
            if dev.is_empty() {
                return None; // inherent a
            }
            return Some(dev);
        }
    }
    None
}

/// Convert IAST (International Alphabet of Sanskrit Transliteration) text
/// to Devanagari script.
///
/// This implementation handles:
/// - Independent vs dependent vowel forms based on position
/// - Consonant conjuncts (halant insertion)
/// - Anusvāra (ṃ), visarga (ḥ), avagraha (')
/// - NFC normalization of input
fn iast_to_devanagari(text: &str) -> String {
    let nfc: String = text.nfc().collect();
    let chars: Vec<char> = nfc.chars().collect();
    let out_len = chars.len() * 3;
    let mut out = String::with_capacity(out_len);
    let mut i = 0;

    type ConsEntry<'a> = (&'a str, &'a str);
    type VowelEntry<'a> = (&'a str, &'a str, Option<&'a str>);

    let mut cons_map: std::collections::HashMap<char, Vec<ConsEntry>> =
        std::collections::HashMap::new();
    for (pat, dev) in IAST_CONSONANT {
        if let Some(c) = pat.chars().next() {
            cons_map.entry(c).or_default().push((pat, dev));
        }
    }
    for entry in cons_map.values_mut() {
        entry.sort_by_key(|b| std::cmp::Reverse(b.0.len()));
    }

    let mut vowel_map: std::collections::HashMap<char, Vec<VowelEntry>> =
        std::collections::HashMap::new();
    for (pat, dev) in IAST_VOWEL_INDEP {
        let dep = find_dependent(pat);
        if let Some(c) = pat.chars().next() {
            vowel_map.entry(c).or_default().push((pat, dev, dep));
        }
    }
    for entry in vowel_map.values_mut() {
        entry.sort_by_key(|b| std::cmp::Reverse(b.0.len()));
    }

    while i < chars.len() {
        let c = chars[i];

        // ---- Try to match a consonant from the lookup map ----
        if let Some(entries) = cons_map.get(&c) {
            let mut matched = false;
            for (pat, dev) in entries {
                let pat_chars: Vec<char> = pat.chars().collect();
                if pat_chars.len() <= chars.len() - i {
                    let slice: Vec<char> = chars[i..i + pat_chars.len()].to_vec();
                    if slice == pat_chars {
                        // Check what follows
                        let next_i = i + pat_chars.len();
                        if next_i < chars.len() {
                            let next_c = chars[next_i];
                            if cons_map.contains_key(&next_c) {
                                // Next is a consonant -> halant
                                out.push_str(dev);
                                out.push_str(HALANT);
                            } else if is_iast_vowel(next_c) {
                                // Next is a vowel -> the vowel sign will follow
                                out.push_str(dev);
                            } else if next_c == 'ṃ' || next_c == 'ṁ' || next_c == 'ḥ' {
                                out.push_str(dev);
                            } else {
                                // Non-IAST character: consonant gets halant
                                out.push_str(dev);
                                out.push_str(HALANT);
                            }
                        } else {
                            // End of word: consonant gets halant (no vowel)
                            out.push_str(dev);
                            out.push_str(HALANT);
                        }
                        i = next_i;
                        matched = true;
                        break;
                    }
                }
            }
            if matched {
                continue;
            }
        }

        // ---- Try to match a vowel ----
        if let Some(entries) = vowel_map.get(&c) {
            let mut matched = false;
            for (pat, indep, dep) in entries {
                let pat_chars: Vec<char> = pat.chars().collect();
                if pat_chars.len() <= chars.len() - i {
                    let slice: Vec<char> = chars[i..i + pat_chars.len()].to_vec();
                    if slice == pat_chars {
                        // Determine if previous output was a Devanagari consonant
                        let prev_is_cons = out
                            .chars()
                            .last()
                            .map(is_devanagari_consonant)
                            .unwrap_or(false);
                        if prev_is_cons && *pat != "a" {
                            // Dependent form
                            if let Some(d) = dep {
                                out.push_str(d);
                            }
                            // "a" dependent is empty (inherent) - nothing to add
                        } else if *pat == "a" && prev_is_cons {
                            // Inherent 'a' after consonant: do nothing
                        } else {
                            // Independent form
                            out.push_str(indep);
                        }
                        i += pat_chars.len();
                        matched = true;
                        break;
                    }
                }
            }
            if matched {
                continue;
            }
        }

        // ---- Anusvāra ----
        if c == 'ṃ' || c == 'ṁ' {
            out.push_str(ANUSVARA);
            i += 1;
            continue;
        }

        // ---- Visarga ----
        if c == 'ḥ' {
            out.push_str(VISARGA);
            i += 1;
            continue;
        }

        // ---- Avagraha ----
        if c == '\'' {
            out.push_str(AVAGRAHA);
            i += 1;
            continue;
        }

        // ---- Any other character (pass through) ----
        out.push(c);
        i += 1;
    }

    out
}

// ---------------------------------------------------------------------------
// Russian Cyrillic → ALA-LC
// ---------------------------------------------------------------------------

const RUSSIAN_ALA_LC: &[(char, &str)] = &[
    ('а', "a"),
    ('б', "b"),
    ('в', "v"),
    ('г', "g"),
    ('д', "d"),
    ('е', "e"),
    ('ё', "ë"),
    ('ж', "zh"),
    ('з', "z"),
    ('и', "i"),
    ('й', "ĭ"),
    ('к', "k"),
    ('л', "l"),
    ('м', "m"),
    ('н', "n"),
    ('о', "o"),
    ('п', "p"),
    ('р', "r"),
    ('с', "s"),
    ('т', "t"),
    ('у', "u"),
    ('ф', "f"),
    ('х', "kh"),
    ('ц', "ts"),
    ('ч', "ch"),
    ('ш', "sh"),
    ('щ', "shch"),
    ('ъ', "ʺ"),
    ('ы', "y"),
    ('ь', "ʹ"),
    ('э', "ė"),
    ('ю', "i͡u"),
    ('я', "i͡a"),
];

const UPPER_ALA_LC: &[(char, &str)] = &[
    ('А', "A"),
    ('Б', "B"),
    ('В', "V"),
    ('Г', "G"),
    ('Д', "D"),
    ('Е', "E"),
    ('Ё', "Ë"),
    ('Ж', "Zh"),
    ('З', "Z"),
    ('И', "I"),
    ('Й', "Ĭ"),
    ('К', "K"),
    ('Л', "L"),
    ('М', "M"),
    ('Н', "N"),
    ('О', "O"),
    ('П', "P"),
    ('Р', "R"),
    ('С', "S"),
    ('Т', "T"),
    ('У', "U"),
    ('Ф', "F"),
    ('Х', "Kh"),
    ('Ц', "Ts"),
    ('Ч', "Ch"),
    ('Ш', "Sh"),
    ('Щ', "Shch"),
    ('Ъ', "ʺ"),
    ('Ы', "Y"),
    ('Ь', "ʹ"),
    ('Э', "Ė"),
    ('Ю', "I͡u"),
    ('Я', "I͡a"),
];

fn russian_to_ala_lc(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for c in text.chars() {
        if let Some(lat) = lookup_ala_lc(c) {
            out.push_str(lat);
        } else {
            out.push(c);
        }
    }
    out
}

fn lookup_ala_lc(c: char) -> Option<&'static str> {
    for (rus, lat) in RUSSIAN_ALA_LC {
        if *rus == c {
            return Some(lat);
        }
    }
    for (rus, lat) in UPPER_ALA_LC {
        if *rus == c {
            return Some(lat);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Russian Cyrillic → BGN/PCGN
// ---------------------------------------------------------------------------

const RUSSIAN_BGN_PCGN: &[(char, &str)] = &[
    ('а', "a"),
    ('б', "b"),
    ('в', "v"),
    ('г', "g"),
    ('д', "d"),
    ('е', "e"),
    ('ё', "ë"),
    ('ж', "zh"),
    ('з', "z"),
    ('и', "i"),
    ('й', "y"),
    ('к', "k"),
    ('л', "l"),
    ('м', "m"),
    ('н', "n"),
    ('о', "o"),
    ('п', "p"),
    ('р', "r"),
    ('с', "s"),
    ('т', "t"),
    ('у', "u"),
    ('ф', "f"),
    ('х', "kh"),
    ('ц', "ts"),
    ('ч', "ch"),
    ('ш', "sh"),
    ('щ', "shch"),
    ('ъ', ""),
    ('ы', "y"),
    ('ь', "'"),
    ('э', "e"),
    ('ю', "yu"),
    ('я', "ya"),
];

const UPPER_BGN_PCGN: &[(char, &str)] = &[
    ('А', "A"),
    ('Б', "B"),
    ('В', "V"),
    ('Г', "G"),
    ('Д', "D"),
    ('Е', "Ye"),
    ('Ё', "Yë"),
    ('Ж', "Zh"),
    ('З', "Z"),
    ('И', "I"),
    ('Й', "Y"),
    ('К', "K"),
    ('Л', "L"),
    ('М', "M"),
    ('Н', "N"),
    ('О', "O"),
    ('П', "P"),
    ('Р', "R"),
    ('С', "S"),
    ('Т', "T"),
    ('У', "U"),
    ('Ф', "F"),
    ('Х', "Kh"),
    ('Ц', "Ts"),
    ('Ч', "Ch"),
    ('Ш', "Sh"),
    ('Щ', "Shch"),
    ('Ъ', ""),
    ('Ы', "Y"),
    ('Ь', "'"),
    ('Э', "E"),
    ('Ю', "Yu"),
    ('Я', "Ya"),
];

fn russian_to_bgn_pcgn(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        // Handle initial е/ё → ye/yë
        if (c == 'Е' || c == 'е') && (i == 0 || is_non_letter(chars[i - 1])) {
            if c.is_uppercase() {
                out.push_str("Ye");
            } else {
                out.push_str("ye");
            }
            i += 1;
            continue;
        }
        if (c == 'Ё' || c == 'ё') && (i == 0 || is_non_letter(chars[i - 1])) {
            if c.is_uppercase() {
                out.push_str("Yë");
            } else {
                out.push_str("yë");
            }
            i += 1;
            continue;
        }
        if let Some(lat) = lookup_bgn_pcgn(c) {
            out.push_str(lat);
        } else {
            out.push(c);
        }
        i += 1;
    }
    out
}

fn is_non_letter(c: char) -> bool {
    !c.is_alphabetic()
}

fn lookup_bgn_pcgn(c: char) -> Option<&'static str> {
    for (rus, lat) in RUSSIAN_BGN_PCGN {
        if *rus == c {
            return Some(lat);
        }
    }
    for (rus, lat) in UPPER_BGN_PCGN {
        if *rus == c {
            return Some(lat);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iast_to_devanagari_simple() {
        assert_eq!(iast_to_devanagari("kumāra"), "कुमार");
    }

    #[test]
    fn iast_to_devanagari_kha() {
        assert_eq!(iast_to_devanagari("kha"), "ख");
    }

    #[test]
    fn iast_to_devanagari_ksetra() {
        assert_eq!(iast_to_devanagari("kṣetra"), "क्षेत्र");
    }

    #[test]
    fn iast_to_devanagari_jivita() {
        assert_eq!(iast_to_devanagari("jīvita"), "जीवित");
    }

    #[test]
    fn iast_to_devanagari_jnana() {
        assert_eq!(iast_to_devanagari("jñāna"), "ज्ञान");
    }

    #[test]
    fn iast_to_devanagari_jvara() {
        assert_eq!(iast_to_devanagari("jvara"), "ज्वर");
    }

    #[test]
    fn iast_to_devanagari_tyaga() {
        assert_eq!(iast_to_devanagari("tyāga"), "त्याग");
    }

    #[test]
    fn iast_to_devanagari_tridasa() {
        assert_eq!(iast_to_devanagari("tridaśa"), "त्रिदश");
    }

    #[test]
    fn iast_to_devanagari_tvid() {
        assert_eq!(iast_to_devanagari("tvid"), "त्विद्");
    }

    #[test]
    fn iast_to_devanagari_vowel_start() {
        assert_eq!(iast_to_devanagari("arka"), "अर्क");
    }

    #[test]
    fn iast_to_devanagari_anusvara() {
        assert_eq!(iast_to_devanagari("saṃskṛta"), "संस्कृत");
    }

    #[test]
    fn iast_to_devanagari_visarga() {
        assert_eq!(iast_to_devanagari("namaḥ"), "नमः");
    }

    #[test]
    fn russian_to_ala_lc_simple() {
        assert_eq!(russian_to_ala_lc("Москва"), "Moskva");
    }

    #[test]
    fn russian_to_ala_lc_with_special() {
        assert_eq!(russian_to_ala_lc("Щука"), "Shchuka");
    }

    #[test]
    fn russian_to_bgn_pcgn_simple() {
        assert_eq!(russian_to_bgn_pcgn("Москва"), "Moskva");
    }

    #[test]
    fn russian_to_bgn_pcgn_initial_e() {
        assert_eq!(russian_to_bgn_pcgn("Ельцин"), "Yel'tsin");
    }

    #[test]
    fn parse_transliteration_xml_bcf_without_ns() {
        let xml = r#"<transliteration entrytype="*">
            <translit langids="sanskrit" target="title" from="iast" to="devanagari"/>
        </transliteration>"#;
        let rules = parse_transliteration_xml(xml);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].entrytype, "*");
        assert_eq!(rules[0].langids.as_ref().unwrap(), &["sanskrit"]);
        assert_eq!(rules[0].target, "title");
        assert_eq!(rules[0].from, "iast");
        assert_eq!(rules[0].to, "devanagari");
    }

    #[test]
    fn parse_transliteration_xml_config() {
        let xml = r#"<transliteration entrytype="*">
            <translit target="title" from="IAST" to="Devanagari"/>
            <translit target="title" from="a" to="b"/>
        </transliteration>"#;
        let rules = parse_transliteration_xml(xml);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[1].from, "a");
        assert_eq!(rules[1].to, "b");
    }

    #[test]
    fn apply_transliteration_matches_rule() {
        let rules = vec![TranslitRule {
            entrytype: "*".into(),
            langids: Some(vec!["sanskrit".into()]),
            target: "title".into(),
            from: "iast".into(),
            to: "devanagari".into(),
        }];
        let result = apply(&rules, "book", Some("sanskrit"), "title", "kumāra");
        assert_eq!(result, "कुमार");
    }

    #[test]
    fn apply_transliteration_no_match_langid() {
        let rules = vec![TranslitRule {
            entrytype: "*".into(),
            langids: Some(vec!["sanskrit".into()]),
            target: "title".into(),
            from: "iast".into(),
            to: "devanagari".into(),
        }];
        let result = apply(&rules, "book", Some("german"), "title", "kumāra");
        assert_eq!(result, "kumāra");
    }

    #[test]
    fn apply_transliteration_no_match_target() {
        let rules = vec![TranslitRule {
            entrytype: "*".into(),
            langids: None,
            target: "author".into(),
            from: "iast".into(),
            to: "devanagari".into(),
        }];
        let result = apply(&rules, "book", None, "title", "kumāra");
        assert_eq!(result, "kumāra");
    }

    #[test]
    fn apply_transliteration_wildcard_target() {
        let rules = vec![TranslitRule {
            entrytype: "*".into(),
            langids: None,
            target: "*".into(),
            from: "iast".into(),
            to: "devanagari".into(),
        }];
        let result = apply(&rules, "book", None, "title", "kumāra");
        assert_eq!(result, "कुमार");
    }

    #[test]
    fn apply_transliteration_no_rules() {
        let result = apply(&[], "book", Some("sanskrit"), "title", "kumāra");
        assert_eq!(result, "kumāra");
    }

    #[test]
    fn rule_config_value_roundtrip() {
        let rule = TranslitRule {
            entrytype: "*".into(),
            langids: Some(vec!["sanskrit".into()]),
            target: "title".into(),
            from: "iast".into(),
            to: "devanagari".into(),
        };
        let cv = rule_to_config_value(&rule);
        let rules = rules_from_config_value(&cv);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].entrytype, "*");
        assert_eq!(rules[0].langids.as_ref().unwrap(), &["sanskrit"]);
        assert_eq!(rules[0].target, "title");
        assert_eq!(rules[0].from, "iast");
        assert_eq!(rules[0].to, "devanagari");
    }

    #[test]
    fn rule_config_value_list_roundtrip() {
        use crate::config::ConfigValue;
        let r1 = TranslitRule {
            entrytype: "*".into(),
            langids: None,
            target: "title".into(),
            from: "iast".into(),
            to: "devanagari".into(),
        };
        let r2 = TranslitRule {
            entrytype: "article".into(),
            langids: Some(vec!["russian".into()]),
            target: "title".into(),
            from: "russian".into(),
            to: "ala-lc".into(),
        };
        let list = ConfigValue::List(vec![rule_to_config_value(&r1), rule_to_config_value(&r2)]);
        let rules = rules_from_config_value(&list);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[1].entrytype, "article");
        assert_eq!(rules[1].from, "russian");
        assert_eq!(rules[1].to, "ala-lc");
    }
}
