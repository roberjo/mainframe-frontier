# Mainframe Frontier — Repo Plan

> A working mainframe you can clone: a realistic COBOL workload running on a real
> (or emulated) mainframe substrate, wrapped in modern tooling for visibility,
> interfacing, development, monitoring, processing, and reliability.

## 1. Why this repo stands out

Most "COBOL demo" repos are either a single `HELLO.cbl` or a slide-deck-style
modernization pitch. This one goes further: a **complete, runnable system**
that behaves like a production mainframe shop, including the parts that usually
go unshown: nightly batch windows, abends, restarts, copybook drift, spool
archaeology, SLAs. Each pillar gets modern tooling.

| Pillar       | What it shows                                                                 |
|--------------|-------------------------------------------------------------------------------|
| Processing   | Real COBOL batch + online transactions, JCL job streams, VSAM-style files, COMP-3, EBCDIC |
| Interfacing  | Copybook → OpenAPI/JSON auto-mapping, REST/gRPC gateway, CDC event stream, **MCP server** for AI agents |
| Development  | Devcontainer, COBOL LSP, COBOL unit tests, CI/CD, impact analysis, lineage graph |
| Visibility   | "Control Room" web UI: job DAG, spool viewer, copybook-aware record decoder, web 3270 |
| Monitoring   | OpenTelemetry spans per job step, SMF-style records → Prometheus/Grafana/Loki |
| Reliability  | Checkpoint/restart, idempotent reruns, abend chaos injection, AI abend triage, batch-window SLOs |

## 2. System architecture

```
                        ┌─────────────────────────────────────────────┐
                        │        Control Room (web UI)                │
                        │ job DAG · spool · datasets · 3270 · lineage │
                        └───────────────┬─────────────────────────────┘
                                        │
  AI agents ──MCP──┐    ┌───────────────▼──────────────┐    ┌──────────────┐
                   ├───►│   Frontier API (control plane)│───►│ Event stream │ (CDC of postings)
  REST/gRPC ───────┘    │ copybook→JSON · jobs · SLOs   │    └──────────────┘
                        └───────────────┬──────────────┘
                                        │  MainframeAdapter interface
             ┌──────────────────────────┼──────────────────────────┐
             ▼                          ▼                          ▼
   ┌──────────────────┐     ┌──────────────────────┐    ┌──────────────────────┐
   │ local            │     │ hercules             │    │ zos                  │
   │ GnuCOBOL 3.2 +   │     │ Hercules Hyperion +  │    │ Real z/OS via Zowe / │
   │ JCL-lite runner  │     │ MVS 3.8j (TK5), JES2 │    │ z/OSMF REST          │
   └──────────────────┘     └──────────────────────┘    └──────────────────────┘
             │                          │                          │
             └────────── OpenTelemetry collector ──► Prometheus · Loki · Tempo · Grafana
```

**The key idea:** one `MainframeAdapter` contract (`submitJob`, `jobStatus`,
`readSpool`, `listDatasets`, `readRecords`, `writeRecords`) with three backends.
Everything above it works the same whether the COBOL runs in a laptop container,
an emulated MVS, or real z/OS.

## 3. Repo layout (Rust core + TypeScript UI)

```
mainframe-frontier/
├── cobol/                    # The workload: "First Frontier Bank"
│   ├── src/                  #   programs (.cbl)
│   ├── copybooks/            #   record layouts (.cpy)
│   ├── jcl/                  #   job streams (.jcl)
│   ├── bms/                  #   online screen maps
│   ├── data/                 #   seed data + EBCDIC fixtures
│   └── tests/                #   COBOL Check unit tests
├── crates/                   # Rust workspace (the core)
│   ├── ebcdic/               #   EBCDIC, COMP-3, COMP, zoned decimal codecs (SIMD-fast)
│   ├── copybook/             #   Copybook parser → AST → JSON Schema / OpenAPI / TS types
│   ├── jcl/                  #   JCL-lite parser + executor (steps, DD, COND, GDG, restart)
│   ├── sort/                 #   DFSORT-style SORT/INCLUDE/SUM control-card engine
│   ├── adapter/              #   MainframeAdapter trait (local impl now; hercules/zos later)
│   ├── frontier-cli/         #   `frontier` CLI: build, seed, submit, jobs, spool, ds
│   ├── lineage/              #   program ↔ copybook ↔ dataset ↔ job graph + impact diff
│   ├── telemetry/            #   SMF-30-style records + OpenTelemetry spans/metrics
│   └── frontier-api/         #   axum: REST + gRPC + WebSocket live job events
├── apps/
│   ├── control-room/         # Next.js UI: job DAG, spool, hex/record viewer, 3270, lineage
│   └── mcp-server/           # TypeScript MCP server (wraps frontier-api)
├── ops/
│   ├── otel/ grafana/ prometheus/ loki/ tempo/
│   ├── chaos/                # Abend injection (S0C7, S0C4, B37, S322, late input)
│   └── runbooks/
├── .devcontainer/            # Rust, GnuCOBOL 3.2, Node, COBOL LSP
├── .github/workflows/        # cargo test · cobol compile + unit test · batch smoke · lineage diff
├── Cargo.toml  package.json  docker-compose.yml  Makefile
```

**Stack split:** Rust owns everything that touches records or runs jobs:
codecs, copybooks, JCL, sort, lineage, telemetry, and the API. TypeScript owns
the Control Room UI and the MCP server, which use TS types generated from
copybooks by the `copybook` crate. GnuCOBOL compiles and runs the COBOL; the
Rust `jcl` executor orchestrates it the way JES2 would.

## 4. The workload: "First Frontier Bank" (default domain)

**Batch (`NIGHTLY.jcl`, 8 steps, all `COND=(4,LT)`):**
1. `SORTTRN` (SORT): sort the channel feed by account + timestamp
2. `VALIDATE` (TRNVALID): edits V001-V006 and a rejects report; RC 4 on rejects, RC 8 above 5%
3. `POST` (ACCTPOST): balanced-line master update; NSF/FRZN/CLSD/NOAC refusals; `POSTLOG(+1)` journal
4. `INTEREST` (INTCALC): Actual/365 accrual, month-end credit with sub-cent residual carried → `ACCTMAST(+1)`
5. `GLRECON` (GLRECON): proves the ledger and accrual identities; RC 8 holds the cycle
6. `STMTGEN` (STMTGEN): customer statements
7. `ARCHIVE` (IEBGENER): feed → `FFB.HIST.TRANFEED(+1)`
8. `CLEANUP` (IEFBR14): consume the feed so it can't be posted twice

`SETUP.jcl` defines the GDGs with IDCAMS and loads `ACCTMAST(+1)` from the conversion file.

**Online (CICS-style pseudo-conversational):** balance inquiry, transfer,
transaction history, rendered as 3270 screens *and* exposed as REST via the
copybook gateway.

**Data realism:** EBCDIC files, packed decimal, REDEFINES, OCCURS DEPENDING ON,
fixed/variable record formats, 1M+ generated accounts for performance demos.

## 5. Headline features (the "wow" list)

1. **`docker compose up` → a full mainframe shop** running a nightly batch in about 60 seconds.
2. **Copybook-aware hex/record viewer.** Click any dataset and see raw EBCDIC hex
   next to decoded fields, with COMP-3 nibbles highlighted.
3. **Live job DAG.** Steps light up as they run, with return codes, CPU/elapsed, and
   records in and out, all driven by OTel spans.
4. **Copybook → OpenAPI in one command.** Change a copybook and the REST contract,
   JSON codecs, and TypeScript types regenerate; CI flags breaking changes.
5. **Impact analysis.** "If I change `ACCTREC.cpy`, which programs, jobs, datasets,
   and APIs break?" is answered from the lineage graph as a PR comment.
6. **MCP server.** Claude (or any agent) can submit jobs, read spool, decode
   datasets, and explain programs. Demo: "why did last night's batch fail?"
7. **AI abend triage.** On S0C7, the agent pulls the spool, maps the offset to the
   COBOL source line, finds the bad record, and proposes a fix plus a unit test.
8. **Chaos mode.** Inject abends, space failures, and late input files; watch
   checkpoint/restart recover and the batch-window SLO burn-down react.
9. **Web 3270 terminal** attached to the online transactions (and to real TSO
   on the Hercules backend).
10. **Same UI across three substrates.** Flip `SUBSTRATE=local|hercules|zos`.

## 6. Roadmap

| Phase | Deliverable | Outcome | Status |
|-------|-------------|---------|--------|
| 0 | Repo skeleton, devcontainer, compose, CI | `make demo` works | **Done** |
| 1 | COBOL workload + GnuCOBOL + JCL-lite runner | Nightly batch runs locally end-to-end | **Done** |
| 2 | `ebcdic` + `copybook` packages, dataset decoder | Records readable in the UI and API | |
| 3 | Control-plane API + Control Room UI (DAG, spool, datasets) | Visual demo | |
| 4 | OTel/SMF telemetry + Grafana dashboards + SLOs | Monitoring story | |
| 5 | Checkpoint/restart, chaos scenarios, runbooks | Reliability story | |
| 6 | Copybook→OpenAPI gateway, CDC stream, lineage/impact analysis | Interfacing + dev story | |
| 7 | MCP server + AI explain/triage | Cutting-edge AI story | |
| 8 (later) | Hercules/TK5 and Zowe/z/OS adapters behind the same trait | "It's a real mainframe" story | |

Each phase ships something demoable on its own.

## 7. Decisions (2026-09-27)

- **Emphasis:** full platform, with all six pillars balanced
- **Substrate:** GnuCOBOL local only for now; `MainframeAdapter` trait keeps Hercules/z/OS pluggable later
- **Domain:** core banking ("First Frontier Bank")
- **Stack:** Rust core (crates + axum API) and TypeScript (Next.js Control Room + MCP server)
