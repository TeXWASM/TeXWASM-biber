//! ISBN, ISSN, ISMN validation.
//!
//! Validates checksums and formats for standard bibliographic identifiers.
//! Ported from the original biber's validation routines.

use tracing::warn;

use crate::processor::Biber;

/// Strip common ISBN/ISSN/ISMN separators (spaces, hyphens, periods).
fn strip_separators(input: &str) -> String {
    input
        .chars()
        .filter(|c| *c != ' ' && *c != '-' && *c != '.')
        .collect()
}

/// Validate an ISBN-10 checksum.
///
/// ISBN-10: 10 characters (digits 0-9, last may be 'X' for 10).
/// Weighted sum: Σ(i × d_i) for i = 1..10; sum % 11 == 0.
fn validate_isbn10(raw: &str) -> bool {
    let cleaned = strip_separators(raw);
    if cleaned.len() != 10 {
        return false;
    }
    let mut sum: u64 = 0;
    for (i, c) in cleaned.chars().enumerate() {
        let digit = if i == 9 && (c == 'X' || c == 'x') {
            10
        } else {
            match c.to_digit(10) {
                Some(d) => d,
                None => return false,
            }
        };
        sum += (i as u64 + 1) * digit as u64;
    }
    sum % 11 == 0
}

/// Validate an ISBN-13 checksum.
///
/// ISBN-13: 13 digits, starts with 978 or 979.
/// Alternating weights 1, 3; sum % 10 == 0.
fn validate_isbn13(raw: &str) -> bool {
    let cleaned = strip_separators(raw);
    if cleaned.len() != 13 {
        return false;
    }
    if !cleaned.starts_with("978") && !cleaned.starts_with("979") {
        return false;
    }
    let mut sum: u64 = 0;
    for (i, c) in cleaned.chars().enumerate() {
        let digit = match c.to_digit(10) {
            Some(d) => d,
            None => return false,
        };
        let weight = if i % 2 == 0 { 1 } else { 3 };
        sum += weight * digit as u64;
    }
    sum % 10 == 0
}

/// Validate an ISBN (ISBN-10 or ISBN-13).
pub fn validate_isbn(raw: &str) -> bool {
    if raw.is_empty() {
        return false;
    }
    let cleaned = strip_separators(raw);
    match cleaned.len() {
        10 => validate_isbn10(&cleaned),
        13 => validate_isbn13(&cleaned),
        _ => false,
    }
}

/// Validate an ISSN checksum.
///
/// ISSN: 8 characters (digits 0-9, last may be 'X' for 10).
/// Weighted sum: 8×d1 + 7×d2 + 6×d3 + 5×d4 + 4×d5 + 3×d6 + 2×d7 + 1×d8;
/// sum % 11 == 0.
pub fn validate_issn(raw: &str) -> bool {
    let cleaned = strip_separators(raw);
    if cleaned.len() != 8 {
        return false;
    }
    let mut sum: u64 = 0;
    for (i, c) in cleaned.chars().enumerate() {
        let digit = if i == 7 && (c == 'X' || c == 'x') {
            10
        } else {
            match c.to_digit(10) {
                Some(d) => d,
                None => return false,
            }
        };
        let weight = 8 - i as u64;
        sum += weight * digit as u64;
    }
    sum % 11 == 0
}

/// Validate an ISMN-10 (legacy) checksum.
///
/// ISMN-10: 'M' followed by 9 digits. 'M' is treated as 3 for the
/// weighted sum. Algorithm identical to ISBN-10.
fn validate_ismn10(raw: &str) -> bool {
    let cleaned = strip_separators(raw);
    if cleaned.len() != 10 {
        return false;
    }
    let first = cleaned.chars().next().unwrap();
    if first != 'M' && first != 'm' {
        return false;
    }
    let mut sum: u64 = 0;
    // Position 1: 'M' treated as digit 3
    sum += 3;
    for (i, c) in cleaned.chars().skip(1).enumerate() {
        let digit = match c.to_digit(10) {
            Some(d) => d,
            None => return false,
        };
        sum += (i as u64 + 2) * digit as u64;
    }
    sum % 11 == 0
}

/// Validate an ISMN-13 (modern) checksum.
///
/// ISMN-13: 13 digits, starts with 9790.
/// Algorithm identical to ISBN-13.
fn validate_ismn13(raw: &str) -> bool {
    let cleaned = strip_separators(raw);
    if cleaned.len() != 13 {
        return false;
    }
    if !cleaned.starts_with("9790") {
        return false;
    }
    let mut sum: u64 = 0;
    for (i, c) in cleaned.chars().enumerate() {
        let digit = match c.to_digit(10) {
            Some(d) => d,
            None => return false,
        };
        let weight = if i % 2 == 0 { 1 } else { 3 };
        sum += weight * digit as u64;
    }
    sum % 10 == 0
}

/// Validate an ISMN (ISMN-10 or ISMN-13).
pub fn validate_ismn(raw: &str) -> bool {
    if raw.is_empty() {
        return false;
    }
    let cleaned = strip_separators(raw);
    if cleaned.is_empty() {
        return false;
    }
    match cleaned.len() {
        10 => {
            let first = cleaned.chars().next().unwrap();
            if first == 'M' || first == 'm' {
                validate_ismn10(&cleaned)
            } else {
                false
            }
        }
        13 => validate_ismn13(&cleaned),
        _ => false,
    }
}

/// Validate ISBN, ISSN, and ISMN fields in all entries for a section.
/// Emits a `warn!` for each invalid identifier.
pub fn validate_entry_fields(biber: &mut Biber, secnum: u32) {
    let citekeys: Vec<String> = biber
        .sections
        .get_section(secnum)
        .map(|s| s.get_citekeys().to_vec())
        .unwrap_or_default();

    for k in &citekeys {
        if let Some(section) = biber.sections.get_section(secnum) {
            if let Some(be) = section.bibentries.get_entry(k) {
                if let Some(isbn) = be.get_field_str("isbn") {
                    if !isbn.is_empty() && !validate_isbn(isbn) {
                        warn!("Entry '{k}': invalid ISBN '{isbn}'");
                    }
                }
                if let Some(issn) = be.get_field_str("issn") {
                    if !issn.is_empty() && !validate_issn(issn) {
                        warn!("Entry '{k}': invalid ISSN '{issn}'");
                    }
                }
                if let Some(ismn) = be.get_field_str("ismn") {
                    if !ismn.is_empty() && !validate_ismn(ismn) {
                        warn!("Entry '{k}': invalid ISMN '{ismn}'");
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- ISBN ----

    #[test]
    fn valid_isbn10() {
        assert!(validate_isbn("0-306-40615-2"));
    }

    #[test]
    fn valid_isbn10_with_x() {
        assert!(validate_isbn("0-201-53082-1"));
    }

    #[test]
    fn valid_isbn13() {
        assert!(validate_isbn("978-0-306-40615-7"));
    }

    #[test]
    fn valid_isbn13_979() {
        assert!(validate_isbn("979-10-90636-07-1"));
    }

    #[test]
    fn invalid_isbn_wrong_checkdigit() {
        assert!(!validate_isbn("0-306-40615-3"));
    }

    #[test]
    fn invalid_isbn_too_short() {
        assert!(!validate_isbn("123456789"));
    }

    #[test]
    fn invalid_isbn_empty() {
        assert!(!validate_isbn(""));
    }

    // ---- ISSN ----

    #[test]
    fn valid_issn() {
        assert!(validate_issn("0024-9319"));
    }

    #[test]
    fn valid_issn_with_x() {
        assert!(validate_issn("0000-006X"));
    }

    #[test]
    fn invalid_issn_wrong_checkdigit() {
        assert!(!validate_issn("0024-9318"));
    }

    #[test]
    fn invalid_issn_too_short() {
        assert!(!validate_issn("1234567"));
    }

    #[test]
    fn invalid_issn_empty() {
        assert!(!validate_issn(""));
    }

    // ---- ISMN ----

    #[test]
    fn valid_ismn10() {
        assert!(validate_ismn("M-1234-5678-1"));
    }

    #[test]
    fn valid_ismn13() {
        // 979-0-2306-7118-7 is the ISMN-13 for the above
        assert!(validate_ismn("979-0-2306-7118-7"));
    }

    #[test]
    fn invalid_ismn_wrong_checkdigit() {
        assert!(!validate_ismn("M-1234-5678-2"));
    }

    #[test]
    fn invalid_ismn_too_short() {
        assert!(!validate_ismn("M-1234-567"));
    }

    #[test]
    fn invalid_ismn_not_starting_with_m() {
        assert!(!validate_ismn("X-1234-5678-1"));
    }

    #[test]
    fn invalid_ismn_empty() {
        assert!(!validate_ismn(""));
    }

    // ---- strip_separators ----

    #[test]
    fn strips_hyphens() {
        assert_eq!(strip_separators("978-0-306-40615-7"), "9780306406157");
    }

    #[test]
    fn strips_spaces() {
        assert_eq!(strip_separators("0 306 40615 2"), "0306406152");
    }

    #[test]
    fn strips_periods() {
        assert_eq!(strip_separators("0.306.40615.2"), "0306406152");
    }

    #[test]
    fn no_separators_unchanged() {
        assert_eq!(strip_separators("9780306406157"), "9780306406157");
    }
}
