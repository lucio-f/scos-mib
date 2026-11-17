//! Consistency checks of tables against the ICD.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::mib::Mib;
use crate::schema::ColumnType;
use crate::table::Table;

/// How serious a [`Diagnostic`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Suspicious content that SCOS-2000 tolerates (e.g. a value that would
    /// be truncated at import).
    Warning,
    /// Content that violates the ICD (e.g. a missing mandatory value).
    Error,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Warning => "warning",
            Self::Error => "error",
        })
    }
}

/// What a [`Diagnostic`] is about.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DiagnosticKind {
    /// The line is empty (or only contains separators).
    BlankLine,
    /// A mandatory column is null.
    MissingMandatory,
    /// A value is longer than the ICD allows (characters for `Char`, digits
    /// for `Number`). SCOS-2000 truncates such values at import.
    TooLong {
        /// Actual length.
        len: usize,
        /// Maximum length.
        max: usize,
    },
    /// A `Number` column holds something that is not an integer.
    NotAnInteger,
    /// The record has non-empty fields beyond the columns of the schema.
    ExtraFields {
        /// Number of fields in the record.
        count: usize,
        /// Number of columns in the schema.
        columns: usize,
    },
    /// Another record has the same values in all key columns.
    DuplicateKey {
        /// Index of the first record with this key.
        first_row: usize,
    },
    /// The value does not exist in the referenced table.
    DanglingReference {
        /// Referenced table.
        table: &'static str,
        /// Referenced column.
        column: &'static str,
    },
}

/// A problem found by [`Mib::validate`] or [`validate_table`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    /// Severity.
    pub severity: Severity,
    /// Table name.
    pub table: String,
    /// Zero-based record index (the file line number minus one).
    pub row: usize,
    /// Column name (`#<n>` for columns without a name), if the problem is
    /// about one column.
    pub column: Option<String>,
    /// The offending value, if any.
    pub value: Option<String>,
    /// What is wrong.
    pub kind: DiagnosticKind,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: {}: ", self.table, self.row + 1, self.severity)?;
        if let Some(column) = &self.column {
            write!(f, "{column}: ")?;
        }
        let value = self.value.as_deref().unwrap_or_default();
        match &self.kind {
            DiagnosticKind::BlankLine => write!(f, "blank line"),
            DiagnosticKind::MissingMandatory => write!(f, "mandatory value is missing"),
            DiagnosticKind::TooLong { len, max } => {
                write!(f, "{value:?} is too long ({len} > {max})")
            }
            DiagnosticKind::NotAnInteger => write!(f, "{value:?} is not an integer"),
            DiagnosticKind::ExtraFields { count, columns } => write!(
                f,
                "record has {count} fields but the table has {columns} columns"
            ),
            DiagnosticKind::DuplicateKey { first_row } => write!(
                f,
                "duplicate key ({value}), first defined on line {}",
                first_row + 1
            ),
            DiagnosticKind::DanglingReference { table, column } => {
                write!(f, "{value:?} not found in {table}.{column}")
            }
        }
    }
}

/// Checks one table against its schema: blank lines, mandatory values,
/// lengths, integer syntax, unexpected extra fields and duplicate keys.
/// Tables without a schema are only checked for blank lines.
pub fn validate_table(table: &Table) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    check_table(table, &mut out);
    out
}

pub(crate) fn check_table(table: &Table, out: &mut Vec<Diagnostic>) {
    let diag = |severity, row, column: Option<usize>, value: Option<&str>, kind| Diagnostic {
        severity,
        table: table.name().to_owned(),
        row,
        column: column.map(|c| table.column_name(c).into_owned()),
        value: value.map(str::to_owned),
        kind,
    };
    let schema = table.schema();
    let columns = schema.map_or(&[][..], |s| s.columns());
    let keys: Vec<usize> = schema
        .map(|s| s.key_indices().collect())
        .unwrap_or_default();
    let mut seen_keys: HashMap<Vec<&str>, usize> = HashMap::new();

    for (row, record) in table.records().iter().enumerate() {
        if record.is_blank() {
            out.push(diag(
                Severity::Warning,
                row,
                None,
                None,
                DiagnosticKind::BlankLine,
            ));
            continue;
        }
        for (i, column) in columns.iter().enumerate() {
            let value = record.get(i).unwrap_or("");
            if value.is_empty() {
                if column.is_mandatory() {
                    out.push(diag(
                        Severity::Error,
                        row,
                        Some(i),
                        None,
                        DiagnosticKind::MissingMandatory,
                    ));
                }
                continue;
            }
            let (len, max) = match column.ty() {
                ColumnType::Char(max) => (value.chars().count(), max),
                ColumnType::Number(max) => {
                    let digits = value.strip_prefix(['+', '-']).unwrap_or(value);
                    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                        out.push(diag(
                            Severity::Error,
                            row,
                            Some(i),
                            Some(value),
                            DiagnosticKind::NotAnInteger,
                        ));
                        continue;
                    }
                    (digits.len(), max)
                }
            };
            if len > usize::from(max) {
                out.push(diag(
                    Severity::Warning,
                    row,
                    Some(i),
                    Some(value),
                    DiagnosticKind::TooLong {
                        len,
                        max: max.into(),
                    },
                ));
            }
        }
        if !columns.is_empty() && record.iter().skip(columns.len()).any(|v| !v.is_empty()) {
            out.push(diag(
                Severity::Warning,
                row,
                None,
                None,
                DiagnosticKind::ExtraFields {
                    count: record.len(),
                    columns: columns.len(),
                },
            ));
        }
        if !keys.is_empty() {
            let key: Vec<&str> = keys.iter().map(|&k| record.get(k).unwrap_or("")).collect();
            if let Some(&first_row) = seen_keys.get(&key) {
                out.push(diag(
                    Severity::Error,
                    row,
                    None,
                    Some(&key.join(", ")),
                    DiagnosticKind::DuplicateKey { first_row },
                ));
            } else {
                seen_keys.insert(key, row);
            }
        }
    }
}

/// References between ICD tables: `(table, column, referenced table,
/// referenced column)`. Only unambiguous references are listed (columns that
/// may point to one of several tables, such as `PCF_CURTX`, are not).
pub const REFERENCES: &[(&str, &str, &str, &str)] = &[
    // monitoring
    ("pcf", "PCF_VALID", "pcf", "PCF_NAME"),
    ("pcf", "PCF_RELATED", "pcf", "PCF_NAME"),
    ("cur", "CUR_PNAME", "pcf", "PCF_NAME"),
    ("cur", "CUR_RLCHK", "pcf", "PCF_NAME"),
    ("cap", "CAP_NUMBR", "caf", "CAF_NUMBR"),
    ("txp", "TXP_NUMBR", "txf", "TXF_NUMBR"),
    ("ocf", "OCF_NAME", "pcf", "PCF_NAME"),
    ("ocp", "OCP_NAME", "ocf", "OCF_NAME"),
    ("ocp", "OCP_RLCHK", "pcf", "PCF_NAME"),
    ("tpcf", "TPCF_SPID", "pid", "PID_SPID"),
    ("plf", "PLF_NAME", "pcf", "PCF_NAME"),
    ("plf", "PLF_SPID", "pid", "PID_SPID"),
    ("vpd", "VPD_NAME", "pcf", "PCF_NAME"),
    ("grpa", "GRPA_GNAME", "grp", "GRP_NAME"),
    ("grpa", "GRPA_PANAME", "pcf", "PCF_NAME"),
    ("grpk", "GRPK_GNAME", "grp", "GRP_NAME"),
    ("grpk", "GRPK_PKSPID", "pid", "PID_SPID"),
    ("dpc", "DPC_NUMBE", "dpf", "DPF_NUMBE"),
    ("dpc", "DPC_NAME", "pcf", "PCF_NAME"),
    ("gpc", "GPC_NUMBE", "gpf", "GPF_NUMBE"),
    ("gpc", "GPC_NAME", "pcf", "PCF_NAME"),
    ("spc", "SPC_NUMBE", "spf", "SPF_NUMBE"),
    ("spc", "SPC_NAME", "pcf", "PCF_NAME"),
    ("ppc", "PPC_NUMBE", "ppf", "PPF_NUMBE"),
    ("ppc", "PPC_NAME", "pcf", "PCF_NAME"),
    // commanding
    ("pcdf", "PCDF_TCNAME", "tcp", "TCP_ID"),
    ("pcdf", "PCDF_PNAME", "pcpc", "PCPC_PNAME"),
    ("ccf", "CCF_PKTID", "tcp", "TCP_ID"),
    ("ccf", "CCF_DEFSET", "psv", "PSV_PVSID"),
    ("cpc", "CPC_PRFREF", "prf", "PRF_NUMBR"),
    ("cpc", "CPC_CCAREF", "cca", "CCA_NUMBR"),
    ("cpc", "CPC_PAFREF", "paf", "PAF_NUMBR"),
    ("cdf", "CDF_CNAME", "ccf", "CCF_CNAME"),
    ("cdf", "CDF_PNAME", "cpc", "CPC_PNAME"),
    ("cdf", "CDF_TMID", "pcf", "PCF_NAME"),
    ("ptv", "PTV_CNAME", "ccf", "CCF_CNAME"),
    ("ptv", "PTV_PARNAM", "pcf", "PCF_NAME"),
    ("csf", "CSF_DEFSET", "psv", "PSV_PVSID"),
    ("css", "CSS_SQNAME", "csf", "CSF_NAME"),
    ("sdf", "SDF_SQNAME", "csf", "CSF_NAME"),
    ("sdf", "SDF_VALSET", "psv", "PSV_PVSID"),
    ("csp", "CSP_SQNAME", "csf", "CSF_NAME"),
    ("csp", "CSP_PRFREF", "prf", "PRF_NUMBR"),
    ("csp", "CSP_CCAREF", "cca", "CCA_NUMBR"),
    ("csp", "CSP_PAFREF", "paf", "PAF_NUMBR"),
    ("cvs", "CVS_SPID", "pid", "PID_SPID"),
    ("cve", "CVE_CVSID", "cvs", "CVS_ID"),
    ("cve", "CVE_PARNAM", "pcf", "PCF_NAME"),
    ("cvp", "CVP_CVSID", "cvs", "CVS_ID"),
    ("psv", "PSV_NAME", "pst", "PST_NAME"),
    ("cps", "CPS_NAME", "pst", "PST_NAME"),
    ("pvs", "PVS_ID", "psv", "PSV_PVSID"),
    ("pvs", "PVS_PSID", "pst", "PST_NAME"),
    ("psm", "PSM_PSNAME", "pst", "PST_NAME"),
    ("ccs", "CCS_NUMBR", "cca", "CCA_NUMBR"),
    ("pas", "PAS_NUMBR", "paf", "PAF_NUMBR"),
    ("prv", "PRV_NUMBR", "prf", "PRF_NUMBR"),
];

/// Reports values of [`REFERENCES`] columns that do not exist in the
/// referenced table. References into a table that is absent from the MIB
/// are not checked (the MIB may be partial).
pub(crate) fn check_references(mib: &Mib, out: &mut Vec<Diagnostic>) {
    for &(from_table, from_column, to_table, to_column) in REFERENCES {
        let (Some(from), Some(to)) = (mib.table(from_table), mib.table(to_table)) else {
            continue;
        };
        let (Ok(from_index), Ok(to_index)) =
            (from.column_index(from_column), to.column_index(to_column))
        else {
            continue;
        };
        let targets: HashSet<&str> = to
            .records()
            .iter()
            .filter_map(|r| r.get(to_index))
            .collect();
        for (row, record) in from.records().iter().enumerate() {
            let value = record.get(from_index).unwrap_or("");
            if !value.is_empty() && !targets.contains(value) {
                out.push(Diagnostic {
                    severity: Severity::Warning,
                    table: from.name().to_owned(),
                    row,
                    column: Some(from_column.to_owned()),
                    value: Some(value.to_owned()),
                    kind: DiagnosticKind::DanglingReference {
                        table: to_table,
                        column: to_column,
                    },
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ReadOptions;

    fn table(name: &str, text: &str) -> Table {
        Table::parse(name, text.as_bytes(), &ReadOptions::default()).unwrap()
    }

    fn kinds(diags: &[Diagnostic]) -> Vec<(usize, Option<&str>, &DiagnosticKind)> {
        diags
            .iter()
            .map(|d| (d.row, d.column.as_deref(), &d.kind))
            .collect()
    }

    #[test]
    fn field_checks() {
        let t = table(
            "tcp",
            "HDR1\tok\n\
             \tmissing id\n\
             HDR_TOO_LONG\t\n\
             \n\
             HDR1\tduplicate\n\
             HDR2\t\textra\n\
             HDR3\t\t\t\n",
        );
        let diags = validate_table(&t);
        assert_eq!(
            kinds(&diags),
            [
                (1, Some("TCP_ID"), &DiagnosticKind::MissingMandatory),
                (
                    2,
                    Some("TCP_ID"),
                    &DiagnosticKind::TooLong { len: 12, max: 8 }
                ),
                (3, None, &DiagnosticKind::BlankLine),
                (4, None, &DiagnosticKind::DuplicateKey { first_row: 0 }),
                (
                    5,
                    None,
                    &DiagnosticKind::ExtraFields {
                        count: 3,
                        columns: 2
                    }
                ),
            ]
        );
        assert_eq!(
            diags[1].to_string(),
            "tcp:3: warning: TCP_ID: \"HDR_TOO_LONG\" is too long (12 > 8)"
        );
        assert_eq!(diags[3].severity, Severity::Error);
        assert_eq!(
            diags[3].to_string(),
            "tcp:5: error: duplicate key (HDR1), first defined on line 1"
        );
    }

    #[test]
    fn number_checks() {
        let t = table("dst", "1\tA\n-12\tB\n+7\tC\nx1\tD\n123456\tE\n-\tF\n");
        let diags = validate_table(&t);
        assert_eq!(
            kinds(&diags),
            [
                (3, Some("DST_APID"), &DiagnosticKind::NotAnInteger),
                (
                    4,
                    Some("DST_APID"),
                    &DiagnosticKind::TooLong { len: 6, max: 5 }
                ),
                (5, Some("DST_APID"), &DiagnosticKind::NotAnInteger),
            ]
        );
    }

    #[test]
    fn tables_without_schema_only_check_blank_lines() {
        let t = table("custom", "a\tb\n\n\t\n");
        assert_eq!(
            kinds(&validate_table(&t)),
            [
                (1, None, &DiagnosticKind::BlankLine),
                (2, None, &DiagnosticKind::BlankLine),
            ]
        );
    }

    #[test]
    fn references() {
        let mut mib = Mib::new();
        mib.insert(table("tcp", "HDR1\t\n"));
        mib.insert(table("pcpc", "P1\tParam\t\n"));
        mib.insert(table(
            "pcdf",
            "HDR1\t\tF\t8\t0\t\t1\t\nHDR9\t\tP\t8\t8\tP2\t0\t\n",
        ));
        // CCF references TCP, PSV is absent: CCF_DEFSET is not checked.
        mib.insert(table(
            "ccf",
            "C1\tCmd\t\t\t\tHDR1\t\t\t\t\t\t\t\t\t\t\t\tSET\n",
        ));
        let diags = mib.validate();
        let refs: Vec<String> = diags.iter().map(ToString::to_string).collect();
        assert_eq!(
            refs,
            [
                "pcdf:2: warning: PCDF_TCNAME: \"HDR9\" not found in tcp.TCP_ID",
                "pcdf:2: warning: PCDF_PNAME: \"P2\" not found in pcpc.PCPC_PNAME",
            ]
        );
    }

    #[test]
    fn reference_columns_exist_in_schemas() {
        for &(from, from_col, to, to_col) in REFERENCES {
            let from_schema = crate::icd_table(from).unwrap();
            let to_schema = crate::icd_table(to).unwrap();
            assert!(from_schema.column(from_col).is_some(), "{from_col}");
            assert!(to_schema.column(to_col).is_some(), "{to_col}");
        }
    }
}
