//! A single record (line) of a MIB table.

use std::fmt;
use std::ops::Index;

/// One record of a MIB table, i.e. one line of a `.dat` file.
///
/// A record is an ordered list of text fields. It keeps exactly the fields
/// found in the file, including empty ("null") fields and trailing empty
/// fields produced by a trailing separator, so that writing a record back
/// reproduces the original line.
///
/// Fields are stored in a single string buffer, so a record costs two
/// allocations regardless of how many fields it has.
///
/// ```
/// use scos_mib::Record;
///
/// let mut r = Record::from_fields(["PARAM001", "Battery voltage", ""]);
/// assert_eq!(r.len(), 3);
/// assert_eq!(r.get(1), Some("Battery voltage"));
/// r.set(4, "V"); // setting past the end pads with empty fields
/// assert_eq!(r.to_vec(), ["PARAM001", "Battery voltage", "", "", "V"]);
/// ```
#[derive(Clone, Default, PartialEq, Eq, Hash)]
pub struct Record {
    /// Concatenated contents of all fields, without separators.
    text: String,
    /// `ends[i]` is the byte offset in `text` where field `i` ends.
    ends: Vec<u32>,
}

impl Record {
    /// Creates a record without any fields.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a record with `len` empty fields.
    pub fn with_len(len: usize) -> Self {
        Self {
            text: String::new(),
            ends: vec![0; len],
        }
    }

    /// Creates a record from a list of field values.
    pub fn from_fields<I, S>(fields: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut record = Self::new();
        for field in fields {
            record.push(field.as_ref());
        }
        record
    }

    /// Splits one line (without its line terminator) on `separator`.
    pub(crate) fn parse(line: &str, separator: char) -> Self {
        let mut record = Self {
            text: String::with_capacity(line.len()),
            ends: Vec::new(),
        };
        for field in line.split(separator) {
            record.push(field);
        }
        record
    }

    /// Number of fields in the record (including empty ones).
    pub fn len(&self) -> usize {
        self.ends.len()
    }

    /// Returns `true` if the record has no fields at all.
    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// Returns `true` if every field is empty (e.g. a blank line).
    pub fn is_blank(&self) -> bool {
        self.text.is_empty()
    }

    fn start(&self, index: usize) -> usize {
        if index == 0 {
            0
        } else {
            self.ends[index - 1] as usize
        }
    }

    /// Returns the raw value of field `index`, or `None` if the record has
    /// fewer fields. An empty field is returned as `Some("")`.
    pub fn get(&self, index: usize) -> Option<&str> {
        let end = *self.ends.get(index)? as usize;
        Some(&self.text[self.start(index)..end])
    }

    /// Iterates over all field values.
    pub fn iter(&self) -> Fields<'_> {
        Fields {
            record: self,
            front: 0,
            back: self.len(),
        }
    }

    /// Appends a field.
    pub fn push(&mut self, value: &str) {
        self.text.push_str(value);
        self.ends.push(offset(self.text.len()));
    }

    /// Sets field `index` to `value`.
    ///
    /// If the record is shorter than `index + 1` fields it is first padded
    /// with empty fields.
    pub fn set(&mut self, index: usize, value: &str) {
        if index >= self.len() {
            self.resize(index);
            self.push(value);
            return;
        }
        let start = self.start(index);
        let end = self.ends[index] as usize;
        self.text.replace_range(start..end, value);
        let new_end = start + value.len();
        // Shift the end offsets of this and all later fields.
        for e in &mut self.ends[index..] {
            let shifted = if new_end >= end {
                *e as usize + (new_end - end)
            } else {
                *e as usize - (end - new_end)
            };
            *e = offset(shifted);
        }
    }

    /// Inserts a field at position `index`, shifting later fields right.
    ///
    /// # Panics
    ///
    /// Panics if `index > len()`.
    pub fn insert(&mut self, index: usize, value: &str) {
        assert!(
            index <= self.len(),
            "insertion index {index} out of bounds (len {})",
            self.len()
        );
        let start = self.start(index);
        self.text.insert_str(start, value);
        let delta = value.len();
        for e in &mut self.ends[index..] {
            *e = offset(*e as usize + delta);
        }
        self.ends.insert(index, offset(start + delta));
    }

    /// Removes field `index` and returns its value, shifting later fields
    /// left.
    ///
    /// # Panics
    ///
    /// Panics if `index >= len()`.
    pub fn remove(&mut self, index: usize) -> String {
        assert!(
            index < self.len(),
            "removal index {index} out of bounds (len {})",
            self.len()
        );
        let start = self.start(index);
        let end = self.ends[index] as usize;
        let value: String = self.text.drain(start..end).collect();
        self.ends.remove(index);
        let delta = end - start;
        for e in &mut self.ends[index..] {
            *e = offset(*e as usize - delta);
        }
        value
    }

    /// Shortens the record to `len` fields. Has no effect if the record is
    /// already shorter.
    pub fn truncate(&mut self, len: usize) {
        if len < self.len() {
            self.text.truncate(self.start(len));
            self.ends.truncate(len);
        }
    }

    /// Resizes the record to exactly `len` fields, padding with empty fields
    /// or truncating as needed.
    pub fn resize(&mut self, len: usize) {
        if len < self.len() {
            self.truncate(len);
        } else {
            let end = offset(self.text.len());
            self.ends.resize(len, end);
        }
    }

    /// Removes trailing empty fields, keeping at least `min_len` fields.
    pub fn trim_trailing_empty(&mut self, min_len: usize) {
        while self.len() > min_len && self.get(self.len() - 1) == Some("") {
            self.ends.pop();
        }
    }

    /// Copies the field values into a vector.
    pub fn to_vec(&self) -> Vec<String> {
        self.iter().map(str::to_owned).collect()
    }
}

fn offset(len: usize) -> u32 {
    u32::try_from(len).expect("a single MIB record cannot exceed 4 GiB")
}

impl Index<usize> for Record {
    type Output = str;

    /// Returns field `index`.
    ///
    /// # Panics
    ///
    /// Panics if the record has fewer than `index + 1` fields; use
    /// [`Record::get`] for a non-panicking version.
    fn index(&self, index: usize) -> &str {
        match self.get(index) {
            Some(value) => value,
            None => panic!(
                "field index {index} out of bounds (record has {} fields)",
                self.len()
            ),
        }
    }
}

impl fmt::Debug for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl<S: AsRef<str>> FromIterator<S> for Record {
    fn from_iter<I: IntoIterator<Item = S>>(iter: I) -> Self {
        Self::from_fields(iter)
    }
}

impl<S: AsRef<str>> Extend<S> for Record {
    fn extend<I: IntoIterator<Item = S>>(&mut self, iter: I) {
        for value in iter {
            self.push(value.as_ref());
        }
    }
}

impl<'a> IntoIterator for &'a Record {
    type Item = &'a str;
    type IntoIter = Fields<'a>;

    fn into_iter(self) -> Fields<'a> {
        self.iter()
    }
}

/// Iterator over the field values of a [`Record`].
#[derive(Clone, Debug)]
pub struct Fields<'a> {
    record: &'a Record,
    front: usize,
    back: usize,
}

impl<'a> Iterator for Fields<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        if self.front == self.back {
            return None;
        }
        let value = self.record.get(self.front);
        self.front += 1;
        value
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.back - self.front;
        (n, Some(n))
    }
}

impl DoubleEndedIterator for Fields<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        if self.front == self.back {
            return None;
        }
        self.back -= 1;
        self.record.get(self.back)
    }
}

impl ExactSizeIterator for Fields<'_> {}

impl std::iter::FusedIterator for Fields<'_> {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keeps_empty_and_trailing_fields() {
        let r = Record::parse("A\t\tC\t", '\t');
        assert_eq!(r.to_vec(), ["A", "", "C", ""]);
        assert_eq!(Record::parse("", '\t').to_vec(), [""]);
        assert_eq!(Record::parse("a|b", '|').to_vec(), ["a", "b"]);
    }

    #[test]
    fn set_shrinks_grows_and_pads() {
        let mut r = Record::from_fields(["one", "two", "three"]);
        r.set(1, "2");
        assert_eq!(r.to_vec(), ["one", "2", "three"]);
        r.set(0, "uno-longer");
        assert_eq!(r.to_vec(), ["uno-longer", "2", "three"]);
        r.set(2, "");
        assert_eq!(r.to_vec(), ["uno-longer", "2", ""]);
        r.set(5, "x");
        assert_eq!(r.to_vec(), ["uno-longer", "2", "", "", "", "x"]);
    }

    #[test]
    fn insert_and_remove() {
        let mut r = Record::from_fields(["a", "c"]);
        r.insert(1, "b");
        r.insert(0, "start");
        r.insert(4, "end");
        assert_eq!(r.to_vec(), ["start", "a", "b", "c", "end"]);
        assert_eq!(r.remove(0), "start");
        assert_eq!(r.remove(3), "end");
        assert_eq!(r.remove(1), "b");
        assert_eq!(r.to_vec(), ["a", "c"]);
    }

    #[test]
    fn resize_truncate_and_trim() {
        let mut r = Record::from_fields(["a", "b", "", ""]);
        r.trim_trailing_empty(3);
        assert_eq!(r.to_vec(), ["a", "b", ""]);
        r.trim_trailing_empty(0);
        assert_eq!(r.to_vec(), ["a", "b"]);
        r.resize(4);
        assert_eq!(r.to_vec(), ["a", "b", "", ""]);
        r.truncate(1);
        assert_eq!(r.to_vec(), ["a"]);
        assert!(!r.is_blank());
        assert!(Record::with_len(3).is_blank());
    }

    #[test]
    fn iterators_and_index() {
        let r: Record = ["x", "y", "z"].into_iter().collect();
        assert_eq!(r.iter().rev().collect::<Vec<_>>(), ["z", "y", "x"]);
        assert_eq!(r.iter().len(), 3);
        assert_eq!(&r[2], "z");
        assert_eq!(r.get(3), None);
        assert_eq!(format!("{r:?}"), r#"["x", "y", "z"]"#);
    }

    #[test]
    fn multibyte_values() {
        let mut r = Record::from_fields(["µs", "°C"]);
        r.set(0, "ms");
        assert_eq!(r.to_vec(), ["ms", "°C"]);
        r.set(1, "Ω");
        assert_eq!(r.to_vec(), ["ms", "Ω"]);
    }
}
