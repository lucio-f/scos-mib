//! Physical layout of MIB table files and reading options.

use crate::schema::TableSchema;

/// Line terminator used in a table file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineEnding {
    /// `\n` (Unix).
    #[default]
    Lf,
    /// `\r\n` (Windows), common in MIBs exported from Windows tools.
    CrLf,
}

impl LineEnding {
    /// The terminator as a string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

/// Character encoding of a table file.
///
/// The ICD specifies ASCII files, which are valid in both encodings. Files
/// that are valid UTF-8 are read as UTF-8; any other file is read as Latin-1
/// (ISO 8859-1), which maps every byte to one character and therefore
/// always round-trips.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// UTF-8 (includes plain ASCII).
    #[default]
    Utf8,
    /// ISO 8859-1: one byte per character.
    Latin1,
}

/// How a table is laid out on disk.
///
/// The format is detected when a file is read and reused when it is written
/// back, so that an unmodified table is reproduced byte for byte.
///
/// ```
/// use scos_mib::{FileFormat, LineEnding};
///
/// let mut format = FileFormat::default(); // tab separated, LF, final newline
/// format.line_ending = LineEnding::CrLf;
/// assert_eq!(format.separator, '\t');
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct FileFormat {
    /// Field separator. The ICD mandates a tab, but missions may override
    /// it.
    pub separator: char,
    /// Line terminator written after each record.
    pub line_ending: LineEnding,
    /// Whether the last record is followed by a line terminator.
    pub final_newline: bool,
    /// Character encoding.
    pub encoding: Encoding,
}

impl Default for FileFormat {
    fn default() -> Self {
        Self {
            separator: '\t',
            line_ending: LineEnding::Lf,
            final_newline: true,
            encoding: Encoding::Utf8,
        }
    }
}

/// Options for reading tables.
///
/// ```
/// use scos_mib::ReadOptions;
///
/// let options = ReadOptions::new().separator('|');
/// assert_eq!(options.get_separator(), '|');
/// ```
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ReadOptions {
    separator: char,
    schemas: Vec<TableSchema>,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            separator: '\t',
            schemas: Vec::new(),
        }
    }
}

impl ReadOptions {
    /// Default options: tab separated fields, ICD schemas.
    pub fn new() -> Self {
        Self::default()
    }

    /// Uses `separator` instead of a tab to split fields.
    pub fn separator(mut self, separator: char) -> Self {
        self.separator = separator;
        self
    }

    /// The configured field separator.
    pub fn get_separator(&self) -> char {
        self.separator
    }

    /// Registers a schema for a table, e.g. a mission specific table or an
    /// ICD table with mission specific columns. It takes precedence over the
    /// built-in ICD schema of the same name.
    pub fn schema(mut self, schema: TableSchema) -> Self {
        self.schemas.retain(|s| s.name() != schema.name());
        self.schemas.push(schema);
        self
    }

    /// Returns the schema to use for table `name`: a registered one, else
    /// the ICD one, else `None`.
    pub(crate) fn schema_for(&self, name: &str) -> Option<TableSchema> {
        self.schemas
            .iter()
            .find(|s| s.name().eq_ignore_ascii_case(name))
            .or_else(|| crate::schema::icd_table(name))
            .cloned()
    }
}
