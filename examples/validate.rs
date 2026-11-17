//! Prints a summary of a MIB directory and the result of its validation.
//!
//! ```text
//! cargo run --example validate -- tests/data/auris/ASCII_RTE
//! ```

use std::process::ExitCode;

use scos_mib::{Mib, Severity};

fn main() -> ExitCode {
    let Some(dir) = std::env::args_os().nth(1) else {
        eprintln!("usage: validate <mib-directory>");
        return ExitCode::FAILURE;
    };
    let mib = match Mib::read_dir(&dir) {
        Ok(mib) => mib,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!("{:<18} {:>8}  schema", "file", "records");
    for table in mib.tables() {
        let schema = match table.schema() {
            Some(s) => format!("{} ({} columns)", s.description(), s.columns().len()),
            None => "-".to_owned(),
        };
        println!("{:<18} {:>8}  {schema}", table.file_name(), table.len());
    }

    let diagnostics = mib.validate();
    let errors = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    println!();
    for diagnostic in &diagnostics {
        println!("{diagnostic}");
    }
    println!(
        "{} tables, {} errors, {} warnings",
        mib.len(),
        errors,
        diagnostics.len() - errors
    );
    if errors > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
