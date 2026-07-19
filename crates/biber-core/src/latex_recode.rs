//! LaTeX ↔ Unicode recode.
//!
//! Ported from `lib/Biber/LaTeX/Recode.pm` + `recode_data.xml`. The XML
//! mapping table is embedded at compile time via `include_str!` (see
//! `vendored.rs`). Builds lookup tables for `latex_decode` (LaTeX macros →
//! Unicode) and `latex_encode` (Unicode → LaTeX macros).
//!
//! Three sets are available: `null` (no conversion), `base` (common macros
//! and diacritics), `full` (everything). The set is chosen by the caller
//! (default `base`).

use std::collections::HashMap;
use tracing::debug;

use unicode_normalization::UnicodeNormalization;

/// NFD-normalize a string (convenience wrapper).
fn nfd_str(s: &str) -> String {
    s.nfd().collect()
}

/// NFKC-normalize a string (convenience wrapper).
fn nfkc_str(s: &str) -> String {
    s.nfkc().collect()
}

/// The recode set to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecodeSet {
    /// No conversion.
    Null,
    /// Most common macros and diacritics (sufficient for Western languages).
    Base,
    /// Also converts punctuation, larger range of diacritics, symbols, Greek, dingbats, etc.
    Full,
}

impl RecodeSet {
    fn matches(self, set_attr: &str) -> bool {
        let sets: Vec<&str> = set_attr.split(',').map(|s| s.trim()).collect();
        match self {
            Self::Null => false,
            Self::Base => sets.contains(&"base"),
            Self::Full => sets.contains(&"full"),
        }
    }
}

/// Map types in the XML, in decode order.
const DECODE_TYPES: &[&str] = &[
    "greek",
    "dings",
    "punctuation",
    "symbols",
    "negatedsymbols",
    "superscripts",
    "cmdsuperscripts",
    "letters",
    "diacritics",
];

/// Map types in the XML, in encode order.
const ENCODE_TYPES: &[&str] = &[
    "greek",
    "dings",
    "negatedsymbols",
    "superscripts",
    "cmdsuperscripts",
    "diacritics",
    "letters",
    "punctuation",
    "symbols",
];

/// A mapping entry: LaTeX macro `from` → Unicode char `to`.
#[derive(Debug, Clone)]
struct MapEntry {
    from: String,
    to: String,
    preferred: bool,
    raw: bool,
}

/// The recode lookup tables, built once from the embedded XML.
pub struct Recoder {
    /// Decode tables: type → (LaTeX macro → Unicode char).
    decode_maps: HashMap<String, HashMap<String, String>>,
    /// Encode tables: type → (Unicode char → LaTeX macro).
    encode_maps: HashMap<String, HashMap<String, String>>,
    /// Raw encode entries (inserted as-is, no `\` or `{}`).
    encode_raw: HashMap<String, bool>,
}

impl Recoder {
    /// Build lookup tables from the embedded `recode_data.xml` for the
    /// given decode and encode sets.
    pub fn new(decode_set: RecodeSet, encode_set: RecodeSet) -> Self {
        let xml = crate::vendored::RECODE_DATA_XML;
        let doc = roxmltree::Document::parse(xml).expect("recode_data.xml is valid XML");

        let mut decode_maps: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut encode_maps: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut encode_raw: HashMap<String, bool> = HashMap::new();

        // Find the root <texmap> element
        let texmap = doc
            .root()
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "texmap")
            .expect("recode_data.xml has a <texmap> root");

        // Collect decode excludes
        let decode_excludes: Vec<String> = texmap
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "decode_exclude")
            .into_iter()
            .flat_map(|n| n.children())
            .filter(|n| n.is_element() && n.tag_name().name() == "char")
            .map(|n| n.text().unwrap_or("").trim().to_string())
            .collect();

        // Collect encode excludes
        let encode_excludes: Vec<String> = texmap
            .children()
            .find(|n| n.is_element() && n.tag_name().name() == "encode_exclude")
            .into_iter()
            .flat_map(|n| n.children())
            .filter(|n| n.is_element() && n.tag_name().name() == "char")
            .map(|n| n.text().unwrap_or("").trim().to_string())
            .collect();

        // Parse all <maps> elements (children of <texmap>)
        for maps_node in texmap
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "maps")
        {
            let typ = maps_node.attribute("type").unwrap_or("");
            let set_attr = maps_node.attribute("set").unwrap_or("");

            let entries: Vec<MapEntry> = maps_node
                .children()
                .filter(|n| n.is_element() && n.tag_name().name() == "map")
                .map(|map_node| {
                    let from = map_node
                        .children()
                        .find(|n| n.is_element() && n.tag_name().name() == "from")
                        .map(|n| n.text().unwrap_or("").trim().to_string())
                        .unwrap_or_default();
                    let to = map_node
                        .children()
                        .find(|n| n.is_element() && n.tag_name().name() == "to")
                        .map(|n| n.text().unwrap_or("").trim().to_string())
                        .unwrap_or_default();
                    let preferred = map_node
                        .children()
                        .find(|n| n.is_element() && n.tag_name().name() == "from")
                        .is_some_and(|n| n.attribute("preferred").is_some());
                    let raw = map_node
                        .children()
                        .find(|n| n.is_element() && n.tag_name().name() == "from")
                        .is_some_and(|n| n.attribute("raw").is_some());
                    MapEntry {
                        from,
                        to,
                        preferred,
                        raw,
                    }
                })
                .collect();

            // Decode set: map from LaTeX macro (NFD) → Unicode char (NFD)
            if decode_set.matches(set_attr) {
                let map = decode_maps.entry(typ.to_string()).or_default();
                for entry in &entries {
                    let key = nfd_str(&entry.from).to_string();
                    if !decode_excludes.contains(&entry.from) {
                        map.insert(key, nfd_str(&entry.to).to_string());
                    }
                }
            }

            // Encode set: map from Unicode char (NFD) → LaTeX macro (NFD)
            if encode_set.matches(set_attr) {
                let map = encode_maps.entry(typ.to_string()).or_default();
                for entry in &entries {
                    let key = nfd_str(&entry.to).to_string();
                    if !encode_excludes.contains(&entry.to) {
                        // Preferred entries override
                        if entry.preferred || !map.contains_key(&key) {
                            map.insert(key, entry.from.clone());
                        }
                    }
                }
                // Track raw entries
                for entry in &entries {
                    if entry.raw {
                        encode_raw.insert(nfd_str(&entry.to).to_string(), true);
                    }
                }
            }
        }

        debug!(
            "Recode tables built: {} decode types, {} encode types",
            decode_maps.len(),
            encode_maps.len()
        );

        Self {
            decode_maps,
            encode_maps,
            encode_raw,
        }
    }

    /// Convert LaTeX macros in `text` to Unicode characters.
    ///
    /// Mirrors `latex_decode()` in Recode.pm. Processes types in the same
    /// order as the Perl code. Normalizes to NFD by default.
    pub fn latex_decode(&self, text: &str) -> String {
        let mut text = text.to_string();

        // Deal with \char macros
        text = replace_char_macros(&text);

        // \foo\ bar -> \foo{} bar
        text = regex_replace_all(&text, r"\\([a-zA-Z]+)\\(\s+)", |caps: &regex::Captures| {
            format!("\\{}{}{}", &caps[1], "{}", &caps[2])
        });

        // Aaaa\o, -> Aaaa\o{},
        text = regex_replace_all(&text, r"([^{]\\\w)([;,.:%])", |caps: &regex::Captures| {
            format!("{}{}{}", &caps[1], "{}", &caps[2])
        });

        for typ in DECODE_TYPES {
            let map = match self.decode_maps.get(*typ) {
                Some(m) => m,
                None => continue,
            };
            if map.is_empty() {
                continue;
            }

            match *typ {
                "negatedsymbols" => {
                    // \not\X -> Unicode
                    let keys: Vec<&str> = map.keys().map(|s| s.as_str()).collect();
                    let pattern = format!(r"\\not\\({})", keys.join("|"));
                    text = regex_replace_all(&text, &pattern, |caps: &regex::Captures| {
                        map.get(&caps[1]).cloned().unwrap_or_default()
                    });
                }
                "superscripts" => {
                    let keys: Vec<&str> = map.keys().map(|s| s.as_str()).collect();
                    let pattern = format!(r"\\textsuperscript\{{({})\}}", keys.join("|"));
                    text = regex_replace_all(&text, &pattern, |caps: &regex::Captures| {
                        map.get(&caps[1]).cloned().unwrap_or_default()
                    });
                }
                "cmdsuperscripts" => {
                    let keys: Vec<&str> = map.keys().map(|s| s.as_str()).collect();
                    let pattern = format!(r"\\textsuperscript\{{\\({})\}}", keys.join("|"));
                    text = regex_replace_all(&text, &pattern, |caps: &regex::Captures| {
                        map.get(&caps[1]).cloned().unwrap_or_default()
                    });
                }
                "dings" => {
                    text = regex_replace_all(
                        &text,
                        r"\\ding\{([2-9AF][0-9A-F])\}",
                        |caps: &regex::Captures| map.get(&caps[1]).cloned().unwrap_or_default(),
                    );
                }
                "letters" => {
                    // Sort keys by descending length to match longer macros first
                    let mut keys: Vec<&String> = map.keys().collect();
                    keys.sort_by_key(|b| std::cmp::Reverse(b.len()));
                    for key in keys {
                        let escaped = regex::escape(key);
                        // Match \key followed by one of:
                        // - {} (consumed)
                        // - whitespace (consumed, re-added in replacement)
                        // - end of string
                        // - non-alpha char (not consumed)
                        // Since regex crate doesn't support lookahead, we use
                        // alternation with capture groups.
                        let pattern = format!(r"\\{escaped}(\{{\}}|\s+|$|[^a-zA-Z\s])");
                        let re = match regex::Regex::new(&pattern) {
                            Ok(re) => re,
                            Err(e) => {
                                debug!("Invalid regex for key '{}': {}", key, e);
                                continue;
                            }
                        };
                        let replacement = map.get(key).cloned().unwrap_or_default();
                        // Preserve the trailing char (if not end-of-string or {})
                        text = re
                            .replace_all(&text, |caps: &regex::Captures| {
                                let trailing = &caps[1];
                                match trailing {
                                    "{}" | " " | "\t" | "\n" | "" => replacement.clone(),
                                    _ => format!("{}{}", replacement, trailing),
                                }
                            })
                            .to_string();
                    }
                }
                "punctuation" | "symbols" | "greek" => {
                    let mut keys: Vec<&String> = map.keys().collect();
                    keys.sort_by_key(|b| std::cmp::Reverse(b.len()));
                    for key in keys {
                        let escaped = regex::escape(key);
                        let pattern = format!(r"\\{escaped}(?:\s+\{{\}}|\{{\}}|\s+|$|[^a-zA-Z\s])");
                        let re = match regex::Regex::new(&pattern) {
                            Ok(re) => re,
                            Err(_) => continue,
                        };
                        let replacement = map.get(key).cloned().unwrap_or_default();
                        text = re
                            .replace_all(&text, |caps: &regex::Captures| {
                                let full = &caps[0];
                                // If the match ends with a non-alpha char, preserve it
                                if full.ends_with('}')
                                    || full.ends_with(' ')
                                    || full.ends_with('\t')
                                    || full.ends_with('\n')
                                {
                                    replacement.clone()
                                } else if full.len() > key.len() + 1 {
                                    // Trailing char preserved
                                    let trailing = &full[full.len() - 1..];
                                    format!("{}{}", replacement, trailing)
                                } else {
                                    replacement.clone()
                                }
                            })
                            .to_string();
                    }
                }
                "diacritics" => {
                    let keys: Vec<String> = map.keys().map(|s| regex::escape(s)).collect();
                    let mut sorted = keys.clone();
                    sorted.sort_by_key(|b| std::cmp::Reverse(b.len()));
                    let re_str = sorted.join("|");
                    // \X{letter} -> letter+diacritic
                    let pattern = format!(r"\{{?\\({})\s*\{{?(\pL\pM*)\}}?", re_str);
                    text = regex_replace_all(&text, &pattern, |caps: &regex::Captures| {
                        let key = nfd_str(&caps[1]).to_string();
                        let replacement = map.get(&key).cloned().unwrap_or_default();
                        format!("{}{}", &caps[2], replacement)
                    });
                }
                _ => {}
            }
        }

        // Normalize to NFD
        nfd_str(&text)
    }

    /// Convert Unicode characters in `text` to LaTeX macros.
    ///
    /// Mirrors `latex_encode()` in Recode.pm. Normalizes input to NFD
    /// first so that combining characters match the map keys.
    pub fn latex_encode(&self, text: &str) -> String {
        // NFD-normalize the input so combining characters match map keys
        let mut text = nfd_str(text);

        for typ in ENCODE_TYPES {
            let map = match self.encode_maps.get(*typ) {
                Some(m) => m,
                None => continue,
            };
            if map.is_empty() {
                continue;
            }

            // Build alternation pattern, sorted by descending char length
            let mut keys: Vec<String> = map.keys().cloned().collect();
            keys.sort_by_key(|b| std::cmp::Reverse(b.len()));
            let pattern: String = keys
                .iter()
                .map(|s| regex::escape(s))
                .collect::<Vec<_>>()
                .join("|");

            match *typ {
                "negatedsymbols" => {
                    text = regex_replace_all(&text, &pattern, |caps: &regex::Captures| {
                        let macro_name = map.get(&caps[0]).cloned().unwrap_or_default();
                        format!("{{$\\not\\{}}}", macro_name)
                    });
                }
                "superscripts" => {
                    text = regex_replace_all(&text, &pattern, |caps: &regex::Captures| {
                        let macro_name = map.get(&caps[0]).cloned().unwrap_or_default();
                        format!("\\textsuperscript{{{}}}", macro_name)
                    });
                }
                "cmdsuperscripts" => {
                    text = regex_replace_all(&text, &pattern, |caps: &regex::Captures| {
                        let macro_name = map.get(&caps[0]).cloned().unwrap_or_default();
                        format!("\\textsuperscript{{\\{}}}", macro_name)
                    });
                }
                "dings" => {
                    text = regex_replace_all(&text, &pattern, |caps: &regex::Captures| {
                        let macro_name = map.get(&caps[0]).cloned().unwrap_or_default();
                        format!("\\ding{{{}}}", macro_name)
                    });
                }
                "letters" => {
                    text = regex_replace_all(&text, &pattern, |caps: &regex::Captures| {
                        let macro_name = map.get(&caps[0]).cloned().unwrap_or_default();
                        let is_raw = self.encode_raw.get(&caps[0]).copied().unwrap_or(false);
                        if is_raw {
                            macro_name
                        } else {
                            format!("\\{}{{}}", macro_name)
                        }
                    });
                }
                "punctuation" | "symbols" | "greek" => {
                    text = regex_replace_all(&text, &pattern, |caps: &regex::Captures| {
                        let macro_name = map.get(&caps[0]).cloned().unwrap_or_default();
                        if macro_name.starts_with("text") || macro_name.starts_with("guil") {
                            format!("\\{}{{}}", macro_name)
                        } else if self.encode_raw.get(&caps[0]).copied().unwrap_or(false) {
                            macro_name
                        } else {
                            format!("{{$\\{}$}}", macro_name)
                        }
                    });
                }
                "diacritics" => {
                    // Special case: i + diacritic -> \X{\i}
                    let diac_re = pattern.clone();
                    // i followed by diacritic
                    let i_pattern = format!("i({})", diac_re);
                    text = regex_replace_all(&text, &i_pattern, |caps: &regex::Captures| {
                        let macro_name = map.get(&caps[1]).cloned().unwrap_or_default();
                        format!("\\{}{{\\i}}", macro_name)
                    });

                    // {letter} followed by diacritic
                    let braced_pattern = format!(r"\{{(\pL\pM*)\}}({})", diac_re);
                    text = regex_replace_all(&text, &braced_pattern, |caps: &regex::Captures| {
                        let macro_name = map.get(&caps[2]).cloned().unwrap_or_default();
                        format!("\\{}{{{}}}", macro_name, &caps[1])
                    });

                    // letter followed by diacritic
                    let plain_pattern = format!(r"(\pL\pM*)({})", diac_re);
                    text = regex_replace_all(&text, &plain_pattern, |caps: &regex::Captures| {
                        let macro_name = map.get(&caps[2]).cloned().unwrap_or_default();
                        format!("\\{}{{{}}}", macro_name, &caps[1])
                    });
                }
                _ => {}
            }
        }

        text
    }
}

/// Handle `\char` macros: hex, octal, decimal.
fn replace_char_macros(text: &str) -> String {
    // \char"XX (hex)
    let text = regex_replace_all(
        text,
        r#"\\char"(\p{ASCII_Hex_Digit}+)"#,
        |caps: &regex::Captures| {
            let code = u32::from_str_radix(&caps[1], 16).unwrap_or(0);
            char::from_u32(code)
                .map(|c| c.to_string())
                .unwrap_or_default()
        },
    );
    // \char'XX (octal)
    let text = regex_replace_all(&text, r"\\char'(\d+)", |caps: &regex::Captures| {
        let code = u32::from_str_radix(&caps[1], 8).unwrap_or(0);
        char::from_u32(code)
            .map(|c| c.to_string())
            .unwrap_or_default()
    });
    // \charXX (decimal)
    regex_replace_all(&text, r"\\char(\d+)", |caps: &regex::Captures| {
        let code: u32 = caps[1].parse().unwrap_or(0);
        char::from_u32(code)
            .map(|c| c.to_string())
            .unwrap_or_default()
    })
}

/// Minimal `regex::replace_all` wrapper that doesn't require the `regex`
/// crate as a direct dependency — uses `regex` via a re-export from
/// `biber-core`.
///
/// Actually, we use the `regex` crate directly. This is a convenience
/// function to keep the call sites clean.
fn regex_replace_all<F: Fn(&regex::Captures) -> String>(
    text: &str,
    pattern: &str,
    replacement: F,
) -> String {
    let re = match regex::Regex::new(pattern) {
        Ok(re) => re,
        Err(e) => {
            debug!("Invalid regex pattern '{}': {}", pattern, e);
            return text.to_string();
        }
    };
    re.replace_all(text, |caps: &regex::Captures| replacement(caps))
        .to_string()
}

/// Convenience: decode LaTeX macros to Unicode using the `base` set.
pub fn latex_decode(text: &str) -> String {
    let recoder = Recoder::new(RecodeSet::Base, RecodeSet::Base);
    recoder.latex_decode(text)
}

/// Convenience: encode Unicode to LaTeX using the `base` set.
pub fn latex_encode(text: &str) -> String {
    let recoder = Recoder::new(RecodeSet::Base, RecodeSet::Base);
    recoder.latex_encode(text)
}

/// Convenience: encode Unicode to LaTeX using the specified set.
pub fn latex_encode_with_set(text: &str, set: RecodeSet) -> String {
    if set == RecodeSet::Null {
        return text.to_string();
    }
    let recoder = Recoder::new(RecodeSet::Null, set);
    recoder.latex_encode(text)
}

/// Normalize UTF-8 encoding string (e.g. "UTF-8", "utf8", "utf-8") to
/// canonical form.
pub fn normalise_utf8(encoding: &str) -> String {
    let lower = encoding.to_lowercase();
    match lower.as_str() {
        "utf8" | "utf-8" => "UTF-8".to_string(),
        _ => nfkc_str(encoding),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_simple_letters() {
        let recoder = Recoder::new(RecodeSet::Base, RecodeSet::Null);
        // \AA -> Å (NFD: A + combining ring above)
        assert_eq!(recoder.latex_decode("\\AA"), nfd_str("Å"));
        assert_eq!(recoder.latex_decode("\\aa"), nfd_str("å"));
        assert_eq!(recoder.latex_decode("\\ss"), nfd_str("ß"));
    }

    #[test]
    fn decode_diacritics() {
        let recoder = Recoder::new(RecodeSet::Base, RecodeSet::Null);
        // \'e -> é (e + combining acute accent, in NFD)
        let result = recoder.latex_decode("\\'{e}");
        let expected = nfd_str("é");
        assert_eq!(result, expected);
    }

    #[test]
    fn decode_text_macros() {
        let recoder = Recoder::new(RecodeSet::Base, RecodeSet::Null);
        // \textquotedblleft -> " (left double quotation mark)
        let result = recoder.latex_decode("\\textquotedblleft{}");
        assert_eq!(result, nfd_str("\u{201C}"));
    }

    #[test]
    fn encode_simple_letters() {
        let recoder = Recoder::new(RecodeSet::Null, RecodeSet::Base);
        // Å encodes to either \AA{} (letters) or \r{A} (diacritics)
        // Both are valid; diacritics wins because ENCODE_TYPES processes
        // diacritics before letters.
        let result = recoder.latex_encode("Å");
        assert!(
            result.contains("AA") || result.contains("\\r"),
            "expected \\AA{{}} or \\r{{A}} in, got {}",
            result
        );
        let result = recoder.latex_encode("ß");
        assert!(
            result.contains("ss"),
            "expected \\ss{{}} in, got {}",
            result
        );
    }

    #[test]
    fn encode_diacritics() {
        let recoder = Recoder::new(RecodeSet::Null, RecodeSet::Base);
        // é -> \'e (or similar)
        let result = recoder.latex_encode("é");
        assert!(
            result.contains("\\"),
            "should have a backslash macro, got {}",
            result
        );
    }

    #[test]
    fn roundtrip_basic() {
        let recoder = Recoder::new(RecodeSet::Base, RecodeSet::Base);
        // Decode then encode should give something recognizable
        let decoded = recoder.latex_decode("\\ss");
        assert_eq!(decoded, nfd_str("ß"));
        let encoded = recoder.latex_encode(&decoded);
        assert!(
            encoded.contains("ss"),
            "should contain 'ss' in: {}",
            encoded
        );
    }

    #[test]
    fn null_set_does_nothing() {
        let recoder = Recoder::new(RecodeSet::Null, RecodeSet::Null);
        let text = "Hello \\ss World";
        assert_eq!(recoder.latex_decode(text), text);
        assert_eq!(recoder.latex_encode("ß"), "ß");
    }

    #[test]
    fn char_macros() {
        assert_eq!(replace_char_macros("\\char\"41"), "A");
        assert_eq!(replace_char_macros("\\char'101"), "A");
        assert_eq!(replace_char_macros("\\char65"), "A");
    }

    #[test]
    fn normalise_utf8_variants() {
        assert_eq!(normalise_utf8("utf8"), "UTF-8");
        assert_eq!(normalise_utf8("utf-8"), "UTF-8");
        assert_eq!(normalise_utf8("UTF-8"), "UTF-8");
    }
}
