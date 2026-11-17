//! Error type of the crate.

use std::fmt;
use std::io;
use std::path::PathBuf;

/// Result type used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Errors returned by this crate.
///
/// Problems with the *content* of a MIB (missing mandatory values, dangling
/// references, ...) are not errors: they are reported as
/// [`Diagnostic`](crate::Diagnostic)s by the validator, because SCOS-2000
/// itself only warns about them at import time. Errors are reserved for
/// conditions that prevent reading or writing the files, or invalid requests
/// made through the API.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// An I/O operation on `path` failed.
    Io {
        /// File or directory being accessed.
        path: PathBuf,
        /// Underlying error.
        source: io::Error,
    },
    /// A column name is not part of the table's schema.
    UnknownColumn {
        /// Table name.
        table: String,
        /// Column name that was requested.
        column: String,
    },
    /// A field value cannot be parsed as the requested type.
    InvalidValue {
        /// Table name.
        table: String,
        /// Zero-based record index.
        row: usize,
        /// Column name, or `#<index>` for unnamed columns.
        column: String,
        /// The offending value.
        value: String,
        /// Why it could not be parsed.
        reason: String,
    },
    /// A field value cannot be written to the file: it contains the field
    /// separator or a line feed.
    UnencodableField {
        /// Table name.
        table: String,
        /// Zero-based record index.
        row: usize,
        /// Zero-based field index.
        field: usize,
        /// The forbidden character.
        character: char,
    },
    /// A table stored as Latin-1 contains a character outside Latin-1.
    UnencodableCharacter {
        /// Table name.
        table: String,
        /// Zero-based record index.
        row: usize,
        /// Zero-based field index.
        field: usize,
        /// The character that cannot be encoded.
        character: char,
    },
    /// Two files in one MIB directory map to the same table name, e.g.
    /// `pcf.dat` and `PCF.dat`.
    DuplicateTable {
        /// Table name (lower case).
        table: String,
    },
    /// A table name is empty or contains characters that are not allowed in
    /// a file name.
    InvalidTableName {
        /// Name that was rejected.
        name: String,
    },
    /// A field separator that can never work (a line terminator).
    InvalidSeparator {
        /// The rejected separator.
        separator: char,
    },
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::UnknownColumn { table, column } => {
                write!(f, "table {table} has no column named {column}")
            }
            Self::InvalidValue {
                table,
                row,
                column,
                value,
                reason,
            } => write!(
                f,
                "{table}, line {}, {column}: invalid value {value:?}: {reason}",
                row + 1
            ),
            Self::UnencodableField {
                table,
                row,
                field,
                character,
            } => write!(
                f,
                "{table}, line {}, field {}: value contains {character:?}, which cannot be \
                 stored in a field",
                row + 1,
                field + 1
            ),
            Self::UnencodableCharacter {
                table,
                row,
                field,
                character,
            } => write!(
                f,
                "{table}, line {}, field {}: {character:?} cannot be encoded as Latin-1",
                row + 1,
                field + 1
            ),
            Self::DuplicateTable { table } => {
                write!(f, "more than one file provides table {table}")
            }
            Self::InvalidTableName { name } => write!(f, "invalid table name {name:?}"),
            Self::InvalidSeparator { separator } => {
                write!(f, "{separator:?} cannot be used as field separator")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}
