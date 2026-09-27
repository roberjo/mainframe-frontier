//! Job execution: the part JES2 and the initiator play on z/OS.
//!
//! A submitted job gets a job number and a spool directory, then each step
//! is taken through the same lifecycle as on a real system:
//!
//! 1. `COND=` evaluation (and flushing after an abend unless `EVEN`/`ONLY`)
//! 2. DD allocation (catalog lookups, GDG relative resolution, temporaries)
//! 3. program execution (built-in utility or a load module from the loadlib)
//! 4. disposition processing (`CATLG`, `PASS`, `DELETE`, GDG roll-off)
//!
//! Everything is recorded in `job.json` (for tools) and in the classic
//! JESMSGLG / JESJCL / JESYSMSG spool files (for humans).

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, DatasetEntry, parse_relative};
use crate::parse::{Dd, DdKind, Disp, DispAction, DispStatus, JclError, Job, Step, parse_job};
use crate::system::{System, now_iso};
use crate::utilities;

// ---------------------------------------------------------------------------
// Job records (job.json)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JobStatus {
    /// Completed; `rc` is the highest step condition code.
    Cc {
        rc: u32,
    },
    Abend {
        code: String,
        step: String,
    },
    JclError {
        message: String,
    },
}

impl JobStatus {
    /// SDSF-style: `CC 0004`, `ABEND S0C7`, `JCL ERROR`.
    pub fn label(&self) -> String {
        match self {
            JobStatus::Cc { rc } => format!("CC {rc:04}"),
            JobStatus::Abend { code, .. } => format!("ABEND {code}"),
            JobStatus::JclError { .. } => "JCL ERROR".into(),
        }
    }

    /// Worst-case numeric severity, for `--max-rc` style checks.
    pub fn severity(&self) -> u32 {
        match self {
            JobStatus::Cc { rc } => *rc,
            _ => u32::MAX,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StepOutcome {
    Executed {
        rc: u32,
    },
    Abended {
        code: String,
        reason: String,
        source: Option<String>,
    },
    NotRun {
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DdRecord {
    pub ddname: String,
    pub dsn: Option<String>,
    /// in | out | sysout | dummy | instream
    pub direction: String,
    /// CATALOGED, KEPT, PASSED, DELETED, UNCATALOGED, SYSOUT, ...
    pub disposition: String,
    pub records: Option<u64>,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRecord {
    pub name: String,
    pub program: String,
    pub parm: Option<String>,
    pub outcome: StepOutcome,
    pub started: Option<String>,
    pub ended: Option<String>,
    pub elapsed_ms: u64,
    pub cpu_ms: u64,
    pub records_in: u64,
    pub records_out: u64,
    pub dds: Vec<DdRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpoolFile {
    /// `JESMSGLG`, `JESJCL`, `JESYSMSG`, or `STEPNAME.DDNAME`.
    pub ddname: String,
    pub step: Option<String>,
    pub lines: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: String,
    pub name: String,
    pub class: String,
    pub submitted: String,
    pub ended: String,
    pub status: JobStatus,
    pub cpu_ms: u64,
    pub elapsed_ms: u64,
    pub symbols: BTreeMap<String, String>,
    pub steps: Vec<StepRecord>,
    pub spool: Vec<SpoolFile>,
}

// ---------------------------------------------------------------------------
// Allocation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub(crate) enum Target {
    Sysout,
    Dummy,
    Instream,
    Temp(String),
    Cataloged {
        dsn: String,
        gdg: Option<(String, u32)>,
    },
    /// A non-temporary dataset created with DISP=(NEW,PASS).
    PassedNamed(String),
}

#[derive(Debug, Clone)]
pub(crate) struct Alloc {
    pub ddname: String,
    pub path: PathBuf,
    pub target: Target,
    /// Name shown in IEF285I messages.
    pub display: String,
    pub new: bool,
    pub output: bool,
    pub normal: DispAction,
    pub abnormal: DispAction,
    pub recfm: Option<String>,
    pub lrecl: Option<u32>,
}

impl Alloc {
    fn simple(ddname: &str, path: PathBuf, target: Target, display: &str) -> Self {
        Alloc {
            ddname: ddname.to_string(),
            path,
            target,
            display: display.to_string(),
            new: false,
            output: false,
            normal: DispAction::Keep,
            abnormal: DispAction::Keep,
            recfm: None,
            lrecl: None,
        }
    }

    fn record_count(&self, bytes: u64) -> Option<u64> {
        match (&self.target, self.lrecl, self.recfm.as_deref()) {
            (Target::Dummy, ..) => Some(0),
            (Target::Sysout | Target::Instream, ..) => {
                fs::read(&self.path).ok().map(|b| count_lines(&b))
            }
            (_, Some(l), Some(r)) if r.starts_with('F') && l > 0 => Some(bytes / l as u64),
            (_, _, Some("LS")) => fs::read(&self.path).ok().map(|b| count_lines(&b)),
            _ => None,
        }
    }
}

fn count_lines(bytes: &[u8]) -> u64 {
    let n = bytes.iter().filter(|b| **b == b'\n').count() as u64;
    if bytes.last().is_some_and(|b| *b != b'\n') {
        n + 1
    } else {
        n
    }
}

fn dispositions(disp: &Disp, new: bool) -> (DispAction, DispAction) {
    let normal = disp.normal.unwrap_or(if new {
        DispAction::Delete
    } else {
        DispAction::Keep
    });
    let abnormal = disp.abnormal.unwrap_or(match normal {
        DispAction::Pass if new => DispAction::Delete,
        DispAction::Pass => DispAction::Keep,
        other => other,
    });
    (normal, abnormal)
}

/// Program completion, as the initiator sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Outcome {
    Rc(u32),
    Abend {
        code: String,
        reason: String,
        source: Option<String>,
    },
}

struct Passed {
    path: PathBuf,
    display: String,
    recfm: Option<String>,
    lrecl: Option<u32>,
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

fn cpu_time() -> Duration {
    fn usage(who: libc::c_int) -> Duration {
        // SAFETY: getrusage only writes into the zeroed struct we pass it.
        let ru = unsafe {
            let mut ru: libc::rusage = std::mem::zeroed();
            libc::getrusage(who, &mut ru);
            ru
        };
        let tv = |t: libc::timeval| Duration::new(t.tv_sec as u64, t.tv_usec as u32 * 1000);
        tv(ru.ru_utime) + tv(ru.ru_stime)
    }
    usage(libc::RUSAGE_SELF) + usage(libc::RUSAGE_CHILDREN)
}

fn fmt_cpu(d: Duration) -> String {
    let secs = d.as_secs_f64();
    let hr = (secs / 3600.0) as u64;
    let min = ((secs % 3600.0) / 60.0) as u64;
    format!("{hr:>5} HR  {min:02} MIN  {:05.2} SEC", secs % 60.0)
}

struct Runner<'a> {
    sys: &'a System,
    id: String,
    dir: PathBuf,
    temp: PathBuf,
    job: Job,
    submitted: DateTime<Local>,
    t0: Instant,
    cpu0: Duration,
    msglg: Vec<String>,
    sysmsg: Vec<String>,
    steps: Vec<StepRecord>,
    passed: BTreeMap<String, Passed>,
    /// GDG generation lists as of each base's first reference in this job.
    snapshot: BTreeMap<String, Vec<u32>>,
    spool_files: Vec<(String, Option<String>)>,
    abend: Option<(String, String)>,
    jcl_error: Option<String>,
}

impl System {
    /// Submit a job and run it to completion.
    pub fn submit(&self, source: &str, overrides: &BTreeMap<String, String>) -> Result<JobRecord> {
        let id = self.next_job_id()?;
        let dir = self.spool_root().join(&id);
        let temp = dir.join("temp");
        fs::create_dir_all(&temp)?;
        let submitted = Local::now();

        let job = match parse_job(source, overrides) {
            Ok(job) => job,
            Err(e) => return self.record_jcl_error(&id, &dir, source, &e, submitted),
        };

        let mut runner = Runner {
            sys: self,
            id,
            dir,
            temp,
            job,
            submitted,
            t0: Instant::now(),
            cpu0: cpu_time(),
            msglg: Vec::new(),
            sysmsg: Vec::new(),
            steps: Vec::new(),
            passed: BTreeMap::new(),
            snapshot: BTreeMap::new(),
            spool_files: Vec::new(),
            abend: None,
            jcl_error: None,
        };
        runner.run()
    }

    fn record_jcl_error(
        &self,
        id: &str,
        dir: &Path,
        source: &str,
        e: &JclError,
        submitted: DateTime<Local>,
    ) -> Result<JobRecord> {
        let name = source
            .lines()
            .next()
            .and_then(|l| l.strip_prefix("//"))
            .and_then(|l| l.split_whitespace().next())
            .unwrap_or("UNKNOWN")
            .chars()
            .take(8)
            .collect::<String>();
        let t = submitted.format("%H.%M.%S");
        let listing: Vec<String> = source
            .lines()
            .enumerate()
            .map(|(i, l)| format!("{:>10} {l}", i + 1))
            .collect();
        let files = [
            (
                "JESMSGLG",
                vec![
                    format!("{t} {id}  IEFC452I {name} - JOB NOT RUN - JCL ERROR"),
                    format!("{t} {id}  $HASP395 {name:<8} ENDED - JCL ERROR"),
                ],
            ),
            ("JESJCL", listing),
            ("JESYSMSG", vec![format!("{:>10} {}", e.line, e.message)]),
        ];
        let mut spool = Vec::new();
        for (dd, lines) in files {
            let text = lines.join("\n") + "\n";
            fs::write(dir.join(dd), &text)?;
            spool.push(SpoolFile {
                ddname: dd.into(),
                step: None,
                lines: lines.len() as u64,
                bytes: text.len() as u64,
            });
        }
        fs::remove_dir_all(dir.join("temp")).ok();
        let record = JobRecord {
            id: id.to_string(),
            name,
            class: "A".into(),
            submitted: submitted.format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string(),
            ended: now_iso(),
            status: JobStatus::JclError {
                message: e.to_string(),
            },
            cpu_ms: 0,
            elapsed_ms: 0,
            symbols: BTreeMap::new(),
            steps: Vec::new(),
            spool,
        };
        fs::write(dir.join("job.json"), serde_json::to_string_pretty(&record)?)?;
        Ok(record)
    }
}

impl Runner<'_> {
    fn log(&mut self, text: impl AsRef<str>) {
        let t = Local::now().format("%H.%M.%S");
        self.msglg
            .push(format!("{t} {}  {}", self.id, text.as_ref()));
    }

    fn register_spool(&mut self, step: &str, dd: &str) -> PathBuf {
        let name = format!("{step}.{dd}");
        if !self.spool_files.iter().any(|(n, _)| *n == name) {
            self.spool_files
                .push((name.clone(), Some(step.to_string())));
        }
        self.dir.join(name)
    }

    fn temp_display(&self, key: &str) -> String {
        format!(
            "SYS{}.T{}.RA000.{}.{}",
            self.submitted.format("%y%j"),
            self.submitted.format("%H%M%S"),
            self.job.name,
            key
        )
    }

    fn run(&mut self) -> Result<JobRecord> {
        let name = self.job.name.clone();
        self.log(format!(
            "$HASP373 {name:<8} STARTED - INIT 1    - CLASS {} - SYS FRNT",
            self.job.class
        ));
        self.log(format!(
            "IEF403I {name} - STARTED - TIME={}",
            Local::now().format("%H.%M.%S")
        ));

        let steps = self.job.steps.clone();
        for step in &steps {
            if self.jcl_error.is_some() {
                self.not_run(step, "NOT RUN - JOB FAILED WITH JCL ERROR".into());
                continue;
            }
            if let Some(reason) = self.bypass_reason(step) {
                if reason.starts_with("COND") {
                    self.sysmsg.push(format!(
                        "IEF202I {name} {} - STEP WAS NOT RUN BECAUSE OF CONDITION CODES",
                        step.name
                    ));
                }
                self.sysmsg.push(format!(
                    "IEF272I {name} {} - STEP WAS NOT EXECUTED.",
                    step.name
                ));
                self.not_run(step, reason);
                continue;
            }
            self.run_step(step)?;
        }

        // Passed datasets nobody picked up are deleted at end of job.
        for (_, p) in std::mem::take(&mut self.passed) {
            fs::remove_file(&p.path).ok();
            self.sysmsg
                .push(format!("IEF285I   {:<44} DELETED", p.display));
        }
        fs::remove_dir_all(&self.temp).ok();

        self.finish()
    }

    fn not_run(&mut self, step: &Step, reason: String) {
        self.steps.push(StepRecord {
            name: step.name.clone(),
            program: step.pgm.clone(),
            parm: step.parm.clone(),
            outcome: StepOutcome::NotRun { reason },
            started: None,
            ended: None,
            elapsed_ms: 0,
            cpu_ms: 0,
            records_in: 0,
            records_out: 0,
            dds: Vec::new(),
        });
    }

    fn bypass_reason(&self, step: &Step) -> Option<String> {
        if self.abend.is_some() {
            if !(step.cond.even || step.cond.only) {
                return Some("FLUSHED - PRIOR STEP ABENDED".into());
            }
        } else if step.cond.only {
            return Some("COND=ONLY - NO PRIOR ABEND".into());
        }
        for test in &step.cond.tests {
            let rcs = self
                .steps
                .iter()
                .filter_map(|s| match (&s.outcome, &test.step) {
                    (StepOutcome::Executed { rc }, Some(want)) if s.name == *want => Some(*rc),
                    (StepOutcome::Executed { rc }, None) => Some(*rc),
                    _ => None,
                });
            for rc in rcs {
                if test.bypasses(rc) {
                    let op = format!("{:?}", test.op).to_uppercase();
                    return Some(format!("COND=({},{op}) MET BY RC {rc:04}", test.code));
                }
            }
        }
        None
    }

    fn run_step(&mut self, step: &Step) -> Result<()> {
        let job = self.job.name.clone();
        let started = Local::now();
        let t = Instant::now();
        let cpu = cpu_time();
        self.sysmsg
            .push(format!("IEF236I ALLOC. FOR {job} {}", step.name));

        let mut allocs = match self.allocate(step) {
            Ok(a) => a,
            Err(msg) => {
                self.sysmsg.push(msg.clone());
                self.sysmsg.push(format!(
                    "IEF272I {job} {} - STEP WAS NOT EXECUTED.",
                    step.name
                ));
                self.log(format!(
                    "IEF453I {job} - JOB FAILED - JCL ERROR - TIME={}",
                    Local::now().format("%H.%M.%S")
                ));
                self.jcl_error = Some(msg.clone());
                self.not_run(step, format!("JCL ERROR: {msg}"));
                return Ok(());
            }
        };
        for a in &allocs {
            let unit = match a.target {
                Target::Sysout => "JES2",
                Target::Dummy => "DMY ",
                Target::Instream => "JES2",
                _ => "DASD",
            };
            self.sysmsg
                .push(format!("IEF237I {unit} ALLOCATED TO {}", a.ddname));
        }

        let outcome = self.execute(step, &mut allocs)?;
        let abended = matches!(outcome, Outcome::Abend { .. });
        let step_outcome = match &outcome {
            Outcome::Rc(rc) => {
                self.sysmsg.push(format!(
                    "IEF142I {job} {} - STEP WAS EXECUTED - COND CODE {rc:04}",
                    step.name
                ));
                StepOutcome::Executed { rc: *rc }
            }
            Outcome::Abend {
                code,
                reason,
                source,
            } => {
                let (sys, user) = if let Some(u) = code.strip_prefix('U') {
                    ("000".to_string(), u.to_string())
                } else {
                    (code.trim_start_matches('S').to_string(), "0000".to_string())
                };
                self.log(format!("IEA995I SYMPTOM DUMP OUTPUT  {}", step.name));
                self.log(format!(
                    "  SYSTEM COMPLETION CODE={sys}  REASON CODE={reason}"
                ));
                if let Some(src) = source {
                    self.log(format!("  PROGRAM={}  SOURCE={src}", step.pgm));
                }
                self.log(format!(
                    "IEF450I {job} {} - ABEND=S{sys} U{user} REASON={reason}",
                    step.name
                ));
                self.sysmsg.push(format!(
                    "IEF472I {job} {} - COMPLETION CODE - SYSTEM={sys} USER={user} REASON={reason}",
                    step.name
                ));
                if self.abend.is_none() {
                    self.abend = Some((code.clone(), step.name.clone()));
                }
                StepOutcome::Abended {
                    code: code.clone(),
                    reason: reason.clone(),
                    source: source.clone(),
                }
            }
        };

        let dds = self.dispose(&mut allocs, abended)?;
        let ended = Local::now();
        let cpu_used = cpu_time().saturating_sub(cpu);
        self.sysmsg.push(format!(
            "IEF373I STEP/{:<8}/START {}",
            step.name,
            started.format("%Y%j.%H%M")
        ));
        self.sysmsg.push(format!(
            "IEF032I STEP/{:<8}/STOP  {}",
            step.name,
            ended.format("%Y%j.%H%M")
        ));
        self.sysmsg.push(format!(
            "        CPU: {}    SRB: {}",
            fmt_cpu(cpu_used),
            fmt_cpu(Duration::ZERO)
        ));

        let sum = |dir: &str| -> u64 {
            dds.iter()
                .filter(|d| d.direction == dir)
                .filter_map(|d| d.records)
                .sum()
        };
        let (records_in, records_out) = (sum("in"), sum("out"));
        self.steps.push(StepRecord {
            name: step.name.clone(),
            program: step.pgm.clone(),
            parm: step.parm.clone(),
            outcome: step_outcome,
            started: Some(started.format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string()),
            ended: Some(ended.format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string()),
            elapsed_ms: t.elapsed().as_millis() as u64,
            cpu_ms: cpu_used.as_millis() as u64,
            records_in,
            records_out,
            dds,
        });
        Ok(())
    }

    fn allocate(&mut self, step: &Step) -> Result<Vec<Alloc>, String> {
        let mut allocs: Vec<Alloc> = Vec::new();
        for dd in &step.dds {
            let result = match &dd.kind {
                DdKind::Sysout { .. } => {
                    let path = self.register_spool(&step.name, &dd.name);
                    fs::File::create(&path).map_err(|e| e.to_string())?;
                    let display = format!("{}.{}.{}", self.job.name, self.id, dd.name);
                    Ok(Alloc::simple(&dd.name, path, Target::Sysout, &display))
                }
                DdKind::Dummy => Ok(Alloc::simple(
                    &dd.name,
                    PathBuf::from("/dev/null"),
                    Target::Dummy,
                    "NULLFILE",
                )),
                DdKind::Instream { data } => {
                    let path = self.temp.join(format!("{}.{}", step.name, dd.name));
                    let mut text = data.join("\n");
                    text.push('\n');
                    fs::write(&path, text).map_err(|e| e.to_string())?;
                    let mut a = Alloc::simple(&dd.name, path, Target::Instream, "IN-STREAM");
                    a.normal = DispAction::Delete;
                    a.abnormal = DispAction::Delete;
                    Ok(a)
                }
                DdKind::Dataset { dsn, disp } => self.allocate_dataset(step, dd, dsn, disp),
            };
            match result {
                Ok(a) => allocs.push(a),
                Err(msg) => {
                    // Undo anything this step created before the failure.
                    for a in allocs.iter().filter(|a| a.new) {
                        fs::remove_file(&a.path).ok();
                    }
                    return Err(msg);
                }
            }
        }
        Ok(allocs)
    }

    fn allocate_dataset(
        &mut self,
        step: &Step,
        dd: &Dd,
        dsn: &str,
        disp: &Disp,
    ) -> Result<Alloc, String> {
        let job = &self.job.name;
        let not_found = || {
            format!(
                "IEF212I {job} {} {} - DATA SET NOT FOUND",
                step.name, dd.name
            )
        };

        // Temporary datasets (&&NAME) live in the job's temp directory.
        if let Some(key) = dsn.strip_prefix("&&") {
            let existing = self.passed.get(key);
            let new = match (disp.status, existing) {
                (DispStatus::New, Some(_)) => {
                    return Err(format!(
                        "IEF344I {job} {} {} - DUPLICATE TEMPORARY DATA SET &&{key}",
                        step.name, dd.name
                    ));
                }
                (DispStatus::New, None) | (DispStatus::Mod, None) => true,
                (_, Some(_)) => false,
                (_, None) => return Err(not_found()),
            };
            let (normal, abnormal) = dispositions(disp, new);
            let (path, recfm, lrecl) = match existing {
                Some(p) => (p.path.clone(), p.recfm.clone(), p.lrecl),
                None => {
                    let path = self.temp.join(key);
                    fs::File::create(&path).map_err(|e| e.to_string())?;
                    (path, dd.recfm.clone(), dd.lrecl)
                }
            };
            return Ok(Alloc {
                ddname: dd.name.clone(),
                path,
                target: Target::Temp(key.to_string()),
                display: self.temp_display(key),
                new,
                output: new || disp.status == DispStatus::Mod,
                normal,
                abnormal,
                recfm,
                lrecl,
            });
        }

        // Relative GDG generation: resolve against this job's snapshot.
        let mut gdg = None;
        let mut name = dsn.to_string();
        if let Some((base, rel)) = parse_relative(dsn) {
            let cat = self.sys.catalog().map_err(|e| e.to_string())?;
            let generations = match self.snapshot.get(&base) {
                Some(g) => g.clone(),
                None => {
                    let g = cat
                        .gdgs
                        .get(&base)
                        .ok_or_else(|| {
                            format!(
                                "IGD07001I GDG BASE {base} NOT DEFINED FOR {} {}",
                                step.name, dd.name
                            )
                        })?
                        .generations
                        .clone();
                    self.snapshot.insert(base.clone(), g.clone());
                    g
                }
            };
            let generation = Catalog::resolve_relative(&generations, rel).ok_or_else(not_found)?;
            if rel <= 0 && disp.status == DispStatus::New {
                return Err(format!(
                    "IEF286I {job} {} {} - DISP=NEW INVALID FOR EXISTING GENERATION {dsn}",
                    step.name, dd.name
                ));
            }
            name = Catalog::generation_name(&base, generation);
            gdg = Some((base, generation));
        }

        // A named dataset passed from an earlier step.
        if disp.status != DispStatus::New
            && let Some(p) = self.passed.get(&name)
        {
            let (normal, abnormal) = dispositions(disp, false);
            return Ok(Alloc {
                ddname: dd.name.clone(),
                path: p.path.clone(),
                target: Target::PassedNamed(name.clone()),
                display: name,
                new: false,
                output: disp.status == DispStatus::Mod,
                normal,
                abnormal,
                recfm: p.recfm.clone(),
                lrecl: p.lrecl,
            });
        }

        let cat = self.sys.catalog().map_err(|e| e.to_string())?;
        let path = self.sys.dataset_path(&name);
        let exists = cat.datasets.contains_key(&name) && path.exists();
        let new = match disp.status {
            DispStatus::New => {
                if cat.datasets.contains_key(&name) {
                    return Err(format!(
                        "IGD17101I DATA SET {name} NOT DEFINED BECAUSE DUPLICATE NAME EXISTS IN CATALOG"
                    ));
                }
                true
            }
            DispStatus::Mod => !exists,
            DispStatus::Old | DispStatus::Shr => {
                if !exists {
                    return Err(not_found());
                }
                false
            }
        };
        if new {
            if let Some((base, _)) = &gdg
                && !cat.gdgs.contains_key(base)
            {
                return Err(format!("IGD07001I GDG BASE {base} NOT DEFINED"));
            }
            fs::File::create(&path).map_err(|e| e.to_string())?;
        }
        let entry = cat.datasets.get(&name);
        let (normal, abnormal) = dispositions(disp, new);
        Ok(Alloc {
            ddname: dd.name.clone(),
            path,
            target: Target::Cataloged {
                dsn: name.clone(),
                gdg,
            },
            display: name,
            new,
            output: new || disp.status == DispStatus::Mod,
            normal,
            abnormal,
            recfm: dd.recfm.clone().or_else(|| entry.map(|e| e.recfm.clone())),
            lrecl: dd.lrecl.or_else(|| entry.and_then(|e| e.lrecl)),
        })
    }

    fn execute(&mut self, step: &Step, allocs: &mut [Alloc]) -> Result<Outcome> {
        let pgm = step.pgm.to_uppercase();
        let mut ctx = utilities::UtilCtx {
            sys: self.sys,
            allocs: &mut *allocs,
        };
        if let Some(outcome) = utilities::run(&pgm, &mut ctx)? {
            return Ok(outcome);
        }

        let exe = self.sys.loadlib().join(&pgm);
        if !exe.is_file() {
            self.log(format!("CSV003I REQUESTED MODULE {pgm} NOT FOUND"));
            return Ok(Outcome::Abend {
                code: "S806".into(),
                reason: "00000004".into(),
                source: None,
            });
        }

        // DISPLAY output goes to the SYSOUT DD, as with Enterprise COBOL.
        let stdout_path = match allocs.iter().find(|a| a.ddname == "SYSOUT") {
            Some(a) => a.path.clone(),
            None => {
                let p = self.register_spool(&step.name, "SYSOUT");
                fs::File::create(&p)?;
                p
            }
        };
        let stdout = fs::OpenOptions::new().append(true).open(&stdout_path)?;
        let stdin = match allocs.iter().find(|a| a.ddname == "SYSIN") {
            Some(a) => Stdio::from(fs::File::open(&a.path)?),
            None => Stdio::null(),
        };
        let stderr_path = self.temp.join(format!("{}.stderr", step.name));
        let stderr = fs::File::create(&stderr_path)?;

        let mut cmd = Command::new(&exe);
        if let Some(parm) = &step.parm {
            cmd.arg(parm);
        }
        for a in allocs.iter() {
            // GnuCOBOL resolves ASSIGN TO <name> through DD_<name>.
            cmd.env(format!("DD_{}", a.ddname), &a.path);
        }
        let status = cmd
            .stdin(stdin)
            .stdout(stdout)
            .stderr(stderr)
            .status()
            .with_context(|| format!("starting {}", exe.display()))?;

        let raw = fs::read_to_string(&stderr_path).unwrap_or_default();
        let stderr_text: String = raw
            .lines()
            .filter(|l| !l.starts_with("Warning: program compiled against libxml"))
            .map(|l| format!("{l}\n"))
            .collect();

        let outcome = if let Some(sig) = status.signal() {
            let (code, reason) = match sig {
                libc::SIGSEGV | libc::SIGBUS => ("S0C4", "00000011"),
                libc::SIGFPE => ("S0CB", "0000000B"),
                libc::SIGKILL | libc::SIGTERM => ("S222", "00000000"),
                libc::SIGXCPU => ("S322", "00000000"),
                _ => ("U4088", "00000000"),
            };
            Outcome::Abend {
                code: code.into(),
                reason: reason.into(),
                source: None,
            }
        } else {
            let rc = status.code().unwrap_or(0) as u32;
            if rc != 0 && stderr_text.contains("libcob: ") {
                classify_libcob(&stderr_text)
            } else {
                Outcome::Rc(rc)
            }
        };

        if !stderr_text.trim().is_empty() {
            let dd = if matches!(outcome, Outcome::Abend { .. }) {
                "CEEDUMP"
            } else {
                "SYSERR"
            };
            let p = self.register_spool(&step.name, dd);
            fs::write(p, &stderr_text)?;
        }
        Ok(outcome)
    }

    fn dispose(&mut self, allocs: &mut [Alloc], abended: bool) -> Result<Vec<DdRecord>> {
        // Reload: utilities such as IDCAMS may have changed the catalog.
        let mut cat = self.sys.catalog()?;
        let mut records = Vec::new();
        let created_by = self.id.clone();

        for a in allocs.iter() {
            let bytes = if matches!(a.target, Target::Dummy) {
                0
            } else {
                fs::metadata(&a.path).map(|m| m.len()).unwrap_or(0)
            };
            let count = a.record_count(bytes);
            let action = if abended { a.abnormal } else { a.normal };
            let mut dsn = None;
            let disposition = match &a.target {
                Target::Sysout => "SYSOUT".to_string(),
                Target::Dummy => "DUMMY".to_string(),
                Target::Instream => "IN-STREAM".to_string(),
                Target::Temp(key) | Target::PassedNamed(key) => {
                    dsn = Some(a.display.clone());
                    match action {
                        DispAction::Delete | DispAction::Uncatlg => {
                            fs::remove_file(&a.path).ok();
                            self.passed.remove(key);
                            "DELETED".to_string()
                        }
                        DispAction::Catlg if matches!(a.target, Target::PassedNamed(_)) => {
                            self.passed.remove(key);
                            cat.datasets.insert(key.clone(), entry_for(a, &created_by));
                            "CATALOGED".to_string()
                        }
                        _ => {
                            self.passed.insert(
                                key.clone(),
                                Passed {
                                    path: a.path.clone(),
                                    display: a.display.clone(),
                                    recfm: a.recfm.clone(),
                                    lrecl: a.lrecl,
                                },
                            );
                            "PASSED".to_string()
                        }
                    }
                }
                Target::Cataloged { dsn: name, gdg } => {
                    dsn = Some(name.clone());
                    if a.new {
                        match action {
                            DispAction::Catlg | DispAction::Keep => {
                                cat.datasets.insert(name.clone(), entry_for(a, &created_by));
                                if let Some((base, generation)) = gdg {
                                    for r in cat.add_generation(base, *generation) {
                                        if r.scratched {
                                            fs::remove_file(self.sys.dataset_path(&r.dsn)).ok();
                                        }
                                        let what = if r.scratched {
                                            "DELETED (ROLLED OFF GDG)"
                                        } else {
                                            "UNCATALOGED (ROLLED OFF GDG)"
                                        };
                                        self.sysmsg.push(format!("IEF285I   {:<44} {what}", r.dsn));
                                    }
                                }
                                if action == DispAction::Catlg {
                                    "CATALOGED"
                                } else {
                                    "KEPT"
                                }
                                .to_string()
                            }
                            DispAction::Pass => {
                                self.passed.insert(
                                    name.clone(),
                                    Passed {
                                        path: a.path.clone(),
                                        display: name.clone(),
                                        recfm: a.recfm.clone(),
                                        lrecl: a.lrecl,
                                    },
                                );
                                "PASSED".to_string()
                            }
                            DispAction::Delete | DispAction::Uncatlg => {
                                fs::remove_file(&a.path).ok();
                                "DELETED".to_string()
                            }
                        }
                    } else {
                        match action {
                            DispAction::Delete => {
                                fs::remove_file(&a.path).ok();
                                cat.uncatalog(name);
                                "DELETED".to_string()
                            }
                            DispAction::Uncatlg => {
                                cat.uncatalog(name);
                                "UNCATALOGED".to_string()
                            }
                            _ => "KEPT".to_string(),
                        }
                    }
                }
            };
            if a.instream_or_sysout() {
                if matches!(a.target, Target::Instream) {
                    fs::remove_file(&a.path).ok();
                } else {
                    self.sysmsg
                        .push(format!("IEF285I   {:<44} SYSOUT", a.display));
                }
            } else if !matches!(a.target, Target::Dummy) {
                self.sysmsg
                    .push(format!("IEF285I   {:<44} {disposition}", a.display));
            }
            let direction = match a.target {
                Target::Sysout => "sysout",
                Target::Dummy => "dummy",
                Target::Instream => "instream",
                _ if a.output => "out",
                _ => "in",
            };
            records.push(DdRecord {
                ddname: a.ddname.clone(),
                dsn,
                direction: direction.into(),
                disposition,
                records: count,
                bytes,
            });
        }
        self.sys.save_catalog(&cat)?;
        Ok(records)
    }

    fn finish(&mut self) -> Result<JobRecord> {
        let name = self.job.name.clone();
        let status = if let Some(msg) = &self.jcl_error {
            JobStatus::JclError {
                message: msg.clone(),
            }
        } else if let Some((code, step)) = &self.abend {
            JobStatus::Abend {
                code: code.clone(),
                step: step.clone(),
            }
        } else {
            let rc = self
                .steps
                .iter()
                .filter_map(|s| match s.outcome {
                    StepOutcome::Executed { rc } => Some(rc),
                    _ => None,
                })
                .max()
                .unwrap_or(0);
            JobStatus::Cc { rc }
        };

        // Step summary table (the SMF-style block in the JES2 job log).
        self.log(format!(
            "-{:<8} {:<8} {:<8} {:>6} {:>10} {:>10} {:>9} {:>9}",
            "JOBNAME", "STEPNAME", "PROGRAM", "RC", "RECS-IN", "RECS-OUT", "CPU(S)", "CLOCK(S)"
        ));
        let rows: Vec<String> = self
            .steps
            .iter()
            .map(|s| {
                let rc = match &s.outcome {
                    StepOutcome::Executed { rc } => format!("{rc:02}"),
                    StepOutcome::Abended { code, .. } => code.clone(),
                    StepOutcome::NotRun { reason } if reason.starts_with("FLUSHED") => {
                        "FLUSH".into()
                    }
                    StepOutcome::NotRun { .. } => "*NR*".into(),
                };
                format!(
                    "-{:<8} {:<8} {:<8} {:>6} {:>10} {:>10} {:>9.2} {:>9.2}",
                    name,
                    s.name,
                    s.program,
                    rc,
                    s.records_in,
                    s.records_out,
                    s.cpu_ms as f64 / 1000.0,
                    s.elapsed_ms as f64 / 1000.0
                )
            })
            .collect();
        for r in rows {
            self.log(r);
        }

        let elapsed = self.t0.elapsed();
        let cpu = cpu_time().saturating_sub(self.cpu0);
        self.log(format!(
            "IEF404I {name} - ENDED - TIME={}",
            Local::now().format("%H.%M.%S")
        ));
        let hasp = match &status {
            JobStatus::Cc { rc } => format!("RC={rc:04}"),
            JobStatus::Abend { code, .. } => format!("ABEND={code}"),
            JobStatus::JclError { .. } => "JCL ERROR".into(),
        };
        self.log(format!("$HASP395 {name:<8} ENDED - {hasp}"));

        self.sysmsg.push(format!(
            "IEF375I  JOB/{name:<8}/START {}",
            self.submitted.format("%Y%j.%H%M")
        ));
        self.sysmsg.push(format!(
            "IEF033I  JOB/{name:<8}/STOP  {}",
            Local::now().format("%Y%j.%H%M")
        ));
        self.sysmsg.push(format!(
            "        CPU: {}    SRB: {}",
            fmt_cpu(cpu),
            fmt_cpu(Duration::ZERO)
        ));

        let banner = vec![
            "                    J E S 2  J O B  L O G  --  S Y S T E M  F R N T  --  N O D E  F R O N T I E R".to_string(),
            String::new(),
            format!(
                "{} {}  ---- {} ----",
                self.submitted.format("%H.%M.%S"),
                self.id,
                self.submitted.format("%A, %d %b %Y").to_string().to_uppercase()
            ),
        ];
        let msglg: Vec<String> = banner.into_iter().chain(self.msglg.drain(..)).collect();
        let jes_files = [
            ("JESMSGLG".to_string(), msglg),
            ("JESJCL".to_string(), self.job.listing.clone()),
            ("JESYSMSG".to_string(), std::mem::take(&mut self.sysmsg)),
        ];

        let mut spool = Vec::new();
        for (dd, lines) in jes_files {
            let mut f = fs::File::create(self.dir.join(&dd))?;
            for l in &lines {
                writeln!(f, "{l}")?;
            }
            let bytes = fs::metadata(self.dir.join(&dd))?.len();
            spool.push(SpoolFile {
                ddname: dd,
                step: None,
                lines: lines.len() as u64,
                bytes,
            });
        }
        for (dd, step) in &self.spool_files {
            let data = fs::read(self.dir.join(dd)).unwrap_or_default();
            spool.push(SpoolFile {
                ddname: dd.clone(),
                step: step.clone(),
                lines: count_lines(&data),
                bytes: data.len() as u64,
            });
        }

        let record = JobRecord {
            id: self.id.clone(),
            name,
            class: self.job.class.clone(),
            submitted: self
                .submitted
                .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
                .to_string(),
            ended: now_iso(),
            status,
            cpu_ms: cpu.as_millis() as u64,
            elapsed_ms: elapsed.as_millis() as u64,
            symbols: self.job.symbols.clone(),
            steps: std::mem::take(&mut self.steps),
            spool,
        };
        fs::write(
            self.dir.join("job.json"),
            serde_json::to_string_pretty(&record)?,
        )?;
        Ok(record)
    }
}

impl Alloc {
    fn instream_or_sysout(&self) -> bool {
        matches!(self.target, Target::Sysout | Target::Instream)
    }
}

fn entry_for(a: &Alloc, created_by: &str) -> DatasetEntry {
    DatasetEntry {
        recfm: a.recfm.clone().unwrap_or_else(|| "U".into()),
        lrecl: a.lrecl,
        created: now_iso(),
        created_by: created_by.to_string(),
    }
}

/// Map a GnuCOBOL runtime error to the abend a z/OS programmer expects.
pub(crate) fn classify_libcob(stderr: &str) -> Outcome {
    let source = stderr
        .split_whitespace()
        .find(|w| w.contains(".cbl:") || w.contains(".CBL:"))
        .map(|w| w.trim_end_matches(':').to_string());
    let lower = stderr.to_lowercase();
    let file_status = lower
        .find("status = ")
        .map(|i| lower[i + 9..].chars().take(2).collect::<String>());
    let (code, reason) = if lower.contains("not numeric") {
        ("S0C7", "00000007") // data exception
    } else if lower.contains("subscript")
        || lower.contains("out of bounds")
        || lower.contains("offset")
    {
        ("S0C4", "00000004") // protection exception
    } else if lower.contains("divi") && lower.contains("zero") {
        ("S0CB", "0000000B") // decimal divide exception
    } else if let Some(fs) = file_status.as_deref() {
        match fs {
            "35" => ("S013", "00000018"), // open failed: dataset not found
            "34" => ("SB37", "00000004"), // out of space
            "39" => ("S013", "00000020"), // DCB conflict
            _ => ("S001", "00000004"),    // I/O error
        }
    } else {
        ("U4038", "00000000") // LE: unhandled condition
    };
    Outcome::Abend {
        code: code.into(),
        reason: reason.into(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_libcob_errors() {
        let s0c7 = "libcob: TRNVALID.cbl:32: error: 'WS-BAD' (Type: PACKED-DECIMAL) not numeric: '0x5a5a5a5a'";
        assert_eq!(
            classify_libcob(s0c7),
            Outcome::Abend {
                code: "S0C7".into(),
                reason: "00000007".into(),
                source: Some("TRNVALID.cbl:32".into())
            }
        );
        let s013 = "libcob: error: file not found (status = 35) for file ACCT-IN";
        assert!(matches!(classify_libcob(s013), Outcome::Abend { code, .. } if code == "S013"));
        assert!(
            matches!(classify_libcob("libcob: error: something odd"), Outcome::Abend { code, .. } if code == "U4038")
        );
    }

    #[test]
    fn default_dispositions() {
        let d = |status, normal, abnormal| Disp {
            status,
            normal,
            abnormal,
        };
        assert_eq!(
            dispositions(&d(DispStatus::New, None, None), true),
            (DispAction::Delete, DispAction::Delete)
        );
        assert_eq!(
            dispositions(&d(DispStatus::Shr, None, None), false),
            (DispAction::Keep, DispAction::Keep)
        );
        assert_eq!(
            dispositions(&d(DispStatus::New, Some(DispAction::Pass), None), true),
            (DispAction::Pass, DispAction::Delete)
        );
        assert_eq!(
            dispositions(
                &d(
                    DispStatus::New,
                    Some(DispAction::Catlg),
                    Some(DispAction::Delete)
                ),
                true
            ),
            (DispAction::Catlg, DispAction::Delete)
        );
    }
}
