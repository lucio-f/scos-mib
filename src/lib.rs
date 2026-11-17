//! Read, edit and write back SCOS-2000 Mission Information Bases (MIBs).
//!
//! A SCOS-2000 MIB is a directory of ASCII tables (`pcf.dat`, `ccf.dat`,
//! ...), one record per line and one field per tab-separated column, as
//! specified by the SCOS-2000 Database Import ICD
//! (EGOS-MCS-S2K-ICD-0001). This crate loads such a directory into a
//! [`Mib`], lets you query and modify any table, and writes it back.
//!
//! # Design
//!
//! * **Generic.** Every `*.dat` file is loaded as a [`Table`] of
//!   [`Record`]s, whether or not the ICD defines it, and records keep all of
//!   their fields, including mission specific columns appended after the ICD
//!   ones. Nothing is dropped or reinterpreted.
//! * **Lossless.** The field separator, line endings (LF or CRLF), presence
//!   of a final newline and character encoding (UTF-8/ASCII or Latin-1) are
//!   detected per file and reused when writing, so an unmodified table is
//!   written back byte for byte, and an edited one only changes where it was
//!   edited.
//! * **Schema aware.** The ICD tables come with their [`TableSchema`]
//!   (column names, types, key and mandatory flags), so columns can be
//!   addressed by name (`row.get("PCF_DESCR")`) and the MIB can be
//!   [validated](Mib::validate). Schemas for mission specific tables can be
//!   supplied with [`ReadOptions::schema`].
//! * **No dependencies.** The crate only uses the standard library and is
//!   linked statically into the programs that use it.
//!
//! # Example
//!
//! ```
//! use scos_mib::{Mib, Severity};
//! # let dir = std::env::temp_dir().join(format!("scos-mib-doc-{}", std::process::id()));
//! # std::fs::create_dir_all(&dir)?;
//! # std::fs::write(dir.join("pcf.dat"), "VOLT0001\tBus voltage\t\tV\t4\t12\t\t\t\tN\tR\n")?;
//! # std::fs::write(dir.join("tpktsimsids.dat"), "1\t42\n")?;
//!
//! let mut mib = Mib::read_dir(&dir)?;
//!
//! // Query: columns are addressed by their ICD name.
//! let pcf = mib.table("pcf").unwrap();
//! let volt = pcf.find("PCF_NAME", "VOLT0001")?.unwrap();
//! assert_eq!(volt.get("PCF_UNIT")?, Some("V"));
//! assert_eq!(volt.parse::<u8>("PCF_PTC")?, Some(4));
//!
//! // Edit: change a value and add a record.
//! let pcf = mib.table_mut("pcf").unwrap();
//! pcf.row_mut(0).unwrap().set("PCF_DESCR", "Main bus voltage")?;
//! pcf.push_named(&[
//!     ("PCF_NAME", "TEMP0001"),
//!     ("PCF_DESCR", "Battery temperature"),
//!     ("PCF_PTC", "3"),
//!     ("PCF_PFC", "12"),
//!     ("PCF_CATEG", "N"),
//!     ("PCF_NATUR", "R"),
//! ])?;
//!
//! // Tables unknown to the ICD are kept and addressed by position.
//! assert_eq!(mib.table("tpktsimsids").unwrap().records()[0].get(1), Some("42"));
//!
//! // Validate and write back.
//! assert!(mib.validate().iter().all(|d| d.severity < Severity::Error));
//! mib.write_dir(&dir)?;
//! # std::fs::remove_dir_all(&dir)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

mod error;
mod format;
mod mib;
mod record;
mod schema;
mod table;
mod validate;

pub use error::{Error, Result};
pub use format::{Encoding, FileFormat, LineEnding, ReadOptions};
pub use mib::Mib;
pub use record::{Fields, Record};
pub use schema::{Column, ColumnType, ICD_TABLES, TableSchema, icd_table};
pub use table::{ColumnRef, Row, RowMut, Table};
pub use validate::{Diagnostic, DiagnosticKind, REFERENCES, Severity, validate_table};

/// Compiles the README examples as doctests.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
