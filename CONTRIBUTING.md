# Contributing

## Setup

You need Docker. Any engine works: Docker Desktop, Colima or OrbStack.

```sh
make image      # build the dev container (Rust 1.97 + GnuCOBOL 3.2), about 1 GB
make demo       # prove everything works end to end
make shell      # interactive shell with cobc, cargo and frontier on PATH
```

Or open the folder in VS Code and choose **Reopen in Container**. The
devcontainer installs rust-analyzer and COBOL language support, and draws
rulers at columns 7, 12 and 72.

Each `make` target runs inside the container when started from the host,
and runs directly when started from `make shell`.

## Development loop

| Changed | Run |
|---------|-----|
| Rust | `make test` then `make lint` (rustfmt + clippy `-D warnings`) |
| A COBOL program or copybook | `make cobol` then `make nightly BUSDATE=20260929`, then `frontier copybook NAME` to confirm the layout |
| A new dataset | Map it to its copybook in `cobol/datasets.toml` so `ds print` / `ds check` can decode it |
| JCL | `frontier submit cobol/jcl/NIGHTLY.jcl --set BUSDATE=...` |
| Anything, before a PR | `make demo && make verify`: both NIGHTLY jobs at `CC 0004`, GLRECON `IN BALANCE`, and every output field valid |

Look at results with `frontier jobs`, `frontier job <id>`,
`frontier spool <id> [DD|ALL]`, and `frontier ds print <dsn> --hex`.

CI (`.github/workflows/ci.yml`) runs the same checks: fmt, clippy, tests,
and a full `make demo` at reduced volume, with the spool uploaded as an
artifact.

## Conventions

### COBOL

- Fixed format: sequence area in columns 1-6, indicator in column 7, area
  A from column 8, area B from column 12, **nothing past column 72**. Check
  with:
  `awk 'length > 72 {print FILENAME":"FNR}' cobol/src/*.cbl cobol/copybooks/*.cpy`
- Every program starts with a header box: purpose, each DD with its layout
  and direction, and the meaning of each return code.
- Record layouts live in copybooks, written with a `:TAG:` prefix and
  pulled in with `COPY X REPLACING ==:TAG:== BY ==PFX==` so one layout can
  be used twice in a program.
- Return codes: 0 OK, 4 warning, 8 error that should stop the cycle,
  12 logic or sequence error, 16 file or parm error.
- Test a numeric field's class before using it in arithmetic or a
  comparison. With `-debug`, invalid data is an S0C7, and that is
  intentional.

### JCL

- Operands end at column 71; continue with a trailing comma and `//` plus
  spaces on the next line.
- Every step after the first carries a `COND=`, and every new dataset gets
  explicit `DISP=(NEW,CATLG,DELETE)` or `(NEW,PASS)`.

### Rust

- rustfmt defaults and clippy clean. Errors use `anyhow` in binaries and
  typed errors where callers branch on them (`JclError`, `SortError`).
- Messages written to spool use real z/OS message IDs where one exists,
  and plain text otherwise. Never make up an IBM message ID.
- New JCL or utility behavior needs a unit test next to the code (see the
  `tests` modules in `parse.rs`, `catalog.rs`, `sort/lib.rs`).

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| `unknown flag: --rm` from `docker compose` | The Compose plugin isn't installed. The Makefile falls back to `docker-compose` automatically; install either one |
| `input/output error` from Docker, or `rustc` segfaults | Usually a full host disk that corrupted image layers. Free space, restart the engine (`colima restart`), then `docker rmi` and rebuild the image |
| `` `cobc` not found `` | You ran `frontier build` on the host. Use `make cobol` or `make shell` |
| NIGHTLY ends in `JCL ERROR ... TRANFEED ... NOT FOUND` | Expected: CLEANUP consumed the feed. Seed a new day with `make nightly BUSDATE=YYYYMMDD` |
| SETUP `IGD17101I ... DUPLICATE NAME` | Start over with `make reset` (or `make demo`) |
| Slow steps on macOS | Bind-mount I/O through the VM. Expect about 12 s per 50,000-transaction cycle |
