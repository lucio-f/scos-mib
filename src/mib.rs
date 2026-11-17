//! A complete MIB: the set of tables stored in one directory.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::format::{FileFormat, ReadOptions};
use crate::table::Table;
use crate::validate::{self, Diagnostic};

/// A SCOS-2000 MIB: all tables of one database directory.
///
/// Every `*.dat` file of the directory is loaded as a [`Table`], whether or
/// not it is defined by the ICD, so mission specific tables are preserved.
/// Tables are keyed by their lower-case name (`pcf`, `ccf`, ...).
///
/// Other files (synthetic parameter expressions, display definitions, ...)
/// are not loaded and are never modified or deleted by
/// [`write_dir`](Mib::write_dir).
///
/// ```no_run
/// use scos_mib::Mib;
///
/// let mut mib = Mib::read_dir("path/to/ASCII")?;
/// let pcf = mib.table_mut("pcf").expect("no pcf.dat");
/// let row = pcf.find("PCF_NAME", "BATV0001")?.map(|r| r.index());
/// if let Some(row) = row {
///     pcf.row_mut(row).unwrap().set("PCF_DESCR", "Battery voltage (main)")?;
/// }
/// for diagnostic in mib.validate() {
///     eprintln!("{diagnostic}");
/// }
/// mib.write_dir("path/to/ASCII")?; // only pcf.dat is rewritten
/// # Ok::<(), scos_mib::Error>(())
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Mib {
    tables: BTreeMap<String, Table>,
    default_format: FileFormat,
}

impl Mib {
    /// Creates an empty MIB.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads every `*.dat` file of `dir` with default options (tab
    /// separated fields).
    pub fn read_dir(dir: impl AsRef<Path>) -> Result<Self> {
        Self::read_dir_with(dir, &ReadOptions::default())
    }

    /// Reads every `*.dat` file of `dir` (the extension is matched
    /// case-insensitively; subdirectories are ignored).
    pub fn read_dir_with(dir: impl AsRef<Path>, options: &ReadOptions) -> Result<Self> {
        let dir = dir.as_ref();
        let mut paths = Vec::new();
        for entry in fs::read_dir(dir).map_err(|e| Error::io(dir, e))? {
            let path = entry.map_err(|e| Error::io(dir, e))?.path();
            let is_dat = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("dat"));
            // `metadata` follows symbolic links.
            if is_dat
                && fs::metadata(&path)
                    .map_err(|e| Error::io(&path, e))?
                    .is_file()
            {
                paths.push(path);
            }
        }
        paths.sort();

        let mut mib = Self::new();
        for path in paths {
            let table = Table::read_file(&path, options)?;
            if mib.tables.contains_key(table.name()) {
                return Err(Error::DuplicateTable {
                    table: table.name().to_owned(),
                });
            }
            mib.tables.insert(table.name().to_owned(), table);
        }
        mib.default_format = mib.dominant_format();
        // An empty file has no detectable layout: records added later
        // should follow the rest of the MIB. (An empty table is written as
        // an empty file whatever its format, so this is still lossless.)
        for table in mib.tables.values_mut() {
            if table.is_empty() {
                *table.format_mut() = mib.default_format.clone();
            }
        }
        Ok(mib)
    }

    /// Most common format among the non-empty tables (ties broken by table
    /// order), used for tables created later.
    fn dominant_format(&self) -> FileFormat {
        let mut counts: HashMap<&FileFormat, (usize, usize)> = HashMap::new();
        for (order, table) in self.tables.values().enumerate() {
            if !table.is_empty() {
                let entry = counts.entry(table.format()).or_insert((0, order));
                entry.0 += 1;
            }
        }
        counts
            .into_iter()
            .max_by_key(|&(_, (count, order))| (count, std::cmp::Reverse(order)))
            .map(|(format, _)| format.clone())
            .unwrap_or_default()
    }

    /// Writes all tables to `dir`, creating it if needed.
    ///
    /// Every table is serialized before anything is written, so a table that
    /// cannot be encoded leaves the directory untouched. Each file is
    /// replaced atomically, and files whose content would not change are not
    /// rewritten. Files of tables that are not in the MIB are left alone.
    pub fn write_dir(&self, dir: impl AsRef<Path>) -> Result<()> {
        let dir = dir.as_ref();
        let files = self
            .tables
            .values()
            .map(|t| Ok((dir.join(t.file_name()), t.to_bytes()?)))
            .collect::<Result<Vec<(PathBuf, Vec<u8>)>>>()?;
        fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        for (path, bytes) in files {
            match fs::read(&path) {
                Ok(existing) if existing == bytes => continue,
                _ => write_atomic(&path, &bytes)?,
            }
        }
        Ok(())
    }

    /// Number of tables.
    pub fn len(&self) -> usize {
        self.tables.len()
    }

    /// Returns `true` if the MIB has no tables.
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }

    /// Returns the table called `name` (case-insensitive).
    pub fn table(&self, name: &str) -> Option<&Table> {
        self.tables.get(&name.to_ascii_lowercase())
    }

    /// Returns the table called `name` (case-insensitive) for editing.
    pub fn table_mut(&mut self, name: &str) -> Option<&mut Table> {
        self.tables.get_mut(&name.to_ascii_lowercase())
    }

    /// Returns the table called `name`, creating an empty one if needed. A
    /// new table uses the ICD schema (if any) and the MIB's
    /// [default format](Mib::default_format).
    pub fn table_or_create(&mut self, name: &str) -> Result<&mut Table> {
        let key = name.to_ascii_lowercase();
        if !self.tables.contains_key(&key) {
            let mut table = Table::new(&key)?;
            *table.format_mut() = self.default_format.clone();
            self.tables.insert(key.clone(), table);
        }
        Ok(self.tables.get_mut(&key).expect("table was just inserted"))
    }

    /// Adds a table, replacing (and returning) any table of the same name.
    pub fn insert(&mut self, table: Table) -> Option<Table> {
        self.tables.insert(table.name().to_owned(), table)
    }

    /// Removes a table from the MIB and returns it. Its file is not deleted
    /// by [`write_dir`](Mib::write_dir).
    pub fn remove(&mut self, name: &str) -> Option<Table> {
        self.tables.remove(&name.to_ascii_lowercase())
    }

    /// Iterates over the tables in name order.
    pub fn tables(&self) -> impl ExactSizeIterator<Item = &Table> {
        self.tables.values()
    }

    /// Iterates mutably over the tables in name order.
    pub fn tables_mut(&mut self) -> impl ExactSizeIterator<Item = &mut Table> {
        self.tables.values_mut()
    }

    /// Format given to tables created by
    /// [`table_or_create`](Mib::table_or_create). After
    /// [`read_dir`](Mib::read_dir) it is the format most tables use.
    pub fn default_format(&self) -> &FileFormat {
        &self.default_format
    }

    /// Changes the format given to tables created later.
    pub fn set_default_format(&mut self, format: FileFormat) {
        self.default_format = format;
    }

    /// Checks every table against its schema and the references between
    /// tables, see [`Diagnostic`].
    pub fn validate(&self) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for table in self.tables.values() {
            validate::check_table(table, &mut out);
        }
        validate::check_references(self, &mut out);
        out
    }
}

/// Writes `bytes` to `path` through a temporary file in the same directory,
/// so readers never see a partially written table.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let file_name =
        path.file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| Error::InvalidTableName {
                name: path.display().to_string(),
            })?;
    let tmp = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    let result = (|| {
        fs::write(&tmp, bytes)?;
        if let Ok(metadata) = fs::metadata(path) {
            fs::set_permissions(&tmp, metadata.permissions())?;
        }
        fs::rename(&tmp, path)
    })();
    result.map_err(|e| {
        let _ = fs::remove_file(&tmp);
        Error::io(path, e)
    })
}
