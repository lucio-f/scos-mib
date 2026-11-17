//! A MIB table: the records of one `.dat` file.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use crate::error::{Error, Result};
use crate::format::{Encoding, FileFormat, LineEnding, ReadOptions};
use crate::record::Record;
use crate::schema::TableSchema;

/// One MIB table, e.g. the content of `pcf.dat`.
///
/// A table is a list of [`Record`]s plus the information needed to write it
/// back exactly as it was read: its file name and [`FileFormat`]. If the
/// table is defined by the ICD (or a schema was supplied through
/// [`ReadOptions::schema`]) it also carries a [`TableSchema`], which allows
/// columns to be addressed by name.
///
/// ```
/// use scos_mib::Table;
///
/// let mut pcf = Table::new("pcf")?;
/// let row = pcf.push_named(&[
///     ("PCF_NAME", "BATV0001"),
///     ("PCF_DESCR", "Battery voltage"),
///     ("PCF_PTC", "4"),
///     ("PCF_PFC", "12"),
///     ("PCF_CATEG", "N"),
///     ("PCF_NATUR", "R"),
/// ])?;
/// assert_eq!(pcf.row(row).unwrap().get("PCF_DESCR")?, Some("Battery voltage"));
///
/// pcf.row_mut(row).unwrap().set("PCF_UNIT", "V")?;
/// let text = String::from_utf8(pcf.to_bytes()?).unwrap();
/// assert!(text.starts_with("BATV0001\tBattery voltage\t\tV\t4\t12\t"));
/// # Ok::<(), scos_mib::Error>(())
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct Table {
    name: String,
    file_name: String,
    schema: Option<TableSchema>,
    format: FileFormat,
    records: Vec<Record>,
}

impl fmt::Debug for Table {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Table")
            .field("name", &self.name)
            .field("file_name", &self.file_name)
            .field("schema", &self.schema.as_ref().map(TableSchema::name))
            .field("format", &self.format)
            .field("records", &self.records.len())
            .finish()
    }
}

impl Table {
    /// Creates an empty table called `name` (stored in `<name>.dat`), using
    /// the ICD schema if the table is defined by the ICD.
    ///
    /// The name is converted to lower case, as required by the ICD.
    pub fn new(name: &str) -> Result<Self> {
        let name = name.to_ascii_lowercase();
        let schema = crate::schema::icd_table(&name).cloned();
        Self::build(name, schema)
    }

    /// Creates an empty table described by `schema`.
    pub fn with_schema(schema: TableSchema) -> Result<Self> {
        Self::build(schema.name().to_owned(), Some(schema))
    }

    fn build(name: String, schema: Option<TableSchema>) -> Result<Self> {
        validate_name(&name)?;
        Ok(Self {
            file_name: format!("{name}.dat"),
            name,
            schema,
            format: FileFormat::default(),
            records: Vec::new(),
        })
    }

    /// Parses the content of a table file.
    ///
    /// `name` is the table name (e.g. `pcf`); the file name is assumed to be
    /// `<name>.dat`. Use [`Table::read_file`] to read from disk.
    pub fn parse(name: &str, bytes: &[u8], options: &ReadOptions) -> Result<Self> {
        let mut table = Self::new(name)?;
        if let Some(schema) = options.schema_for(&table.name) {
            table.schema = Some(schema);
        }
        table.load(bytes, options.get_separator())?;
        Ok(table)
    }

    /// Reads a table file. The table name is the file name without its
    /// extension, in lower case.
    pub fn read_file(path: impl AsRef<Path>, options: &ReadOptions) -> Result<Self> {
        let path = path.as_ref();
        let invalid = || Error::InvalidTableName {
            name: path.display().to_string(),
        };
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(invalid)?;
        let stem = path
            .file_stem()
            .and_then(|n| n.to_str())
            .ok_or_else(invalid)?;
        let bytes = fs::read(path).map_err(|e| Error::io(path, e))?;
        let mut table = Self::parse(stem, &bytes, options)?;
        table.file_name = file_name.to_owned();
        Ok(table)
    }

    fn load(&mut self, bytes: &[u8], separator: char) -> Result<()> {
        check_separator(separator)?;
        let (text, encoding) = match std::str::from_utf8(bytes) {
            Ok(text) => (Cow::Borrowed(text), Encoding::Utf8),
            Err(_) => (
                Cow::Owned(bytes.iter().map(|&b| char::from(b)).collect()),
                Encoding::Latin1,
            ),
        };
        self.format.separator = separator;
        self.format.encoding = encoding;
        self.records.clear();
        if text.is_empty() {
            return Ok(());
        }

        let final_newline = text.ends_with('\n');
        let body = if final_newline {
            &text[..text.len() - 1]
        } else {
            &text[..]
        };
        let lines: Vec<&str> = body.split('\n').collect();
        // Lines followed by a '\n' in the file.
        let terminated = if final_newline {
            lines.len()
        } else {
            lines.len() - 1
        };
        // CRLF only if *every* terminated line ends with '\r'; otherwise any
        // '\r' stays part of the data, so the file still round-trips.
        let crlf = terminated > 0 && lines[..terminated].iter().all(|l| l.ends_with('\r'));

        self.format.final_newline = final_newline;
        self.format.line_ending = if crlf {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        };
        self.records.reserve(lines.len());
        for (i, line) in lines.iter().enumerate() {
            let line = if crlf && i < terminated {
                &line[..line.len() - 1]
            } else {
                line
            };
            self.records.push(Record::parse(line, separator));
        }
        Ok(())
    }

    /// Serializes the table in its [`FileFormat`].
    ///
    /// Fails if a field contains the separator or a line feed, or if the
    /// table is Latin-1 encoded and a field contains a character outside
    /// Latin-1.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        let format = &self.format;
        check_separator(format.separator)?;
        let eol = format.line_ending.as_str();
        let size: usize = self
            .records
            .iter()
            .map(|r| r.iter().map(str::len).sum::<usize>() + r.len() + eol.len())
            .sum();
        let mut out = Vec::with_capacity(size);
        let mut sep = [0; 4];
        let sep = format.separator.encode_utf8(&mut sep);

        for (row, record) in self.records.iter().enumerate() {
            for (field, value) in record.iter().enumerate() {
                if field > 0 {
                    self.encode(&mut out, sep, row, field)?;
                }
                if let Some(character) = value.chars().find(|&c| c == '\n' || c == format.separator)
                {
                    return Err(Error::UnencodableField {
                        table: self.name.clone(),
                        row,
                        field,
                        character,
                    });
                }
                self.encode(&mut out, value, row, field)?;
            }
            if row + 1 < self.records.len() || format.final_newline {
                out.extend_from_slice(eol.as_bytes());
            }
        }
        Ok(out)
    }

    fn encode(&self, out: &mut Vec<u8>, text: &str, row: usize, field: usize) -> Result<()> {
        match self.format.encoding {
            Encoding::Utf8 => out.extend_from_slice(text.as_bytes()),
            Encoding::Latin1 => {
                for character in text.chars() {
                    let byte = u8::try_from(u32::from(character)).map_err(|_| {
                        Error::UnencodableCharacter {
                            table: self.name.clone(),
                            row,
                            field,
                            character,
                        }
                    })?;
                    out.push(byte);
                }
            }
        }
        Ok(())
    }

    /// Writes the table to `path` (replacing the file atomically).
    pub fn write_file(&self, path: impl AsRef<Path>) -> Result<()> {
        crate::mib::write_atomic(path.as_ref(), &self.to_bytes()?)
    }

    /// Table name in lower case, e.g. `pcf`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// File name the table is read from and written to, e.g. `pcf.dat`.
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    /// Schema of the table, if known.
    pub fn schema(&self) -> Option<&TableSchema> {
        self.schema.as_ref()
    }

    /// Replaces the schema (e.g. to add mission specific columns). The
    /// records are not modified.
    pub fn set_schema(&mut self, schema: Option<TableSchema>) {
        self.schema = schema;
    }

    /// File format used when writing the table.
    pub fn format(&self) -> &FileFormat {
        &self.format
    }

    /// Mutable access to the file format, e.g. to convert line endings.
    pub fn format_mut(&mut self) -> &mut FileFormat {
        &mut self.format
    }

    /// Number of records.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Returns `true` if the table has no records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// All records.
    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// Mutable access to the records, e.g. to sort them or edit them in
    /// bulk.
    pub fn records_mut(&mut self) -> &mut Vec<Record> {
        &mut self.records
    }

    /// Appends a record and returns its index.
    pub fn push(&mut self, record: Record) -> usize {
        self.records.push(record);
        self.records.len() - 1
    }

    /// Inserts a record at `index`.
    ///
    /// # Panics
    ///
    /// Panics if `index > len()`.
    pub fn insert(&mut self, index: usize, record: Record) {
        self.records.insert(index, record);
    }

    /// Removes the record at `index` and returns it.
    ///
    /// # Panics
    ///
    /// Panics if `index >= len()`.
    pub fn remove(&mut self, index: usize) -> Record {
        self.records.remove(index)
    }

    /// Removes all records.
    pub fn clear(&mut self) {
        self.records.clear();
    }

    /// Keeps only the rows for which `keep` returns `true`, and returns the
    /// number of removed rows.
    pub fn retain(&mut self, mut keep: impl FnMut(Row<'_>) -> bool) -> usize {
        let mask: Vec<bool> = self.rows().map(&mut keep).collect();
        let before = self.records.len();
        let mut mask = mask.into_iter();
        self.records.retain(|_| mask.next().unwrap_or(true));
        before - self.records.len()
    }

    /// Returns the row at `index`.
    pub fn row(&self, index: usize) -> Option<Row<'_>> {
        (index < self.records.len()).then_some(Row { table: self, index })
    }

    /// Returns a mutable view of the row at `index`.
    pub fn row_mut(&mut self, index: usize) -> Option<RowMut<'_>> {
        (index < self.records.len()).then_some(RowMut { table: self, index })
    }

    /// Iterates over all rows.
    pub fn rows(&self) -> impl ExactSizeIterator<Item = Row<'_>> + DoubleEndedIterator {
        (0..self.records.len()).map(move |index| Row { table: self, index })
    }

    /// Resolves a column (by name or index) to its index.
    pub fn column_index(&self, column: impl ColumnRef) -> Result<usize> {
        column.resolve(self)
    }

    /// Display name of column `index`: its schema name, or `#<n>` (1-based)
    /// for columns not described by the schema.
    pub fn column_name(&self, index: usize) -> Cow<'_, str> {
        match self.schema.as_ref().and_then(|s| s.columns().get(index)) {
            Some(column) => Cow::Borrowed(column.name()),
            None => Cow::Owned(format!("#{}", index + 1)),
        }
    }

    /// Returns the first row whose `column` has exactly the value `value`.
    pub fn find(&self, column: impl ColumnRef, value: &str) -> Result<Option<Row<'_>>> {
        let index = column.resolve(self)?;
        Ok(self
            .rows()
            .find(|row| row.record().get(index).unwrap_or("") == value))
    }

    /// Returns all rows whose `column` has exactly the value `value`.
    pub fn filter<'a>(
        &'a self,
        column: impl ColumnRef,
        value: &'a str,
    ) -> Result<impl Iterator<Item = Row<'a>> + 'a> {
        let index = column.resolve(self)?;
        Ok(self
            .rows()
            .filter(move |row| row.record().get(index).unwrap_or("") == value))
    }

    /// Builds a map from the values of `column` to the index of the first
    /// row holding that value, for fast repeated lookups. Empty values are
    /// skipped.
    pub fn index_by(&self, column: impl ColumnRef) -> Result<HashMap<&str, usize>> {
        let index = column.resolve(self)?;
        let mut map = HashMap::with_capacity(self.records.len());
        for (row, record) in self.records.iter().enumerate() {
            match record.get(index) {
                Some(value) if !value.is_empty() => {
                    map.entry(value).or_insert(row);
                }
                _ => {}
            }
        }
        Ok(map)
    }

    /// Number of fields given to new records: the most common field count
    /// of the existing records, or the number of schema columns for an
    /// empty table. This keeps new records consistent with the ICD issue
    /// the file was produced for.
    pub fn record_width(&self) -> usize {
        let mut counts: HashMap<usize, usize> = HashMap::new();
        for record in &self.records {
            *counts.entry(record.len()).or_default() += 1;
        }
        counts
            .into_iter()
            .max_by_key(|&(width, count)| (count, width))
            .map(|(width, _)| width)
            .unwrap_or_else(|| self.schema.as_ref().map_or(0, |s| s.columns().len()))
    }

    /// Creates an empty record with [`record_width`](Self::record_width)
    /// fields (it is not added to the table).
    pub fn new_record(&self) -> Record {
        Record::with_len(self.record_width())
    }

    /// Appends a new record with the given column values (all other columns
    /// empty) and returns its index.
    ///
    /// Nothing is added if a column is unknown or a value cannot be stored.
    pub fn push_named<C: ColumnRef + Copy>(&mut self, values: &[(C, &str)]) -> Result<usize> {
        let mut record = self.new_record();
        let row = self.records.len();
        for &(column, value) in values {
            let index = column.resolve(self)?;
            self.check_value(row, index, value)?;
            record.set(index, value);
        }
        Ok(self.push(record))
    }

    fn check_value(&self, row: usize, field: usize, value: &str) -> Result<()> {
        for character in value.chars() {
            if character == '\n' || character == self.format.separator {
                return Err(Error::UnencodableField {
                    table: self.name.clone(),
                    row,
                    field,
                    character,
                });
            }
            if self.format.encoding == Encoding::Latin1 && u32::from(character) > 0xFF {
                return Err(Error::UnencodableCharacter {
                    table: self.name.clone(),
                    row,
                    field,
                    character,
                });
            }
        }
        Ok(())
    }

    fn invalid_value(&self, row: usize, column: usize, value: &str, reason: String) -> Error {
        Error::InvalidValue {
            table: self.name.clone(),
            row,
            column: self.column_name(column).into_owned(),
            value: value.to_owned(),
            reason,
        }
    }
}

fn validate_name(name: &str) -> Result<()> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name
            .chars()
            .any(|c| c == '/' || c == '\\' || c.is_control());
    if bad {
        return Err(Error::InvalidTableName {
            name: name.to_owned(),
        });
    }
    Ok(())
}

fn check_separator(separator: char) -> Result<()> {
    if separator == '\n' || separator == '\r' {
        return Err(Error::InvalidSeparator { separator });
    }
    Ok(())
}

/// Something that identifies a column: its zero-based index (`usize`) or
/// its name (`&str`, case-insensitive; requires a schema).
pub trait ColumnRef {
    /// Returns the column index within `table`.
    fn resolve(&self, table: &Table) -> Result<usize>;
}

impl ColumnRef for usize {
    fn resolve(&self, _table: &Table) -> Result<usize> {
        Ok(*self)
    }
}

impl ColumnRef for &str {
    fn resolve(&self, table: &Table) -> Result<usize> {
        table
            .schema
            .as_ref()
            .and_then(|s| s.column_index(self))
            .ok_or_else(|| Error::UnknownColumn {
                table: table.name.clone(),
                column: (*self).to_owned(),
            })
    }
}

impl ColumnRef for &String {
    fn resolve(&self, table: &Table) -> Result<usize> {
        self.as_str().resolve(table)
    }
}

/// A read-only view of one row of a [`Table`], with access by column name.
#[derive(Clone, Copy)]
pub struct Row<'a> {
    table: &'a Table,
    index: usize,
}

impl fmt::Debug for Row<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Row")
            .field("table", &self.table.name)
            .field("index", &self.index)
            .field("record", self.record())
            .finish()
    }
}

impl<'a> Row<'a> {
    /// Zero-based index of the row in its table.
    pub fn index(&self) -> usize {
        self.index
    }

    /// One-based line number of the row in the table file.
    pub fn line(&self) -> usize {
        self.index + 1
    }

    /// The table the row belongs to.
    pub fn table(&self) -> &'a Table {
        self.table
    }

    /// The underlying record (raw fields).
    pub fn record(&self) -> &'a Record {
        &self.table.records[self.index]
    }

    /// Value of `column`, or `None` if it is null (empty, or omitted at the
    /// end of the record).
    pub fn get(&self, column: impl ColumnRef) -> Result<Option<&'a str>> {
        let index = column.resolve(self.table)?;
        Ok(self.record().get(index).filter(|v| !v.is_empty()))
    }

    /// Value of `column`, or `""` if it is null.
    pub fn get_or_empty(&self, column: impl ColumnRef) -> Result<&'a str> {
        Ok(self.get(column)?.unwrap_or(""))
    }

    /// Parses the value of `column` as `T`; `None` if it is null.
    ///
    /// Surrounding spaces are ignored.
    pub fn parse<T>(&self, column: impl ColumnRef) -> Result<Option<T>>
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        let index = column.resolve(self.table)?;
        parse_field(self.table, self.index, index)
    }
}

fn parse_field<T>(table: &Table, row: usize, column: usize) -> Result<Option<T>>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    match table.records[row].get(column) {
        None | Some("") => Ok(None),
        Some(value) => value
            .trim()
            .parse()
            .map(Some)
            .map_err(|e: T::Err| table.invalid_value(row, column, value, e.to_string())),
    }
}

/// A mutable view of one row of a [`Table`], with access by column name.
pub struct RowMut<'a> {
    table: &'a mut Table,
    index: usize,
}

impl fmt::Debug for RowMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_row().fmt(f)
    }
}

impl RowMut<'_> {
    /// Read-only view of the same row.
    pub fn as_row(&self) -> Row<'_> {
        Row {
            table: self.table,
            index: self.index,
        }
    }

    /// Zero-based index of the row in its table.
    pub fn index(&self) -> usize {
        self.index
    }

    /// The underlying record (raw fields).
    pub fn record(&self) -> &Record {
        &self.table.records[self.index]
    }

    /// Mutable access to the underlying record.
    pub fn record_mut(&mut self) -> &mut Record {
        &mut self.table.records[self.index]
    }

    /// Value of `column`, or `None` if it is null.
    pub fn get(&self, column: impl ColumnRef) -> Result<Option<&str>> {
        let index = column.resolve(self.table)?;
        Ok(self.record().get(index).filter(|v| !v.is_empty()))
    }

    /// Parses the value of `column` as `T`; `None` if it is null.
    pub fn parse<T>(&self, column: impl ColumnRef) -> Result<Option<T>>
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        let index = column.resolve(self.table)?;
        parse_field(self.table, self.index, index)
    }

    /// Sets `column` to `value`. The record is padded with empty fields if
    /// it has fewer fields than needed.
    ///
    /// Fails if the value contains the separator or a line feed, or a
    /// character that the table's encoding cannot represent.
    pub fn set(&mut self, column: impl ColumnRef, value: impl AsRef<str>) -> Result<&mut Self> {
        let index = column.resolve(self.table)?;
        let value = value.as_ref();
        self.table.check_value(self.index, index, value)?;
        self.record_mut().set(index, value);
        Ok(self)
    }

    /// Sets `column` to null (empty). Fields beyond the end of the record
    /// are already null and are not added.
    pub fn clear(&mut self, column: impl ColumnRef) -> Result<&mut Self> {
        let index = column.resolve(self.table)?;
        if index < self.record().len() {
            self.record_mut().set(index, "");
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(bytes: &[u8]) -> Table {
        Table::parse("pcf", bytes, &ReadOptions::default()).unwrap()
    }

    fn roundtrip(bytes: &[u8]) {
        let table = parse(bytes);
        assert_eq!(
            table.to_bytes().unwrap(),
            bytes,
            "round trip of {:?}",
            String::from_utf8_lossy(bytes)
        );
    }

    #[test]
    fn round_trips_layout_variants() {
        for case in [
            &b""[..],
            b"\n",
            b"\r\n",
            b"a",
            b"a\n",
            b"a\r\n",
            b"a\tb\t\n\tc\n",
            b"a\r\nb",
            b"a\r\nb\r",
            b"a\r\nb\n",
            b"a\n\nb\n",
            b"a\tb\r\n\r\n\tc\t\t\r\n",
            b"caf\xe9\t\xb5s\n",
            "°C\tµs\n".as_bytes(),
        ] {
            roundtrip(case);
        }
    }

    #[test]
    fn detects_format() {
        let t = parse(b"a\tb\r\nc\td");
        assert_eq!(t.format().line_ending, LineEnding::CrLf);
        assert!(!t.format().final_newline);
        assert_eq!(t.records()[1].to_vec(), ["c", "d"]);

        let mixed = parse(b"a\r\nb\n");
        assert_eq!(mixed.format().line_ending, LineEnding::Lf);
        assert_eq!(mixed.records()[0].to_vec(), ["a\r"]);

        let latin1 = parse(b"\xb0C\n");
        assert_eq!(latin1.format().encoding, Encoding::Latin1);
        assert_eq!(latin1.records()[0].get(0), Some("°C"));

        let empty = parse(b"");
        assert!(empty.is_empty());
    }

    #[test]
    fn custom_separator() {
        let options = ReadOptions::new().separator('|');
        let t = Table::parse("tcp", b"HDR1|Header one\nHDR2|\n", &options).unwrap();
        assert_eq!(t.row(1).unwrap().get("TCP_ID").unwrap(), Some("HDR2"));
        assert_eq!(t.to_bytes().unwrap(), b"HDR1|Header one\nHDR2|\n");
        assert!(matches!(
            Table::parse("tcp", b"", &ReadOptions::new().separator('\n')),
            Err(Error::InvalidSeparator { .. })
        ));
    }

    #[test]
    fn named_access_and_parse() {
        let t = parse(b"P1\tDescr\t\t\t3\t16\n");
        let row = t.row(0).unwrap();
        assert_eq!(row.get("pcf_name").unwrap(), Some("P1"));
        assert_eq!(row.get("PCF_PID").unwrap(), None);
        assert_eq!(row.get("PCF_ENDIAN").unwrap(), None); // beyond record end
        assert_eq!(row.get_or_empty(2).unwrap(), "");
        assert_eq!(row.parse::<u32>("PCF_PFC").unwrap(), Some(16));
        assert_eq!(row.parse::<u32>("PCF_PID").unwrap(), None);
        assert!(matches!(
            row.parse::<u32>("PCF_DESCR"),
            Err(Error::InvalidValue { ref column, .. }) if column == "PCF_DESCR"
        ));
        assert!(matches!(row.get("NOPE"), Err(Error::UnknownColumn { .. })));
        assert_eq!(row.line(), 1);
    }

    #[test]
    fn editing() {
        let mut t = parse(b"P1\tA\nP2\tB\nP3\tC\n");
        t.row_mut(1)
            .unwrap()
            .set("PCF_DESCR", "Bee")
            .unwrap()
            .set("PCF_UNIT", "V")
            .unwrap();
        t.row_mut(0).unwrap().clear("PCF_DESCR").unwrap();
        t.row_mut(0).unwrap().clear("PCF_UNIT").unwrap(); // past the end: no-op
        assert_eq!(t.retain(|r| r.get(0).unwrap() != Some("P3")), 1);
        let new = t
            .push_named(&[("PCF_NAME", "P4"), ("PCF_PTC", "3")])
            .unwrap();
        assert_eq!(new, 2);
        assert_eq!(t.to_bytes().unwrap(), b"P1\t\nP2\tBee\t\tV\nP4\t\t\t\t3\n");

        let mut row = t.row_mut(0).unwrap();
        assert!(matches!(
            row.set("PCF_DESCR", "a\tb"),
            Err(Error::UnencodableField {
                character: '\t',
                ..
            })
        ));
        assert!(row.set("PCF_DESCR", "line\nbreak").is_err());
        // A failed push leaves the table untouched.
        assert!(t.push_named(&[("PCF_NAME", "X"), ("BAD", "1")]).is_err());
        assert_eq!(t.len(), 3);
    }

    #[test]
    fn latin1_tables_reject_unencodable_characters() {
        let mut t = parse(b"\xb5\n");
        t.row_mut(0).unwrap().set(0, "é").unwrap();
        assert_eq!(t.to_bytes().unwrap(), b"\xe9\n");
        assert!(matches!(
            t.row_mut(0).unwrap().set(0, "€"),
            Err(Error::UnencodableCharacter {
                character: '€', ..
            })
        ));
        t.records_mut()[0].set(0, "€");
        assert!(matches!(
            t.to_bytes(),
            Err(Error::UnencodableCharacter { .. })
        ));
    }

    #[test]
    fn write_rejects_separator_in_raw_records() {
        let mut t = parse(b"a\n");
        t.records_mut()[0].set(1, "x\ty");
        assert!(matches!(
            t.to_bytes(),
            Err(Error::UnencodableField {
                row: 0,
                field: 1,
                ..
            })
        ));
    }

    #[test]
    fn lookup_helpers() {
        let t = parse(b"P1\tA\nP2\tB\nP1\tC\n\tD\n");
        assert_eq!(t.find("PCF_NAME", "P2").unwrap().unwrap().index(), 1);
        assert_eq!(t.filter("PCF_NAME", "P1").unwrap().count(), 2);
        let index = t.index_by("PCF_NAME").unwrap();
        assert_eq!(index.len(), 2);
        assert_eq!(index["P1"], 0);
        assert_eq!(t.record_width(), 2);
        assert_eq!(Table::new("pcf").unwrap().record_width(), 23);
        assert_eq!(Table::new("custom").unwrap().record_width(), 0);
    }

    #[test]
    fn names() {
        let t = Table::new("PCF").unwrap();
        assert_eq!((t.name(), t.file_name()), ("pcf", "pcf.dat"));
        assert!(t.schema().is_some());
        assert!(Table::new("custom").unwrap().schema().is_none());
        for bad in ["", ".", "..", "a/b", "a\\b", "a\0"] {
            assert!(Table::new(bad).is_err(), "{bad:?}");
        }
        assert_eq!(t.column_name(0), "PCF_NAME");
        assert_eq!(t.column_name(30), "#31");
    }
}
