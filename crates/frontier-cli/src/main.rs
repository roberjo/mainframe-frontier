mod compile;
mod records;
mod seed;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use frontier_adapter::layouts::{Layout, LayoutRegistry};
use frontier_adapter::{JobRecord, JobStatus, LocalAdapter, MainframeAdapter, StepOutcome};
use frontier_copybook::{Encoding, TranscodeReport, json_schema, transcode};
use frontier_jcl::System;
use records::{Format, View, parse_encoding};

#[derive(Parser)]
#[command(
    name = "frontier",
    version,
    about = "Mainframe Frontier: run a COBOL shop from the command line"
)]
struct Cli {
    /// System home (datasets, catalog, spool, loadlib)
    #[arg(long, global = true, env = "FRONTIER_HOME", default_value = "var")]
    home: PathBuf,

    /// Copybook directory
    #[arg(long, global = true, default_value = "cobol/copybooks")]
    copybooks: PathBuf,

    /// Dataset → copybook registry
    #[arg(long, global = true, default_value = "cobol/datasets.toml")]
    layouts: PathBuf,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Compile COBOL programs into the load library
    Build {
        #[arg(long, default_value = "cobol/src")]
        src: PathBuf,
        #[arg(long, default_value = "cobol/copybooks")]
        copybooks: PathBuf,
        /// Only these programs (default: all)
        programs: Vec<String>,
    },
    /// Generate seed datasets
    Seed {
        #[command(subcommand)]
        what: SeedCmd,
    },
    /// Submit a job and wait for it to end
    Submit {
        jcl: PathBuf,
        /// Override a JCL symbol: --set BUSDATE=20260930
        #[arg(long = "set", value_parser = parse_symbol)]
        set: Vec<(String, String)>,
        /// Exit non-zero if the job's highest RC exceeds this
        #[arg(long)]
        max_rc: Option<u32>,
        #[arg(long)]
        json: bool,
    },
    /// List jobs on the spool
    Jobs {
        #[arg(long)]
        json: bool,
    },
    /// Show a job's steps and datasets
    Job {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// List a job's spool files, or print one (use ALL for everything)
    Spool { id: String, ddname: Option<String> },
    /// Dataset utilities
    Ds {
        #[command(subcommand)]
        cmd: DsCmd,
    },
    /// Show a copybook's layout (offsets, lengths, 88-levels) or its JSON Schema
    Copybook {
        /// Name in the copybook directory (ACCTREC) or a path
        name: String,
        /// Level-01 record for --schema (default: the first)
        #[arg(long)]
        record: Option<String>,
        /// Print JSON Schema instead of the listing
        #[arg(long)]
        schema: bool,
        /// Print the parsed layout as JSON
        #[arg(long)]
        json: bool,
    },
    /// Decode a host file (e.g. downloaded from z/OS in binary) with a copybook
    Decode {
        file: PathBuf,
        #[arg(long)]
        layout: String,
        #[arg(long)]
        record: Option<String>,
        /// local, 037 or 1047
        #[arg(long, default_value = "037", value_parser = parse_encoding)]
        encoding: Encoding,
        /// Record length (default: the layout length)
        #[arg(long)]
        lrecl: Option<usize>,
        #[command(flatten)]
        view: ViewArgs,
    },
}

#[derive(clap::Args)]
struct ViewArgs {
    #[arg(long, value_enum)]
    format: Option<Format>,
    #[arg(long, default_value_t = 20)]
    limit: usize,
    #[arg(long, default_value_t = 0)]
    skip: usize,
    /// Only these fields, comma-separated (names or paths)
    #[arg(long, value_delimiter = ',')]
    fields: Option<Vec<String>>,
    /// Include REDEFINES views
    #[arg(long)]
    redefines: bool,
}

#[derive(Subcommand)]
enum SeedCmd {
    /// Account conversion file FFB.SEED.ACCTLOAD (input to the SETUP job)
    Accounts {
        #[arg(long, default_value_t = 10_000)]
        count: u32,
        #[arg(long, default_value_t = 42)]
        seed: u64,
    },
    /// Daily transaction feed FFB.DAILY.TRANFEED (input to the NIGHTLY job)
    Feed {
        /// Business date, YYYYMMDD
        #[arg(long)]
        date: String,
        #[arg(long, default_value_t = 50_000)]
        count: u32,
        /// Fraction of deliberately malformed records
        #[arg(long, default_value_t = 0.002)]
        bad_rate: f64,
        #[arg(long)]
        seed: Option<u64>,
    },
}

#[derive(Subcommand)]
enum DsCmd {
    /// List cataloged datasets (pattern: FFB, FFB.DAILY.*, FFB.**)
    List { pattern: Option<String> },
    /// Print records, decoded with the dataset's copybook; accepts FFB.ACCTMAST(0)
    Print {
        dsn: String,
        /// Copybook to use instead of the registered one
        #[arg(long)]
        layout: Option<String>,
        #[arg(long)]
        record: Option<String>,
        /// Shorthand for --format hex
        #[arg(long)]
        hex: bool,
        #[command(flatten)]
        view: ViewArgs,
    },
    /// Validate every field of every record against the copybook (finds S0C7 data)
    Check {
        dsn: String,
        #[arg(long)]
        layout: Option<String>,
        #[arg(long)]
        record: Option<String>,
        /// How many problems to list
        #[arg(long, default_value_t = 20)]
        show: usize,
    },
    /// Write a dataset as an EBCDIC host file, converting field by field
    Export {
        dsn: String,
        file: PathBuf,
        #[arg(long, default_value = "037", value_parser = parse_encoding)]
        encoding: Encoding,
        #[arg(long)]
        layout: Option<String>,
        #[arg(long)]
        record: Option<String>,
    },
    /// Catalog an EBCDIC host file as a local dataset, converting field by field
    Import {
        file: PathBuf,
        dsn: String,
        #[arg(long, default_value = "037", value_parser = parse_encoding)]
        encoding: Encoding,
        #[arg(long)]
        layout: Option<String>,
        #[arg(long)]
        record: Option<String>,
        /// Record length (default: the layout length)
        #[arg(long)]
        lrecl: Option<usize>,
        /// Replace the dataset if it exists
        #[arg(long)]
        replace: bool,
    },
}

fn parse_symbol(s: &str) -> Result<(String, String), String> {
    let (k, v) = s.split_once('=').ok_or("expected NAME=VALUE")?;
    Ok((k.to_uppercase(), v.to_string()))
}

fn main() -> ExitCode {
    // Behave like a Unix filter: exit quietly when piped into `head`.
    // SAFETY: restoring the default disposition of a signal before any threads exist.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("frontier: {e:#}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    let system = System::open(&cli.home)?;
    let adapter = LocalAdapter::new(system);
    let sys = adapter.system();
    let registry = LayoutRegistry::load(&cli.layouts, &cli.copybooks)?;
    // An explicit --layout wins; otherwise the registry entry for the dataset.
    let layout_for =
        |dsn: &str, explicit: &Option<String>, record: &Option<String>| -> Result<Option<Layout>> {
            match explicit {
                Some(name) => registry.load_layout(name, record.as_deref()).map(Some),
                None => registry.for_dsn(dsn),
            }
        };

    match cli.cmd {
        Cmd::Build {
            src,
            copybooks,
            programs,
        } => {
            let ok = compile::build(sys, &src, &copybooks, &programs)?;
            return Ok(if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(8)
            });
        }
        Cmd::Seed { what } => match what {
            SeedCmd::Accounts { count, seed } => seed::accounts(sys, count, seed)?,
            SeedCmd::Feed {
                date,
                count,
                bad_rate,
                seed,
            } => seed::feed(sys, &date, count, bad_rate, seed)?,
        },
        Cmd::Submit {
            jcl,
            set,
            max_rc,
            json,
        } => {
            let source = std::fs::read_to_string(&jcl)
                .with_context(|| format!("reading {}", jcl.display()))?;
            let symbols: BTreeMap<String, String> = set.into_iter().collect();
            let job = adapter.submit(&source, &symbols)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&job)?);
            } else {
                print_job(&job);
            }
            let failed = match (&job.status, max_rc) {
                (JobStatus::Cc { rc }, Some(max)) => *rc > max,
                (JobStatus::Cc { .. }, None) => false,
                _ => true,
            };
            return Ok(if failed {
                ExitCode::from(8)
            } else {
                ExitCode::SUCCESS
            });
        }
        Cmd::Jobs { json } => {
            let jobs = adapter.jobs()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&jobs)?);
            } else {
                println!(
                    "{:<9} {:<8} {:<11} {:<19} {:>5} {:>8} {:>9}",
                    "JOBID", "JOBNAME", "STATUS", "SUBMITTED", "STEPS", "CPU(S)", "CLOCK(S)"
                );
                for j in jobs {
                    println!(
                        "{:<9} {:<8} {:<11} {:<19} {:>5} {:>8.2} {:>9.2}",
                        j.id,
                        j.name,
                        j.status.label(),
                        j.submitted
                            .get(..19)
                            .unwrap_or(&j.submitted)
                            .replace('T', " "),
                        j.steps.len(),
                        j.cpu_ms as f64 / 1000.0,
                        j.elapsed_ms as f64 / 1000.0
                    );
                }
            }
        }
        Cmd::Job { id, json } => {
            let job = adapter.job(&id)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&job)?);
            } else {
                print_job(&job);
                println!();
                for s in &job.steps {
                    for d in &s.dds {
                        println!(
                            "  {:<8} {:<8} {:<8} {:<44} {:<12} {:>10}",
                            s.name,
                            d.ddname,
                            d.direction,
                            d.dsn.as_deref().unwrap_or("-"),
                            d.disposition,
                            d.records
                                .map(|r| r.to_string())
                                .unwrap_or_else(|| "-".into())
                        );
                    }
                }
            }
        }
        Cmd::Spool { id, ddname } => {
            let job = adapter.job(&id)?;
            match ddname.as_deref() {
                None => {
                    println!("{} {}  {}", job.id, job.name, job.status.label());
                    println!(
                        "  {:<20} {:<8} {:>8} {:>10}",
                        "DDNAME", "STEP", "LINES", "BYTES"
                    );
                    for f in &job.spool {
                        println!(
                            "  {:<20} {:<8} {:>8} {:>10}",
                            f.ddname,
                            f.step.as_deref().unwrap_or(""),
                            f.lines,
                            f.bytes
                        );
                    }
                }
                Some(dd) if dd.eq_ignore_ascii_case("ALL") => {
                    for f in &job.spool {
                        println!("-------- {} {} --------", job.id, f.ddname);
                        print!("{}", adapter.spool(&job.id, &f.ddname)?);
                    }
                }
                Some(dd) => print!("{}", adapter.spool(&job.id, dd)?),
            }
        }
        Cmd::Ds { cmd } => match cmd {
            DsCmd::List { pattern } => {
                let cat = sys.catalog()?;
                println!(
                    "{:<44} {:<5} {:>6} {:>10} {:>12}  CREATED BY",
                    "DSNAME", "RECFM", "LRECL", "RECORDS", "BYTES"
                );
                for d in adapter.datasets(pattern.as_deref())? {
                    println!(
                        "{:<44} {:<5} {:>6} {:>10} {:>12}  {}",
                        d.dsn,
                        d.entry.recfm,
                        d.entry
                            .lrecl
                            .map(|l| l.to_string())
                            .unwrap_or_else(|| "-".into()),
                        d.records
                            .map(|r| r.to_string())
                            .unwrap_or_else(|| "-".into()),
                        d.bytes,
                        d.entry.created_by
                    );
                }
                let bases: Vec<_> = cat
                    .gdgs
                    .iter()
                    .filter(|(n, _)| {
                        pattern
                            .as_deref()
                            .is_none_or(|p| frontier_jcl::system::dsn_matches(p, n))
                    })
                    .collect();
                if !bases.is_empty() {
                    println!(
                        "\n{:<44} {:>5} {:>6}  CURRENT (0)",
                        "GDG BASE", "LIMIT", "GENS"
                    );
                    for (name, g) in bases {
                        let current = g
                            .generations
                            .last()
                            .map(|n| frontier_jcl::Catalog::generation_name(name, *n))
                            .unwrap_or_else(|| "-".into());
                        println!(
                            "{:<44} {:>5} {:>6}  {current}",
                            name,
                            g.limit,
                            g.generations.len()
                        );
                    }
                }
            }
            DsCmd::Print {
                dsn,
                layout,
                record,
                hex,
                view,
            } => {
                let name = sys.resolve_dsn(&dsn)?;
                let (entry, data) = adapter.read_dataset(&name)?;
                let lrecl = match entry.lrecl {
                    Some(l) if entry.is_fixed() => l as usize,
                    _ => {
                        // Line-oriented or unknown: print as text.
                        for line in String::from_utf8_lossy(&data)
                            .lines()
                            .skip(view.skip)
                            .take(view.limit)
                        {
                            println!("{line}");
                        }
                        return Ok(ExitCode::SUCCESS);
                    }
                };
                let layout = layout_for(&name, &layout, &record)?;
                records::print(&View {
                    title: format!("{name}  RECFM={}", entry.recfm),
                    data: &data,
                    lrecl,
                    layout: layout.as_ref(),
                    encoding: Encoding::Local,
                    skip: view.skip,
                    limit: view.limit,
                    format: if hex { Some(Format::Hex) } else { view.format },
                    fields: view.fields,
                    redefines: view.redefines,
                })?;
            }
            DsCmd::Check {
                dsn,
                layout,
                record,
                show,
            } => {
                let name = sys.resolve_dsn(&dsn)?;
                let (entry, data) = adapter.read_dataset(&name)?;
                let layout = layout_for(&name, &layout, &record)?.with_context(|| {
                    format!(
                        "no layout for {name}: pass --layout or map it in {}",
                        cli.layouts.display()
                    )
                })?;
                let lrecl = entry
                    .lrecl
                    .map_or(layout.record().total_size(), |l| l as usize);
                let r = records::check(&layout, &data, lrecl, Encoding::Local, show);
                println!(
                    "{name}: {} records checked against {} {} ({lrecl} bytes)",
                    r.records,
                    layout.name(),
                    layout.record().display_name()
                );
                if r.bad_fields == 0 {
                    println!("  all fields valid");
                    return Ok(ExitCode::SUCCESS);
                }
                println!(
                    "  {} invalid fields in {} records",
                    r.bad_fields, r.bad_records
                );
                for (field, n) in &r.by_field {
                    println!("    {field:<30} {n:>8}");
                }
                println!();
                for e in &r.examples {
                    println!("  {e}");
                }
                return Ok(ExitCode::from(4));
            }
            DsCmd::Export {
                dsn,
                file,
                encoding,
                layout,
                record,
            } => {
                let name = sys.resolve_dsn(&dsn)?;
                let (entry, data) = adapter.read_dataset(&name)?;
                let layout = layout_for(&name, &layout, &record)?.with_context(|| {
                    format!("no layout for {name}: export converts field by field and needs one")
                })?;
                let lrecl = entry
                    .lrecl
                    .map_or(layout.record().total_size(), |l| l as usize);
                let mut report = TranscodeReport::default();
                let out = transcode(
                    layout.record(),
                    &data,
                    lrecl,
                    Encoding::Local,
                    encoding,
                    &mut report,
                )
                .map_err(anyhow::Error::msg)?;
                std::fs::write(&file, &out)?;
                println!(
                    "{name} -> {}: {} records, LRECL {lrecl}, {encoding}",
                    file.display(),
                    report.records
                );
                return Ok(transcode_summary(&report));
            }
            DsCmd::Import {
                file,
                dsn,
                encoding,
                layout,
                record,
                lrecl,
                replace,
            } => {
                if dsn.contains('(') {
                    anyhow::bail!("import to a plain dataset name, not a GDG generation");
                }
                let dsn = dsn.to_uppercase();
                if !replace && sys.catalog()?.datasets.contains_key(&dsn) {
                    anyhow::bail!("{dsn} already exists (use --replace)");
                }
                let layout = layout_for(&dsn, &layout, &record)?
                    .with_context(|| format!("no layout for {dsn}: pass --layout"))?;
                let lrecl = lrecl.unwrap_or(layout.record().total_size());
                let data =
                    std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
                let mut report = TranscodeReport::default();
                let out = transcode(
                    layout.record(),
                    &data,
                    lrecl,
                    encoding,
                    Encoding::Local,
                    &mut report,
                )
                .map_err(anyhow::Error::msg)?;
                sys.write_dataset(&dsn, "FB", Some(lrecl as u32), &out, "IMPORT")?;
                println!(
                    "{} -> {dsn}: {} records, LRECL {lrecl}, from {encoding}",
                    file.display(),
                    report.records
                );
                return Ok(transcode_summary(&report));
            }
        },
        Cmd::Copybook {
            name,
            record,
            schema,
            json,
        } => {
            let layout = registry.load_layout(&name, record.as_deref())?;
            if schema {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json_schema(layout.record()))?
                );
            } else if json {
                println!("{}", serde_json::to_string_pretty(&layout.copybook)?);
            } else {
                print!("{}", layout.copybook.listing());
            }
        }
        Cmd::Decode {
            file,
            layout,
            record,
            encoding,
            lrecl,
            view,
        } => {
            let layout = registry.load_layout(&layout, record.as_deref())?;
            let data =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let lrecl = lrecl.unwrap_or(layout.record().total_size());
            if lrecl == 0 || !data.len().is_multiple_of(lrecl) {
                anyhow::bail!(
                    "{} is {} bytes, not a multiple of LRECL {lrecl}",
                    file.display(),
                    data.len()
                );
            }
            records::print(&View {
                title: file.display().to_string(),
                data: &data,
                lrecl,
                layout: Some(&layout),
                encoding,
                skip: view.skip,
                limit: view.limit,
                format: view.format,
                fields: view.fields,
                redefines: view.redefines,
            })?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn transcode_summary(report: &TranscodeReport) -> ExitCode {
    if report.floats_unconverted > 0 {
        println!(
            "  note: {} COMP-1/COMP-2 fields copied without float conversion",
            report.floats_unconverted
        );
    }
    if report.invalid_numeric == 0 {
        return ExitCode::SUCCESS;
    }
    println!(
        "  {} zoned fields were not valid numbers and were translated as text:",
        report.invalid_numeric
    );
    for p in &report.problems {
        println!("    {p}");
    }
    ExitCode::from(4)
}

fn print_job(job: &JobRecord) {
    println!("{} {:<8} {}", job.id, job.name, job.status.label());
    println!(
        "  {:<8} {:<8} {:>6} {:>10} {:>10} {:>8} {:>9}  NOTE",
        "STEPNAME", "PROGRAM", "RC", "RECS-IN", "RECS-OUT", "CPU(S)", "CLOCK(S)"
    );
    for s in &job.steps {
        let (rc, note) = match &s.outcome {
            StepOutcome::Executed { rc } => (format!("{rc:02}"), String::new()),
            StepOutcome::Abended { code, source, .. } => (
                code.clone(),
                source
                    .as_ref()
                    .map(|s| format!("at {s}"))
                    .unwrap_or_default(),
            ),
            StepOutcome::NotRun { reason } => ("--".into(), reason.clone()),
        };
        println!(
            "  {:<8} {:<8} {:>6} {:>10} {:>10} {:>8.2} {:>9.2}  {}",
            s.name,
            s.program,
            rc,
            s.records_in,
            s.records_out,
            s.cpu_ms as f64 / 1000.0,
            s.elapsed_ms as f64 / 1000.0,
            note
        );
    }
    if let JobStatus::JclError { message } = &job.status {
        println!("  {message}");
    }
}
