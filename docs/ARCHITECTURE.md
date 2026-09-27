# Architecture

How Mainframe Frontier works today (Phases 0-2), and where the later phases
plug in. For why the project exists, see the
[README](../README.md#why-this-project-exists); for the long-range plan,
see [PLAN.md](../PLAN.md).

In short: the COBOL workload is the fixed point, and every component here
exists to make it visible, interoperable, observable or safer to change,
without rewriting it.

## The big picture

```
  cobol/jcl/NIGHTLY.jcl                         cobol/src/*.cbl + copybooks/*.cpy
          │                                                 │
          │ frontier submit                                 │ frontier build (cobc -x -std=ibm -debug)
          ▼                                                 ▼
  ┌─────────────────┐   parse   ┌───────────────┐    ┌──────────────┐
  │ frontier-cli    │──────────►│ jcl::parse    │    │ var/loadlib/ │  one Linux executable per program
  │ (clap)          │           └───────┬───────┘    └──────┬───────┘
  └────────┬────────┘                   │ Job AST           │ exec, DD_<name>=<path> env vars
           │ MainframeAdapter           ▼                   │
           │ (LocalAdapter)     ┌───────────────┐           │
           └───────────────────►│ jcl::jes      │───────────┘
                                │ JES + initiator│──► utilities: SORT (frontier-sort), IDCAMS, IEBGENER, IEFBR14
                                └───┬───────┬───┘
                        catalog.json│       │spool/JOBnnnnn/
                                    ▼       ▼
                         var/datasets/   JESMSGLG · JESJCL · JESYSMSG · STEP.DD · job.json
```

Everything runs inside the dev container (`.devcontainer/Dockerfile`: Rust
1.97 slim plus GnuCOBOL 3.2 on Debian trixie). The repo is bind-mounted at
`/workspace`, and runtime state lives in `/workspace/var` (`$FRONTIER_HOME`).

## Crates

| Crate | Role | Key types |
|-------|------|-----------|
| `frontier-ebcdic` (lib `ebcdic`) | IBM-037/1047 code pages; packed, zoned, binary, hex-float codecs; exact `Decimal` | `CodePage`, `Encoding`, `Decimal`, `packed`/`zoned`/`binary` |
| `frontier-copybook` | Copybook → layout; decode to JSON / flat fields; JSON Schema; transcoding | `parse`, `Item`, `decode`, `flatten`, `json_schema`, `transcode` |
| `frontier-sort` | DFSORT subset over fixed-length records | `parse_control`, `run`, `SortSpec` |
| `frontier-jcl` | JCL parser, catalog, job executor, system utilities | `parse_job`, `System`, `Catalog`, `JobRecord` |
| `frontier-adapter` | The contract every higher layer uses, plus the dataset→copybook registry | `MainframeAdapter`, `LocalAdapter`, `LayoutRegistry` |
| `frontier-cli` | The `frontier` binary: build, seed, submit, inspect, decode | clap subcommands |

Dependency direction: `cli → adapter → {jcl → sort, copybook → ebcdic}`.
Nothing depends on the CLI, so the Phase 3 API server will sit beside it as
another adapter consumer.

## Life of a job

`System::submit` in `crates/jcl/src/jes.rs`:

1. **Job number.** `jes.json` holds the counter. `spool/JOBnnnnn/` and a
   `temp/` directory are created.
2. **Parse.** `parse_job` handles `SET` symbols (command-line `--set`
   overrides win), continuations, in-stream data and columns 72-80.
   A parse error still produces a job, with status `JCL ERROR` and the
   message in `JESYSMSG`, as on z/OS.
3. **Each step in order:**
   1. **Bypass check.** After an abend, steps are flushed unless the step
      has `COND=EVEN` or `ONLY`. Otherwise the step is skipped when any
      `COND=(code,op[,step])` test is true against earlier return codes
      (skipping when `code op rc` holds, as on z/OS).
   2. **Allocation**, per DD:
      - `SYSOUT` → a spool file.
      - `DUMMY` → `/dev/null`.
      - `DD *` → a temp file (also fed to stdin for `SYSIN`).
      - `&&TEMP` → the job's temp directory.
      - `BASE(+n)` → resolved against the GDG snapshot taken at the
        base's first reference in the job, so `(+1)` means the same
        generation in every step.
      - Anything else → a catalog lookup.

      A failure (dataset not found, duplicate catalog name, GDG base
      missing) is a JCL error that ends the job. Files this step already
      created are removed.
   3. **Execution.** Built-in utilities are dispatched by `PGM=`. Anything
      else must be an executable in `var/loadlib/`, otherwise `S806`. The
      COBOL program gets `DD_<ddname>=<path>` for every DD (GnuCOBOL's
      external file mapping), `PARM` as `argv[1]`, stdout appended to the
      `SYSOUT` DD, and stdin from `SYSIN`. CPU time comes from
      `getrusage(RUSAGE_CHILDREN)` deltas.
   4. **Completion.** A non-zero exit together with a `libcob:` message
      on stderr becomes an abend (see the table below). Otherwise the exit
      code is the step's return code.
   5. **Dispositions.** Each dataset gets its normal or abnormal action
      (`CATLG`, `KEEP`, `PASS`, `DELETE`, `UNCATLG`). Cataloging a GDG
      generation may roll older ones off (`SCRATCH` deletes the file).
      Record counts are taken before any delete.
4. **End of job.** Passed temporaries that nobody received are deleted,
   the step summary table goes into `JESMSGLG`, and `job.json` is written.

### Abend mapping

| GnuCOBOL runtime condition | Reported as |
|----------------------------|-------------|
| `... not numeric` (invalid packed/zoned data) | `S0C7` reason `00000007` |
| subscript / offset out of bounds | `S0C4` |
| divide by zero | `S0CB` |
| file status 35 (not found) | `S013-18` |
| file status 34 (boundary) | `SB37-04` |
| file status 39 (attribute conflict) | `S013-20` |
| SIGSEGV / SIGBUS / SIGFPE / SIGKILL / SIGXCPU | `S0C4` / `S0C4` / `S0CB` / `S222` / `S322` |
| program not in loadlib | `S806` |
| any other `libcob` error | `U4038` (LE unhandled condition) |

The `file.cbl:line` from the libcob message is recorded as the step's
`source`, and the full stderr is kept as the `STEP.CEEDUMP` spool file.
Programs are compiled with `-debug` precisely so that bad data becomes an
S0C7 instead of silently wrong arithmetic.

### `job.json`

The structured record every higher layer consumes. Abridged:

```json
{
  "id": "JOB00002", "name": "NIGHTLY", "class": "A",
  "status": { "kind": "cc", "rc": 4 },
  "cpu_ms": 2750, "elapsed_ms": 12220,
  "symbols": { "BUSDATE": "20260929" },
  "steps": [{
    "name": "POST", "program": "ACCTPOST",
    "outcome": { "kind": "executed", "rc": 0 },
    "cpu_ms": 1190, "elapsed_ms": 5570, "records_in": 59885, "records_out": 59885,
    "dds": [{ "ddname": "POSTLOG", "dsn": "FFB.DAILY.POSTLOG.G0001V00",
              "direction": "out", "disposition": "CATALOGED", "records": 49885, "bytes": 4988500 }]
  }],
  "spool": [{ "ddname": "JESMSGLG", "step": null, "lines": 16, "bytes": 1234 }]
}
```

`status.kind` is `cc`, `abend` (with `code` and `step`) or `jcl_error`.
`outcome.kind` is `executed`, `abended` (with `code`, `reason`, `source`) or
`not_run` (with `reason`).

## Datasets

- One file per dataset under `var/datasets/<DSN>`. Fixed-block (`FB`)
  files are raw concatenated records with no newlines, exactly like the
  bytes of a real FB dataset. Record format and length live in
  `catalog.json`, the way a VTOC entry does on DASD.
- Reports are written as SYSOUT, one text line per print line.
- GDG generations are named `BASE.GnnnnV00`. The catalog stores the limit,
  the `SCRATCH` and `EMPTY` flags, and the generation list (oldest first).

### Record layouts

Positions are 1-based, as in DFSORT control cards. PD means packed decimal
(COMP-3).

**ACCTREC**: account master, `FFB.ACCTMAST`, FB 100

| Pos | Len | Field | Format |
|----:|----:|-------|--------|
| 1 | 10 | ACCT-ID | CH, ascending key |
| 11 | 30 | CUST-NAME | CH |
| 41 | 1 | ACCT-TYPE | `C` checking, `S` savings |
| 42 | 1 | STATUS | `A` active, `F` frozen, `C` closed |
| 43 | 8 | OPEN-DATE | ZD YYYYMMDD |
| 51 | 7 | BALANCE | PD S9(11)V99 |
| 58 | 3 | INT-RATE | PD 9V9(4) (0.0425 = 4.25%) |
| 61 | 5 | OD-LIMIT | PD S9(7)V99 |
| 66 | 7 | ACCR-INT | PD S9(7)V9(6) |
| 73 | 8 | LAST-ACTIVITY | ZD YYYYMMDD |
| 81 | 3 | TXN-COUNT-MTD | PD S9(5) |
| 84 | 17 | filler | |

**TRANFEED**: channel feed, `FFB.DAILY.TRANFEED`, FB 80, all display fields

| Pos | Len | Field |
|----:|----:|-------|
| 1 | 10 | ACCT-ID |
| 11 | 14 | TIMESTAMP YYYYMMDDHHMMSS |
| 25 | 12 | TXN-ID |
| 37 | 2 | TXN-TYPE: `DP` `WD` `FE` `TI` `TO` |
| 39 | 11 | AMOUNT, ZD 9(9)V99, unsigned |
| 50 | 3 | CHANNEL |
| 53 | 20 | DESCRIPTION |

**TRANREC** (validated, FB 80): same as TRANFEED through pos 38, then
AMOUNT PD S9(9)V99 at 39-44 (debits negative), CHANNEL 45-47,
DESCRIPTION 48-67.

**POSTREC** (posting journal, `FFB.DAILY.POSTLOG`, FB 100): TRANREC
fields through AMOUNT (1-44), then BAL-AFTER PD S9(11)V99 at 45-51,
STATUS at 52 (`P` posted, `R` refused), REASON at 53-56
(`NSF`/`FRZN`/`CLSD`/`NOAC`), and DESCRIPTION at 57-76.

**INTREC** (accrual journal, `FFB.DAILY.INTLOG`, FB 50): ACCT-ID 1-10,
BUS-DATE 11-18, BAL-BASIS PD 19-25, RATE PD 26-28, DAILY-ACCR PD
S9(7)V9(6) 29-35, CREDITED PD S9(7)V99 36-40.

**ACCTLOAD** (conversion feed, `FFB.SEED.ACCTLOAD`, FB 100): ACCTREC's
first 50 bytes in display form, then BALANCE `SIGN LEADING SEPARATE` at
51-64, INT-RATE at 65-69 and OD-LIMIT at 70-78.

The authoritative definitions are the copybooks in `cobol/copybooks/`, and
`frontier copybook <NAME>` prints the same information generated from
them.

## Reading data: copybooks and encodings

### From copybook to layout

`frontier_copybook::parse` works in four passes:

1. **Source lines.** Fixed format uses columns 8-72; comments are `*` or `/`
   in column 7, or `*>` anywhere. A `-` in column 7 continues a literal.
   `REPLACING` pairs are applied, or, when none are given, `:TAG:` prefixes
   are stripped (`:AR:-BALANCE` → `BALANCE`).
2. **Tokens and entries.** Each sentence (ending in a period) is one data
   description: level, name or FILLER, then clauses in any order. Level-88
   entries attach to the entry before them.
3. **Tree.** Level numbers nest items. A copybook that starts below 01 is
   wrapped in a synthetic 01 named after the file. Group `USAGE` and `SIGN`
   clauses are inherited by the elementary items under them.
4. **Layout.** Elementary sizes come from PICTURE and USAGE:
   - DISPLAY: the picture length, plus one for `SEPARATE`
   - COMP-3: digits/2+1
   - COMP/COMP-5: 2, 4 or 8 bytes
   - COMP-1/COMP-2: 4 or 8 bytes

   Offsets accumulate, `REDEFINES` reuses the target's offset, and an
   `OCCURS` item takes `size × max` bytes. `OCCURS DEPENDING ON` is only
   allowed as the last item, so fixed offsets never depend on data.

Offsets are absolute and 0-based internally; listings show 1-based `POS`
to match DFSORT and compiler maps.

### Decoding

`decode(record, bytes, &DecodeOptions)` walks the layout into a `Node`
tree (groups, arrays and leaves). Each leaf keeps its offset, raw bytes, and
either a value or the reason the bytes are invalid. The same tree feeds all
of these:

- `to_json()`: COBOL names as keys, in layout order. Decimals with a scale
  are strings (`"4969.00"`), so money never passes through an IEEE double.
  Integers up to 2^53 are numbers. Invalid fields are `null`. FILLER and
  REDEFINES views are omitted unless requested.
- `flatten()`: elementary fields with path, position, type, value, and the
  88-level names that are true. This backs `ds print --format fields|table`
  and `ds check`.
- `OCCURS DEPENDING ON` counts come from counter fields already decoded
  earlier in the same record.

`json_schema(record)` describes the same JSON (draft 2020-12). Decimals are
string patterns and integers have min/max bounds. Every property carries an
`x-cobol` block (level, offset, length, type, usage, 88-levels, and
occurs/dependingOn on arrays), which is what the Phase 6 gateway needs to
map JSON back to bytes.

### Encodings

| | `Encoding::Local` (GnuCOBOL here) | `Encoding::Ebcdic(cp)` (z/OS) |
|---|---|---|
| Text | ISO-8859-1 | IBM-037 / IBM-1047 |
| Zoned sign | negative `d` → `0x70+d` (`p`..`y`); positive plain | zone nibble `C`/`F` positive, `D` negative |
| COMP / BINARY | big-endian | big-endian |
| COMP-5 | little-endian (native) | big-endian |
| COMP-1/2 | IEEE 754 | IBM hexadecimal float |

These facts were measured from bytes GnuCOBOL 3.2 wrote, not assumed; the
probe values are the codec unit tests. The code-page tables are generated
from glibc `iconv` (`scripts/gen-codepages.sh`), and a test asserts that
037 and 1047 differ in exactly six positions.

`transcode` converts records field by field:
- text and FILLER are translated
- zoned decimals are decoded and re-encoded, because the sign conventions
  differ
- COMP-3 and COMP are copied unchanged
- COMP-5 bytes are swapped when the platforms' byte orders differ

A zoned field holding invalid data is translated as text and reported.
REDEFINES views are ignored during conversion (the original definition
wins), and COMP-1/2 are copied without conversion and counted.

Verified against independent references:
- `ds export` of the all-DISPLAY feed is byte-identical to glibc `iconv`.
- The packed master round-trips through IBM-037 byte-for-byte.
- Decoded ledger and posting totals equal GLRECON's to the cent.

### The registry

`cobol/datasets.toml` maps dataset-name patterns to copybooks (and
optionally a record). `LayoutRegistry` lives in `frontier-adapter`, so the
CLI now, and later the API and MCP server, resolve layouts the same way. A
bare pattern like `FFB.ACCTMAST` covers every generation.

## Design decisions

| Decision | Why |
|----------|-----|
| Balanced-line sequential update instead of VSAM/indexed files | This is the canonical batch pattern, it produces a new GDG generation (so there is a free rollback point), and the files stay plain bytes that any tool can decode |
| GDG relative numbers snapshotted per job | Matches z/OS: `(+1)` refers to the same generation in every step of a job |
| `-std=ibm -debug` on every compile | IBM semantics for truncation and signs, and runtime checks so data errors surface as abends |
| Utilities built into the runner | SORT/IDCAMS/IEBGENER have no GnuCOBOL equivalents, and owning them lets later phases instrument them |
| Messages use real IBM message IDs (`IEF142I`, `IDC0001I`, `ICE054I`) | People who know z/OS can read the output unchanged. Product names are never claimed; the sort identifies itself as "FRONTIER SORT" |
| One `MainframeAdapter` trait | The API, UI and MCP server must work unchanged against Hercules or real z/OS later |
| ASCII data inside the shop, EBCDIC at the boundary | GnuCOBOL's native mode keeps the batch simple. Export, import and decode produce and consume true z/OS byte layouts where files cross platforms |
| Decimals as JSON strings | Exactness beats convenience for money: `S9(15)V99` does not fit an IEEE double |
| Invalid data decodes to an error, not a guess | A value that would S0C7 on z/OS should be visible (`ds check`, `null` in JSON), never silently turned into a number |

## Extension points for later phases

- **Phase 3:** `crates/frontier-api` (axum) as a second adapter consumer,
  serving `job.json`, spool and decoded records (`LayoutRegistry` +
  `decode(...).to_json()`), plus the Next.js Control Room.
- **Phase 4:** step records already carry CPU, elapsed time and record
  counts. Emitting them as OTel spans and SMF-30-style records is additive.
- **Phase 5:** the executor already keeps passed temporaries and GDG
  snapshots in one place, which is the state a `RESTART=step` needs.
