# Architecture

How Mainframe Frontier works today (Phases 0-1), and where the later phases
plug in. For the long-range plan, see [PLAN.md](../PLAN.md).

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
| `frontier-sort` | DFSORT subset over fixed-length records | `parse_control`, `run`, `SortSpec` |
| `frontier-jcl` | JCL parser, catalog, job executor, system utilities | `parse_job`, `System`, `Catalog`, `JobRecord` |
| `frontier-adapter` | The contract every higher layer uses | `MainframeAdapter`, `LocalAdapter` |
| `frontier-cli` | The `frontier` binary: build, seed, submit, inspect | clap subcommands |

Dependency direction: `cli → adapter → jcl → sort`. Nothing depends on the
CLI, so the Phase 3 API server will sit beside it as another adapter
consumer.

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

The authoritative definitions are the copybooks in `cobol/copybooks/`. The
Phase 2 `copybook` crate will parse them so that tables like these, and
field decoding in `frontier ds print`, are generated rather than
hand-written.

## Design decisions

| Decision | Why |
|----------|-----|
| Balanced-line sequential update instead of VSAM/indexed files | This is the canonical batch pattern, it produces a new GDG generation (so there is a free rollback point), and the files stay plain bytes that any tool can decode |
| GDG relative numbers snapshotted per job | Matches z/OS: `(+1)` refers to the same generation in every step of a job |
| `-std=ibm -debug` on every compile | IBM semantics for truncation and signs, and runtime checks so data errors surface as abends |
| Utilities built into the runner | SORT/IDCAMS/IEBGENER have no GnuCOBOL equivalents, and owning them lets later phases instrument them |
| Messages use real IBM message IDs (`IEF142I`, `IDC0001I`, `ICE054I`) | People who know z/OS can read the output unchanged. Product names are never claimed; the sort identifies itself as "FRONTIER SORT" |
| One `MainframeAdapter` trait | The API, UI and MCP server must work unchanged against Hercules or real z/OS later |
| ASCII data for now | GnuCOBOL's native mode. EBCDIC conversion is Phase 2 |

## Extension points for later phases

- **Phase 2:** `crates/ebcdic`, `crates/copybook`, and field-level decoding
  in `ds print`.
- **Phase 3:** `crates/frontier-api` (axum) as a second adapter consumer,
  plus the Next.js Control Room reading `job.json` and the spool.
- **Phase 4:** step records already carry CPU, elapsed time and record
  counts. Emitting them as OTel spans and SMF-30-style records is additive.
- **Phase 5:** the executor already keeps passed temporaries and GDG
  snapshots in one place, which is the state a `RESTART=step` needs.
