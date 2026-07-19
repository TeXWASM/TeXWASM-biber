//! Graphviz `.dot` output writer.
//!
//! Ported from `lib/Biber/Output/dot.pm` (384 lines). Generates a
//! Graphviz `digraph` visualising the citation relationships between
//! entries: crossrefs, xdata, entry sets, and related entries.

use std::collections::HashMap;

use biber_core::config::ConfigValue;
use biber_core::entry::Entry;
use biber_core::processor::Biber;

/// Graph edge types.
#[derive(Debug, Clone, PartialEq, Eq)]
enum EdgeType {
    Crossref,
    Xdata,
    Entryset,
    Related,
    RelatedClone,
}

/// A graph edge between two entry nodes.
#[derive(Debug, Clone)]
struct Edge {
    source: String,
    target: String,
    edge_type: EdgeType,
}

/// Generate the `.dot` (Graphviz) output from a processed `Biber` struct.
pub fn write_dot(biber: &Biber) -> String {
    let mut out = String::new();

    out.push_str("digraph Biberdata {\n");
    out.push_str("  rankdir=LR;\n");
    out.push_str("  node [shape=record style=filled];\n\n");

    // Determine what to include
    let dot_include = parse_dot_include(biber);

    for section in biber.sections.get_sections() {
        let secnum = section.number;
        out.push_str(&format!("  subgraph \"cluster_section{secnum}\" {{\n"));
        out.push_str(&format!("    label = \"Section {secnum}\";\n"));
        out.push_str("    style=filled;\n");
        out.push_str("    fillcolor=lightgrey;\n");
        out.push_str("    color=grey;\n\n");

        // Collect entryset groups
        let entryset_groups = if dot_include.entryset {
            collect_entryset_groups(section)
        } else {
            HashMap::new()
        };

        // Build all cited keys for citation-status coloring
        let cited_keys: Vec<String> = section.get_citekeys().to_vec();

        // Group entries by set
        let mut in_set: HashMap<String, Option<String>> = HashMap::new();
        for (set_key, members) in &entryset_groups {
            for member in members {
                in_set.insert(member.clone(), Some(set_key.clone()));
            }
        }

        // Write set subgraphs
        let mut written_sets: Vec<String> = Vec::new();
        for (set_key, members) in &entryset_groups {
            written_sets.push(set_key.clone());
            out.push_str(&format!(
                "    subgraph \"cluster_{secnum}/set_{set_key}\" {{\n"
            ));
            out.push_str("      style=filled;\n");
            out.push_str("      fillcolor=palegoldenrod;\n");
            out.push_str("      color=goldenrod;\n");
            out.push_str(&format!("      label = \"entryset: {set_key}\";\n\n"));

            for member in members {
                if let Some(be) = section.bibentry(member) {
                    let node_label = make_node_label(be, &cited_keys, member);
                    out.push_str(&format!("      \"{secnum}/{member}\" {node_label}\n"));
                }
            }
            out.push_str("    }\n\n");
        }

        // Write remaining (non-set) entries
        for key in &cited_keys {
            if in_set.get(key).is_some_and(|s| s.is_some()) {
                continue;
            }
            if let Some(be) = section.bibentry(key) {
                let node_label = make_node_label(be, &cited_keys, key);
                out.push_str(&format!("    \"{secnum}/{key}\" {node_label}\n"));
            }
        }

        // Write uncited entries that are not in sets
        for (key, be) in section.bibentries.entries() {
            if cited_keys.iter().any(|ck| ck == key) {
                continue;
            }
            if in_set.get(key).is_some_and(|s| s.is_some()) {
                continue;
            }
            let node_label = make_node_label(be, &cited_keys, key);
            out.push_str(&format!("    \"{secnum}/{key}\" {node_label}\n"));
        }

        out.push_str("  }\n\n");

        // Edges
        let edges = collect_edges(section, &dot_include);
        for edge in &edges {
            write_edge(&mut out, edge, secnum);
        }
    }

    out.push_str("}\n");
    out
}

/// Configuration parsed from the `dot_include` biber option.
struct DotInclude {
    section: bool,
    xdata: bool,
    crossref: bool,
    entryset: bool,
    related: bool,
}

fn parse_dot_include(biber: &Biber) -> DotInclude {
    let raw = biber.config.getoption("dot_include");
    let mut di = DotInclude {
        section: true,
        xdata: true,
        crossref: true,
        entryset: true,
        related: false,
    };
    if let Some(ConfigValue::Raw(s)) = raw {
        if s.contains("section") && s.contains("0") {
            di.section = false;
        }
        if s.contains("xdata") && s.contains("0") {
            di.xdata = false;
        }
        if s.contains("crossref") && s.contains("0") {
            di.crossref = false;
        }
        if s.contains("entryset") && s.contains("1") {
            di.entryset = true;
        }
        if s.contains("related") && s.contains("1") {
            di.related = true;
        }
    }
    di
}

fn make_node_label(be: &Entry, cited_keys: &[String], key: &str) -> String {
    let is_cited = cited_keys.iter().any(|ck| ck == key);
    let fillcolor = if is_cited { "lightblue" } else { "azure2" };
    let label = format!(
        "{} | type: {} | {}",
        be.citekey,
        be.entrytype,
        be.get_field_str("crossref")
            .map(|cr| format!("crossref: {}", cr))
            .unwrap_or_default()
    );
    format!(
        "[label=\"{}\" fillcolor={}]",
        label.replace('\"', "\\\""),
        fillcolor
    )
}

fn collect_entryset_groups(section: &biber_core::section::Section) -> HashMap<String, Vec<String>> {
    let mut groups: HashMap<String, Vec<String>> = HashMap::new();
    for (_key, be) in section.bibentries.entries() {
        if let Some(es) = be.get_field("entryset") {
            let members: Vec<String> = match es {
                ConfigValue::List(list) => list
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect(),
                ConfigValue::Str(s) => s.split(',').map(|s| s.trim().to_string()).collect(),
                _ => continue,
            };
            for member in &members {
                if !member.is_empty() {
                    groups
                        .entry(_key.to_string())
                        .or_default()
                        .push(member.clone());
                }
            }
        }
    }
    groups
}

fn collect_edges(section: &biber_core::section::Section, di: &DotInclude) -> Vec<Edge> {
    let mut edges = Vec::new();

    for (key, be) in section.bibentries.entries() {
        // Crossref edges
        if di.crossref {
            if let Some(cr) = be.get_field_str("crossref") {
                if !cr.is_empty() && section.bibentry(cr).is_some() {
                    edges.push(Edge {
                        source: key.to_string(),
                        target: cr.to_string(),
                        edge_type: EdgeType::Crossref,
                    });
                }
            }
        }

        // Xdata edges
        if di.xdata {
            if let Some(xd) = be.get_field_str("xdata") {
                for xref in xd.split(',').map(|s| s.trim()) {
                    if !xref.is_empty() && section.bibentry(xref).is_some() {
                        edges.push(Edge {
                            source: key.to_string(),
                            target: xref.to_string(),
                            edge_type: EdgeType::Xdata,
                        });
                    }
                }
            }
        }

        // Entryset edges (member → set)
        if di.entryset {
            if let Some(es) = be.get_field("entryset") {
                let members: Vec<String> = match es {
                    ConfigValue::List(list) => list
                        .iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect(),
                    ConfigValue::Str(s) => s.split(',').map(|s| s.trim().to_string()).collect(),
                    _ => Vec::new(),
                };
                for member in members {
                    if !member.is_empty() && section.bibentry(&member).is_some() {
                        edges.push(Edge {
                            source: key.to_string(),
                            target: member,
                            edge_type: EdgeType::Entryset,
                        });
                    }
                }
            }
        }

        // Related edges
        if di.related {
            if let Some(related_val) = be.get_field_str("related") {
                for clone_key in related_val
                    .split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                {
                    if section.bibentry(clone_key).is_some() {
                        // Clone → parent (related link)
                        edges.push(Edge {
                            source: clone_key.to_string(),
                            target: key.to_string(),
                            edge_type: EdgeType::Related,
                        });
                    }
                    if let Some(orig_key) = section.get_relclonetokey(clone_key) {
                        if section.bibentry(orig_key).is_some() {
                            // Original related entry → clone (clone link)
                            edges.push(Edge {
                                source: orig_key.to_string(),
                                target: clone_key.to_string(),
                                edge_type: EdgeType::RelatedClone,
                            });
                        }
                    }
                }
            }
        }
    }

    edges
}

fn write_edge(out: &mut String, edge: &Edge, secnum: u32) {
    let (color, style, label) = match edge.edge_type {
        EdgeType::Crossref => ("#7d7879", "solid", "crossref"),
        EdgeType::Xdata => ("#2ca314", "solid", "xdata"),
        EdgeType::Entryset => ("#ff8c00", "dashed", "entryset"),
        EdgeType::Related => ("#ad1741", "solid", "related"),
        EdgeType::RelatedClone => ("#ad1741", "dashed", "clone"),
    };
    out.push_str(&format!(
        "  \"{secnum}/{src}\" -> \"{secnum}/{tgt}\" [color=\"{color}\" style=\"{style}\" label=\"{label}\"]\n",
        src = edge.source,
        tgt = edge.target,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use biber_core::datalist::DataList;
    use biber_core::entry::Entry;
    use biber_core::processor::Biber;
    use biber_core::section::Section;

    type EntrySpec<'a> = (&'a str, &'a str, Vec<(&'a str, &'a str)>);

    fn make_biber_with_entries(entries: Vec<EntrySpec>) -> Biber {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.set_allkeys(true);
        let mut cited = Vec::new();
        for (key, etype, fields) in entries {
            let mut entry = Entry::new(key, etype);
            for (k, v) in fields {
                entry.set_field_str(k, v);
            }
            section.bibentries.add_entry(entry);
            cited.push(key.to_string());
        }
        section.add_citekeys(cited.clone());
        biber.sections.add_section(section);
        let mut dl = DataList::new(
            0,
            "nty",
            "global",
            "global",
            "global",
            "global",
            "",
            "nty/global//global/global/global",
        );
        dl.state.entries = cited;
        biber.datalists.add_list(dl);
        biber
    }

    #[test]
    fn empty_biber_returns_basic_digraph() {
        let biber = Biber::new();
        let result = write_dot(&biber);
        assert!(result.starts_with("digraph Biberdata {"));
        assert!(result.ends_with("}\n"));
    }

    #[test]
    fn simple_entry_creates_node() {
        let biber = make_biber_with_entries(vec![("smith2020", "book", vec![("title", "A Book")])]);
        let result = write_dot(&biber);
        assert!(result.contains("smith2020"));
        assert!(result.contains("book"));
        assert!(result.contains("fillcolor=lightblue"));
    }

    #[test]
    fn crossref_creates_edge() {
        let biber = make_biber_with_entries(vec![
            ("child", "article", vec![("crossref", "parent")]),
            ("parent", "book", vec![]),
        ]);
        let result = write_dot(&biber);
        assert!(result.contains("crossref"));
        assert!(result.contains("#7d7879"));
        assert!(result.contains("child"));
        assert!(result.contains("parent"));
    }

    #[test]
    fn uncited_entry_is_azure() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.set_allkeys(true);
        let entry = Entry::new("uncited1", "misc");
        section.bibentries.add_entry(entry);
        // Don't add to citekeys
        biber.sections.add_section(section);
        let result = write_dot(&biber);
        assert!(result.contains("uncited1"));
        assert!(result.contains("fillcolor=azure2"));
    }

    #[test]
    fn entryset_group_creates_subgraph() {
        let mut biber = Biber::new();
        let mut section = Section::new(0);
        section.set_allkeys(true);
        let mut set_entry = Entry::new("set1", "set");
        set_entry.set_field_str("entryset", "child1,child2");
        section.bibentries.add_entry(set_entry);
        let child1 = Entry::new("child1", "article");
        section.bibentries.add_entry(child1);
        let child2 = Entry::new("child2", "article");
        section.bibentries.add_entry(child2);
        section.add_citekeys(vec![
            "set1".to_string(),
            "child1".to_string(),
            "child2".to_string(),
        ]);
        biber.sections.add_section(section);
        let result = write_dot(&biber);
        assert!(result.contains("entryset: set1"));
        assert!(result.contains("palegoldenrod"));
    }

    #[test]
    fn dot_include_defaults() {
        let biber = Biber::new();
        let di = parse_dot_include(&biber);
        assert!(di.section);
        assert!(di.xdata);
        assert!(di.crossref);
        assert!(di.entryset);
    }

    #[test]
    fn xdata_creates_edge() {
        let biber = make_biber_with_entries(vec![
            ("source", "article", vec![("xdata", "xdata1")]),
            ("xdata1", "xdata", vec![]),
        ]);
        let result = write_dot(&biber);
        assert!(result.contains("#2ca314"));
        assert!(result.contains("xdata"));
    }
}
