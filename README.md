# Mainframe Frontier

[![ci](https://github.com/roberjo/mainframe-frontier/actions/workflows/ci.yml/badge.svg)](https://github.com/roberjo/mainframe-frontier/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

**A working mainframe shop you can clone.** First Frontier Bank runs a real
COBOL nightly batch cycle, with JCL job streams, generation data groups,
packed-decimal master files, a general-ledger reconciliation, and customer
statements. A JES-style job runner written in Rust executes it and writes
the same job logs, condition codes and abend codes you'd see on z/OS.

```
$ make demo
...
JOBID     JOBNAME  STATUS      SUBMITTED           STEPS   CPU(S)  CLOCK(S)
JOB00001  SETUP    CC 0000     2026-09-27 15:32:16     3     0.12      0.68
JOB00002  NIGHTLY  CC 0004     2026-09-27 15:32:17     8     2.75     12.22
JOB00003  NIGHTLY  CC 0004     2026-09-27 15:32:30     8     2.62     11.61
```

The goal is a showcase of modern tooling around a realistic COBOL workload:
visibility, interfacing, development, monitoring, processing and
reliability. The batch shop is the foundation; later phases add a web
Control Room, OpenTelemetry monitoring, chaos and restart tooling, a
copybook-driven API gateway, and an MCP server so AI agents can operate it.

| Who you are | Start here |
|-------------|------------|
| Curious about mainframes | [Quick start](#quick-start), then the [tour](#a-10-minute-tour) |
| Contributing code | [CONTRIBUTING.md](CONTRIBUTING.md) |
| Want to know how it works | [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) |
| Want to know where it's going | [PLAN.md](PLAN.md) |

## Project status

| Phase | Scope | Status |
|-------|-------|--------|
| 0 | Repo, dev container, Compose, CI | ✅ Done |
| 1 | COBOL workload, GnuCOBOL, JCL-lite runner, SORT/IDCAMS/IEBGENER | ✅ Done |
| 2 | EBCDIC codecs, copybook parser, field-level decoding, data-quality checks, EBCDIC export/import | ✅ Done |
| 3 | REST/WebSocket API and Control Room web UI | Next |
| 4 | OpenTelemetry / SMF-style telemetry, Grafana, batch-window SLOs | Planned |
| 5 | Checkpoint/restart, chaos (abend injection), runbooks | Planned |
| 6 | Copybook → OpenAPI gateway, CDC events, lineage and impact analysis | Planned |
| 7 | MCP server, AI program explanation and abend triage | Planned |
| 8 | Hercules/MVS and z/OS (z/OSMF) backends behind the same adapter | Later |

## Quick start

**Requirements:** Docker, with Compose v2 (`docker compose`) or the
standalone `docker-compose`. That's all; GnuCOBOL 3.2 and Rust 1.97 live in
the dev container. It runs on Linux and macOS (Intel or Apple Silicon); the
first image build downloads about 1 GB.

```sh
git clone https://github.com/roberjo/mainframe-frontier.git
cd mainframe-frontier
make demo       # build Rust + COBOL, SETUP, then two nightly cycles (~30 s after the first build)
```

`make demo` does the following:

1. Compiles the Rust workspace and the six COBOL programs into `var/loadlib/`.
2. Seeds 10,000 accounts and runs **SETUP**, which defines the GDGs with
   IDCAMS and loads generation 1 of the account master.
3. Seeds 50,000 transactions for 2026-09-29 and runs **NIGHTLY**.
4. Seeds 2026-09-30 and runs **NIGHTLY** again. That day is month-end, so
   interest is credited.

`docker compose up` runs the same thing. Run `make help` to see every target.

## A 10-minute tour

Open a shell in the container with `make shell`, then try the following.

**The job log.** This is what an operator reads in SDSF:

```
$ frontier spool 3 JESMSGLG
15.28.23 JOB00003  $HASP373 NIGHTLY  STARTED - INIT 1    - CLASS A - SYS FRNT
15.28.35 JOB00003  -JOBNAME  STEPNAME PROGRAM      RC    RECS-IN   RECS-OUT    CPU(S)  CLOCK(S)
15.28.35 JOB00003  -NIGHTLY  SORTTRN  SORT         00      50000      50000      0.08      0.09
15.28.35 JOB00003  -NIGHTLY  VALIDATE TRNVALID     04      50000      49881      0.96      4.48
...
15.28.35 JOB00003  $HASP395 NIGHTLY  ENDED - RC=0004
```

**Proof that the books balance:**

```
$ frontier spool 3 GLRECON.SYSPRINT
LEDGER BALANCE
  OPENING LEDGER        (ACCTMAST 0)                 344,418,664.29
  + NET POSTINGS        (POSTLOG)                     10,003,812.44
  + INTEREST CREDITED   (INTLOG)                          17,105.32
  = EXPECTED CLOSING LEDGER                          354,439,582.05
  ACTUAL CLOSING LEDGER (ACCTMAST +1)                354,439,582.05
  DIFFERENCE                                                   0.00    IN BALANCE
```

**A customer statement:**

```
$ frontier spool 3 STMTGEN.STMTRPT | less
ACCOUNT 4000000002  MICHAEL Y. THOMAS               CHECKING
   TIME      TXN-ID        TYPE  DESCRIPTION                  AMOUNT              BALANCE  NOTE
   OPENING BALANCE                                                                  68.72
   02:54:54  260930027486  WD    GROCERY                     -497.44                68.72  REFUSED NSF
   09:25:45  260930036355  DP    PAYROLL DEPOSIT            1,590.68             1,659.40
```

**The catalog and GDGs:**

```
$ frontier ds list FFB
GDG BASE                                     LIMIT   GENS  CURRENT (0)
FFB.ACCTMAST                                     7      3  FFB.ACCTMAST.G0003V00
```

**Read the data through its copybook.** `cobol/datasets.toml` maps each
dataset to the copybook that describes it, so records decode field by field.
Packed decimal, 88-levels and all:

```
$ frontier ds print 'FFB.ACCTMAST(0)' --limit 3 --fields ACCT-ID,CUST-NAME,ACCT-TYPE,BALANCE,INT-RATE,ACCR-INT
     REC  ACCT-ID     CUST-NAME             ACCT-TYPE  BALANCE   INT-RATE  ACCR-INT
       1  4000000001  SUSAN B. JACKSON      S          4969.00   0.0215    -0.000598
       2  4000000002  MICHAEL Y. THOMAS     C          2898.81   0.0000    0.000000
       3  4000000003  ELIZABETH K. SANCHEZ  C          10606.15  0.0000    0.000000

$ frontier copybook ACCTREC            # the layout map: every field's position and length
05      BALANCE                          S9(11)V99 COMP-3               51      7
05      INT-RATE                         9V9(04) COMP-3                 58      3

$ frontier ds print 'FFB.ACCTMAST(0)' --format jsonl | head -1   # JSON for tools
{"ACCT-ID":"4000000001","CUST-NAME":"SUSAN B. JACKSON","ACCT-TYPE":"S","STATUS":"A","OPEN-DATE":20030221,"BALANCE":"4969.00",...}
```

Other formats are `--format fields` (one field per line, with 88-level
names), `table`, `json`, `raw` and `hex`. `frontier copybook ACCTREC --schema`
prints a JSON Schema. Decoded balances summed in Python match GLRECON's
closing ledger to the cent.

**Ship it to a mainframe, and back.** Export converts field by field:
text and zoned decimals go to EBCDIC, while packed and binary fields are
kept as-is, which a plain code-page conversion would corrupt. The result
can be sent to z/OS as a binary FTP transfer, and host files can be read
directly:

```
$ frontier ds export 'FFB.ACCTMAST(0)' acct.ebcdic --encoding 037
$ frontier decode acct.ebcdic --layout ACCTREC --limit 2     # read an EBCDIC host file
$ frontier ds import acct.ebcdic FFB.RESTORED.ACCTMAST --layout ACCTREC
```

**Break it on purpose.** Corrupt a packed field. `ds check` finds it
before the batch does, and then the runner reports a real S0C7 (Phase 5
turns this into a proper chaos tool):

```
$ printf 'ZZZZZZZ' | dd of=var/datasets/FFB.ACCTMAST.G0003V00 bs=1 seek=4150 conv=notrunc
$ frontier ds check 'FFB.ACCTMAST(0)'
  1 invalid fields in 1 records
  record      42  BALANCE   pos   51 len   7  S9(11)V99 COMP-3   invalid digit at byte 0 (hex 5A5A5A5A5A5A5A)
$ frontier seed feed --date 20261001 --count 5000
$ frontier submit cobol/jcl/NIGHTLY.jcl --set BUSDATE=20261001
JOB00004 NIGHTLY  ABEND S0C7
  POST     ACCTPOST   S0C7  ...  at cobol/src/ACCTPOST.cbl:140
  INTEREST INTCALC      --  ...  FLUSHED - PRIOR STEP ABENDED
$ frontier spool 4 POST.CEEDUMP     # libcob traceback: paragraph and line
$ exit && make demo                 # rebuild a clean system
```

The half-written `POSTLOG(+1)` is deleted and the catalog is left
untouched, just as `DISP=(NEW,CATLG,DELETE)` promises.

## The nightly cycle

`cobol/jcl/NIGHTLY.jcl` has 8 steps. Every step after the first has
`COND=(4,LT)`, so an RC above 4 anywhere stops the cycle.

| Step | Program | What it does |
|------|---------|--------------|
| SORTTRN | `SORT` | Sorts the unsorted channel feed by account and time (DFSORT control cards) |
| VALIDATE | `TRNVALID` | Edits every record (V001-V006) and prints a rejects report. RC 4 means rejects within tolerance; RC 8 means more than 5% rejected |
| POST | `ACCTPOST` | Old-master/new-master balanced-line update. Refusal reasons: NSF, FRZN (frozen), CLSD (closed), NOAC (no account) |
| INTEREST | `INTCALC` | Daily Actual/365 accrual to 6 decimals, credited at month-end with the sub-cent residual carried forward; writes `ACCTMAST(+1)` |
| GLRECON | `GLRECON` | Proves opening + postings + interest = closing, for both the ledger and the accrual. RC 8 holds the cycle |
| STMTGEN | `STMTGEN` | Customer statements with opening balance, items, interest and closing balance |
| ARCHIVE | `IEBGENER` | Copies the feed to `FFB.HIST.TRANFEED(+1)` |
| CLEANUP | `IEFBR14` | `DISP=(OLD,DELETE)` consumes the feed so the same day can't be posted twice |

The data is realistic where it matters: fixed-block records with no line
terminators, `COMP-3` packed decimal, `SIGN LEADING SEPARATE`, 88-levels,
`COPY ... REPLACING ==:TAG:==` copybooks, and about 0.2% deliberately
malformed feed records. The byte-level record layouts are in
[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#record-layouts).

## What the job runner emulates

- **JCL:** `JOB`, `EXEC PGM=,PARM=,COND=`, `DD` (`DSN`, `DISP`, `SYSOUT`,
  `DUMMY`, `*`/`DATA`, `DCB`/`RECFM`/`LRECL`), `SET` and `&SYMBOL`
  substitution, continuations, and column 72-80 sequence numbers.
- **Condition codes:** `COND=(n,op[,step])`, `EVEN`, `ONLY`. Steps after an
  abend are flushed.
- **Datasets:** a catalog; GDGs with `(+1)`/`(0)`/`(-1)` fixed for the
  whole job, limit roll-off, and `SCRATCH`/`EMPTY`; `&&TEMP` datasets
  passed between steps; normal and abnormal `DISP` handling.
- **Utilities:** `SORT` (a DFSORT subset: SORT/COPY, INCLUDE/OMIT, SUM
  FIELDS=NONE, CH/BI/ZD/PD keys), `IDCAMS` (DEFINE GDG, DELETE, LISTCAT,
  SET MAXCC), `IEBGENER`, `IEFBR14`.
- **Abends:** GnuCOBOL runtime errors become `S0C7`, `S0C4`, `S0CB`,
  `S013`, `SB37`, `S806` or `U4038`, with the COBOL source line attached.
- **Spool:** `JESMSGLG`, `JESJCL`, `JESYSMSG`, one file per SYSOUT DD,
  `CEEDUMP` on abend, and a structured `job.json` with per-step RC, CPU,
  elapsed time and record counts.

Not supported yet (these are rejected with a clear JCL error): PROCs,
`IF/THEN/ELSE`, DD concatenation, VSAM, and restart. See
[PLAN.md](PLAN.md) for when each arrives.

## What the data tools understand

- **Copybooks:**
  - fixed or free format, with sequence and identification areas, comments,
    literal continuation and `:TAG:` replacing
  - levels 01-49, 77 and 88 (`VALUE`, `THRU`)
  - `PIC` categories, including `S`, `V` and `P` scaling, and edited pictures
  - `USAGE` DISPLAY, COMP/COMP-4/BINARY, COMP-5, COMP-3, COMP-1/COMP-2,
    INDEX, POINTER
  - `SIGN LEADING/TRAILING [SEPARATE]`, `OCCURS`, `OCCURS DEPENDING ON`
    (at the end of a record), `REDEFINES`, and group `USAGE` inheritance
- **Encodings:**
  - IBM-037 and IBM-1047, with tables generated from glibc `iconv`
  - zoned signs in both GnuCOBOL (`0x70+d`) and z/OS (`C`/`D` zone) form
  - COMP-5 byte order per platform
  - IBM hex float
- **JSON:** COBOL names as keys, in layout order. Decimals are strings
  (`"4969.00"`) so no precision is lost, integers are numbers, and invalid
  fields are `null`. `ds check` reports each invalid field with its position
  and hex.

## Repository layout

```
cobol/
  src/          ACCTLOAD ACCTPOST GLRECON INTCALC STMTGEN TRNVALID
  copybooks/    record layouts (ACCTREC, TRANFEED, TRANREC, POSTREC, INTREC, ...)
  jcl/          SETUP.jcl  NIGHTLY.jcl
  datasets.toml which copybook describes which dataset
crates/
  ebcdic/       IBM-037/1047 code pages; packed, zoned, binary, hex-float codecs
  copybook/     copybook parser, layouts, decoding, JSON Schema, EBCDIC transcoding
  sort/         DFSORT-compatible sort engine
  jcl/          JCL parser, catalog and GDGs, JES executor, IDCAMS/IEBGENER/IEFBR14
  adapter/      MainframeAdapter trait + LocalAdapter, dataset layout registry
  frontier-cli/ the `frontier` command
docs/           ARCHITECTURE.md
scripts/        gen-codepages.sh (regenerates the code-page tables from iconv)
.devcontainer/  dev image (Rust 1.97 slim + GnuCOBOL 3.2) and VS Code config
.github/        CI: fmt, clippy, tests, full batch run with spool artifact
var/            runtime state (gitignored): datasets/ catalog.json loadlib/ spool/
```

## CLI reference

```
frontier build [PROGRAM...]                   compile cobol/src into the loadlib
frontier seed accounts [--count N] [--seed S] FFB.SEED.ACCTLOAD conversion file
frontier seed feed --date YYYYMMDD [--count N] [--bad-rate F]
                                              FFB.DAILY.TRANFEED channel feed
frontier submit JCL [--set SYM=VAL]... [--max-rc N] [--json]
frontier jobs [--json]                        spool listing
frontier job ID [--json]                      steps + every DD with disposition and record count
frontier spool ID [DDNAME|ALL]                list or print spool files
frontier ds list [PATTERN]                    catalog (FFB, FFB.DAILY.*, FFB.**)
frontier ds print DSN [--format raw|hex|fields|table|json|jsonl] [--fields A,B]
                      [--layout COPYBOOK] [--skip N] [--limit N] [--redefines]
frontier ds check DSN [--layout COPYBOOK]     validate every field (exit 4 on bad data)
frontier ds export DSN FILE [--encoding 037|1047]
frontier ds import FILE DSN [--encoding 037|1047] [--layout COPYBOOK] [--replace]
frontier copybook NAME [--schema|--json]      layout map, JSON Schema, or parsed layout
frontier decode FILE --layout COPYBOOK [--encoding 037|1047|local] [view options]
```

`submit` exits non-zero on an abend or JCL error, or when `--max-rc` is
exceeded; `ds check` exits 4 when it finds invalid data. Both can gate
scripts and CI (`make verify` runs `ds check` over the cycle's outputs).

## Emulated vs. real

The COBOL is real and compiles with GnuCOBOL in IBM dialect. The main gap
for Enterprise COBOL is PARM handling: these programs use
`ACCEPT ... FROM COMMAND-LINE` where z/OS passes a LINKAGE SECTION
parameter. The runner, catalog and utilities are faithful emulations of a
documented subset, and they use real z/OS message IDs so the output reads
naturally to mainframe people. The batch runs on ASCII data because that is
GnuCOBOL's native mode. EBCDIC appears at the boundary: `ds export`,
`ds import` and `decode` produce and read real z/OS byte layouts.

## License

[Apache-2.0](LICENSE)
