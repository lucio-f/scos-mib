# scos-mib

A Rust library to **read, edit and write back SCOS-2000 Mission Information
Bases (MIBs)**: the directories of tab-separated ASCII tables (`pcf.dat`,
`ccf.dat`, ...) specified by the SCOS-2000 Database Import ICD
(EGOS-MCS-S2K-ICD-0001).

- **Generic**: every `*.dat` file is loaded, including tables the ICD does not
  define, and records keep every field, including mission-specific columns
  appended after the ICD ones.
- **Lossless**: separator, LF/CRLF line endings, final newline and encoding
  (UTF-8/ASCII or Latin-1) are detected per file and preserved. An unmodified
  MIB is written back byte for byte, and an edited table changes only where it
  was edited.
- **Schema aware**: the 53 ICD tables (issue 6.7) come with column names,
  types, key and mandatory flags, so columns can be addressed by name and the
  MIB can be validated.
- **Self-contained**: no dependencies beyond `std`, no `unsafe`. The library
  is linked statically into the programs that use it, and those programs are
  fully static executables (see [Static linking](#static-linking)).

## Usage

```toml
[dependencies]
scos-mib = { git = "<this repository>" }
```

```rust,no_run
use scos_mib::{Mib, Severity};

fn main() -> scos_mib::Result<()> {
    let mut mib = Mib::read_dir("ASCII")?;

    // Query by column name.
    let pid = mib.table("pid").expect("no pid.dat");
    if let Some(packet) = pid.find("PID_SPID", "71")? {
        println!("{:?} {:?}", packet.get("PID_DESCR")?, packet.parse::<u32>("PID_INTER")?);
    }

    // Edit: change a value, add a record, delete records.
    let pcf = mib.table_mut("pcf").expect("no pcf.dat");
    if let Some(row) = pcf.find("PCF_NAME", "DPTV")?.map(|r| r.index()) {
        pcf.row_mut(row).unwrap().set("PCF_DESCR", "Dynamic PTV check")?;
    }
    pcf.push_named(&[
        ("PCF_NAME", "TSTP0001"),
        ("PCF_PTC", "4"),
        ("PCF_PFC", "16"),
        ("PCF_CATEG", "N"),
        ("PCF_NATUR", "R"),
    ])?;
    mib.table_mut("cdf")
        .expect("no cdf.dat")
        .retain(|row| row.get("CDF_CNAME").unwrap() != Some("S2KTC601"));

    // Validate and write back (only modified files are rewritten).
    for diagnostic in mib.validate() {
        if diagnostic.severity == Severity::Error {
            eprintln!("{diagnostic}");
        }
    }
    mib.write_dir("ASCII")?;
    Ok(())
}
```

The example program prints a summary and the validation report of a MIB:

```sh
cargo run --example validate -- tests/data/auris/ASCII_RTE
```

## Data model

| Type | Purpose |
|------|---------|
| `Mib` | All tables of a directory, keyed by lower-case name; `read_dir`, `write_dir`, `validate`. |
| `Table` | One `.dat` file: records, file name, `FileFormat` and optional `TableSchema`. |
| `Record` | One line: the raw list of fields, exactly as in the file. |
| `Row` / `RowMut` | Views of a record with access by column name (`get`, `parse`, `set`, `clear`). |
| `TableSchema` / `Column` | Column names, types (`Char(n)`, `Number(n)`), key and mandatory flags. |
| `Diagnostic` | A validation finding (severity, table, line, column, value, kind). |

Records may have fewer fields than the schema (MIBs produced for an older ICD
issue) or more (mission extensions). Both are accepted. New records get the
field count most records of the table already have. Schemas for
mission-specific tables, or ICD tables with extra columns, can be provided
with `ReadOptions::schema`. A mission-specific field separator can be set with
`ReadOptions::separator`.

## Validation

`Mib::validate` reports, per record:

- **errors**: missing mandatory values, non-integer values in `Number`
  columns, duplicate keys;
- **warnings**: values longer than the ICD allows (SCOS-2000 truncates
  them at import), unexpected non-empty extra fields, blank lines, and
  references to missing records (for example `PLF_NAME` → `PCF_NAME` or
  `CDF_CNAME` → `CCF_CNAME`; see `REFERENCES`).

Validation never blocks reading or writing.

## Static linking

The crate is an ordinary Rust library (`rlib`), so it is compiled into each
program that uses it. [`.cargo/config.toml`](.cargo/config.toml) additionally
links the C runtime statically, so executables built here are fully static:

```sh
cargo build                                    # static glibc (default)
cargo build --target x86_64-unknown-linux-musl # static musl (needs musl-tools)
```

## Tests

```sh
cargo test                                      # unit, integration and doc tests
cargo test --target x86_64-unknown-linux-musl
```

The integration tests in [`tests/real_mibs.rs`](tests/real_mibs.rs) run on
three real MIBs from the open-source AURIS mission control system (BSD
3-Clause, see [`tests/data/auris/README.md`](tests/data/auris/README.md)).
They check byte-identical round trips, field values, validation results, an
edit-and-write-back workflow, and LF ↔ CRLF conversion.

## Limitations

- Only the `.dat` tables are managed. Synthetic parameter expression files and
  other files in the MIB directory are neither read nor modified.
- `write_dir` does not delete files: removing a table from a `Mib` leaves its
  file on disk.
- The built-in schema follows ICD issue 6.7. Columns added by later issues are
  kept as extra fields and can be named by supplying a schema.
