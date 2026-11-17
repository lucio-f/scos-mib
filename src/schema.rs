//! Table schemas: column names, types and constraints.
//!
//! The crate ships the schema of every table defined in the SCOS-2000
//! Database Import ICD (EGOS-MCS-S2K-ICD-0001, issue 7.0), see
//! [`icd_table`] and [`ICD_TABLES`]. Schemas are used to address columns by
//! name and to validate records; they never restrict what can be read or
//! written. Tables that are not in the ICD (mission specific tables) are
//! handled without a schema, and records may have fewer columns than the
//! schema (older ICD issues) or more (mission specific extensions, which the
//! ICD requires to be appended at the end of a record).
//!
//! Issue 7.0 is a superset of the earlier issues: from issue 6.7 it adds
//! `CPC_ENDIAN` and `CVS_UNCERTAINTY` at the end of their tables, makes
//! `PIC_APID` part of the PIC key, narrows `CVE_CVSID` and `CVP_CVSID` to
//! `Number(5)` (the size of `CVS_ID` they refer to), and removes the fixed
//! length of the command value fields (`CDF_VALUE`, `CPC_DEFVAL`, ...), see
//! [`ColumnType::Text`]. MIBs written for older issues therefore validate
//! against it.

use std::borrow::Cow;
use std::fmt;

/// Data type of a column, as specified by the ICD.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColumnType {
    /// Character string of at most this many characters (`Char(n)`).
    Char(u16),
    /// Character string without a fixed maximum length (`Char` in the ICD).
    /// Used for command and sequence parameter values, whose allowed size
    /// depends on the type of the parameter they apply to.
    Text,
    /// Integer of at most this many digits (`Number(n)`), optionally signed.
    Number(u16),
}

impl ColumnType {
    /// Maximum number of characters (for `Char`) or digits (for `Number`);
    /// `None` for [`Text`](Self::Text).
    pub fn max_len(self) -> Option<u16> {
        match self {
            Self::Char(n) | Self::Number(n) => Some(n),
            Self::Text => None,
        }
    }
}

impl fmt::Display for ColumnType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Char(n) => write!(f, "Char({n})"),
            Self::Text => f.write_str("Char"),
            Self::Number(n) => write!(f, "Number({n})"),
        }
    }
}

/// Definition of one column of a table.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Column {
    name: Cow<'static, str>,
    ty: ColumnType,
    mandatory: bool,
    key: bool,
}

impl Column {
    /// Creates an optional, non-key column.
    pub fn new(name: impl Into<Cow<'static, str>>, ty: ColumnType) -> Self {
        Self {
            name: name.into(),
            ty,
            mandatory: false,
            key: false,
        }
    }

    const fn builtin(name: &'static str, ty: ColumnType) -> Self {
        Self {
            name: Cow::Borrowed(name),
            ty,
            mandatory: false,
            key: false,
        }
    }

    /// Marks the column as mandatory (it must not be left empty).
    pub const fn mandatory(mut self) -> Self {
        self.mandatory = true;
        self
    }

    /// Marks the column as part of the table key.
    pub const fn key(mut self) -> Self {
        self.key = true;
        self
    }

    /// Column name, e.g. `PCF_NAME`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Column data type.
    pub fn ty(&self) -> ColumnType {
        self.ty
    }

    /// Whether a value is required.
    pub fn is_mandatory(&self) -> bool {
        self.mandatory
    }

    /// Whether the column is part of the table key (the combination of all
    /// key columns identifies a record).
    pub fn is_key(&self) -> bool {
        self.key
    }
}

/// Schema of a table: its name and ordered list of columns.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct TableSchema {
    name: Cow<'static, str>,
    description: Cow<'static, str>,
    columns: Cow<'static, [Column]>,
}

impl TableSchema {
    /// Creates a schema, e.g. for a mission specific table.
    ///
    /// `name` is the table name; it is stored in lower case.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        columns: Vec<Column>,
    ) -> Self {
        Self {
            name: Cow::Owned(name.into().to_ascii_lowercase()),
            description: Cow::Owned(description.into()),
            columns: Cow::Owned(columns),
        }
    }

    const fn builtin(
        name: &'static str,
        description: &'static str,
        columns: &'static [Column],
    ) -> Self {
        Self {
            name: Cow::Borrowed(name),
            description: Cow::Borrowed(description),
            columns: Cow::Borrowed(columns),
        }
    }

    /// Table name in lower case, e.g. `pcf`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Human readable description of the table.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Columns in file order.
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// Returns a copy of this schema with `column` appended, e.g. to describe
    /// a mission specific extension of an ICD table.
    pub fn with_column(&self, column: Column) -> Self {
        let mut columns = self.columns.to_vec();
        columns.push(column);
        Self {
            columns: Cow::Owned(columns),
            ..self.clone()
        }
    }

    /// Returns the position of the column called `name` (case-insensitive).
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Returns the column called `name` (case-insensitive).
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.column_index(name).map(|i| &self.columns[i])
    }

    /// Positions of the key columns.
    pub fn key_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.columns
            .iter()
            .enumerate()
            .filter(|(_, c)| c.key)
            .map(|(i, _)| i)
    }
}

/// Returns the ICD schema of the table called `name` (case-insensitive), or
/// `None` if the table is not defined by the ICD.
///
/// ```
/// let pcf = scos_mib::icd_table("PCF").unwrap();
/// assert_eq!(pcf.columns()[0].name(), "PCF_NAME");
/// assert_eq!(pcf.column_index("PCF_PTC"), Some(4));
/// ```
pub fn icd_table(name: &str) -> Option<&'static TableSchema> {
    ICD_TABLES
        .iter()
        .find(|t| t.name.eq_ignore_ascii_case(name))
}

const fn c(name: &'static str, len: u16) -> Column {
    Column::builtin(name, ColumnType::Char(len))
}

const fn n(name: &'static str, digits: u16) -> Column {
    Column::builtin(name, ColumnType::Number(digits))
}

const fn t(name: &'static str) -> Column {
    Column::builtin(name, ColumnType::Text)
}

// The schemas live in named statics: `Column` and `TableSchema` contain
// `Cow`s, which have drop glue, so they cannot be promoted from temporary
// array expressions (`&[...]`) inside a static initializer.
macro_rules! icd_tables {
    ($($name:literal, $descr:literal => [$($col:expr),* $(,)?];)*) => {
        static ICD_TABLE_ARRAY: [TableSchema; [$($name),*].len()] = [
            $({
                static COLUMNS: [Column; [$(stringify!($col)),*].len()] = [$($col),*];
                TableSchema::builtin($name, $descr, &COLUMNS)
            },)*
        ];

        /// Schemas of all tables defined by the SCOS-2000 Database Import ICD
        /// (EGOS-MCS-S2K-ICD-0001, issue 7.0), in ICD order.
        pub static ICD_TABLES: &[TableSchema] = &ICD_TABLE_ARRAY;
    };
}

icd_tables! {
    // ---------------------------------------------------------------- general
    "vdf", "Database version definition" => [
        c("VDF_NAME", 8).mandatory(),
        c("VDF_COMMENT", 32),
        n("VDF_DOMAINID", 5),
        n("VDF_RELEASE", 5),
        n("VDF_ISSUE", 5),
    ];

    // ------------------------------------------------------------- monitoring
    "pcf", "Monitoring parameter characteristics" => [
        c("PCF_NAME", 8).key().mandatory(),
        c("PCF_DESCR", 24),
        n("PCF_PID", 10),
        c("PCF_UNIT", 4),
        n("PCF_PTC", 2).mandatory(),
        n("PCF_PFC", 5).mandatory(),
        n("PCF_WIDTH", 6),
        c("PCF_VALID", 8),
        c("PCF_RELATED", 8),
        c("PCF_CATEG", 1).mandatory(),
        c("PCF_NATUR", 1).mandatory(),
        c("PCF_CURTX", 10),
        c("PCF_INTER", 1),
        c("PCF_USCON", 1),
        n("PCF_DECIM", 3),
        c("PCF_PARVAL", 14),
        c("PCF_SUBSYS", 8),
        n("PCF_VALPAR", 5),
        c("PCF_SPTYPE", 1),
        c("PCF_CORR", 1),
        n("PCF_OBTID", 5),
        c("PCF_DARC", 1),
        c("PCF_ENDIAN", 1),
    ];
    "cur", "Calibration curve selection" => [
        c("CUR_PNAME", 8).key().mandatory(),
        n("CUR_POS", 2).key().mandatory(),
        c("CUR_RLCHK", 8).mandatory(),
        n("CUR_VALPAR", 5).mandatory(),
        c("CUR_SELECT", 10).mandatory(),
    ];
    "caf", "Numerical calibrations" => [
        c("CAF_NUMBR", 10).key().mandatory(),
        c("CAF_DESCR", 32),
        c("CAF_ENGFMT", 1).mandatory(),
        c("CAF_RAWFMT", 1).mandatory(),
        c("CAF_RADIX", 1),
        c("CAF_UNIT", 4),
        n("CAF_NCURVE", 3),
        c("CAF_INTER", 1),
    ];
    "cap", "Numerical calibration curve points" => [
        c("CAP_NUMBR", 10).key().mandatory(),
        c("CAP_XVALS", 14).key().mandatory(),
        c("CAP_YVALS", 14).mandatory(),
    ];
    "txf", "Textual calibrations" => [
        c("TXF_NUMBR", 10).key().mandatory(),
        c("TXF_DESCR", 32),
        c("TXF_RAWFMT", 1).mandatory(),
        n("TXF_NALIAS", 3),
    ];
    "txp", "Textual calibration definitions" => [
        c("TXP_NUMBR", 10).key().mandatory(),
        c("TXP_FROM", 14).key().mandatory(),
        c("TXP_TO", 14).mandatory(),
        c("TXP_ALTXT", 14).mandatory(),
    ];
    "mcf", "Polynomial calibrations" => [
        c("MCF_IDENT", 10).key().mandatory(),
        c("MCF_DESCR", 32),
        c("MCF_POL1", 14).mandatory(),
        c("MCF_POL2", 14),
        c("MCF_POL3", 14),
        c("MCF_POL4", 14),
        c("MCF_POL5", 14),
    ];
    "lgf", "Logarithmic calibrations" => [
        c("LGF_IDENT", 10).key().mandatory(),
        c("LGF_DESCR", 32),
        c("LGF_POL1", 14).mandatory(),
        c("LGF_POL2", 14),
        c("LGF_POL3", 14),
        c("LGF_POL4", 14),
        c("LGF_POL5", 14),
    ];
    "ocf", "Monitoring checks" => [
        c("OCF_NAME", 8).key().mandatory(),
        n("OCF_NBCHCK", 2).mandatory(),
        n("OCF_NBOOL", 2).mandatory(),
        c("OCF_INTER", 1).mandatory(),
        c("OCF_CODIN", 1).mandatory(),
    ];
    "ocp", "Monitoring check definitions" => [
        c("OCP_NAME", 8).key().mandatory(),
        n("OCP_POS", 2).key().mandatory(),
        c("OCP_TYPE", 1).mandatory(),
        c("OCP_LVALU", 14),
        c("OCP_HVALU", 14),
        c("OCP_RLCHK", 8),
        n("OCP_VALPAR", 5),
    ];
    "pid", "Telemetry packet identification" => [
        n("PID_TYPE", 3).key().mandatory(),
        n("PID_STYPE", 3).key().mandatory(),
        n("PID_APID", 5).key().mandatory(),
        n("PID_PI1_VAL", 10).key(),
        n("PID_PI2_VAL", 10).key(),
        n("PID_SPID", 10).key().mandatory(),
        c("PID_DESCR", 64),
        c("PID_UNIT", 8),
        n("PID_TPSD", 10),
        n("PID_DFHSIZE", 2).mandatory(),
        c("PID_TIME", 1),
        n("PID_INTER", 8),
        c("PID_VALID", 1),
        n("PID_CHECK", 1),
        c("PID_EVENT", 1),
        c("PID_EVID", 17),
    ];
    "pic", "Telemetry packet identification criteria" => [
        n("PIC_TYPE", 3).key().mandatory(),
        n("PIC_STYPE", 3).key().mandatory(),
        n("PIC_PI1_OFF", 5).mandatory(),
        n("PIC_PI1_WID", 3).mandatory(),
        n("PIC_PI2_OFF", 5).mandatory(),
        n("PIC_PI2_WID", 3).mandatory(),
        n("PIC_APID", 5).key(),
    ];
    "tpcf", "Telemetry packet characteristics" => [
        n("TPCF_SPID", 10).key().mandatory(),
        c("TPCF_NAME", 12),
        n("TPCF_SIZE", 8),
    ];
    "plf", "Parameter location in fixed packets" => [
        c("PLF_NAME", 8).key().mandatory(),
        n("PLF_SPID", 10).key().mandatory(),
        n("PLF_OFFBY", 5).mandatory(),
        n("PLF_OFFBI", 1).mandatory(),
        n("PLF_NBOCC", 4),
        n("PLF_LGOCC", 5),
        n("PLF_TIME", 9),
        n("PLF_TDOCC", 9),
    ];
    "vpd", "Variable packet definition" => [
        n("VPD_TPSD", 10).key().mandatory(),
        n("VPD_POS", 4).key().mandatory(),
        c("VPD_NAME", 8).mandatory(),
        n("VPD_GRPSIZE", 3),
        n("VPD_FIXREP", 3),
        c("VPD_CHOICE", 1),
        c("VPD_PIDREF", 1),
        c("VPD_DISDESC", 16),
        n("VPD_WIDTH", 2).mandatory(),
        c("VPD_JUSTIFY", 1),
        c("VPD_NEWLINE", 1),
        n("VPD_DCHAR", 1),
        c("VPD_FORM", 1),
        n("VPD_OFFSET", 6),
    ];
    "grp", "Parameter and packet groups" => [
        c("GRP_NAME", 14).key().mandatory(),
        c("GRP_DESCR", 24).mandatory(),
        c("GRP_GTYPE", 2).mandatory(),
    ];
    "grpa", "Parameter group members" => [
        c("GRPA_GNAME", 14).key().mandatory(),
        c("GRPA_PANAME", 8).key().mandatory(),
    ];
    "grpk", "Packet group members" => [
        c("GRPK_GNAME", 14).key().mandatory(),
        n("GRPK_PKSPID", 10).key().mandatory(),
    ];
    "dpf", "Alphanumeric display definitions" => [
        c("DPF_NUMBE", 8).key().mandatory(),
        c("DPF_TYPE", 1).mandatory(),
        c("DPF_HEAD", 32),
    ];
    "dpc", "Alphanumeric display parameters" => [
        c("DPC_NUMBE", 8).key().mandatory(),
        c("DPC_NAME", 8),
        n("DPC_FLDN", 2).key().mandatory(),
        n("DPC_COMM", 4),
        c("DPC_MODE", 1),
        c("DPC_FORM", 1),
        c("DPC_TEXT", 32),
    ];
    "gpf", "Graphic display definitions" => [
        c("GPF_NUMBE", 8).key().mandatory(),
        c("GPF_TYPE", 1).mandatory(),
        c("GPF_HEAD", 32),
        c("GPF_SCROL", 1),
        c("GPF_HCOPY", 1),
        n("GPF_DAYS", 2).mandatory(),
        n("GPF_HOURS", 2).mandatory(),
        n("GPF_MINUT", 2).mandatory(),
        c("GPF_AXCLR", 1).mandatory(),
        n("GPF_XTICK", 2).mandatory(),
        n("GPF_YTICK", 2).mandatory(),
        n("GPF_XGRID", 2).mandatory(),
        n("GPF_YGRID", 2).mandatory(),
        n("GPF_UPUN", 2),
    ];
    "gpc", "Graphic display parameters" => [
        c("GPC_NUMBE", 8).key().mandatory(),
        n("GPC_POS", 1).key().mandatory(),
        c("GPC_WHERE", 1).mandatory(),
        c("GPC_NAME", 8).mandatory(),
        c("GPC_RAW", 1),
        c("GPC_MINIM", 14).mandatory(),
        c("GPC_MAXIM", 14).mandatory(),
        c("GPC_PRCLR", 1).mandatory(),
        c("GPC_SYMB0", 1),
        c("GPC_LINE", 1),
        n("GPC_DOMAIN", 5),
    ];
    "spf", "Scrolling display definitions" => [
        c("SPF_NUMBE", 8).key().mandatory(),
        c("SPF_HEAD", 32),
        n("SPF_NPAR", 1).mandatory(),
        n("SPF_UPUN", 2),
    ];
    "spc", "Scrolling display parameters" => [
        c("SPC_NUMBE", 8).key().mandatory(),
        n("SPC_POS", 1).key().mandatory(),
        c("SPC_NAME", 8).mandatory(),
        c("SPC_UPDT", 1),
        c("SPC_MODE", 1),
        c("SPC_FORM", 1),
        c("SPC_BACK", 1),
        c("SPC_FORE", 1).mandatory(),
    ];
    "ppf", "Printout proforma definitions" => [
        c("PPF_NUMBE", 4).key().mandatory(),
        c("PPF_HEAD", 32),
        n("PPF_NBPR", 2),
    ];
    "ppc", "Printout proforma parameters" => [
        c("PPC_NUMBE", 4).key().mandatory(),
        n("PPC_POS", 2).key().mandatory(),
        c("PPC_NAME", 8).mandatory(),
        c("PPC_FORM", 1),
    ];

    // ------------------------------------------------------------- commanding
    "tcp", "Telecommand packet headers" => [
        c("TCP_ID", 8).key().mandatory(),
        c("TCP_DESC", 24),
    ];
    "pcpc", "Packet header parameter characteristics" => [
        c("PCPC_PNAME", 8).key().mandatory(),
        c("PCPC_DESC", 24).mandatory(),
        c("PCPC_CODE", 1),
    ];
    "pcdf", "Packet header definitions" => [
        c("PCDF_TCNAME", 8).key().mandatory(),
        c("PCDF_DESC", 24),
        c("PCDF_TYPE", 1).mandatory(),
        n("PCDF_LEN", 4).mandatory(),
        n("PCDF_BIT", 4).key().mandatory(),
        c("PCDF_PNAME", 8),
        c("PCDF_VALUE", 10).mandatory(),
        c("PCDF_RADIX", 1),
    ];
    "ccf", "Command characteristics" => [
        c("CCF_CNAME", 8).key().mandatory(),
        c("CCF_DESCR", 24).mandatory(),
        c("CCF_DESCR2", 64),
        c("CCF_CTYPE", 8),
        c("CCF_CRITICAL", 1),
        c("CCF_PKTID", 8).mandatory(),
        n("CCF_TYPE", 3),
        n("CCF_STYPE", 3),
        n("CCF_APID", 5),
        n("CCF_NPARS", 3),
        c("CCF_PLAN", 1),
        c("CCF_EXEC", 1),
        c("CCF_ILSCOPE", 1),
        c("CCF_ILSTAGE", 1),
        n("CCF_SUBSYS", 3),
        c("CCF_HIPRI", 1),
        n("CCF_MAPID", 2),
        c("CCF_DEFSET", 8),
        n("CCF_RAPID", 5),
        n("CCF_ACK", 2),
        n("CCF_SUBSCHEDID", 5),
    ];
    "dst", "Command routing" => [
        n("DST_APID", 5).key().mandatory(),
        c("DST_ROUTE", 30).mandatory(),
    ];
    "cpc", "Command parameter characteristics" => [
        c("CPC_PNAME", 8).key().mandatory(),
        c("CPC_DESCR", 24),
        n("CPC_PTC", 2).mandatory(),
        n("CPC_PFC", 5).mandatory(),
        c("CPC_DISPFMT", 1),
        c("CPC_RADIX", 1),
        c("CPC_UNIT", 4),
        c("CPC_CATEG", 1),
        c("CPC_PRFREF", 10),
        c("CPC_CCAREF", 10),
        c("CPC_PAFREF", 10),
        c("CPC_INTER", 1),
        t("CPC_DEFVAL"),
        c("CPC_CORR", 1),
        n("CPC_OBTID", 5),
        c("CPC_ENDIAN", 1),
    ];
    "cdf", "Command details" => [
        c("CDF_CNAME", 8).key().mandatory(),
        c("CDF_ELTYPE", 1).mandatory(),
        c("CDF_DESCR", 24),
        n("CDF_ELLEN", 4).mandatory(),
        n("CDF_BIT", 4).key().mandatory(),
        n("CDF_GRPSIZE", 2).key(),
        c("CDF_PNAME", 8),
        c("CDF_INTER", 1),
        t("CDF_VALUE"),
        c("CDF_TMID", 8),
    ];
    "ptv", "Command pre-transmission validation" => [
        c("PTV_CNAME", 8).key().mandatory(),
        c("PTV_PARNAM", 8).key().mandatory(),
        c("PTV_INTER", 1),
        c("PTV_VAL", 17).mandatory(),
    ];
    "csf", "Command sequence characteristics" => [
        c("CSF_NAME", 8).key().mandatory(),
        c("CSF_DESC", 24).mandatory(),
        c("CSF_DESC2", 64),
        c("CSF_IFTT", 1),
        n("CSF_NFPARS", 3),
        n("CSF_ELEMS", 5),
        c("CSF_CRITICAL", 1),
        c("CSF_PLAN", 1),
        c("CSF_EXEC", 1),
        n("CSF_SUBSYS", 3),
        c("CSF_GENTIME", 17),
        c("CSF_DOCNAME", 32),
        c("CSF_ISSUE", 10),
        c("CSF_DATE", 17),
        c("CSF_DEFSET", 8),
        n("CSF_SUBSCHEDID", 5),
    ];
    "css", "Command sequence elements" => [
        c("CSS_SQNAME", 8).key().mandatory(),
        c("CSS_COMM", 32),
        n("CSS_ENTRY", 5).key().mandatory(),
        c("CSS_TYPE", 1).mandatory(),
        c("CSS_ELEMID", 8),
        n("CSS_NPARS", 3),
        c("CSS_MANDISP", 1),
        c("CSS_RELTYPE", 1),
        c("CSS_RELTIME", 8),
        c("CSS_EXTIME", 17),
        c("CSS_PREVREL", 1),
        c("CSS_GROUP", 1),
        c("CSS_BLOCK", 1),
        c("CSS_ILSCOPE", 1),
        c("CSS_ILSTAGE", 1),
        c("CSS_DYNPTV", 1),
        c("CSS_STAPTV", 1),
        c("CSS_CEV", 1),
    ];
    "sdf", "Sequence element parameter values" => [
        c("SDF_SQNAME", 8).key().mandatory(),
        n("SDF_ENTRY", 5).key().mandatory(),
        c("SDF_ELEMID", 8).mandatory(),
        n("SDF_POS", 4).key().mandatory(),
        c("SDF_PNAME", 8).key().mandatory(),
        c("SDF_FTYPE", 1),
        c("SDF_VTYPE", 1),
        t("SDF_VALUE"),
        c("SDF_VALSET", 8),
        n("SDF_REPPOS", 4),
    ];
    "csp", "Command sequence formal parameters" => [
        c("CSP_SQNAME", 8).key().mandatory(),
        c("CSP_FPNAME", 8).key().mandatory(),
        n("CSP_FPNUM", 5).mandatory(),
        c("CSP_DESCR", 24),
        n("CSP_PTC", 2).mandatory(),
        n("CSP_PFC", 5).mandatory(),
        c("CSP_DISPFMT", 1),
        c("CSP_RADIX", 1),
        c("CSP_TYPE", 1).mandatory(),
        c("CSP_VTYPE", 1),
        t("CSP_DEFVAL"),
        c("CSP_CATEG", 1),
        c("CSP_PRFREF", 10),
        c("CSP_CCAREF", 10),
        c("CSP_PAFREF", 10),
        c("CSP_UNIT", 4),
    ];
    "cvs", "Verification stages" => [
        n("CVS_ID", 5).key().mandatory(),
        c("CVS_TYPE", 1).mandatory(),
        c("CVS_SOURCE", 1).mandatory(),
        n("CVS_START", 5).mandatory(),
        n("CVS_INTERVAL", 5).mandatory(),
        n("CVS_SPID", 10),
        n("CVS_UNCERTAINTY", 5),
    ];
    "cve", "Verification expressions" => [
        n("CVE_CVSID", 5).key().mandatory(),
        c("CVE_PARNAM", 8).key().mandatory(),
        c("CVE_INTER", 1),
        c("CVE_VAL", 17),
        c("CVE_TOL", 17),
        c("CVE_CHECK", 1),
    ];
    "cvp", "Verification profiles" => [
        c("CVP_TASK", 8).key().mandatory(),
        c("CVP_TYPE", 1).key(),
        n("CVP_CVSID", 5).key().mandatory(),
    ];
    "pst", "Parameter sets" => [
        c("PST_NAME", 8).key().mandatory(),
        c("PST_DESCR", 24),
    ];
    "psv", "Parameter value sets" => [
        c("PSV_NAME", 8).mandatory(),
        c("PSV_PVSID", 8).key().mandatory(),
        c("PSV_DESCR", 24),
    ];
    "cps", "Parameter set members" => [
        c("CPS_NAME", 8).key().mandatory(),
        c("CPS_PAR", 8).mandatory(),
        n("CPS_BIT", 4).key().mandatory(),
    ];
    "pvs", "Parameter value set values" => [
        c("PVS_ID", 8).key().mandatory(),
        c("PVS_PSID", 8).mandatory(),
        c("PVS_PNAME", 8).mandatory(),
        c("PVS_INTER", 1),
        t("PVS_VALS"),
        n("PVS_BIT", 4).key().mandatory(),
    ];
    "psm", "Parameter set mapping" => [
        c("PSM_NAME", 8).key().mandatory(),
        c("PSM_TYPE", 1).key().mandatory(),
        c("PSM_PARSET", 8).key().mandatory(),
    ];
    "cca", "Command parameter numerical (de-)calibrations" => [
        c("CCA_NUMBR", 10).key().mandatory(),
        c("CCA_DESCR", 24),
        c("CCA_ENGFMT", 1),
        c("CCA_RAWFMT", 1),
        c("CCA_RADIX", 1),
        c("CCA_UNIT", 4),
        n("CCA_NCURVE", 3),
    ];
    "ccs", "Command parameter numerical (de-)calibration points" => [
        c("CCS_NUMBR", 10).key().mandatory(),
        t("CCS_XVALS").mandatory(),
        t("CCS_YVALS").key().mandatory(),
    ];
    "paf", "Command parameter textual (de-)calibrations" => [
        c("PAF_NUMBR", 10).key().mandatory(),
        c("PAF_DESCR", 24),
        c("PAF_RAWFMT", 1),
        n("PAF_NALIAS", 3),
    ];
    "pas", "Command parameter textual (de-)calibration definitions" => [
        c("PAS_NUMBR", 10).key().mandatory(),
        t("PAS_ALTXT").mandatory(),
        t("PAS_ALVAL").key().mandatory(),
    ];
    "prf", "Command parameter range sets" => [
        c("PRF_NUMBR", 10).key().mandatory(),
        c("PRF_DESCR", 24),
        c("PRF_INTER", 1),
        c("PRF_DSPFMT", 1),
        c("PRF_RADIX", 1),
        n("PRF_NRANGE", 3),
        c("PRF_UNIT", 4),
    ];
    "prv", "Command parameter range values" => [
        c("PRV_NUMBR", 10).key().mandatory(),
        t("PRV_MINVAL").key().mandatory(),
        t("PRV_MAXVAL"),
    ];
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn icd_tables_are_consistent() {
        assert_eq!(ICD_TABLES.len(), 53);
        let mut names = HashSet::new();
        for table in ICD_TABLES {
            assert!(names.insert(table.name()), "duplicate {}", table.name());
            assert_eq!(table.name(), table.name().to_ascii_lowercase());
            let prefix = format!("{}_", table.name().to_ascii_uppercase());
            let mut columns = HashSet::new();
            for column in table.columns() {
                assert!(
                    column.name().starts_with(&prefix),
                    "{} in {}",
                    column.name(),
                    table.name()
                );
                assert!(columns.insert(column.name()), "duplicate {}", column.name());
                assert!(column.ty().max_len() != Some(0));
            }
        }
    }

    #[test]
    fn lookup_is_case_insensitive() {
        let pid = icd_table("PID").unwrap();
        assert_eq!(pid.name(), "pid");
        assert_eq!(pid.column_index("pid_spid"), Some(5));
        assert_eq!(pid.column("PID_SPID").unwrap().ty(), ColumnType::Number(10));
        assert!(icd_table("xyz").is_none());
    }

    #[test]
    fn issue_7_0_changes() {
        let names = |table: &str| -> Vec<String> {
            icd_table(table)
                .unwrap()
                .columns()
                .iter()
                .map(|c| c.name().to_owned())
                .collect()
        };
        assert_eq!(names("cpc").last().unwrap(), "CPC_ENDIAN");
        assert_eq!(names("cpc").len(), 16);
        assert_eq!(names("cvs").last().unwrap(), "CVS_UNCERTAINTY");
        assert_eq!(names("psm"), ["PSM_NAME", "PSM_TYPE", "PSM_PARSET"]);

        let pic = icd_table("pic").unwrap();
        assert_eq!(pic.key_indices().collect::<Vec<_>>(), [0, 1, 6]);

        let cve = icd_table("cve").unwrap();
        assert_eq!(cve.column("CVE_CVSID").unwrap().ty(), ColumnType::Number(5));

        let text: Vec<&str> = ICD_TABLES
            .iter()
            .flat_map(|t| t.columns())
            .filter(|c| c.ty() == ColumnType::Text)
            .map(Column::name)
            .collect();
        assert_eq!(
            text,
            [
                "CPC_DEFVAL",
                "CDF_VALUE",
                "SDF_VALUE",
                "CSP_DEFVAL",
                "PVS_VALS",
                "CCS_XVALS",
                "CCS_YVALS",
                "PAS_ALTXT",
                "PAS_ALVAL",
                "PRV_MINVAL",
                "PRV_MAXVAL",
            ]
        );
        assert_eq!(ColumnType::Text.to_string(), "Char");
        assert_eq!(ColumnType::Text.max_len(), None);
    }

    #[test]
    fn custom_schema_and_extension() {
        let ext = icd_table("tcp")
            .unwrap()
            .with_column(Column::new("TCP_MISSION", ColumnType::Char(20)));
        assert_eq!(ext.columns().len(), 3);
        assert_eq!(ext.column_index("TCP_MISSION"), Some(2));
        let custom = TableSchema::new(
            "GSID",
            "Ground station identifiers",
            vec![
                Column::new("GSID_ID", ColumnType::Number(5))
                    .key()
                    .mandatory(),
                Column::new("GSID_NAME", ColumnType::Char(16)),
            ],
        );
        assert_eq!(custom.name(), "gsid");
        assert_eq!(custom.key_indices().collect::<Vec<_>>(), [0]);
        assert_eq!(ColumnType::Char(8).to_string(), "Char(8)");
    }
}
