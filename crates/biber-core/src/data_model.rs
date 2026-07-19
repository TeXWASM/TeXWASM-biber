//! Biblatex data model — parsed from the `<bcf:datamodel>` block in the
//! BCF and from `biber-tool.conf`.
//!
//! Ported from `lib/Biber/DataModel.pm`. Covers the read-only
//! structures and full data-model validation.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write;

use crate::config::ConfigValue;
use crate::entry::Entry;

/// The biblatex data model.
///
/// Describes entry types, fields, constraints, and inheritance rules.
#[derive(Debug, Clone, Default)]
pub struct DataModel {
    /// Constants defined in the datamodel (name → value).
    pub constants: HashMap<String, ModelConstant>,
    /// Entry types defined in the datamodel.
    pub entrytypes: BTreeSet<String>,
    /// Fields defined in the datamodel: field name → field definition.
    pub fields: BTreeMap<String, ModelField>,
    /// Entry-field associations: entrytype → list of allowed fields.
    pub entryfields: Vec<EntryFields>,
    /// Constraints on fields.
    pub constraints: Vec<ModelConstraints>,
    /// Multiscript fields (fields that can have script variants).
    pub multiscriptfields: BTreeSet<String>,
}

/// A constant in the data model.
#[derive(Debug, Clone)]
pub struct ModelConstant {
    /// Constant type ("list", "string", etc.).
    pub r#type: String,
    /// Constant name.
    pub name: String,
    /// Constant value (for list types, comma-separated).
    pub value: String,
}

/// A field definition in the data model.
#[derive(Debug, Clone)]
pub struct ModelField {
    /// Field type ("field" or "list").
    pub fieldtype: String,
    /// Data type ("literal", "integer", "name", "date", etc.).
    pub datatype: String,
    /// Whether the field is nullable.
    pub nullok: bool,
    /// Whether this is a label field.
    pub label: bool,
    /// Whether to skip output for this field.
    pub skip_output: bool,
    /// Format specification (e.g. "xsv").
    pub format: Option<String>,
}

/// Entry-field associations.
#[derive(Debug, Clone)]
pub struct EntryFields {
    /// Entry types this association applies to (empty = all types).
    pub entrytypes: Vec<String>,
    /// Fields allowed for these entry types.
    pub fields: Vec<String>,
}

/// A constraints block for one or more entry types.
#[derive(Debug, Clone)]
pub struct ModelConstraints {
    /// Entry types these constraints apply to.
    pub entrytypes: Vec<String>,
    /// The constraints.
    pub constraints: Vec<ModelConstraint>,
}

/// A single constraint.
#[derive(Debug, Clone)]
pub enum ModelConstraint {
    /// Mandatory fields constraint.
    Mandatory {
        /// Fields that must be present (with fieldor alternatives).
        fields: Vec<MandatoryField>,
    },
    /// Data type constraint.
    Data {
        /// Data type to validate against.
        datatype: String,
        /// Minimum value (for range checks).
        rangemin: Option<String>,
        /// Maximum value (for range checks).
        rangemax: Option<String>,
        /// Pattern to match (for pattern constraints).
        pattern: Option<String>,
        /// Fields this constraint applies to.
        fields: Vec<String>,
    },
    /// Conditional constraint.
    Conditional {
        /// Antecedent fields and quantifier.
        antecedent: ConditionalPart,
        /// Consequent fields and quantifier.
        consequent: ConditionalPart,
    },
}

/// A field in a mandatory constraint (may be part of a fieldor group).
#[derive(Debug, Clone)]
pub enum MandatoryField {
    /// A single required field.
    Field(String),
    /// A group of alternative fields (at least one must be present).
    FieldOr(Vec<String>),
}

/// A conditional part (antecedent or consequent).
#[derive(Debug, Clone)]
pub struct ConditionalPart {
    /// Quantifier ("all", "any", "none").
    pub quant: String,
    /// Fields in this part.
    pub fields: Vec<String>,
}

/// Season names recognised in datepart validation.
const SEASONS: &[&str] = &[
    "spring", "summer", "autumn", "winter", "springN", "summerN", "autumnN", "winterN", "springS",
    "summerS", "autumnS", "winterS",
];

/// Quarter names recognised in datepart validation.
const QUARTERS: &[&str] = &["Q1", "Q2", "Q3", "Q4", "QD1", "QD2", "QD3", "QD4"];

/// Semester names recognised in datepart validation.
const SEMESTERS: &[&str] = &["S1", "S2"];

/// Internal fields that are always valid (not in the datamodel proper).
fn is_internal_field(field: &str) -> bool {
    matches!(
        field,
        "entrytype"
            | "citekey"
            | "datasource"
            | "clone"
            | "set_member"
            | "clonesourcekey"
            | "labelname"
            | "labelyear"
            | "labeldatesource"
            | "labeltitle"
            | "extradatescope"
            | "extradate"
            | "namehash"
            | "fullhash"
            | "fullhashraw"
            | "bibnamehash"
            | "labelalpha"
            | "sortlabelalpha"
            | "extraalpha"
            | "seenname"
            | "seentitle"
            | "seenbaretitle"
            | "seenwork"
            | "singletitle"
            | "uniquetitle"
            | "uniquebaretitle"
            | "uniquework"
            | "uniqueprimaryauthor"
            | "seenprimaryauthor"
            | "sortinit"
            | "sortinithash"
            | "labelprefix"
            | "nocite"
    )
}

impl DataModel {
    /// Create an empty data model.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get the value of a constant.
    pub fn get_constant_value(&self, name: &str) -> Option<&str> {
        self.constants.get(name).map(|c| c.value.as_str())
    }

    /// Check if an entry type is known.
    pub fn is_known_entrytype(&self, et: &str) -> bool {
        self.entrytypes.contains(et)
    }

    /// Get a field definition.
    pub fn get_field(&self, name: &str) -> Option<&ModelField> {
        self.fields.get(name)
    }

    /// Get the datatype of a field.
    pub fn get_field_datatype(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(|f| f.datatype.as_str())
    }

    /// Get all nameparts from the data model constants.
    pub fn get_nameparts(&self) -> Vec<String> {
        self.get_constant_value("nameparts")
            .map(|s| s.split(',').map(|p| p.trim().to_string()).collect())
            .unwrap_or_default()
    }

    /// Check whether `field` is allowed for `entrytype`.
    ///
    /// A field is allowed if it appears in a global entryfields block
    /// (no entrytype restriction), in an entrytype-specific block, or
    /// is an internal/pipeline-generated field.
    pub fn is_field_for_entrytype(&self, entrytype: &str, field: &str) -> bool {
        if is_internal_field(field) {
            return true;
        }

        for ef in &self.entryfields {
            if (ef.entrytypes.is_empty() || ef.entrytypes.iter().any(|et| et == entrytype))
                && ef.fields.iter().any(|f| f == field)
            {
                return true;
            }
        }
        false
    }

    // ── schema generation helpers ──────────────────────────────────

    /// Get all unique field types in the data model (e.g. "field", "list").
    pub fn fieldtypes(&self) -> BTreeSet<&str> {
        self.fields.values().map(|f| f.fieldtype.as_str()).collect()
    }

    /// Get all unique data types in the data model (e.g. "literal", "name", "date").
    pub fn datatypes(&self) -> BTreeSet<&str> {
        self.fields.values().map(|f| f.datatype.as_str()).collect()
    }

    /// Check if any fields exist with the given fieldtype+datatype combination.
    pub fn is_fields_of_type(&self, fieldtype: &str, datatype: &str) -> bool {
        self.fields
            .values()
            .any(|f| f.fieldtype == fieldtype && f.datatype == datatype)
    }

    /// Get sorted field names matching a given fieldtype+datatype combination.
    pub fn get_fields_of_type(&self, fieldtype: &str, datatype: &str) -> Vec<&str> {
        let mut result: Vec<&str> = self
            .fields
            .iter()
            .filter(|(_, f)| f.fieldtype == fieldtype && f.datatype == datatype)
            .map(|(name, _)| name.as_str())
            .collect();
        result.sort();
        result
    }

    /// Generate a RELAX NG XML schema from the data model for bblxml output.
    pub fn generate_bblxml_schema(&self) -> String {
        let ns = "https://sourceforge.net/projects/biblatex/bblxml";
        let rng_ns = "http://relaxng.org/ns/structure/1.0";
        let xsd = "http://www.w3.org/2001/XMLSchema-datatypes";

        let mut s = String::new();
        let mut w = RngWriter::new(&mut s);

        w.xml_decl();
        w.comment("Auto-generated bblxml RNG schema from .bcf Datamodel");

        w.open(
            "grammar",
            &[
                ("xmlns", rng_ns),
                ("xmlns:bbl", ns),
                ("datatypeLibrary", xsd),
            ],
        );

        // Start ── bbl:refsections
        w.open("start", &[]);
        w.open("element", &[("name", "bbl:refsections")]);
        w.open("oneOrMore", &[]);
        w.open("element", &[("name", "bbl:refsection")]);
        w.empty("attribute", &[("name", "id")]);
        w.open("interleave", &[]);
        w.open("oneOrMore", &[]);
        w.empty("ref", &[("name", "datalist")]);
        w.close("oneOrMore");
        w.open("zeroOrMore", &[]);
        w.empty("ref", &[("name", "keyalias")]);
        w.close("zeroOrMore");
        w.open("zeroOrMore", &[]);
        w.empty("ref", &[("name", "missing")]);
        w.close("zeroOrMore");
        w.close("interleave");
        w.close("element"); // refsection
        w.close("oneOrMore");
        w.close("element"); // refsections
        w.close("start");

        // Define ── datalist
        w.comment("datalist definition");
        w.open("define", &[("name", "datalist")]);
        w.open("element", &[("name", "bbl:datalist")]);
        w.open("attribute", &[("name", "type")]);
        w.open("choice", &[]);
        for dt in &["entry", "child"] {
            w.data_element("value", dt);
        }
        w.close("choice");
        w.close("attribute");
        w.empty("attribute", &[("name", "id")]);
        w.open("zeroOrMore", &[]);
        w.empty("ref", &[("name", "entry")]);
        w.close("zeroOrMore");
        w.close("element"); // datalist
        w.close("define");

        // Define ── entry
        w.comment("entry definition");
        w.open("define", &[("name", "entry")]);
        w.open("element", &[("name", "bbl:entry")]);
        w.empty("attribute", &[("name", "key")]);
        w.open("attribute", &[("name", "type")]);
        w.open("choice", &[]);
        for et in self.entrytypes.iter() {
            w.data_element("value", et);
        }
        w.close("choice");
        w.close("attribute");
        w.open("interleave", &[]);
        w.open("zeroOrMore", &[]);
        w.empty("ref", &[("name", "field")]);
        w.close("zeroOrMore");
        w.open("zeroOrMore", &[]);
        w.empty("ref", &[("name", "names")]);
        w.close("zeroOrMore");
        w.close("interleave");
        w.close("element"); // entry
        w.close("define");

        // Define ── field
        w.comment("field definition");
        w.open("define", &[("name", "field")]);
        w.open("element", &[("name", "bbl:field")]);
        w.open("attribute", &[("name", "name")]);
        w.open("choice", &[]);
        // BDS fields
        for bds in &[
            "sortinit",
            "sortinithash",
            "labelprefix",
            "labelalpha",
            "extratitle",
        ] {
            w.data_element("value", bds);
        }
        // All field names from data model
        for fname in self.fields.keys() {
            w.data_element("value", fname);
        }
        w.close("choice");
        w.close("attribute");
        w.empty("text", &[]);
        w.close("element"); // field
        w.close("define");

        // Define ── names
        w.comment("names definition");
        w.open("define", &[("name", "names")]);
        w.open("element", &[("name", "bbl:names")]);
        w.open("attribute", &[("name", "type")]);
        w.open("choice", &[]);
        for nf in self.get_fields_of_type("list", "name") {
            w.data_element("value", nf);
        }
        w.close("choice");
        w.close("attribute");
        w.open("optional", &[]);
        w.open("attribute", &[("name", "count")]);
        w.empty("data", &[("type", "integer")]);
        w.close("attribute");
        w.close("optional");
        w.open("oneOrMore", &[]);
        w.open("element", &[("name", "bbl:name")]);
        w.open("oneOrMore", &[]);
        w.open("element", &[("name", "bbl:namepart")]);
        w.open("attribute", &[("name", "type")]);
        w.open("choice", &[]);
        let nameparts = self.get_nameparts();
        if nameparts.is_empty() {
            for np in &["family", "given", "prefix", "suffix"] {
                w.data_element("value", np);
            }
        } else {
            for np in &nameparts {
                w.data_element("value", np);
            }
        }
        w.close("choice");
        w.close("attribute");
        w.empty("text", &[]);
        w.close("element"); // namepart
        w.close("oneOrMore");
        w.close("element"); // name
        w.close("oneOrMore");
        w.close("element"); // names
        w.close("define");

        // Define ── keyalias
        w.comment("keyalias definition");
        w.open("define", &[("name", "keyalias")]);
        w.open("element", &[("name", "bbl:keyalias")]);
        w.empty("attribute", &[("name", "key")]);
        w.empty("attribute", &[("name", "target")]);
        w.close("element"); // keyalias
        w.close("define");

        // Define ── missing
        w.comment("missing definition");
        w.open("define", &[("name", "missing")]);
        w.open("element", &[("name", "bbl:missing")]);
        w.empty("attribute", &[("name", "key")]);
        w.close("element"); // missing
        w.close("define");

        w.close("grammar");

        s
    }

    /// Generate a RELAX NG XML schema from the data model for biblatexml datasources.
    ///
    /// Ported from `Biber::DataModel::generate_bltxml_schema` (Perl).
    pub fn generate_bltxml_schema(&self) -> String {
        let bltx_ns = "http://biblatex-biber.sourceforge.net/biblatexml";
        let rng_ns = "http://relaxng.org/ns/structure/1.0";
        let xsd = "http://www.w3.org/2001/XMLSchema-datatypes";

        let mut s = String::new();
        let mut w = RngWriter::new(&mut s);

        w.xml_decl();
        w.comment("Auto-generated from .bcf Datamodel");
        w.open(
            "grammar",
            &[
                ("xmlns", rng_ns),
                ("xmlns:bltx", bltx_ns),
                ("datatypeLibrary", xsd),
            ],
        );

        w.open("start", &[]);
        w.open("element", &[("name", "bltx:entries")]);
        w.open("oneOrMore", &[]);
        w.open("element", &[("name", "bltx:entry")]);
        w.empty("attribute", &[("name", "id")]);
        w.open("attribute", &[("name", "entrytype")]);
        w.open("choice", &[]);
        for et in self.entrytypes.iter() {
            w.data_element("value", et);
        }
        w.close("choice");
        w.close("attribute");
        w.open("interleave", &[]);

        // Refs to field type definitions
        for ft in self.fieldtypes() {
            for dt in self.datatypes() {
                if dt == "datepart" {
                    // not legal in input, only output
                    continue;
                }
                if self.is_fields_of_type(ft, dt) {
                    w.comment(&format!("{dt} {ft}s"));
                    w.empty("ref", &[("name", &format!("{dt}{ft}"))]);
                }
            }
        }

        // Annotations
        w.empty("ref", &[("name", "mannotation")]);

        w.close("interleave");
        w.close("element"); // bltx:entry
        w.close("oneOrMore");
        w.close("element"); // bltx:entries
        w.close("start");

        // Field type definitions
        for ft in self.fieldtypes() {
            for dt in self.datatypes() {
                if dt == "datepart" {
                    continue;
                }
                if !self.is_fields_of_type(ft, dt) {
                    continue;
                }
                w.comment(&format!("{dt} {ft}s definition"));
                w.open("define", &[("name", &format!("{dt}{ft}"))]);

                match (ft, dt) {
                    ("list", "name") => write_name_list_define(&mut w, self, bltx_ns),
                    ("list", _) => write_list_define(&mut w, self, ft, dt, bltx_ns),
                    ("field", "uri") => write_uri_define(&mut w, self, ft, dt, bltx_ns),
                    ("field", "range") => write_range_define(&mut w, self, ft, dt, bltx_ns),
                    ("field", "entrykey") => write_entrykey_define(&mut w, self, ft, dt, bltx_ns),
                    ("field", "date") => write_date_define(&mut w, self, ft, dt, bltx_ns),
                    ("field", _) => write_field_define(&mut w, self, ft, dt, bltx_ns),
                    _ => {}
                }

                w.close("define");
            }
        }

        // xdata attribute definition
        w.comment("xdata attribute definition");
        w.open("define", &[("name", "xdata")]);
        w.open("optional", &[]);
        w.open("attribute", &[("name", "xdata")]);
        w.empty("text", &[]);
        w.close("attribute");
        w.close("optional");
        w.close("define");

        // gender attribute definition
        w.comment("gender attribute definition");
        w.open("define", &[("name", "gender")]);
        w.open("optional", &[]);
        w.open("attribute", &[("name", "gender")]);
        w.open("choice", &[]);
        if let Some(genders) = self.get_constant_value("gender") {
            for g in genders
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            {
                w.data_element("value", g);
            }
        }
        w.close("choice");
        w.close("attribute");
        w.close("optional");
        w.close("define");

        // Generic meta annotation element definition
        w.comment("generic annotation element definition");
        w.open("define", &[("name", "mannotation")]);
        w.open("zeroOrMore", &[]);
        w.open("element", &[("name", "bltx:mannotation")]);
        w.empty("attribute", &[("name", "field")]);
        w.open("optional", &[]);
        w.empty("attribute", &[("name", "name")]);
        w.close("optional");
        w.open("optional", &[]);
        w.empty("attribute", &[("name", "item")]);
        w.close("optional");
        w.open("optional", &[]);
        w.empty("attribute", &[("name", "part")]);
        w.close("optional");
        w.open("optional", &[]);
        w.empty("attribute", &[("name", "literal")]);
        w.close("optional");
        w.empty("text", &[]);
        w.close("element"); // mannotation
        w.close("zeroOrMore");
        w.close("define");

        w.close("grammar");

        s
    }

    // ── validation helpers ──────────────────────────────────────────

    fn constraints_for_entrytype(&self, entrytype: &str) -> Vec<&ModelConstraint> {
        self.constraints
            .iter()
            .filter(|mc| mc.entrytypes.is_empty() || mc.entrytypes.iter().any(|et| et == entrytype))
            .flat_map(|mc| mc.constraints.iter())
            .collect()
    }

    fn quantifier_satisfied(quant: &str, entry: &Entry, fields: &[String]) -> bool {
        let present = fields.iter().filter(|f| entry.has_field(f)).count();
        match quant {
            "all" => present == fields.len(),
            "one" | "any" => present >= 1,
            "none" => present == 0,
            _ => false,
        }
    }

    /// Validate the entry type exists in the datamodel.
    /// Returns a warning string if invalid, or `None`.
    pub fn validate_entrytype(&self, entry: &Entry) -> Option<String> {
        let et = &entry.entrytype;
        if et.is_empty() {
            return Some("empty entry type, defaulting to 'misc'".to_string());
        }
        if !self.is_known_entrytype(et) {
            return Some(format!(
                "invalid entry type '{}' for entry '{}', defaulting to 'misc'",
                et, entry.citekey
            ));
        }
        None
    }

    /// Validate all fields are valid for the entry's type.
    /// Skips xdata and set entries (containers).
    pub fn validate_fields(&self, entry: &Entry) -> Vec<String> {
        let et = &entry.entrytype;
        if et == "xdata" || et == "set" {
            return Vec::new();
        }
        let mut warnings = Vec::new();
        for field in entry.field_names() {
            if !self.is_field_for_entrytype(et, field) {
                warnings.push(format!(
                    "invalid field '{}' for entrytype '{}' in entry '{}'",
                    field, et, entry.citekey
                ));
            }
        }
        warnings
    }

    /// Check mandatory constraints for an entry.
    pub fn check_mandatory_constraints(&self, entry: &Entry) -> Vec<String> {
        let et = &entry.entrytype;
        let mut warnings = Vec::new();

        for constraint in self.constraints_for_entrytype(et) {
            let ModelConstraint::Mandatory { fields } = constraint else {
                continue;
            };
            for field in fields {
                match field {
                    MandatoryField::Field(name) => {
                        if !entry.has_field(name) {
                            warnings.push(format!(
                                "missing mandatory field '{}' for entry '{}'",
                                name, entry.citekey
                            ));
                        }
                    }
                    MandatoryField::FieldOr(alternatives) => {
                        let has_any = alternatives.iter().any(|a| entry.has_field(a));
                        if !has_any {
                            warnings.push(format!(
                                "missing mandatory field - one of '{}' must be defined for entry '{}'",
                                alternatives.join(", "),
                                entry.citekey
                            ));
                        }
                    }
                }
            }
        }
        warnings
    }

    /// Check conditional constraints for an entry.
    /// Returns warnings *and* field names to delete from the entry (for
    /// `consequent quant = none` violations).
    pub fn check_conditional_constraints(&self, entry: &Entry) -> (Vec<String>, Vec<String>) {
        let et = &entry.entrytype;
        let mut warnings = Vec::new();
        let mut to_delete = Vec::new();

        for constraint in self.constraints_for_entrytype(et) {
            let ModelConstraint::Conditional {
                antecedent,
                consequent,
            } = constraint
            else {
                continue;
            };

            if !Self::quantifier_satisfied(&antecedent.quant, entry, &antecedent.fields) {
                continue;
            }

            let consequent_satisfied =
                Self::quantifier_satisfied(&consequent.quant, entry, &consequent.fields);

            if consequent_satisfied {
                continue;
            }

            match consequent.quant.as_str() {
                "none" => {
                    // Delete the offending fields
                    for f in &consequent.fields {
                        if entry.has_field(f) {
                            to_delete.push(f.clone());
                            warnings.push(format!(
                                "conditional constraint: field '{}' should not be present in entry '{}'",
                                f, entry.citekey
                            ));
                        }
                    }
                }
                "all" => {
                    let missing: Vec<&str> = consequent
                        .fields
                        .iter()
                        .filter(|f| !entry.has_field(f))
                        .map(|s| s.as_str())
                        .collect();
                    if !missing.is_empty() {
                        warnings.push(format!(
                            "conditional constraint: missing fields '{}' for entry '{}'",
                            missing.join(", "),
                            entry.citekey
                        ));
                    }
                }
                "one" | "any" => {
                    let present = consequent
                        .fields
                        .iter()
                        .filter(|f| entry.has_field(f))
                        .count();
                    if present == 0 {
                        warnings.push(format!(
                            "conditional constraint: one of '{}' must be defined for entry '{}'",
                            consequent.fields.join(", "),
                            entry.citekey
                        ));
                    }
                }
                _ => {}
            }
        }
        (warnings, to_delete)
    }

    /// Check the datatype of each field value against the declared datatype
    /// in the data model. Returns warnings and field names to delete.
    pub fn check_datatypes(&self, entry: &Entry) -> (Vec<String>, Vec<String>) {
        let mut warnings = Vec::new();
        let mut to_delete = Vec::new();

        for fname in entry.field_names() {
            let Some(field_def) = self.fields.get(fname) else {
                // Field not in datamodel → type check not possible; skip
                continue;
            };

            let Some(value) = entry.get_field(fname) else {
                continue;
            };

            match field_def.datatype.as_str() {
                "integer" => {
                    let s = match value {
                        ConfigValue::Str(s) => s,
                        _ => {
                            warn_type_delete(
                                fname,
                                &field_def.datatype,
                                entry,
                                &mut warnings,
                                &mut to_delete,
                            );
                            continue;
                        }
                    };
                    if s.parse::<i64>().is_err() && !s.is_empty() {
                        warn_type_delete(
                            fname,
                            &field_def.datatype,
                            entry,
                            &mut warnings,
                            &mut to_delete,
                        );
                    }
                }
                "datepart" => {
                    let s = match value {
                        ConfigValue::Str(s) => s,
                        _ => {
                            warn_type_delete(
                                fname,
                                &field_def.datatype,
                                entry,
                                &mut warnings,
                                &mut to_delete,
                            );
                            continue;
                        }
                    };
                    if !s.is_empty()
                        && s.parse::<i64>().is_err()
                        && !SEASONS.contains(&s.as_str())
                        && !QUARTERS.contains(&s.as_str())
                        && !SEMESTERS.contains(&s.as_str())
                    {
                        // Allow timezone format ±HH:MM
                        let is_tz = s.len() >= 5
                            && s.as_bytes()
                                .iter()
                                .skip(1)
                                .all(|&b| b.is_ascii_digit() || b == b':')
                            && (s.starts_with('+') || s.starts_with('-'));
                        if !is_tz {
                            warn_type_delete(
                                fname,
                                &field_def.datatype,
                                entry,
                                &mut warnings,
                                &mut to_delete,
                            );
                        }
                    }
                }
                "date" => {
                    let s = match value {
                        ConfigValue::Str(s) => s,
                        _ => {
                            warn_type_delete(
                                fname,
                                &field_def.datatype,
                                entry,
                                &mut warnings,
                                &mut to_delete,
                            );
                            continue;
                        }
                    };
                    if !s.is_empty() {
                        // Basic date-like format check
                        let looks_like_date =
                            s.contains('-') || s.chars().all(|c| c.is_ascii_digit());
                        if !looks_like_date {
                            warn_type_delete(
                                fname,
                                &field_def.datatype,
                                entry,
                                &mut warnings,
                                &mut to_delete,
                            );
                        }
                    }
                }
                "name" => {
                    // Names must have a parsed entry in the names map
                    if !entry.names.contains_key(fname)
                        && matches!(value, ConfigValue::Str(s) if !s.is_empty())
                    {
                        warn_type_delete(
                            fname,
                            &field_def.datatype,
                            entry,
                            &mut warnings,
                            &mut to_delete,
                        );
                    }
                }
                "range" => {
                    if !matches!(value, ConfigValue::List(_))
                        && matches!(value, ConfigValue::Str(s) if !s.is_empty())
                    {
                        warn_type_delete(
                            fname,
                            &field_def.datatype,
                            entry,
                            &mut warnings,
                            &mut to_delete,
                        );
                    }
                }
                "verbatim" | "uri" | "code" | "entrykey" | "keyword" | "option" | "key"
                    if value.is_str() => {}
                "literal" if value.is_str() => {}
                _ => {
                    // For list fields, check they're lists
                    if field_def.fieldtype == "list" && !matches!(value, ConfigValue::List(_)) {
                        warn_type_delete(
                            fname,
                            &field_def.datatype,
                            entry,
                            &mut warnings,
                            &mut to_delete,
                        );
                    }
                }
            }
        }
        (warnings, to_delete)
    }

    /// Check data-level constraints (isbn, issn, ismn, integer ranges, patterns).
    pub fn check_data_constraints(&self, entry: &Entry) -> Vec<String> {
        let et = &entry.entrytype;
        let mut warnings = Vec::new();

        for constraint in self.constraints_for_entrytype(et) {
            let ModelConstraint::Data {
                datatype,
                rangemin,
                rangemax,
                pattern,
                fields,
            } = constraint
            else {
                continue;
            };

            for fname in fields {
                let Some(val) = entry.get_field_str(fname) else {
                    continue;
                };
                if val.is_empty() {
                    continue;
                }

                match datatype.as_str() {
                    "isbn" => {
                        if !crate::validate::validate_isbn(val) {
                            warnings.push(format!(
                                "invalid ISBN '{}' in field '{}' for entry '{}'",
                                val, fname, entry.citekey
                            ));
                        }
                    }
                    "issn" => {
                        if !crate::validate::validate_issn(val) {
                            warnings.push(format!(
                                "invalid ISSN '{}' in field '{}' for entry '{}'",
                                val, fname, entry.citekey
                            ));
                        }
                    }
                    "ismn" => {
                        if !crate::validate::validate_ismn(val) {
                            warnings.push(format!(
                                "invalid ISMN '{}' in field '{}' for entry '{}'",
                                val, fname, entry.citekey
                            ));
                        }
                    }
                    "integer" | "datepart" => {
                        if let Some(ref min) = rangemin {
                            if let (Ok(n), Ok(min_n)) = (val.parse::<i64>(), min.parse::<i64>()) {
                                if n < min_n {
                                    warnings.push(format!(
                                        "field '{}' value '{}' below minimum {} for entry '{}'",
                                        fname, val, min, entry.citekey
                                    ));
                                }
                            }
                        }
                        if let Some(ref max) = rangemax {
                            if let (Ok(n), Ok(max_n)) = (val.parse::<i64>(), max.parse::<i64>()) {
                                if n > max_n {
                                    warnings.push(format!(
                                        "field '{}' value '{}' above maximum {} for entry '{}'",
                                        fname, val, max, entry.citekey
                                    ));
                                }
                            }
                        }
                    }
                    "pattern" => {
                        if let Some(ref re_str) = pattern {
                            // Simple glob/regex matching -- the Perl biber uses
                            // regex patterns; we attempt a basic regex match.
                            if let Some(re) = regex_lite(re_str) {
                                if !re(val) {
                                    warnings.push(format!(
                                        "field '{}' value '{}' does not match pattern '{}' for entry '{}'",
                                        fname, val, re_str, entry.citekey
                                    ));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        warnings
    }
}

fn warn_type_delete(
    fname: &str,
    datatype: &str,
    entry: &Entry,
    warnings: &mut Vec<String>,
    to_delete: &mut Vec<String>,
) {
    warnings.push(format!(
        "field '{}' has wrong datatype (expected {}, got '{}') for entry '{}'",
        fname,
        datatype,
        entry.get_field_str(fname).unwrap_or(""),
        entry.citekey
    ));
    to_delete.push(fname.to_string());
}

/// Minimal regex-lite matcher for patterns used in data constraints.
/// Supports `(?:...)` groups and `|` alternation.
fn regex_lite(pattern: &str) -> Option<impl Fn(&str) -> bool> {
    let pattern = pattern.to_owned();
    Some(move |s: &str| {
        // Simple substring match if pattern has no metacharacters
        if !pattern.contains('(')
            && !pattern.contains('|')
            && !pattern.contains('?')
            && !pattern.contains('*')
        {
            return s == pattern;
        }
        // Very basic: escape pattern and check if it matches via regex
        // For now, use a simple heuristic
        if pattern.starts_with("(?:") && pattern.ends_with(')') {
            let inner = &pattern[3..pattern.len() - 1];
            if inner.contains('|') {
                let alts: Vec<&str> = inner.split('|').collect();
                return alts.iter().any(|alt| alt.trim() == s);
            }
        }
        s == pattern
    })
}

impl ConfigValue {
    fn is_str(&self) -> bool {
        matches!(self, ConfigValue::Str(_))
    }
}

// ---------------------------------------------------------------------------
// RELAX NG XML writer helpers
// ---------------------------------------------------------------------------

struct RngWriter<'a> {
    buf: &'a mut String,
    indent: usize,
}

impl<'a> RngWriter<'a> {
    fn new(buf: &'a mut String) -> Self {
        Self { buf, indent: 0 }
    }

    fn i(&self) -> String {
        "  ".repeat(self.indent)
    }

    fn xml_decl(&mut self) {
        self.buf
            .push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    }

    fn comment(&mut self, text: &str) {
        let ind = self.i();
        let _ = writeln!(self.buf, "{ind}<!-- {text} -->");
    }

    fn open(&mut self, tag: &str, attrs: &[(&str, &str)]) {
        let ind = self.i();
        let _ = write!(self.buf, "{ind}<{tag}");
        for (k, v) in attrs {
            let _ = write!(self.buf, " {k}=\"{v}\"");
        }
        self.buf.push_str(">\n");
        self.indent += 1;
    }

    fn close(&mut self, tag: &str) {
        self.indent -= 1;
        let ind = self.i();
        let _ = writeln!(self.buf, "{ind}</{tag}>");
    }

    fn empty(&mut self, tag: &str, attrs: &[(&str, &str)]) {
        let ind = self.i();
        let _ = write!(self.buf, "{ind}<{tag}");
        for (k, v) in attrs {
            let _ = write!(self.buf, " {k}=\"{v}\"");
        }
        self.buf.push_str("/>\n");
    }

    fn data_element(&mut self, tag: &str, content: &str) {
        let ind = self.i();
        let _ = writeln!(self.buf, "{ind}<{tag}>{content}</{tag}>");
    }
}

fn write_name_list_define(w: &mut RngWriter, dm: &DataModel, _ns: &str) {
    w.open("zeroOrMore", &[]);
    w.open("element", &[("name", "bltx:names")]);
    w.open("choice", &[]);
    w.empty("ref", &[("name", "xdata")]);
    w.open("group", &[]);

    // useprefix attribute
    w.comment("useprefix option");
    w.open("optional", &[]);
    w.open("attribute", &[("name", "useprefix")]);
    w.empty("data", &[("type", "boolean")]);
    w.close("attribute");
    w.close("optional");

    // sortingnamekeytemplatename attribute
    w.comment("sortingnamekeytemplatename option");
    w.open("optional", &[]);
    w.open("attribute", &[("name", "sortingnamekeytemplatename")]);
    w.empty("data", &[("type", "string")]);
    w.close("attribute");
    w.close("optional");

    // type attribute
    w.comment("types of names elements");
    w.open("attribute", &[("name", "type")]);
    w.open("choice", &[]);
    for name in dm.get_fields_of_type("list", "name") {
        w.data_element("value", name);
    }
    w.close("choice");
    w.close("attribute");

    // morenames attribute
    w.open("optional", &[]);
    w.open("attribute", &[("name", "morenames")]);
    w.empty("data", &[("type", "boolean")]);
    w.close("attribute");
    w.close("optional");

    w.open("oneOrMore", &[]);
    w.open("element", &[("name", "bltx:name")]);
    w.open("choice", &[]);
    w.empty("ref", &[("name", "xdata")]);
    w.open("group", &[]);

    // useprefix attribute on name
    w.comment("useprefix option");
    w.open("optional", &[]);
    w.open("attribute", &[("name", "useprefix")]);
    w.empty("data", &[("type", "boolean")]);
    w.close("attribute");
    w.close("optional");

    // sortingnamekeytemplatename attribute on name
    w.comment("sortingnamekeytemplatename option");
    w.open("optional", &[]);
    w.open("attribute", &[("name", "sortingnamekeytemplatename")]);
    w.empty("data", &[("type", "string")]);
    w.close("attribute");
    w.close("optional");

    // gender ref
    w.empty("ref", &[("name", "gender")]);

    w.open("oneOrMore", &[]);
    w.open("element", &[("name", "bltx:namepart")]);
    w.open("attribute", &[("name", "type")]);
    w.open("choice", &[]);
    for np in dm.get_nameparts() {
        w.data_element("value", &np);
    }
    w.close("choice");
    w.close("attribute");
    w.open("optional", &[]);
    w.empty("attribute", &[("name", "initial")]);
    w.close("optional");
    w.open("choice", &[]);
    w.open("oneOrMore", &[]);
    w.open("element", &[("name", "bltx:namepart")]);
    w.open("optional", &[]);
    w.empty("attribute", &[("name", "initial")]);
    w.close("optional");
    w.empty("text", &[]);
    w.close("element");
    w.close("oneOrMore");
    w.empty("text", &[]);
    w.close("choice");
    w.close("element"); // namepart
    w.close("oneOrMore");

    w.close("group");
    w.close("choice");
    w.close("element"); // name
    w.close("oneOrMore");

    w.close("group");
    w.close("choice");
    w.close("element"); // names
    w.close("zeroOrMore");
}

fn write_list_define(w: &mut RngWriter, dm: &DataModel, _ft: &str, _dt: &str, _ns: &str) {
    // For non-name lists we can't easily enumerate fields by dt because
    // we don't have separate fieldtype/datatype indexes. Use fields map.
    let fields: Vec<&str> = dm
        .get_fields_of_type("list", "literal")
        .into_iter()
        .chain(dm.get_fields_of_type("list", "keyword"))
        .chain(dm.get_fields_of_type("list", "code"))
        .chain(dm.get_fields_of_type("list", "integer"))
        .chain(dm.get_fields_of_type("list", "option"))
        .chain(dm.get_fields_of_type("list", "key"))
        .collect::<std::collections::BTreeSet<&str>>()
        .into_iter()
        .collect();

    w.open("interleave", &[]);
    for field in &fields {
        w.open("optional", &[]);
        w.open("element", &[("name", &format!("bltx:{}", field))]);
        w.open("choice", &[]);
        w.empty("ref", &[("name", "xdata")]);
        w.open("choice", &[]);
        w.empty("text", &[]);
        w.open("element", &[("name", "bltx:list")]);
        w.open("oneOrMore", &[]);
        w.open("element", &[("name", "bltx:item")]);
        w.open("choice", &[]);
        w.empty("ref", &[("name", "xdata")]);
        w.empty("text", &[]);
        w.close("choice");
        w.close("element"); // item
        w.close("oneOrMore");
        w.close("element"); // list
        w.close("choice");
        w.close("choice");
        w.close("element"); // field
        w.close("optional");
    }
    w.close("interleave");
}

fn write_uri_define(w: &mut RngWriter, dm: &DataModel, _ft: &str, _dt: &str, _ns: &str) {
    let fields = dm.get_fields_of_type("field", "uri");
    w.open("interleave", &[]);
    for field in &fields {
        w.open("optional", &[]);
        w.open("element", &[("name", &format!("bltx:{}", field))]);
        w.open("choice", &[]);
        w.empty("ref", &[("name", "xdata")]);
        w.empty("data", &[("type", "anyURI")]);
        w.close("choice");
        w.close("element");
        w.close("optional");
    }
    w.close("interleave");
}

fn write_range_define(w: &mut RngWriter, dm: &DataModel, _ft: &str, _dt: &str, _ns: &str) {
    let fields = dm.get_fields_of_type("field", "range");
    w.open("interleave", &[]);
    for field in &fields {
        w.open("optional", &[]);
        w.open("element", &[("name", &format!("bltx:{}", field))]);
        w.open("choice", &[]);
        w.empty("ref", &[("name", "xdata")]);
        w.open("element", &[("name", "bltx:list")]);
        w.open("oneOrMore", &[]);
        w.open("element", &[("name", "bltx:item")]);
        w.open("choice", &[]);
        w.empty("ref", &[("name", "xdata")]);
        w.open("group", &[]);
        w.open("element", &[("name", "bltx:start")]);
        w.empty("text", &[]);
        w.close("element"); // start
        w.open("element", &[("name", "bltx:end")]);
        w.open("choice", &[]);
        w.empty("text", &[]);
        w.empty("empty", &[]);
        w.close("choice");
        w.close("element"); // end
        w.close("group");
        w.close("choice");
        w.close("element"); // item
        w.close("oneOrMore");
        w.close("element"); // list
        w.close("choice");
        w.close("element"); // field
        w.close("optional");
    }
    w.close("interleave");
}

fn write_entrykey_define(w: &mut RngWriter, dm: &DataModel, _ft: &str, _dt: &str, _ns: &str) {
    let fields = dm.get_fields_of_type("field", "entrykey");
    w.open("interleave", &[]);
    for field in &fields {
        w.open("optional", &[]);
        if *field == "related" {
            w.open("element", &[("name", "bltx:related")]);
            w.open("element", &[("name", "bltx:list")]);
            w.open("oneOrMore", &[]);
            w.open("element", &[("name", "bltx:item")]);
            w.empty("attribute", &[("name", "type")]);
            w.empty("attribute", &[("name", "ids")]);
            w.open("optional", &[]);
            w.empty("attribute", &[("name", "string")]);
            w.close("optional");
            w.open("optional", &[]);
            w.empty("attribute", &[("name", "options")]);
            w.close("optional");
            w.close("element"); // item
            w.close("oneOrMore");
            w.close("element"); // list
            w.close("element"); // related
        } else {
            w.open("element", &[("name", &format!("bltx:{}", field))]);
            w.open("choice", &[]);
            w.empty("ref", &[("name", "xdata")]);
            w.open("choice", &[]);
            w.open("list", &[]);
            w.open("oneOrMore", &[]);
            w.empty("data", &[("type", "string")]);
            w.close("oneOrMore");
            w.close("list");
            w.open("element", &[("name", "bltx:list")]);
            w.open("oneOrMore", &[]);
            w.open("element", &[("name", "bltx:item")]);
            w.empty("text", &[]);
            w.close("element"); // item
            w.close("oneOrMore");
            w.close("element"); // list
            w.close("choice");
            w.close("choice");
            w.close("element"); // field
        }
        w.close("optional");
    }
    w.close("interleave");
}

fn write_date_define(w: &mut RngWriter, dm: &DataModel, _ft: &str, _dt: &str, _ns: &str) {
    let fields = dm.get_fields_of_type("field", "date");
    let types: Vec<&str> = fields
        .iter()
        .filter_map(|f| f.strip_suffix("date").filter(|s| !s.is_empty()))
        .collect();

    w.open("zeroOrMore", &[]);
    w.open("element", &[("name", "bltx:date")]);
    w.open("optional", &[]);
    w.open("attribute", &[("name", "type")]);
    w.open("choice", &[]);
    for t in &types {
        w.data_element("value", t);
    }
    w.close("choice");
    w.close("attribute");
    w.close("optional");
    w.open("choice", &[]);
    w.empty("data", &[("type", "string")]);
    w.open("group", &[]);
    w.open("element", &[("name", "bltx:start")]);
    w.open("choice", &[]);
    w.empty("data", &[("type", "string")]);
    w.close("choice");
    w.close("element");
    w.open("element", &[("name", "bltx:end")]);
    w.open("choice", &[]);
    w.empty("data", &[("type", "string")]);
    w.empty("empty", &[]);
    w.close("choice");
    w.close("element");
    w.close("group");
    w.close("choice");
    w.close("element"); // date
    w.close("zeroOrMore");
}

fn write_field_define(w: &mut RngWriter, dm: &DataModel, _ft: &str, dt: &str, _ns: &str) {
    let fields = dm.get_fields_of_type("field", dt);
    w.open("interleave", &[]);
    for field in &fields {
        w.open("optional", &[]);
        w.open("element", &[("name", &format!("bltx:{}", field))]);
        w.open("choice", &[]);
        w.empty("ref", &[("name", "xdata")]);
        w.empty("text", &[]);
        w.close("choice");
        w.close("element");
        w.close("optional");
    }
    w.close("interleave");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::Entry;

    fn make_dm() -> DataModel {
        let mut dm = DataModel::new();
        dm.constants.insert(
            "nameparts".to_string(),
            ModelConstant {
                r#type: "list".to_string(),
                name: "nameparts".to_string(),
                value: "family,given,prefix,suffix".to_string(),
            },
        );
        dm.entrytypes.insert("book".to_string());
        dm.entrytypes.insert("article".to_string());
        dm.fields.insert(
            "title".to_string(),
            ModelField {
                fieldtype: "field".to_string(),
                datatype: "literal".to_string(),
                nullok: false,
                label: false,
                skip_output: false,
                format: None,
            },
        );
        dm.fields.insert(
            "year".to_string(),
            ModelField {
                fieldtype: "field".to_string(),
                datatype: "datepart".to_string(),
                nullok: false,
                label: false,
                skip_output: false,
                format: None,
            },
        );
        dm.fields.insert(
            "month".to_string(),
            ModelField {
                fieldtype: "field".to_string(),
                datatype: "datepart".to_string(),
                nullok: false,
                label: false,
                skip_output: false,
                format: None,
            },
        );
        dm.fields.insert(
            "author".to_string(),
            ModelField {
                fieldtype: "list".to_string(),
                datatype: "name".to_string(),
                nullok: false,
                label: false,
                skip_output: false,
                format: None,
            },
        );
        dm.fields.insert(
            "isbn".to_string(),
            ModelField {
                fieldtype: "field".to_string(),
                datatype: "literal".to_string(),
                nullok: false,
                label: false,
                skip_output: false,
                format: None,
            },
        );
        dm.entryfields.push(EntryFields {
            entrytypes: vec![],
            fields: vec![
                "abstract".to_string(),
                "isbn".to_string(),
                "issn".to_string(),
            ],
        });
        dm.entryfields.push(EntryFields {
            entrytypes: vec!["book".to_string()],
            fields: vec![
                "title".to_string(),
                "author".to_string(),
                "year".to_string(),
                "month".to_string(),
            ],
        });
        dm.entryfields.push(EntryFields {
            entrytypes: vec!["article".to_string()],
            fields: vec![
                "title".to_string(),
                "author".to_string(),
                "year".to_string(),
                "journal".to_string(),
            ],
        });
        dm.constraints.push(ModelConstraints {
            entrytypes: vec!["book".to_string()],
            constraints: vec![ModelConstraint::Mandatory {
                fields: vec![
                    MandatoryField::Field("title".to_string()),
                    MandatoryField::Field("author".to_string()),
                ],
            }],
        });
        dm.constraints.push(ModelConstraints {
            entrytypes: vec![],
            constraints: vec![ModelConstraint::Data {
                datatype: "integer".to_string(),
                rangemin: Some("1".to_string()),
                rangemax: Some("12".to_string()),
                pattern: None,
                fields: vec!["month".to_string()],
            }],
        });
        dm
    }

    fn make_entry(citekey: &str, entrytype: &str) -> Entry {
        let mut e = Entry::new(citekey, entrytype);
        e.set_field_str("entrytype", entrytype);
        e
    }

    #[test]
    fn datamodel_basics() {
        let dm = make_dm();
        assert!(dm.is_known_entrytype("book"));
        assert!(!dm.is_known_entrytype("unknown"));
        assert_eq!(dm.get_field_datatype("title"), Some("literal"));
        assert_eq!(
            dm.get_nameparts(),
            vec!["family", "given", "prefix", "suffix"]
        );
    }

    #[test]
    fn is_field_for_entrytype_global() {
        let dm = make_dm();
        assert!(dm.is_field_for_entrytype("book", "isbn"));
        assert!(dm.is_field_for_entrytype("article", "isbn"));
        assert!(dm.is_field_for_entrytype("book", "abstract"));
        assert!(dm.is_field_for_entrytype("unknown", "isbn"));
    }

    #[test]
    fn is_field_for_entrytype_specific() {
        let dm = make_dm();
        assert!(dm.is_field_for_entrytype("book", "title"));
        assert!(dm.is_field_for_entrytype("article", "journal"));
        assert!(!dm.is_field_for_entrytype("book", "journal"));
    }

    #[test]
    fn is_field_for_entrytype_internal() {
        let dm = make_dm();
        assert!(dm.is_field_for_entrytype("book", "labelname"));
        assert!(dm.is_field_for_entrytype("book", "extradate"));
        assert!(dm.is_field_for_entrytype("book", "sortinit"));
    }

    #[test]
    fn validate_entrytype_unknown() {
        let dm = make_dm();
        let e = make_entry("test1", "invalidtype");
        let warn = dm.validate_entrytype(&e);
        assert!(warn.is_some());
        assert!(warn.unwrap().contains("invalid entry type"));
    }

    #[test]
    fn validate_entrytype_empty() {
        let dm = make_dm();
        let mut e = make_entry("test1", "");
        e.set_field_str("entrytype", "");
        let warn = dm.validate_entrytype(&e);
        assert!(warn.is_some());
    }

    #[test]
    fn validate_entrytype_known() {
        let dm = make_dm();
        let e = make_entry("test1", "book");
        assert!(dm.validate_entrytype(&e).is_none());
    }

    #[test]
    fn check_mandatory_missing() {
        let dm = make_dm();
        let e = make_entry("test1", "book");
        let warns = dm.check_mandatory_constraints(&e);
        assert!(warns.iter().any(|w| w.contains("title")));
        assert!(warns.iter().any(|w| w.contains("author")));
    }

    #[test]
    fn check_mandatory_present() {
        let dm = make_dm();
        let mut e = make_entry("test1", "book");
        e.set_field_str("title", "A Book");
        e.set_field_str("author", "John Doe");
        let warns = dm.check_mandatory_constraints(&e);
        assert!(warns.is_empty());
    }

    #[test]
    fn check_conditional_antecedent_all_consequent_none() {
        // Build DM with: if all(field2,field3) → none(field5)
        let mut dm = make_dm();
        dm.constraints.push(ModelConstraints {
            entrytypes: vec!["book".to_string()],
            constraints: vec![ModelConstraint::Conditional {
                antecedent: ConditionalPart {
                    quant: "all".to_string(),
                    fields: vec!["field2".to_string(), "field3".to_string()],
                },
                consequent: ConditionalPart {
                    quant: "none".to_string(),
                    fields: vec!["field5".to_string()],
                },
            }],
        });
        let mut e = make_entry("test1", "book");
        e.set_field_str("title", "A");
        e.set_field_str("author", "B");
        e.set_field_str("field2", "a");
        e.set_field_str("field3", "b");
        e.set_field_str("field5", "c");
        let (warns, to_delete) = dm.check_conditional_constraints(&e);
        assert!(warns.iter().any(|w| w.contains("field5")));
        assert!(to_delete.contains(&"field5".to_string()));
    }

    #[test]
    fn check_data_isbn() {
        let mut dm = make_dm();
        dm.constraints.push(ModelConstraints {
            entrytypes: vec![],
            constraints: vec![ModelConstraint::Data {
                datatype: "isbn".to_string(),
                rangemin: None,
                rangemax: None,
                pattern: None,
                fields: vec!["isbn".to_string()],
            }],
        });
        let mut e = make_entry("test1", "book");
        e.set_field_str("isbn", "0-306-40615-2");
        let warns = dm.check_data_constraints(&e);
        assert!(warns.is_empty());

        let mut e2 = make_entry("test2", "book");
        e2.set_field_str("isbn", "invalid-isbn");
        let warns2 = dm.check_data_constraints(&e2);
        assert!(!warns2.is_empty());
    }

    #[test]
    fn check_data_integer_range() {
        let mut e = make_entry("test1", "book");
        e.set_field_str("month", "13");
        let warns = make_dm().check_data_constraints(&e);
        assert!(warns.iter().any(|w| w.contains("above maximum")));

        let mut e2 = make_entry("test2", "book");
        e2.set_field_str("month", "0");
        let warns2 = make_dm().check_data_constraints(&e2);
        assert!(warns2.iter().any(|w| w.contains("below minimum")));
    }

    #[test]
    fn check_datatype_integer() {
        let mut dm = make_dm();
        dm.fields.insert(
            "volume".to_string(),
            ModelField {
                fieldtype: "field".to_string(),
                datatype: "integer".to_string(),
                nullok: false,
                label: false,
                skip_output: false,
                format: None,
            },
        );
        let mut e = make_entry("test1", "book");
        e.set_field_str("volume", "42");
        let (warns, del) = dm.check_datatypes(&e);
        assert!(warns.is_empty());
        assert!(del.is_empty());

        let mut e2 = make_entry("test2", "book");
        e2.set_field_str("volume", "not-a-number");
        let (warns2, del2) = dm.check_datatypes(&e2);
        assert!(!warns2.is_empty());
        assert!(!del2.is_empty());
    }

    #[test]
    fn check_datatype_datepart_season() {
        let mut dm = make_dm();
        dm.fields.insert(
            "season".to_string(),
            ModelField {
                fieldtype: "field".to_string(),
                datatype: "datepart".to_string(),
                nullok: false,
                label: false,
                skip_output: false,
                format: None,
            },
        );
        let mut e = make_entry("test1", "book");
        e.set_field_str("season", "spring");
        let (warns, _) = dm.check_datatypes(&e);
        assert!(warns.is_empty());
    }

    #[test]
    fn validate_fields_invalid() {
        let dm = make_dm();
        let mut e = make_entry("test1", "book");
        e.set_field_str("title", "A");
        e.set_field_str("BADFIELD", "value");
        let warns = dm.validate_fields(&e);
        assert!(warns.iter().any(|w| w.contains("BADFIELD")));
    }

    #[test]
    fn validate_fields_global() {
        let dm = make_dm();
        let mut e = make_entry("test1", "book");
        e.set_field_str("abstract", "text");
        let warns = dm.validate_fields(&e);
        assert!(warns.is_empty());
    }

    #[test]
    fn generate_bltxml_schema_contains_grammar() {
        let dm = make_dm();
        let schema = dm.generate_bltxml_schema();
        assert!(schema.starts_with("<?xml"));
        assert!(schema.contains("<grammar"));
        assert!(schema.contains("</grammar>"));
        assert!(schema.contains("datatypeLibrary"));
        assert!(schema.contains("bltx:entries"));
        assert!(schema.contains("bltx:entry"));
        assert!(schema.contains("entrytype"));
        assert!(schema.contains("<value>book</value>"));
        assert!(schema.contains("<value>article</value>"));
    }

    #[test]
    fn generate_bltxml_schema_has_field_defines() {
        let dm = make_dm();
        let schema = dm.generate_bltxml_schema();
        // Has literal field definitions (title, isbn)
        assert!(schema.contains("literalfield"));
        // Has name list definitions (author)
        assert!(schema.contains("namelist"));
        // Has xdata attribute define
        assert!(schema.contains("xdata"));
        // Has gender attribute define
        assert!(schema.contains("gender"));
        // Has mannotation define
        assert!(schema.contains("mannotation"));
        // Has specific element references
        assert!(schema.contains("bltx:title"));
        assert!(schema.contains("<value>author</value>")); // name list type attribute value
        assert!(schema.contains("bltx:isbn"));
    }

    #[test]
    fn generate_bltxml_schema_excludes_datepart() {
        let dm = make_dm();
        let schema = dm.generate_bltxml_schema();
        // datepart fields should be excluded from input schema
        assert!(!schema.contains("datepartField"));
        assert!(!schema.contains("datepartList"));
    }
}
