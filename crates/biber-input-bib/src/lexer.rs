//! BibTeX lexer and entry scanner.
//!
//! Ported from the btparse C library (the backend of `Text::BibTeX`).
//! Handles:
//!
//! * `@type{...}` entry scanning (brace- and parenthesis-delimited)
//! * `@string`, `@preamble`, `@comment` special entries
//! * Field-value parsing with brace balancing and `#` concatenation
//! * Macro expansion (bare-word references resolved against `@string` defs)
//! * Month macro defaults (`jan`→`1`, etc.)

use std::collections::HashMap;

use biber_core::constants::months;

/// Error type for BibTeX parsing.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ParseError {
    /// Unexpected end of input.
    #[error("unexpected end of input at offset {0}")]
    UnexpectedEof(usize),
    /// Invalid character.
    #[error("invalid character {char:?} at offset {offset}")]
    InvalidChar { char: char, offset: usize },
    /// Malformed entry.
    #[error("malformed entry at offset {offset}: {message}")]
    Malformed { offset: usize, message: String },
}

/// The type of a BibTeX entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BibEntryType {
    /// A regular entry (`@article`, `@book`, etc.).
    Regular,
    /// A `@string` macro definition.
    String,
    /// A `@preamble` entry.
    Preamble,
    /// A `@comment` entry.
    Comment,
    /// An unknown/invalid entry.
    Unknown,
}

/// A parsed BibTeX entry.
#[derive(Debug, Clone)]
pub struct BibEntry {
    /// Entry type as written (e.g. "article", "book"). Lowercased.
    pub typ: String,
    /// The entry type classification.
    pub entry_type: BibEntryType,
    /// Citekey (empty for non-regular entries).
    pub key: String,
    /// Fields: field name (lowercased) → value (after macro expansion).
    pub fields: Vec<(String, String)>,
    /// Whether parsing succeeded.
    pub parse_ok: bool,
}

impl BibEntry {
    /// Get a field value by name (case-insensitive lookup).
    pub fn get(&self, name: &str) -> Option<&str> {
        let lower = name.to_lowercase();
        self.fields
            .iter()
            .find(|(k, _)| *k == lower)
            .map(|(_, v)| v.as_str())
    }

    /// Get the list of field names in order.
    pub fn field_list(&self) -> Vec<&str> {
        self.fields.iter().map(|(k, _)| k.as_str()).collect()
    }

    /// Get a field value (owned) by name.
    pub fn get_owned(&self, name: &str) -> Option<String> {
        self.get(name).map(|s| s.to_string())
    }
}

/// The BibTeX file parser.
pub struct BibFile {
    chars: Vec<char>,
    pos: usize,
    /// Macro definitions from `@string` entries.
    macros: HashMap<String, String>,
}

impl BibFile {
    /// Create a new parser for the given input.
    pub fn new(input: &str) -> Self {
        Self {
            chars: input.chars().collect(),
            pos: 0,
            macros: HashMap::new(),
        }
    }

    /// Parse all entries in the input.
    pub fn parse_all(&mut self) -> Result<Vec<BibEntry>, ParseError> {
        // Initialize month macros
        for (m, v) in &months() {
            self.macros.insert((*m).to_string(), (*v).to_string());
        }

        let mut entries = Vec::new();
        while self.pos < self.chars.len() {
            self.skip_whitespace_and_comments();
            if self.pos >= self.chars.len() {
                break;
            }
            if self.chars[self.pos] != '@' {
                // Not an entry start; skip to next '@'
                self.skip_to_at();
                continue;
            }
            let entry = self.parse_entry()?;
            entries.push(entry);
        }
        Ok(entries)
    }

    /// Skip whitespace and BibTeX comments (text outside entries).
    fn skip_whitespace_and_comments(&mut self) {
        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            if c.is_whitespace() {
                self.pos += 1;
            } else if c == '%' {
                // Line comment (biber extension)
                while self.pos < self.chars.len() && self.chars[self.pos] != '\n' {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    /// Skip to the next '@' character.
    fn skip_to_at(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos] != '@' {
            self.pos += 1;
        }
    }

    /// Parse a single `@type{...}` entry.
    fn parse_entry(&mut self) -> Result<BibEntry, ParseError> {
        let _start_pos = self.pos;
        debug_assert_eq!(self.chars[self.pos], '@');
        self.pos += 1; // skip '@'

        // Parse entry type name
        let typ = self.parse_name();
        let typ_lower = typ.to_lowercase();
        let entry_type = match typ_lower.as_str() {
            "string" => BibEntryType::String,
            "preamble" => BibEntryType::Preamble,
            "comment" => BibEntryType::Comment,
            _ => BibEntryType::Regular,
        };

        // Skip whitespace before opening brace
        self.skip_ws();

        // Determine delimiter: '{' or '('
        let open_delim = if self.pos < self.chars.len() {
            match self.chars[self.pos] {
                '{' => '{',
                '(' => '(',
                _ => {
                    return Err(ParseError::Malformed {
                        offset: self.pos,
                        message: format!(
                            "expected '{{' or '(' after @{}, got {:?}",
                            typ,
                            self.chars.get(self.pos).copied().unwrap_or(' ')
                        ),
                    });
                }
            }
        } else {
            return Err(ParseError::UnexpectedEof(self.pos));
        };
        let close_delim = if open_delim == '{' { '}' } else { ')' };
        self.pos += 1; // skip opening delimiter

        let result = match entry_type {
            BibEntryType::String => self.parse_string_entry(),
            BibEntryType::Preamble => self.parse_preamble_entry(close_delim),
            BibEntryType::Comment => self.parse_comment_entry(close_delim),
            BibEntryType::Regular => self.parse_regular_entry(close_delim),
            BibEntryType::Unknown => self.parse_regular_entry(close_delim),
        };

        match result {
            Ok(mut entry) => {
                entry.typ = typ_lower;
                entry.entry_type = entry_type;
                Ok(entry)
            }
            Err(e) => {
                // Try to recover by skipping to the matching close delim
                self.skip_to_close_delim(open_delim, close_delim);
                Err(e)
            }
        }
    }

    /// Parse an identifier name (letters, digits, etc.).
    fn parse_name(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ':' {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.chars[start..self.pos].iter().collect()
    }

    /// Skip whitespace (spaces, tabs, newlines).
    fn skip_ws(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    /// Parse a regular entry: `key, field = value, field = value, ...`
    fn parse_regular_entry(&mut self, close_delim: char) -> Result<BibEntry, ParseError> {
        let mut entry = BibEntry {
            typ: String::new(),
            entry_type: BibEntryType::Regular,
            key: String::new(),
            fields: Vec::new(),
            parse_ok: true,
        };

        self.skip_ws();

        // Parse citekey (everything up to the first comma or close delim)
        let key = self.parse_citekey(close_delim);
        entry.key = key;

        self.skip_ws();

        // Parse fields
        while self.pos < self.chars.len() {
            if self.chars[self.pos] == close_delim {
                self.pos += 1; // consume close delim
                return Ok(entry);
            }
            if self.chars[self.pos] == ',' {
                self.pos += 1;
                self.skip_ws();
                continue;
            }

            // Parse field name
            let field_name = self.parse_name();
            if field_name.is_empty() {
                // Unexpected character, skip to next comma or close
                self.skip_to_comma_or_close(close_delim);
                continue;
            }
            self.skip_ws();

            // Expect '='
            if self.pos >= self.chars.len() || self.chars[self.pos] != '=' {
                self.skip_to_comma_or_close(close_delim);
                continue;
            }
            self.pos += 1; // skip '='
            self.skip_ws();

            // Parse field value
            let value = self.parse_field_value()?;
            entry.fields.push((field_name.to_lowercase(), value));

            self.skip_ws();
        }

        entry.parse_ok = false;
        Err(ParseError::UnexpectedEof(self.pos))
    }

    /// Parse a citekey (everything up to the first comma or close delim).
    fn parse_citekey(&mut self, close_delim: char) -> String {
        let start = self.pos;
        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            if c == ',' || c == close_delim {
                break;
            }
            self.pos += 1;
        }
        self.chars[start..self.pos]
            .iter()
            .collect::<String>()
            .trim()
            .to_string()
    }

    /// Parse a field value: a concatenation of strings/macros separated by `#`.
    fn parse_field_value(&mut self) -> Result<String, ParseError> {
        let mut parts = Vec::new();

        loop {
            self.skip_ws();
            if self.pos >= self.chars.len() {
                return Err(ParseError::UnexpectedEof(self.pos));
            }

            let c = self.chars[self.pos];
            if c == '{' {
                // Brace-delimited string
                let s = self.parse_braced_string()?;
                parts.push(s);
            } else if c == '"' {
                // Quote-delimited string
                let s = self.parse_quoted_string()?;
                parts.push(s);
            } else if c.is_alphabetic() || c == '_' {
                // Macro reference (bare word)
                let macro_name = self.parse_name();
                let macro_lower = macro_name.to_lowercase();
                let value = self.macros.get(&macro_lower).cloned().unwrap_or_default();
                parts.push(value);
            } else if c.is_ascii_digit() {
                // Numeric literal
                let start = self.pos;
                while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_digit() {
                    self.pos += 1;
                }
                let num: String = self.chars[start..self.pos].iter().collect();
                parts.push(num);
            } else {
                // Unexpected character
                break;
            }

            self.skip_ws();
            if self.pos < self.chars.len() && self.chars[self.pos] == '#' {
                self.pos += 1; // consume '#'
                continue;
            }
            break;
        }

        Ok(parts.concat())
    }

    /// Parse a `{...}` delimited string, handling nested braces.
    fn parse_braced_string(&mut self) -> Result<String, ParseError> {
        debug_assert_eq!(self.chars[self.pos], '{');
        self.pos += 1; // skip '{'
        let mut result = String::new();
        let mut depth = 1;

        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            match c {
                '{' => {
                    depth += 1;
                    result.push(c);
                    self.pos += 1;
                }
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        self.pos += 1;
                        return Ok(result);
                    }
                    result.push(c);
                    self.pos += 1;
                }
                '\\' => {
                    // Escaped character
                    result.push(c);
                    self.pos += 1;
                    if self.pos < self.chars.len() {
                        result.push(self.chars[self.pos]);
                        self.pos += 1;
                    }
                }
                _ => {
                    result.push(c);
                    self.pos += 1;
                }
            }
        }
        Err(ParseError::UnexpectedEof(self.pos))
    }

    /// Parse a `"..."` delimited string, handling nested braces.
    fn parse_quoted_string(&mut self) -> Result<String, ParseError> {
        debug_assert_eq!(self.chars[self.pos], '"');
        self.pos += 1; // skip '"'
        let mut result = String::new();
        let mut depth = 0;

        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            match c {
                '{' => {
                    depth += 1;
                    result.push(c);
                    self.pos += 1;
                }
                '}' => {
                    depth -= 1;
                    result.push(c);
                    self.pos += 1;
                }
                '"' if depth == 0 => {
                    self.pos += 1;
                    return Ok(result);
                }
                '\\' => {
                    result.push(c);
                    self.pos += 1;
                    if self.pos < self.chars.len() {
                        result.push(self.chars[self.pos]);
                        self.pos += 1;
                    }
                }
                _ => {
                    result.push(c);
                    self.pos += 1;
                }
            }
        }
        Err(ParseError::UnexpectedEof(self.pos))
    }

    /// Parse a `@string{macro = "value", ...}` entry.
    fn parse_string_entry(&mut self) -> Result<BibEntry, ParseError> {
        let mut entry = BibEntry {
            typ: String::new(),
            entry_type: BibEntryType::String,
            key: String::new(),
            fields: Vec::new(),
            parse_ok: true,
        };

        loop {
            self.skip_ws();
            if self.pos >= self.chars.len() {
                return Err(ParseError::UnexpectedEof(self.pos));
            }
            if self.chars[self.pos] == '}' || self.chars[self.pos] == ')' {
                self.pos += 1;
                return Ok(entry);
            }

            // Parse macro name
            let macro_name = self.parse_name();
            if macro_name.is_empty() {
                self.skip_to_comma_or_close('}');
                continue;
            }
            self.skip_ws();

            // Expect '='
            if self.pos >= self.chars.len() || self.chars[self.pos] != '=' {
                self.skip_to_comma_or_close('}');
                continue;
            }
            self.pos += 1;
            self.skip_ws();

            // Parse value
            let value = self.parse_field_value()?;
            self.macros.insert(macro_name.to_lowercase(), value.clone());
            entry.fields.push((macro_name.to_lowercase(), value));

            self.skip_ws();
            if self.pos < self.chars.len() && self.chars[self.pos] == ',' {
                self.pos += 1;
            }
        }
    }

    /// Parse a `@preamble{"..."}` entry.
    fn parse_preamble_entry(&mut self, close_delim: char) -> Result<BibEntry, ParseError> {
        let mut entry = BibEntry {
            typ: String::new(),
            entry_type: BibEntryType::Preamble,
            key: String::new(),
            fields: Vec::new(),
            parse_ok: true,
        };

        self.skip_ws();
        let value = self.parse_field_value()?;
        entry.fields.push((String::new(), value));

        self.skip_ws();
        if self.pos < self.chars.len() && self.chars[self.pos] == close_delim {
            self.pos += 1;
        }
        Ok(entry)
    }

    /// Parse a `@comment{...}` entry.
    fn parse_comment_entry(&mut self, close_delim: char) -> Result<BibEntry, ParseError> {
        let mut entry = BibEntry {
            typ: String::new(),
            entry_type: BibEntryType::Comment,
            key: String::new(),
            fields: Vec::new(),
            parse_ok: true,
        };

        // Read everything until the matching close delim
        let mut depth = 1;
        let mut content = String::new();
        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            match c {
                '{' | '(' => {
                    depth += 1;
                    content.push(c);
                }
                '}' | ')' if depth > 1 => {
                    depth -= 1;
                    content.push(c);
                }
                _ if c == close_delim && depth == 1 => {
                    self.pos += 1;
                    entry.fields.push((String::new(), content));
                    return Ok(entry);
                }
                _ => content.push(c),
            }
            self.pos += 1;
        }
        Err(ParseError::UnexpectedEof(self.pos))
    }

    /// Skip to the next comma or close delimiter.
    fn skip_to_comma_or_close(&mut self, close_delim: char) {
        let mut depth = 0;
        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            match c {
                '{' | '(' => depth += 1,
                '}' | ')' if depth > 0 => depth -= 1,
                ',' if depth == 0 => return,
                _ if c == close_delim && depth == 0 => return,
                _ => {}
            }
            self.pos += 1;
        }
    }

    /// Skip to the matching close delimiter (for error recovery).
    fn skip_to_close_delim(&mut self, open: char, close: char) {
        let mut depth = 1;
        while self.pos < self.chars.len() {
            let c = self.chars[self.pos];
            if c == open {
                depth += 1;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    self.pos += 1;
                    return;
                }
            }
            self.pos += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_entry() {
        let input = r#"@book{smith2020,
  author = {John Smith},
  title = {A Book},
  year = {2020}
}"#;
        let mut file = BibFile::new(input);
        let entries = file.parse_all().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].typ, "book");
        assert_eq!(entries[0].key, "smith2020");
        assert_eq!(entries[0].get("author"), Some("John Smith"));
        assert_eq!(entries[0].get("title"), Some("A Book"));
        assert_eq!(entries[0].get("year"), Some("2020"));
    }

    #[test]
    fn parse_multiple_entries() {
        let input = r#"@book{key1, author = {Doe}, title = {First}}
@article{key2, author = {Roe}, title = {Second}}"#;
        let entries = BibFile::new(input).parse_all().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "key1");
        assert_eq!(entries[1].key, "key2");
    }

    #[test]
    fn parse_string_macro() {
        let input = r#"@string{pub = "Oxford UP"}
@book{key1, publisher = pub, title = {A Book}}"#;
        let entries = BibFile::new(input).parse_all().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].entry_type, BibEntryType::String);
        assert_eq!(entries[1].get("publisher"), Some("Oxford UP"));
    }

    #[test]
    fn parse_month_macros() {
        let input = r#"@book{key1, month = jan}"#;
        let entries = BibFile::new(input).parse_all().unwrap();
        assert_eq!(entries[0].get("month"), Some("1"));
    }

    #[test]
    fn parse_concatenation() {
        let input = r#"@string{foo = "Foo"}
@book{key1, title = foo # " Bar"}"#;
        let entries = BibFile::new(input).parse_all().unwrap();
        assert_eq!(entries[1].get("title"), Some("Foo Bar"));
    }

    #[test]
    fn parse_paren_delimited() {
        let input = r#"@book(key1, author = {Doe}, title = {A Book})"#;
        let entries = BibFile::new(input).parse_all().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].key, "key1");
        assert_eq!(entries[0].get("author"), Some("Doe"));
    }

    #[test]
    fn parse_preamble() {
        let input = r#"@preamble{"\newcommand{\nop}{No.}"}"#;
        let entries = BibFile::new(input).parse_all().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].entry_type, BibEntryType::Preamble);
    }

    #[test]
    fn parse_comment() {
        let input = r#"@comment{This is a comment}
@book{key1, title = {A}}"#;
        let entries = BibFile::new(input).parse_all().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].entry_type, BibEntryType::Comment);
    }

    #[test]
    fn parse_quoted_value() {
        let input = r#"@book{key1, title = "A Title"}"#;
        let entries = BibFile::new(input).parse_all().unwrap();
        assert_eq!(entries[0].get("title"), Some("A Title"));
    }

    #[test]
    fn parse_nested_braces() {
        let input = r#"@book{key1, title = {A {Nested} Title}}"#;
        let entries = BibFile::new(input).parse_all().unwrap();
        assert_eq!(entries[0].get("title"), Some("A {Nested} Title"));
    }

    #[test]
    fn parse_numeric_value() {
        let input = r#"@book{key1, volume = 42}"#;
        let entries = BibFile::new(input).parse_all().unwrap();
        assert_eq!(entries[0].get("volume"), Some("42"));
    }

    #[test]
    fn parse_comments_between_entries() {
        let input = r#"% This is a comment
@book{key1, title = {A}}
% Another comment
@book{key2, title = {B}}"#;
        let entries = BibFile::new(input).parse_all().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "key1");
        assert_eq!(entries[1].key, "key2");
    }

    #[test]
    fn parse_empty_fields() {
        let input = r#"@book{key1, author = {}, title = {A}}"#;
        let entries = BibFile::new(input).parse_all().unwrap();
        assert_eq!(entries[0].get("author"), Some(""));
    }

    #[test]
    fn parse_escaped_chars() {
        let input = r#"@book{key1, title = {A\{B\}C}}"#;
        let entries = BibFile::new(input).parse_all().unwrap();
        assert_eq!(entries[0].get("title"), Some("A\\{B\\}C"));
    }

    #[test]
    fn parse_into_map() {
        use crate::parse_bib_into_map;
        let input = r#"@string{pub = "OP"}
@book{key1, publisher = pub, title = {A}}
@book{key2, title = {B}}"#;
        let (map, order, preambles) = parse_bib_into_map(input).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(order, vec!["key1", "key2"]);
        assert!(preambles.is_empty());
        assert_eq!(map.get("key1").unwrap().get("publisher"), Some("OP"));
    }
}
