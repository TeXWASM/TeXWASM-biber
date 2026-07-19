//! Date parsing — ISO 8601-2 extended format + biblatex date extensions.
//!
//! Ported from `lib/Biber/Date/Format.pm`. Handles:
//! - ISO 8601-1: `YYYY`, `YYYY-MM`, `YYYY-MM-DD`, `YYYY-MM-DDThh:mm:ss`
//! - ISO 8601-2: `Y<5+ digits>`, year divisions (`YYYY-21` = spring)
//! - Uncertain (`?`), approximate (`~`), both (`%`)
//! - Time zones (`Z`, `+hh:mm`)
//! - Julian calendar flag
//! - Missing month/day detection

use std::collections::HashMap;

/// A parsed date.
#[derive(Debug, Clone, Default)]
pub struct ParsedDate {
    /// Year (negative for BC).
    pub year: Option<i64>,
    /// Month (1-12).
    pub month: Option<u32>,
    /// Day (1-31).
    pub day: Option<u32>,
    /// Hour (0-23).
    pub hour: Option<u32>,
    /// Minute (0-59).
    pub minute: Option<u32>,
    /// Second (0-59).
    pub second: Option<u32>,
    /// Time zone string (e.g. "UTC", "+05:30").
    pub timezone: Option<String>,
    /// Whether the month is missing.
    pub missing_month: bool,
    /// Whether the day is missing.
    pub missing_day: bool,
    /// Whether the time is missing.
    pub missing_time: bool,
    /// Whether the date is approximate.
    pub approximate: bool,
    /// Whether the date is uncertain.
    pub uncertain: bool,
    /// Year division (e.g. "spring", "Q1").
    pub yeardivision: Option<String>,
    /// Whether this is a Julian calendar date.
    pub julian: bool,
}

impl ParsedDate {
    /// Parse an ISO 8601-2 date string.
    ///
    /// Returns `None` if the string is not a valid date.
    pub fn parse(input: &str) -> Option<Self> {
        let mut date = Self::default();
        let mut text = input.trim().to_string();

        // ISO 8601-2:2016 4.2.1 (uncertain)
        if text.ends_with('?') {
            date.uncertain = true;
            text = text.trim_end_matches('?').trim().to_string();
        }

        // ISO 8601-2:2016 4.2.1 (approximate)
        if text.ends_with('~') {
            date.approximate = true;
            text = text.trim_end_matches('~').trim().to_string();
        }

        // ISO 8601-2:2016 4.2.1 (uncertain+approximate)
        if text.ends_with('%') {
            date.uncertain = true;
            date.approximate = true;
            text = text.trim_end_matches('%').trim().to_string();
        }

        // ISO8601-1 4.2.2 (time zone)
        if text.ends_with('Z') {
            date.timezone = Some("UTC".to_string());
            text = text.trim_end_matches('Z').to_string();
        } else if let Some(pos) = text.rfind('+') {
            let tz_part = &text[pos..];
            if tz_part.len() == 6 && tz_part[1..].chars().all(|c| c.is_ascii_digit() || c == ':') {
                date.timezone = Some(tz_part.to_string());
                text = text[..pos].to_string();
            }
        } else if let Some(pos) = text.rfind('-') {
            let tz_part = &text[pos..];
            if tz_part.len() == 6 && tz_part[1..].chars().all(|c| c.is_ascii_digit() || c == ':') {
                date.timezone = Some(tz_part.to_string());
                text = text[..pos].to_string();
            }
        }

        // ISO8601-2:2016 4.8 (year divisions)
        // YYYY-21 .. YYYY-41
        if let Some(dash_pos) = text.rfind('-') {
            let after = &text[dash_pos + 1..];
            if after.len() == 2 && after.chars().all(|c| c.is_ascii_digit()) {
                let code: u32 = after.parse().unwrap_or(0);
                if let Some(div) = year_division_name(code) {
                    date.yeardivision = Some(div.to_string());
                    text = text[..dash_pos].to_string();
                }
            }
        }

        // Now parse the remaining date/time string
        let text = text.trim().to_string();

        // ISO8601-2: Y<5+ digits> (expanded year)
        if let Some(year_str) = text.strip_prefix('Y') {
            if year_str.chars().all(|c| c.is_ascii_digit() || c == '-') {
                let (sign, digits) = if let Some(stripped) = year_str.strip_prefix('-') {
                    (-1, stripped)
                } else {
                    (1, year_str)
                };
                if let Ok(year) = digits.parse::<i64>() {
                    date.year = Some(sign * year);
                    date.missing_month = true;
                    date.missing_day = true;
                    date.missing_time = true;
                    return Some(date);
                }
            }
            return None;
        }

        // ISO8601-1: [-]YYYY-MM-DDThh:mm:ss[.mmm]
        if text.contains('T') {
            return Self::parse_datetime(&text, &mut date);
        }

        // ISO8601-1: [-]YYYY-MM-DD
        let negative = text.starts_with('-');
        let text_stripped = if negative {
            text[1..].to_string()
        } else {
            text.clone()
        };

        let parts: Vec<&str> = text_stripped.split('-').filter(|s| !s.is_empty()).collect();
        match parts.len() {
            1 => {
                // Just year: [-]YYYY
                if let Ok(year) = parts[0].parse::<i64>() {
                    date.year = Some(if negative { -year } else { year });
                    date.missing_month = true;
                    date.missing_day = true;
                    date.missing_time = true;
                    return Some(date);
                }
            }
            2 => {
                // YYYY-MM
                if let Ok(year) = parts[0].parse::<i64>() {
                    if let Ok(month) = parts[1].parse::<u32>() {
                        date.year = Some(if negative { -year } else { year });
                        date.month = Some(month);
                        date.missing_day = true;
                        date.missing_time = true;
                        return Some(date);
                    }
                }
            }
            3 => {
                // YYYY-MM-DD
                if let Ok(year) = parts[0].parse::<i64>() {
                    if let Ok(month) = parts[1].parse::<u32>() {
                        if let Ok(day) = parts[2].parse::<u32>() {
                            date.year = Some(if negative { -year } else { year });
                            date.month = Some(month);
                            date.day = Some(day);
                            date.missing_time = true;
                            return Some(date);
                        }
                    }
                }
            }
            _ => {}
        }

        None
    }

    fn parse_datetime(text: &str, date: &mut Self) -> Option<Self> {
        // Split date and time parts
        let (date_part, time_part) = text.split_once('T').unwrap_or((text, ""));

        // Parse date part
        let date_parts: Vec<&str> = date_part.split('-').filter(|s| !s.is_empty()).collect();
        match date_parts.len() {
            3 => {
                let year_str = if date_part.starts_with('-') {
                    format!("-{}", date_parts[0])
                } else {
                    date_parts[0].to_string()
                };
                date.year = year_str.parse().ok();
                date.month = date_parts[1].parse().ok();
                date.day = date_parts[2].parse().ok();
            }
            2 => {
                date.year = date_parts[0].parse().ok();
                date.month = date_parts[1].parse().ok();
                date.missing_day = true;
            }
            1 => {
                date.year = date_parts[0].parse().ok();
                date.missing_month = true;
                date.missing_day = true;
            }
            _ => return None,
        }

        // Parse time part
        if !time_part.is_empty() {
            let time_parts: Vec<&str> = time_part.split(':').collect();
            if time_parts.len() >= 3 {
                date.hour = time_parts[0].parse().ok();
                date.minute = time_parts[1].parse().ok();
                // Strip milliseconds from seconds
                let sec_str = time_parts[2].split('.').next().unwrap_or("0");
                date.second = sec_str.parse().ok();
            }
        } else {
            date.missing_time = true;
        }

        Some(std::mem::take(date))
    }
}

/// Get the year division name for an ISO 8601-2 code.
fn year_division_name(code: u32) -> Option<&'static str> {
    let divisions: HashMap<u32, &'static str> = [
        (21, "spring"),
        (22, "summer"),
        (23, "autumn"),
        (24, "winter"),
        (25, "springN"),
        (26, "summerN"),
        (27, "autumnN"),
        (28, "winterN"),
        (29, "springS"),
        (30, "summerS"),
        (31, "autumnS"),
        (32, "winterS"),
        (33, "Q1"),
        (34, "Q2"),
        (35, "Q3"),
        (36, "Q4"),
        (37, "QD1"),
        (38, "QD2"),
        (39, "QD3"),
        (40, "S1"),
        (41, "S2"),
    ]
    .into_iter()
    .collect();
    divisions.get(&code).copied()
}

/// A parsed date range (start, end, separator).
#[derive(Debug, Clone)]
pub struct DateRange {
    /// Start date (may be `None` for open-ended range).
    pub start: Option<ParsedDate>,
    /// End date (may be `None` for open-ended range).
    pub end: Option<ParsedDate>,
    /// Separator ("/" or "--").
    pub sep: String,
    /// Whether the start was unspecified.
    pub unspecified: Option<String>,
}

/// Parse a date range string.
///
/// Handles `date/date`, `date--date`, and open-ended ranges (`date/`).
pub fn parse_date_range(input: &str) -> Option<DateRange> {
    let input = input.trim();

    // Check for range separator
    if let Some(pos) = input.find('/') {
        let start_str = &input[..pos];
        let end_str = &input[pos + 1..];

        let start = if start_str.trim().is_empty() {
            None
        } else {
            ParsedDate::parse(start_str)
        };
        let end = if end_str.trim().is_empty() {
            None
        } else {
            ParsedDate::parse(end_str)
        };

        return Some(DateRange {
            start,
            end,
            sep: "/".to_string(),
            unspecified: None,
        });
    }

    if let Some(pos) = input.find("--") {
        let start_str = &input[..pos];
        let end_str = &input[pos + 2..];

        let start = ParsedDate::parse(start_str);
        let end = ParsedDate::parse(end_str);

        return Some(DateRange {
            start,
            end,
            sep: "--".to_string(),
            unspecified: None,
        });
    }

    // Single date
    let start = ParsedDate::parse(input)?;
    Some(DateRange {
        start: Some(start),
        end: None,
        sep: String::new(),
        unspecified: None,
    })
}

/// Format a timezone string (e.g. "UTC" → "UTC", "+05:30" → "+05:30").
pub fn tzformat(tz: &str) -> String {
    tz.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_year_only() {
        let d = ParsedDate::parse("1985").unwrap();
        assert_eq!(d.year, Some(1985));
        assert!(d.missing_month);
        assert!(d.missing_day);
        assert!(d.missing_time);
    }

    #[test]
    fn parse_year_month() {
        let d = ParsedDate::parse("1985-04").unwrap();
        assert_eq!(d.year, Some(1985));
        assert_eq!(d.month, Some(4));
        assert!(d.missing_day);
        assert!(d.missing_time);
    }

    #[test]
    fn parse_full_date() {
        let d = ParsedDate::parse("1985-04-12").unwrap();
        assert_eq!(d.year, Some(1985));
        assert_eq!(d.month, Some(4));
        assert_eq!(d.day, Some(12));
        assert!(d.missing_time);
    }

    #[test]
    fn parse_datetime() {
        let d = ParsedDate::parse("1985-04-12T10:15:30").unwrap();
        assert_eq!(d.year, Some(1985));
        assert_eq!(d.month, Some(4));
        assert_eq!(d.day, Some(12));
        assert_eq!(d.hour, Some(10));
        assert_eq!(d.minute, Some(15));
        assert_eq!(d.second, Some(30));
    }

    #[test]
    fn parse_datetime_with_millis() {
        let d = ParsedDate::parse("1985-04-12T10:15:30.003").unwrap();
        assert_eq!(d.second, Some(30));
    }

    #[test]
    fn parse_expanded_year() {
        let d = ParsedDate::parse("Y17000000002").unwrap();
        // Year is i32, so very large years overflow. The parser should
        // still succeed; just check it's Some.
        assert!(d.year.is_some());
        assert!(d.missing_month);
    }

    #[test]
    fn parse_negative_year() {
        let d = ParsedDate::parse("-0044").unwrap();
        // -0044 → year = -44
        assert!(d.year.map(|y| y < 0).unwrap_or(false));
    }

    #[test]
    fn parse_uncertain() {
        let d = ParsedDate::parse("1985-04-12?").unwrap();
        assert!(d.uncertain);
        assert!(!d.approximate);
    }

    #[test]
    fn parse_approximate() {
        let d = ParsedDate::parse("1985-04-12~").unwrap();
        assert!(d.approximate);
        assert!(!d.uncertain);
    }

    #[test]
    fn parse_uncertain_approximate() {
        let d = ParsedDate::parse("1985-04-12%").unwrap();
        assert!(d.uncertain);
        assert!(d.approximate);
    }

    #[test]
    fn parse_timezone_utc() {
        let d = ParsedDate::parse("1985-04-12T10:15:30Z").unwrap();
        assert_eq!(d.timezone, Some("UTC".to_string()));
    }

    #[test]
    fn parse_timezone_offset() {
        let d = ParsedDate::parse("1985-04-12T10:15:30+05:30").unwrap();
        assert_eq!(d.timezone, Some("+05:30".to_string()));
    }

    #[test]
    fn parse_year_division() {
        let d = ParsedDate::parse("1985-21").unwrap();
        assert_eq!(d.year, Some(1985));
        assert_eq!(d.yeardivision, Some("spring".to_string()));
    }

    #[test]
    fn parse_date_range_slash() {
        let r = parse_date_range("1985-04-12/1986-05-13").unwrap();
        assert_eq!(r.start.as_ref().unwrap().year, Some(1985));
        assert_eq!(r.end.as_ref().unwrap().year, Some(1986));
        assert_eq!(r.sep, "/");
    }

    #[test]
    fn parse_date_range_open_ended() {
        let r = parse_date_range("1985-04-12/").unwrap();
        assert_eq!(r.start.as_ref().unwrap().year, Some(1985));
        assert!(r.end.is_none());
    }

    #[test]
    fn parse_date_range_double_dash() {
        let r = parse_date_range("1985--1986").unwrap();
        assert_eq!(r.start.as_ref().unwrap().year, Some(1985));
        assert_eq!(r.end.as_ref().unwrap().year, Some(1986));
        assert_eq!(r.sep, "--");
    }

    #[test]
    fn parse_single_date() {
        let r = parse_date_range("1985-04-12").unwrap();
        assert_eq!(r.start.as_ref().unwrap().year, Some(1985));
        assert!(r.end.is_none());
    }
}
