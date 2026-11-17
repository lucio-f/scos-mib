# AURIS reference MIBs

Real SCOS-2000 MIB tables used by the integration tests in
[`tests/real_mibs.rs`](../../real_mibs.rs).

They come from the `esa-mib` package of the open source AURIS mission control
system by Michael Oswald, distributed under the BSD 3-Clause license (see
[`LICENSE`](LICENSE)):

- Repository: <https://github.com/oswald2/AURIS>
- Commit: `31579ecd7198194bc61f43e0b989f81387c06b08` (2024-08-13)
- Source directories: `esa-mib/ASCII_CSIM`, `esa-mib/ASCII_RTE`,
  `esa-mib/ASCII_Eden_Router`

Only the `*.dat` table files were copied, using `git cat-file blob` so that
their bytes are exactly those of the upstream repository (LF line endings,
some files without a final newline, some files empty). Other files of the
upstream directories (CVS metadata, synthetic parameter expressions, display
templates, scripts) were left out.

| Directory           | Tables | Content                                                        |
|---------------------|-------:|----------------------------------------------------------------|
| `ASCII_RTE`         |     57 | Complete monitoring and commanding MIB of a reference test environment |
| `ASCII_CSIM`        |     57 | Same mission, with a reduced monitoring part (references SCOS-2000 system parameters it does not define) |
| `ASCII_Eden_Router` |     54 | Small MIB of an EDEN router; most tables are empty files       |

Besides the ICD tables, the directories contain tables that are not defined
by the ICD (`mdf.dat`, `gsid.dat`, `gpd.dat` and the SCOS-2000 packetiser and
simulator tables `TPKT*.dat`, one of which contains `#` comment lines), which
exercise the generic table handling.

Do not edit these files: the tests rely on their exact content. Git end of
line conversion is disabled for them in [`.gitattributes`](../../../.gitattributes).
