mod compile;
mod seed;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use frontier_adapter::{JobRecord, JobStatus, LocalAdapter, MainframeAdapter, StepOutcome};
use frontier_jcl::System;

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
    /// Print records; accepts relative GDG names like FFB.ACCTMAST(0)
    Print {
        dsn: String,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long, default_value_t = 0)]
        skip: usize,
        /// Hex dump instead of characters
        #[arg(long)]
        hex: bool,
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
                limit,
                skip,
                hex,
            } => {
                let (entry, data) = adapter.read_dataset(&dsn)?;
                let lrecl = match entry.lrecl {
                    Some(l) if entry.is_fixed() => l as usize,
                    _ => {
                        // Line-oriented or unknown: print as text.
                        for line in String::from_utf8_lossy(&data)
                            .lines()
                            .skip(skip)
                            .take(limit)
                        {
                            println!("{line}");
                        }
                        return Ok(ExitCode::SUCCESS);
                    }
                };
                println!(
                    "{} RECFM={} LRECL={lrecl} RECORDS={}",
                    sys.resolve_dsn(&dsn)?,
                    entry.recfm,
                    data.len() / lrecl
                );
                for (i, rec) in data.chunks(lrecl).enumerate().skip(skip).take(limit) {
                    if hex {
                        println!("{:>8}", i + 1);
                        for (off, chunk) in rec.chunks(32).enumerate() {
                            let h: Vec<String> = chunk
                                .chunks(4)
                                .map(|w| w.iter().map(|b| format!("{b:02X}")).collect())
                                .collect();
                            println!(
                                "  +{:04} {:<72} |{}|",
                                off * 32,
                                h.join(" "),
                                printable(chunk)
                            );
                        }
                    } else {
                        println!("{:>8} {}", i + 1, printable(rec));
                    }
                }
            }
        },
    }
    Ok(ExitCode::SUCCESS)
}

fn printable(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if (0x20..0x7f).contains(&b) {
                b as char
            } else {
                '.'
            }
        })
        .collect()
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
