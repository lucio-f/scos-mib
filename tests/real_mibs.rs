//! Integration tests on real SCOS-2000 MIBs (see `tests/data/auris/README.md`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use scos_mib::{
    DiagnosticKind, Encoding, LineEnding, Mib, ReadOptions, Record, Severity, Table, icd_table,
};

const MIBS: [&str; 3] = ["ASCII_RTE", "ASCII_CSIM", "ASCII_Eden_Router"];

fn data_dir(mib: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data/auris")
        .join(mib)
}

/// A fresh directory under Cargo's per-target temporary directory, removed
/// when dropped.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// All `*.dat` files of a directory: file name -> content.
fn dat_files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "dat"))
        .map(|p| {
            let name = p.file_name().unwrap().to_str().unwrap().to_owned();
            (name, fs::read(&p).unwrap())
        })
        .collect()
}

fn copy_dir(from: &Path, to: &Path) {
    for (name, bytes) in dat_files(from) {
        fs::write(to.join(name), bytes).unwrap();
    }
}

fn read(mib: &str) -> Mib {
    Mib::read_dir(data_dir(mib)).unwrap()
}

fn lines(bytes: &[u8]) -> Vec<&str> {
    std::str::from_utf8(bytes).unwrap().lines().collect()
}

#[test]
fn reads_every_table_file() {
    for (mib_name, tables) in [
        ("ASCII_RTE", 57),
        ("ASCII_CSIM", 57),
        ("ASCII_Eden_Router", 54),
    ] {
        let mib = read(mib_name);
        assert_eq!(mib.len(), tables, "{mib_name}");
        // One table per .dat file, with its original file name.
        let files: Vec<String> = dat_files(&data_dir(mib_name)).into_keys().collect();
        let mut names: Vec<String> = mib.tables().map(|t| t.file_name().to_owned()).collect();
        names.sort();
        assert_eq!(names, files, "{mib_name}");
        // The record count of each table equals the number of lines.
        for table in mib.tables() {
            let bytes = fs::read(data_dir(mib_name).join(table.file_name())).unwrap();
            assert_eq!(table.len(), lines(&bytes).len(), "{}", table.file_name());
            assert_eq!(table.format().encoding, Encoding::Utf8);
            assert_eq!(table.format().line_ending, LineEnding::Lf);
        }
    }
}

#[test]
fn record_counts_and_schemas() {
    let rte = read("ASCII_RTE");
    let counts: Vec<(&str, usize)> = [
        "pcf", "plf", "pid", "vpd", "ccf", "cdf", "cpc", "cvp", "dst",
    ]
    .into_iter()
    .map(|t| (t, rte.table(t).unwrap().len()))
    .collect();
    assert_eq!(
        counts,
        [
            ("pcf", 299),
            ("plf", 393),
            ("pid", 130),
            ("vpd", 244),
            ("ccf", 144),
            ("cdf", 378),
            ("cpc", 188),
            ("cvp", 271),
            ("dst", 0),
        ]
    );

    // ICD tables get their schema, mission specific ones are kept without.
    for table in rte.tables() {
        assert_eq!(
            table.schema().is_some(),
            icd_table(table.name()).is_some(),
            "{}",
            table.name()
        );
    }
    let unknown: Vec<&str> = rte
        .tables()
        .filter(|t| t.schema().is_none())
        .map(|t| t.file_name())
        .collect();
    assert_eq!(
        unknown,
        [
            "gsid.dat",
            "mdf.dat",
            "TPKTconfigTable.dat",
            "TPKTconnTable.dat",
            "TPKTsimGSIDs.dat",
            "TPKTsimSIDs.dat",
        ]
    );
    // Lookup is case-insensitive, file names keep their case.
    let config = rte.table("TPKTconfigTable").unwrap();
    assert_eq!(config.name(), "tpktconfigtable");
    assert_eq!(config.file_name(), "TPKTconfigTable.dat");

    // Records hold the fields of the ICD issue they were produced for: the
    // RTE PCF has 19 of the 23 ICD columns, CSIM has all of them.
    assert_eq!(rte.table("pcf").unwrap().record_width(), 19);
    assert_eq!(read("ASCII_CSIM").table("pcf").unwrap().record_width(), 23);
}

#[test]
fn field_values() -> scos_mib::Result<()> {
    let rte = read("ASCII_RTE");

    let vdf = rte.table("vdf").unwrap().row(0).unwrap();
    assert_eq!(vdf.get("VDF_NAME")?, Some("RTE50"));
    assert_eq!(vdf.get("VDF_COMMENT")?, Some("RTE Data Bse"));
    assert_eq!(vdf.parse::<u32>("VDF_RELEASE")?, Some(1));

    let pid = rte.table("pid").unwrap();
    let time_report = pid.find("PID_SPID", "71")?.unwrap();
    assert_eq!(time_report.index(), 0);
    assert_eq!(time_report.get("PID_DESCR")?, Some("DH_Time_Report"));
    assert_eq!(time_report.parse::<u8>("PID_TYPE")?, Some(9));
    assert_eq!(time_report.parse::<i64>("PID_TPSD")?, Some(-1));
    assert_eq!(time_report.parse::<u32>("PID_INTER")?, Some(35000));
    assert_eq!(time_report.get("PID_EVID")?, None); // trailing empty field
    // The last line of pid.dat has no final newline.
    let last = pid.row(pid.len() - 1).unwrap();
    assert_eq!(last.get("PID_SPID")?, Some("154004"));
    assert_eq!(last.get("PID_EVENT")?, Some("N"));
    assert!(!pid.format().final_newline);

    let ccf = rte.table("ccf").unwrap();
    let cmd = ccf.find("CCF_CNAME", "S2KTC001")?.unwrap();
    assert_eq!(cmd.get("CCF_DESCR")?, Some("TC(2,1)"));
    assert_eq!(cmd.get("CCF_PKTID")?, Some("S2KHDR02"));
    assert_eq!(cmd.get("CCF_DEFSET")?, Some("S2KPSV01"));
    assert_eq!(cmd.parse::<u16>("CCF_RAPID")?, Some(1000));
    assert_eq!(cmd.get("CCF_SUBSCHEDID")?, None);
    assert_eq!(
        rte.table("cdf")
            .unwrap()
            .filter("CDF_CNAME", "S2KTC001")?
            .count(),
        3
    );

    // Supercommutated parameter S2KTP001 is located in several packets.
    let plf = rte.table("plf").unwrap();
    let spids: Vec<u32> = plf
        .filter("PLF_NAME", "S2KTP001")?
        .map(|r| r.parse("PLF_SPID").unwrap().unwrap())
        .collect();
    assert_eq!(&spids[..3], [1, 2, 3]);

    // Tables without a schema are accessed by position; comment lines and
    // records of different widths are kept as they are.
    let gsids = rte.table("tpktsimgsids").unwrap();
    let rows: Vec<Vec<String>> = gsids.records().iter().map(Record::to_vec).collect();
    assert_eq!(rows[0], ["#id     optional comment"]);
    assert_eq!(rows[1], ["#"]);
    assert_eq!(rows[2], ["101", "a simulation GSID"]);
    assert!(gsids.row(2).unwrap().get("GSID").is_err());
    assert_eq!(gsids.row(2).unwrap().get(1)?, Some("a simulation GSID"));

    let eden = read("ASCII_Eden_Router");
    let eden_pid = eden.table("pid").unwrap().row(0).unwrap();
    assert_eq!(eden_pid.get("PID_DESCR")?, Some("EDEN Router HK TM"));
    assert_eq!(eden_pid.record().len(), 17); // 16 columns plus a trailing tab
    assert!(eden.table("ccf").unwrap().is_empty());
    Ok(())
}

#[test]
fn write_is_byte_identical() {
    for mib_name in MIBS {
        let original = dat_files(&data_dir(mib_name));
        let mib = read(mib_name);
        for table in mib.tables() {
            assert_eq!(
                table.to_bytes().unwrap(),
                original[table.file_name()],
                "{mib_name}/{}",
                table.file_name()
            );
        }
        let out = TempDir::new(mib_name);
        mib.write_dir(out.path()).unwrap();
        assert_eq!(dat_files(out.path()), original, "{mib_name}");
        // Reading the copy gives the same MIB.
        assert_eq!(Mib::read_dir(out.path()).unwrap(), mib, "{mib_name}");
    }
}

#[test]
fn writing_back_unchanged_tables_does_not_touch_files() {
    let dir = TempDir::new("in-place");
    copy_dir(&data_dir("ASCII_RTE"), dir.path());
    let mut mib = Mib::read_dir(dir.path()).unwrap();
    // Make every file read-only: rewriting any of them would still succeed
    // (atomic rename), but would reset the permission bits we check below.
    for table in mib.tables() {
        let path = dir.path().join(table.file_name());
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&path, perms).unwrap();
    }
    let before = |name: &str| {
        fs::metadata(dir.path().join(name))
            .unwrap()
            .modified()
            .unwrap()
    };
    let stamps: BTreeMap<String, _> = mib
        .tables()
        .map(|t| (t.file_name().to_owned(), before(t.file_name())))
        .collect();

    mib.table_mut("tcp")
        .unwrap()
        .row_mut(0)
        .unwrap()
        .set("TCP_DESC", "Edited")
        .unwrap();
    mib.write_dir(dir.path()).unwrap();

    for (name, stamp) in &stamps {
        let meta = fs::metadata(dir.path().join(name)).unwrap();
        if name == "tcp.dat" {
            assert!(
                String::from_utf8(fs::read(dir.path().join(name)).unwrap())
                    .unwrap()
                    .contains("\tEdited\n")
            );
            // The replaced file keeps the permissions of the original.
            assert!(meta.permissions().readonly());
        } else {
            assert_eq!(meta.modified().unwrap(), *stamp, "{name} was rewritten");
        }
    }
    // Restore write permission so the directory can be cleaned up.
    for name in stamps.keys() {
        let path = dir.path().join(name);
        let mut perms = fs::metadata(&path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        fs::set_permissions(&path, perms).unwrap();
    }
}

#[test]
fn validation_of_real_mibs() {
    let summary = |mib: &Mib| {
        let mut counts: BTreeMap<(Severity, String, String, &'static str), usize> = BTreeMap::new();
        for d in mib.validate() {
            let kind = match d.kind {
                DiagnosticKind::TooLong { .. } => "too long",
                DiagnosticKind::DanglingReference { .. } => "dangling reference",
                _ => "other",
            };
            *counts
                .entry((d.severity, d.table, d.column.unwrap_or_default(), kind))
                .or_default() += 1;
        }
        counts
            .into_iter()
            .map(|((severity, table, column, kind), n)| {
                format!("{severity} {table}.{column} {kind}: {n}")
            })
            .collect::<Vec<_>>()
    };

    // The complete MIB is consistent: no errors, every reference resolves.
    // Only values longer than the ICD allows are reported.
    assert_eq!(
        summary(&read("ASCII_RTE")),
        [
            "warning dpc.DPC_TEXT too long: 2",
            "warning gpf.GPF_UPUN too long: 5",
        ]
    );
    assert_eq!(
        summary(&read("ASCII_Eden_Router")),
        ["warning pcf.PCF_UNIT too long: 4"]
    );

    // CSIM uses SCOS-2000 system parameters and packets without defining
    // them in its own pcf.dat and pid.dat.
    let csim = read("ASCII_CSIM");
    let diagnostics = csim.validate();
    assert!(diagnostics.iter().all(|d| d.severity == Severity::Warning));
    let dangling = diagnostics
        .iter()
        .filter(|d| matches!(d.kind, DiagnosticKind::DanglingReference { .. }))
        .count();
    assert_eq!(dangling, 543);
    let rte_pcf = read("ASCII_RTE");
    let rte_pcf = rte_pcf.table("pcf").unwrap();
    for d in &diagnostics {
        if let DiagnosticKind::DanglingReference { table: "pcf", .. } = d.kind {
            let name = d.value.as_deref().unwrap();
            assert!(
                rte_pcf.find("PCF_NAME", name).unwrap().is_some() || name.starts_with("MISC"),
                "{d}"
            );
        }
    }
}

#[test]
fn edit_and_write_back() -> scos_mib::Result<()> {
    let original = dat_files(&data_dir("ASCII_RTE"));
    let mut mib = read("ASCII_RTE");

    // 1. Change a description.
    let pcf = mib.table_mut("pcf").unwrap();
    let dptv = pcf.find("PCF_NAME", "DPTV")?.unwrap().index();
    pcf.row_mut(dptv)
        .unwrap()
        .set("PCF_DESCR", "Dynamic PTV check")?;

    // 2. Add a parameter and place it in the time report packet (SPID 71).
    let new_param = pcf.push_named(&[
        ("PCF_NAME", "TSTP0001"),
        ("PCF_DESCR", "Test parameter"),
        ("PCF_UNIT", "degC"),
        ("PCF_PTC", "4"),
        ("PCF_PFC", "16"),
        ("PCF_CATEG", "N"),
        ("PCF_NATUR", "R"),
    ])?;
    assert_eq!(pcf.row(new_param).unwrap().record().len(), 19);
    mib.table_mut("plf").unwrap().push_named(&[
        ("PLF_NAME", "TSTP0001"),
        ("PLF_SPID", "71"),
        ("PLF_OFFBY", "20"),
        ("PLF_OFFBI", "0"),
    ])?;

    // 3. Delete a command together with its details, pre-transmission
    //    validation and verification profile.
    let command = "S2KTC601";
    let ccf = mib.table_mut("ccf").unwrap();
    assert_eq!(
        ccf.retain(|r| r.get("CCF_CNAME").unwrap() != Some(command)),
        1
    );
    for (table, column, removed) in [
        ("cdf", "CDF_CNAME", 13),
        ("ptv", "PTV_CNAME", 1),
        ("cvp", "CVP_TASK", 6),
    ] {
        let table = mib.table_mut(table).unwrap();
        assert_eq!(
            table.retain(|r| r.get(column).unwrap() != Some(command)),
            removed
        );
    }

    // 4. Fill the empty routing table.
    let dst = mib.table_or_create("dst")?;
    dst.push_named(&[("DST_APID", "17"), ("DST_ROUTE", "SIMULATOR")])?;
    dst.push_named(&[("DST_APID", "1540"), ("DST_ROUTE", "SIMULATOR")])?;

    // 5. Create a table that did not exist.
    let ppf = mib.table_or_create("PPF")?;
    ppf.push_named(&[
        ("PPF_NUMBE", "PR01"),
        ("PPF_HEAD", "Test printout"),
        ("PPF_NBPR", "1"),
    ])?;

    // The edited MIB is still consistent.
    let diagnostics = mib.validate();
    assert!(
        diagnostics
            .iter()
            .all(|d| d.severity == Severity::Warning
                && matches!(d.kind, DiagnosticKind::TooLong { .. })),
        "{diagnostics:#?}"
    );

    let out = TempDir::new("edited");
    mib.write_dir(out.path())?;
    let written = dat_files(out.path());

    // Untouched tables are byte identical.
    let edited = [
        "pcf.dat", "plf.dat", "ccf.dat", "cdf.dat", "ptv.dat", "cvp.dat", "dst.dat",
    ];
    for (name, bytes) in &original {
        if !edited.contains(&name.as_str()) {
            assert_eq!(&written[name], bytes, "{name}");
        }
    }
    assert_eq!(written.len(), original.len() + 1);

    // Edited tables differ exactly where they were edited.
    let pcf_before = lines(&original["pcf.dat"]);
    let pcf_after = lines(&written["pcf.dat"]);
    assert_eq!(pcf_after.len(), pcf_before.len() + 1);
    for (i, (before, after)) in pcf_before.iter().zip(&pcf_after).enumerate() {
        if i == dptv {
            assert_eq!(
                *before,
                "DPTV\tDYNAMIC PTV\t\t\t3\t0\t\t\t\tN\tR\t\t\tN\t\t\t\t1\tE"
            );
            assert_eq!(
                *after,
                "DPTV\tDynamic PTV check\t\t\t3\t0\t\t\t\tN\tR\t\t\tN\t\t\t\t1\tE"
            );
        } else {
            assert_eq!(before, after);
        }
    }
    assert_eq!(
        pcf_after[pcf_before.len()],
        "TSTP0001\tTest parameter\t\tdegC\t4\t16\t\t\t\tN\tR\t\t\t\t\t\t\t\t"
    );
    // plf.dat had no final newline: the previous last line gets one, the
    // new last line does not.
    let plf = String::from_utf8(written["plf.dat"].clone()).unwrap();
    let plf_original = String::from_utf8(original["plf.dat"].clone()).unwrap();
    assert_eq!(plf, format!("{plf_original}\nTSTP0001\t71\t20\t0\t\t\t\t"));

    for (name, removed) in [
        ("ccf.dat", 1),
        ("cdf.dat", 13),
        ("ptv.dat", 1),
        ("cvp.dat", 6),
    ] {
        let before = lines(&original[name]);
        let after = lines(&written[name]);
        let kept: Vec<&str> = before
            .iter()
            .copied()
            .filter(|l| !l.starts_with("S2KTC601\t"))
            .collect();
        assert_eq!(after, kept, "{name}");
        assert_eq!(before.len() - after.len(), removed, "{name}");
    }
    // New and previously empty tables use the MIB's dominant format.
    assert_eq!(written["dst.dat"], b"17\tSIMULATOR\n1540\tSIMULATOR\n");
    assert_eq!(written["ppf.dat"], b"PR01\tTest printout\t1\n");

    // Reading the result back gives the edited MIB.
    let reread = Mib::read_dir(out.path())?;
    assert_eq!(
        reread.table("pcf").unwrap().records(),
        mib.table("pcf").unwrap().records()
    );
    assert!(
        reread
            .table("ccf")
            .unwrap()
            .find("CCF_CNAME", command)?
            .is_none()
    );
    assert_eq!(
        reread
            .table("plf")
            .unwrap()
            .find("PLF_NAME", "TSTP0001")?
            .unwrap()
            .parse::<u32>("PLF_OFFBY")?,
        Some(20)
    );
    Ok(())
}

#[test]
fn line_ending_conversion_round_trip() {
    let original = dat_files(&data_dir("ASCII_CSIM"));
    let mut mib = read("ASCII_CSIM");
    for table in mib.tables_mut() {
        table.format_mut().line_ending = LineEnding::CrLf;
    }
    let crlf_dir = TempDir::new("crlf");
    mib.write_dir(crlf_dir.path()).unwrap();
    let crlf = dat_files(crlf_dir.path());
    for (name, bytes) in &original {
        let expected = String::from_utf8(bytes.clone())
            .unwrap()
            .replace('\n', "\r\n");
        assert_eq!(crlf[name], expected.as_bytes(), "{name}");
    }

    // A CRLF MIB (e.g. checked out with core.autocrlf) reads into the same
    // records and is written back unchanged.
    let reread = Mib::read_dir(crlf_dir.path()).unwrap();
    for table in reread.tables() {
        let source = mib.table(table.name()).unwrap();
        assert_eq!(table.records(), source.records(), "{}", table.name());
        if !table.is_empty() && table.len() + usize::from(table.format().final_newline) > 1 {
            assert_eq!(
                table.format().line_ending,
                LineEnding::CrLf,
                "{}",
                table.name()
            );
        }
    }
    let back = TempDir::new("crlf-back");
    reread.write_dir(back.path()).unwrap();
    assert_eq!(dat_files(back.path()), crlf);
}

#[test]
fn single_table_files_and_custom_schemas() -> scos_mib::Result<()> {
    use scos_mib::{Column, ColumnType, TableSchema};

    let path = data_dir("ASCII_RTE").join("TPKTsimSIDs.dat");
    let schema = TableSchema::new(
        "tpktsimsids",
        "Simulated SIDs",
        vec![
            Column::new("SID_ID", ColumnType::Number(5))
                .key()
                .mandatory(),
            Column::new("SID_VALUE", ColumnType::Char(32)),
        ],
    );
    let options = ReadOptions::new().schema(schema);
    let table = Table::read_file(&path, &options)?;
    assert_eq!(table.file_name(), "TPKTsimSIDs.dat");
    assert!(table.schema().is_some());
    assert_eq!(table.to_bytes()?, fs::read(&path).unwrap());
    let ids: Vec<&str> = table
        .rows()
        .map(|r| r.get_or_empty("SID_ID").unwrap())
        .collect();
    assert_eq!(ids.len(), table.len());

    // The same schema applies when reading a whole directory.
    let mib = Mib::read_dir_with(data_dir("ASCII_RTE"), &options)?;
    assert!(mib.table("tpktsimsids").unwrap().schema().is_some());
    Ok(())
}
