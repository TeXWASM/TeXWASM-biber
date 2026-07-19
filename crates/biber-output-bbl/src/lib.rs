//! `.bbl` output writer.
//!
//! Ported from `lib/Biber/Output/bbl.pm` (809 lines) + `base.pm`. Generates
//! the `.bbl` file that biblatex reads. The format is a series of LaTeX
//! macros: `\refsection`, `\datalist`, `\entry`, `\field`, `\name`, `\strng`,
//! `\true`, `\endentry`, `\enddatalist`, `\endrefsection`, `\endinput`.
//!
//! Generates a well-formed `.bbl` from the processed `Biber` struct.
//! The `<BDS>` placeholders are replaced with real values as the
//! processing passes are completed.

use biber_core::annotation::AnnotationStore;
use biber_core::config::ConfigValue;
use biber_core::constants::BBL_VERSION;
use biber_core::entry::Entry;
use biber_core::latex_recode::{latex_encode_with_set, RecodeSet};
use biber_core::processor::Biber;
use textwrap::core::display_width;

/// Generate the `.bbl` output from a processed `Biber` struct.
pub fn write_bbl(biber: &Biber) -> String {
    let mut out = String::new();

    // Determine safechars encoding parameters
    let safechars = biber
        .config
        .getoption_str("output_safechars")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let encode_set: Option<RecodeSet> = if safechars {
        Some(
            biber
                .config
                .getoption_str("output_safecharsset")
                .map(|s| match s {
                    "null" => RecodeSet::Null,
                    "full" => RecodeSet::Full,
                    _ => RecodeSet::Base,
                })
                .unwrap_or(RecodeSet::Base),
        )
    } else {
        None
    };

    // HEAD
    write_head(&mut out);

    // Per-section output
    for section in biber.sections.get_sections() {
        let secnum = section.number;
        out.push_str(&format!("\n\\refsection{{{secnum}}}\n"));

        // Get datalists for this section, with global sort list last
        let global_ss = biber
            .config
            .getblxoption_str("sortingtemplatename")
            .unwrap_or("nty")
            .to_string();
        let lists = biber.datalists.get_lists_for_section(secnum);

        // Non-global lists first (sorted by sortingtemplatename), then global
        let mut non_global: Vec<&biber_core::datalist::DataList> = lists
            .iter()
            .copied()
            .filter(|l| !(l.r#type == "entry" && l.sortingtemplatename == global_ss))
            .collect();
        non_global.sort_by(|a, b| a.sortingtemplatename.cmp(&b.sortingtemplatename));

        let global_lists: Vec<&biber_core::datalist::DataList> = lists
            .iter()
            .copied()
            .filter(|l| l.r#type == "entry" && l.sortingtemplatename == global_ss)
            .collect();

        let ordered_lists: Vec<&biber_core::datalist::DataList> =
            non_global.into_iter().chain(global_lists).collect();

        for list in ordered_lists {
            if list.state.entries.is_empty() {
                continue;
            }
            out.push_str(&format!("  \\datalist[{}]{{{}}}\n", list.r#type, list.name));

            for key in &list.state.entries {
                if let Some(be) = section.bibentry(key) {
                    let mut entry_out =
                        write_entry(be, section.number, biber, &section.annotations);
                    // Replace BDS placeholders with per-list state values
                    let columns = wraplines_columns(&biber.config);
                    let sortinit = list.state.sortinit.get(key.as_str());
                    let sortinithash = list.state.sortinithash.get(key.as_str());
                    let labelprefix = list.state.labelprefix_data.get(key.as_str());
                    let labelalpha = list.state.labelalphadata.get(key.as_str());
                    let sortlabelalpha = list.state.sortlabelalphadata.get(key.as_str());
                    let extratitle = list.state.extratitledata.get(key.as_str());
                    entry_out = entry_out.replace(
                        "      <BDS>SORTINIT</BDS>\n",
                        &sortinit
                            .map(|v| wrap_field("field", "sortinit", v, columns))
                            .unwrap_or_default(),
                    );
                    entry_out = entry_out.replace(
                        "      <BDS>SORTINITHASH</BDS>\n",
                        &sortinithash
                            .map(|v| wrap_field("field", "sortinithash", v, columns))
                            .unwrap_or_default(),
                    );
                    entry_out = entry_out.replace(
                        "      <BDS>LABELPREFIX</BDS>\n",
                        &labelprefix
                            .map(|v| wrap_field("field", "labelprefix", v, columns))
                            .unwrap_or_default(),
                    );
                    entry_out = entry_out.replace(
                        "      <BDS>LABELALPHA</BDS>\n",
                        &labelalpha
                            .map(|v| wrap_field("field", "labelalpha", v, columns))
                            .unwrap_or_default(),
                    );
                    // Skip sortlabelalpha when identical to labelalpha
                    let sortla = match (labelalpha, sortlabelalpha) {
                        (Some(la), Some(sla)) if la == sla => None,
                        _ => sortlabelalpha,
                    };
                    entry_out = entry_out.replace(
                        "      <BDS>SORTLABELALPHA</BDS>\n",
                        &sortla
                            .map(|v| wrap_field("field", "sortlabelalpha", v, columns))
                            .unwrap_or_default(),
                    );
                    entry_out = entry_out.replace(
                        "      <BDS>EXTRATITLE</BDS>\n",
                        &extratitle
                            .map(|v| wrap_field("field", "extratitle", v, columns))
                            .unwrap_or_default(),
                    );
                    // Apply safechars encoding to entry output
                    if let Some(set) = encode_set {
                        entry_out = latex_encode_with_set(&entry_out, set);
                    }
                    out.push_str(&entry_out);
                }
            }

            out.push_str("  \\enddatalist\n");
        }

        // Key aliases (sorted for deterministic output)
        let mut aliases: Vec<(String, String)> = section
            .get_citekey_aliases()
            .map(|(a, k)| (a.to_string(), k.to_string()))
            .collect();
        aliases.sort();
        for (alias, key) in &aliases {
            out.push_str(&format!("  \\keyalias{{{alias}}}{{{key}}}\n"));
        }

        // Missing keys (sorted)
        let missing: Vec<_> = section.get_undef_citekeys().to_vec();
        for k in &missing {
            out.push_str(&format!("  \\missing{{{k}}}\n"));
        }

        out.push_str("\\endrefsection\n");
    }

    // TAIL
    write_tail(&mut out);

    out
}

/// Write the `.bbl` header.
fn write_head(out: &mut String) {
    out.push_str(&format!(
        "% $ biblatex auxiliary file $\n\
         % $ biblatex bbl format version {BBL_VERSION} $\n\
         % Do not modify the above lines!\n\
         %\n\
         % This is an auxiliary file used by the 'biblatex' package.\n\
         % This file may safely be deleted. It will be recreated by\n\
         % biber as required.\n\
         %\n\
         \\begingroup\n\
         \\makeatletter\n\
         \\@ifundefined{{ver@biblatex.sty}}\n  \
           {{\\@latex@error\n     {{Missing 'biblatex' package}}\n     \
            {{The bibliography requires the 'biblatex' package.}}\n      \
             \\aftergroup\\endinput}}\n  \
           {{}}\n\
         \\endgroup\n\n"
    ));
}

/// Write the `.bbl` tail.
fn write_tail(out: &mut String) {
    out.push_str("\\endinput\n\n");
}

/// Get the wraplines column width from config (0 = disabled).
fn wraplines_columns(config: &biber_core::Config) -> usize {
    config
        .getoption_str("wraplines")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0)
}

/// Format a `\field` or `\strng` line with optional line wrapping, matching
/// the Perl `_printfield` in `bbl.pm`. Three cases:
///
/// 1. Total length > 2×columns: split value onto separate wrapped lines.
/// 2. Total length > columns: wrap the entire `\type{name}{value}` construct.
/// 3. Short enough: single line.
fn wrap_field(field_type: &str, field_name: &str, value: &str, columns: usize) -> String {
    if columns == 0 {
        return format!("      \\{field_type}{{{field_name}}}{{{value}}}\n");
    }

    // 11 = 6 spaces + `\` + `{` + `}` + `{` + `}`
    // For "field" / "strng": 11 + 5 = 16
    let prefix_width = 11 + field_type.len();
    let name_width = display_width(field_name);
    let val_width = display_width(value);
    let total_width = prefix_width + name_width + val_width;

    let indent = "      ";

    if total_width > 2 * columns {
        // Very long: split the value onto multiple wrapped lines
        let wrapped = textwrap::fill(
            value,
            textwrap::Options::new(columns)
                .initial_indent(indent)
                .subsequent_indent(indent),
        );
        format!("      \\{field_type}{{{field_name}}}{{%\n{wrapped}\n%\n      }}\n")
    } else if total_width > columns {
        // Moderate: wrap the entire construct
        let text = format!("\\{field_type}{{{field_name}}}{{{value}}}");
        textwrap::fill(
            &text,
            textwrap::Options::new(columns)
                .initial_indent(indent)
                .subsequent_indent(indent),
        ) + "\n"
    } else {
        format!("      \\{field_type}{{{field_name}}}{{{value}}}\n")
    }
}

/// Write a single entry in `.bbl` format.
fn write_entry(be: &Entry, _secnum: u32, biber: &Biber, annotations: &AnnotationStore) -> String {
    let key = &be.citekey;
    let entrytype = be.get_field_str("entrytype").unwrap_or(&be.entrytype);
    let outtype = entrytype;

    let mut acc = String::new();
    acc.push_str(&format!("    \\entry{{{key}}}{{{outtype}}}{{}}{{}}\n"));

    // Set entries are special
    if entrytype == "set" {
        // Set parents get \entryset
        if let Some(ConfigValue::List(members)) = be.get_field("entryset") {
            let mems: Vec<String> = members
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            acc.push_str(&format!("      \\inset{{{}}}\n", mems.join(",")));
        }
        // Set-specific fields
        if let Some(lab) = be.get_field_str("label") {
            acc.push_str(&format!("      \\field{{label}}{{{lab}}}\n"));
        }
        if let Some(sh) = be.get_field_str("shorthand") {
            acc.push_str(&format!("      \\field{{shorthand}}{{{sh}}}\n"));
        }
        if let Some(ConfigValue::List(kw)) = be.get_field("keywords") {
            let kws: Vec<String> = kw
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            if !kws.is_empty() {
                acc.push_str(&format!("      \\keyw{{{}}}\n", kws.join(",")));
            }
        }
        acc.push_str("    \\endentry\n");
        return acc;
    }

    // Non-set entries: check for entryset membership
    if let Some(ConfigValue::List(members)) = be.get_field("entryset") {
        let _ = members; // Only set entries have entryset; skip for members
    }

    // Output name fields using parsed Name structs
    let name_fields = ["author", "editor", "translator", "bookauthor"];
    for namefield in &name_fields {
        let names = match be.names.get(*namefield) {
            Some(n) if !n.is_empty() => n,
            _ => continue,
        };
        let total = names.count();
        acc.push_str(&format!("      \\name{{{namefield}}}{{{total}}}{{}}{{%\n"));
        for name in names.iter() {
            let hash = biber_core::name::compute_name_hash(name, &biber.config);
            let up = if name.uniquepart.is_empty() {
                "base"
            } else {
                &name.uniquepart
            };
            acc.push_str(&format!(
                "        {{{{un={},uniquepart={},hash={}}}{{%\n",
                name.un, up, hash
            ));
            let namepart_order = ["family", "given", "prefix", "suffix"];
            let mut part_count = 0;
            for np in &namepart_order {
                if let Some(val) = name.get_namepart(np) {
                    part_count += 1;
                    acc.push_str(&format!("           {np}={{{val}}},\n"));
                    if *np == "family" || *np == "given" {
                        let initials = biber_core::name::gen_initials(val);
                        let init_str = initials
                            .iter()
                            .map(|i| format!("{i}\\bibinitperiod"))
                            .collect::<Vec<_>>()
                            .join("");
                        if !init_str.is_empty() {
                            acc.push_str(&format!("           {np}i={{{init_str}}},\n"));
                        }
                    }
                }
            }
            if part_count > 0 {
                let gu = if name.givenun.is_empty() {
                    "0"
                } else {
                    &name.givenun
                };
                acc.push_str(&format!("           givenun={gu}}}}}%\n"));
            } else {
                // Fallback: use rawstring as family
                acc.push_str(&format!(
                    "           family={{{}}},\n           givenun=0}}}}%\n",
                    name.rawstring
                ));
            }
        }
        acc.push_str("      }\n");
    }

    // Output list fields (location, publisher, organization, etc.)
    let list_fields = [
        "location",
        "publisher",
        "organization",
        "institution",
        "origlocation",
        "origpublisher",
    ];
    for listfield in &list_fields {
        if let Some(val) = be.get_field_str(listfield) {
            let items: Vec<&str> = val.split(" and ").collect();
            let total = items.len();
            acc.push_str(&format!("      \\list{{{listfield}}}{{{total}}}{{%\n"));
            for item in &items {
                acc.push_str(&format!("        {{{item}}}%\n"));
            }
            acc.push_str("      }\n");
        }
    }

    // Hashes (stored as entry fields)
    let hash_name_fields = ["author", "editor", "translator", "bookauthor"];
    if let Some(h) = be.get_field_str("namehash") {
        acc.push_str(&format!("      \\strng{{namehash}}{{{h}}}\n"));
    }
    if let Some(h) = be.get_field_str("fullhash") {
        acc.push_str(&format!("      \\strng{{fullhash}}{{{h}}}\n"));
    }
    if let Some(h) = be.get_field_str("fullhashraw") {
        acc.push_str(&format!("      \\strng{{fullhashraw}}{{{h}}}\n"));
    }
    if let Some(h) = be.get_field_str("bibnamehash") {
        acc.push_str(&format!("      \\strng{{bibnamehash}}{{{h}}}\n"));
    }
    for field in &hash_name_fields {
        let key = format!("{field}bibnamehash");
        if let Some(h) = be.get_field_str(&key) {
            acc.push_str(&format!("      \\strng{{{key}}}{{{h}}}\n"));
        }
        let key = format!("{field}namehash");
        if let Some(h) = be.get_field_str(&key) {
            acc.push_str(&format!("      \\strng{{{key}}}{{{h}}}\n"));
        }
        let key = format!("{field}fullhash");
        if let Some(h) = be.get_field_str(&key) {
            acc.push_str(&format!("      \\strng{{{key}}}{{{h}}}\n"));
        }
        let key = format!("{field}fullhashraw");
        if let Some(h) = be.get_field_str(&key) {
            acc.push_str(&format!("      \\strng{{{key}}}{{{h}}}\n"));
        }
    }

    // Labelalpha
    let labelalpha = biber
        .config
        .getblxoption_str("labelalpha")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if labelalpha {
        acc.push_str("      <BDS>LABELALPHA</BDS>\n");
        acc.push_str("      <BDS>SORTLABELALPHA</BDS>\n");
    }

    // Sortinit
    acc.push_str("      <BDS>SORTINIT</BDS>\n");
    acc.push_str("      <BDS>SORTINITHASH</BDS>\n");

    // Extradate (stored as entry field)
    if biber
        .config
        .getblxoption_str("labeldateparts")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        if let Some(val) = be.get_field_str("extradate") {
            acc.push_str(&format!("      \\field{{extradate}}{{{val}}}\n"));
        }
        if let Some(src) = be.get_field_str("labeldatesource") {
            acc.push_str(&format!("      \\field{{labeldatesource}}{{{src}}}\n"));
        }
    }

    // Labelprefix
    if be.get_field_str("shorthand").is_none() {
        acc.push_str("      <BDS>LABELPREFIX</BDS>\n");
    }

    // Labeltitle
    if biber
        .config
        .getblxoption_str("labeltitle")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        acc.push_str("      <BDS>EXTRATITLE</BDS>\n");
    }

    // Uniqueness fields (stored as entry fields)
    if be.has_field("singletitle") {
        acc.push_str("      \\true{singletitle}\n");
    }
    if let Some(val) = be.get_field_str("uniquetitle") {
        acc.push_str(&format!("      \\field{{uniquetitle}}{{{val}}}\n"));
    }
    if let Some(val) = be.get_field_str("uniquebaretitle") {
        acc.push_str(&format!("      \\field{{uniquebaretitle}}{{{val}}}\n"));
    }
    if let Some(val) = be.get_field_str("uniquework") {
        acc.push_str(&format!("      \\field{{uniquework}}{{{val}}}\n"));
    }
    if let Some(val) = be.get_field_str("uniqueprimaryauthor") {
        acc.push_str(&format!("      \\field{{uniqueprimaryauthor}}{{{val}}}\n"));
    }

    // Labelname/labeltitle source
    if let Some(lni) = be.get_field_str("labelname") {
        acc.push_str(&format!("      \\field{{labelnamesource}}{{{lni}}}\n"));
    }
    if let Some(lti) = be.get_field_str("labeltitle") {
        acc.push_str(&format!("      \\field{{labeltitlesource}}{{{lti}}}\n"));
    }

    // Output regular fields
    // Skip fields that are output specially or are metadata
    let skip_fields = [
        "entrytype",
        "citekey",
        "datatype",
        "entryset",
        "labelname",
        "labeltitle",
        "labelyear",
        "labeldatesource",
        "crossrefsource",
        "xrefsource",
        "nocite",
        "options",
        "rawdata",
        "warnings",
        "author",
        "editor",
        "translator",
        "bookauthor",
        "location",
        "publisher",
        "organization",
        "institution",
        "origlocation",
        "origpublisher",
        "keywords",
        // Hashes (emitted via \strng above)
        "namehash",
        "fullhash",
        "fullhashraw",
        "bibnamehash",
        // Uniqueness fields (emitted via \true or \field above)
        "singletitle",
        "uniquetitle",
        "uniquebaretitle",
        "uniquework",
        "uniqueprimaryauthor",
        // Extradate (emitted via \field above)
        "extradate",
        // Per-field hashes (emitted via \strng above)
        "authornamehash",
        "authorfullhash",
        "authorfullhashraw",
        "authorbibnamehash",
        "editornamehash",
        "editorfullhash",
        "editorfullhashraw",
        "editorbibnamehash",
        "translatornamehash",
        "translatorfullhash",
        "translatorfullhashraw",
        "translatorbibnamehash",
        "bookauthornamehash",
        "bookauthorfullhash",
        "bookauthorfullhashraw",
        "bookauthorbibnamehash",
        // Internal fields that should not appear in output
        "ids",
        "seenname",
        "sortshorthand",
        "sortinit",
        "sortinithash",
        "labelalpha",
        "sortlabelalpha",
    ];

    let columns = wraplines_columns(&biber.config);

    for (field, value) in &be.fields {
        let field = field.as_str();
        if skip_fields.contains(&field) {
            continue;
        }
        if let Some(val) = value.as_str() {
            if val.is_empty() {
                continue;
            }
            // crossref/xref use \strng
            if field == "crossref" || field == "xref" {
                acc.push_str(&wrap_field("strng", field, val, columns));
            } else if field == "labelyear" {
                acc.push_str(&wrap_field("field", "labelyear", val, columns));
            } else if field == "shorthand" {
                acc.push_str(&wrap_field("field", "shorthand", val, columns));
            } else {
                acc.push_str(&wrap_field("field", field, val, columns));
            }
        }
    }

    // nocite
    if be.get_field_str("nocite").is_some() {
        acc.push_str("      \\true{nocite}\n");
    }

    // crossrefsource
    if be.get_field_str("crossrefsource").is_some() {
        acc.push_str("      \\true{crossrefsource}\n");
    }

    // Keywords
    if let Some(ConfigValue::List(kw)) = be.get_field("keywords") {
        let kws: Vec<String> = kw
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        if !kws.is_empty() {
            acc.push_str(&format!("      \\keyw{{{}}}\n", kws.join(",")));
        }
    }

    // Annotations
    if annotations.has_annotations(key) {
        // Field-scope annotations
        for (_, field, name, ann) in annotations.iter_field().filter(|(ck, _, _, _)| *ck == key) {
            let lit = if ann.literal { "1" } else { "0" };
            acc.push_str(&format!(
                "      \\annotation{{field}}{{{field}}}{{{name}}}{{}}{{}}{{{lit}}}{{{}}}\n",
                ann.value
            ));
        }
        // Item-scope annotations
        for (_, field, name, count, ann) in annotations
            .iter_item()
            .filter(|(ck, _, _, _, _)| *ck == key)
        {
            let lit = if ann.literal { "1" } else { "0" };
            acc.push_str(&format!(
                "      \\annotation{{item}}{{{field}}}{{{name}}}{{{count}}}{{}}{{{lit}}}{{{}}}\n",
                ann.value
            ));
        }
        // Part-scope annotations
        for (_, field, name, count, part, ann) in annotations
            .iter_part()
            .filter(|(ck, _, _, _, _, _)| *ck == key)
        {
            let lit = if ann.literal { "1" } else { "0" };
            acc.push_str(&format!(
                "      \\annotation{{part}}{{{field}}}{{{name}}}{{{count}}}{{{part}}}{{{lit}}}{{{}}}\n",
                ann.value
            ));
        }
    }

    acc.push_str("    \\endentry\n");
    acc
}

#[cfg(test)]
mod tests {
    use super::*;
    use biber_core::entry::Entry;
    use biber_core::section::Section;
    use std::collections::BTreeMap;

    #[test]
    fn write_bbl_for_empty_biber() {
        let biber = Biber::new();
        let bbl = write_bbl(&biber);
        assert!(bbl.contains("biblatex bbl format version"));
        assert!(bbl.contains("\\endinput"));
    }

    #[test]
    fn write_bbl_for_simple_entry() {
        let mut biber = Biber::new();
        // Set sortingtemplatename to match the datalist
        biber
            .config
            .setblxoption(None, "sortingtemplatename", "nty".into());
        let mut section = Section::new(0);
        section.add_cite("smith2020");

        let mut entry = Entry::new("smith2020", "book");
        entry.set_field_str("citekey", "smith2020");
        entry.set_field_str("entrytype", "book");
        entry.set_field_str("author", "John Smith");
        entry.set_field_str("title", "A Book Title");
        entry.set_field_str("year", "2020");
        entry.set_field_str("labelname", "author");
        entry.set_field_str("labeltitle", "title");
        entry.set_field_str("labelyear", "2020");
        entry.set_field_str("labeldatesource", "year");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);

        // Add a datalist
        let dl = biber_core::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);

        // Run the pipeline to populate the datalist entries
        biber_core::pipeline::prepare(&mut biber);

        let bbl = write_bbl(&biber);

        assert!(bbl.contains("\\refsection{0}"));
        assert!(bbl.contains("\\datalist[entry]{nty/global//global/global/global}"));
        assert!(bbl.contains("\\entry{smith2020}{book}{}{}"));
        assert!(bbl.contains("\\name{author}{1}{}{%"));
        assert!(bbl.contains("\\field{title}{A Book Title}"));
        assert!(bbl.contains("\\field{year}{2020}"));
        assert!(bbl.contains("\\field{labelnamesource}{author}"));
        assert!(bbl.contains("\\field{labeltitlesource}{title}"));
        // labelyear is set by the pipeline, not the writer
        assert!(bbl.contains("\\endentry"));
        assert!(bbl.contains("\\enddatalist"));
        assert!(bbl.contains("\\endrefsection"));
        assert!(bbl.contains("\\endinput"));
    }

    #[test]
    fn bbl_version_matches_constants() {
        let biber = Biber::new();
        let bbl = write_bbl(&biber);
        assert!(bbl.contains(&format!("version {BBL_VERSION}")));
    }

    #[test]
    fn write_bbl_has_no_bds_tags() {
        let mut biber = Biber::new();
        biber
            .config
            .setblxoption(None, "sortingtemplatename", "nty".into());
        let mut section = Section::new(0);
        section.add_cite("test2020");

        let mut entry = Entry::new("test2020", "article");
        entry.set_field_str("author", "Jane Doe");
        entry.set_field_str("title", "A Test");
        entry.set_field_str("year", "2020");
        entry.set_field_str("labelname", "author");
        entry.set_field_str("labeltitle", "title");
        entry.set_field_str("labelyear", "2020");
        // Add an extradate field (as set by pipeline)
        entry.set_field_str("extradate", "a");
        // Add uniqueness fields (as set by pipeline)
        entry.set_field_str("singletitle", "1");
        entry.set_field_str("uniquetitle", "1");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);

        // Set up namehashtemplate so names get proper hashes
        let template = ConfigValue::Map(BTreeMap::from([(
            "global".to_string(),
            ConfigValue::List(vec![
                ConfigValue::Map(BTreeMap::from([
                    (
                        "namepart".to_string(),
                        ConfigValue::Str("family".to_string()),
                    ),
                    ("hashscope".to_string(), ConfigValue::Str("1".to_string())),
                ])),
                ConfigValue::Map(BTreeMap::from([
                    (
                        "namepart".to_string(),
                        ConfigValue::Str("given".to_string()),
                    ),
                    ("hashscope".to_string(), ConfigValue::Str("1".to_string())),
                ])),
            ]),
        )]));
        biber
            .config
            .setblxoption(None, "namehashtemplate", template);
        // Enable labelalpha
        biber.config.setblxoption(None, "labelalpha", "true".into());

        let dl = biber_core::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);

        biber_core::pipeline::prepare(&mut biber);

        let bbl = write_bbl(&biber);

        // No BDS placeholders in the output
        assert!(
            !bbl.contains("<BDS>"),
            "BBL output should not contain any <BDS> tags"
        );

        // Hash macros are emitted (not zero-filled)
        assert!(bbl.contains("\\strng{namehash}"));
        assert!(bbl.contains("\\strng{fullhash}"));

        // Name output has real hash (not all zeros)
        assert!(
            !bbl.contains("hash=00000000000000000000000000000000"),
            "Name hashes should be real, not zero-filled"
        );

        // Uniqueness fields emitted inline (not as BDS)
        assert!(bbl.contains("\\true{singletitle}"));
        assert!(bbl.contains("\\field{uniquetitle}"));
        assert!(bbl.contains("\\field{extradate}"));

        // All remaining BDS tags in write_entry (SORTINIT, LABELPREFIX, etc.)
        // are replaced in write_bbl's post-processing loop (to real values or empty).
        assert!(!bbl.contains("<BDS>"), "no literal <BDS> tags in output");
    }

    #[test]
    fn safechars_encodes_unicode_in_bbl() {
        let mut biber = Biber::new();
        biber.config.setoption_str("output_safechars", "1");
        biber.config.setoption_str("output_safecharsset", "base");
        biber
            .config
            .setblxoption(None, "sortingtemplatename", "nty".into());
        let mut section = Section::new(0);
        section.add_cite("muller2020");

        let mut entry = Entry::new("muller2020", "book");
        entry.set_field_str("citekey", "muller2020");
        entry.set_field_str("entrytype", "book");
        entry.set_field_str("author", "M\u{fc}ller"); // Müller with ü
        entry.set_field_str("title", "A \u{fc}ber Title"); // über
        entry.set_field_str("year", "2020");
        entry.set_field_str("labelname", "author");
        entry.set_field_str("labeltitle", "title");
        entry.set_field_str("labelyear", "2020");
        entry.set_field_str("labeldatesource", "year");
        section.bibentries.add_entry(entry);
        biber.sections.add_section(section);

        let dl = biber_core::datalist::DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        biber.datalists.add_list(dl);

        biber_core::pipeline::prepare(&mut biber);
        let bbl = write_bbl(&biber);

        // The ü (NFD: u + combining diaeresis) should be encoded to \"u
        assert!(
            bbl.contains("\\\"{u}") || bbl.contains("\\\"u"),
            "Expected ü to be encoded to LaTeX macro, got: ...{}...",
            &bbl[bbl.find("Title").unwrap_or(0)..]
        );
    }
}
