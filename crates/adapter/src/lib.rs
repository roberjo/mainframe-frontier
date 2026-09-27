//! The `MainframeAdapter` contract.
//!
//! Everything above this layer (CLI, API, Control Room, MCP server) talks to
//! "a mainframe" only through this trait, so the same tooling works whether
//! jobs run on the local GnuCOBOL substrate, an emulated MVS (Hercules), or
//! real z/OS through z/OSMF. Only the local backend exists today.

use std::collections::BTreeMap;

use anyhow::Result;
use frontier_jcl::System;
pub use frontier_jcl::{DatasetEntry, DatasetInfo, JobRecord, JobStatus, StepOutcome};

pub trait MainframeAdapter {
    /// Short backend name: `local`, `hercules`, `zos`.
    fn substrate(&self) -> &'static str;

    /// Submit JCL. `symbols` override `SET` statements in the job.
    /// The local backend runs the job to completion before returning.
    fn submit(&self, jcl: &str, symbols: &BTreeMap<String, String>) -> Result<JobRecord>;

    fn jobs(&self) -> Result<Vec<JobRecord>>;

    fn job(&self, id: &str) -> Result<JobRecord>;

    /// One spool file: `JESMSGLG`, `JESJCL`, `JESYSMSG`, or `STEP.DDNAME`.
    fn spool(&self, id: &str, ddname: &str) -> Result<String>;

    /// Cataloged datasets, optionally filtered by a z/OS-style pattern (`FFB.**`).
    fn datasets(&self, pattern: Option<&str>) -> Result<Vec<DatasetInfo>>;

    /// Raw dataset contents. Accepts relative GDG names such as `FFB.ACCTMAST(0)`.
    fn read_dataset(&self, dsn: &str) -> Result<(DatasetEntry, Vec<u8>)>;
}

/// GnuCOBOL + JCL-lite running against a local directory.
pub struct LocalAdapter {
    system: System,
}

impl LocalAdapter {
    pub fn new(system: System) -> Self {
        Self { system }
    }

    pub fn from_env() -> Result<Self> {
        Ok(Self::new(System::from_env()?))
    }

    pub fn system(&self) -> &System {
        &self.system
    }
}

impl MainframeAdapter for LocalAdapter {
    fn substrate(&self) -> &'static str {
        "local"
    }

    fn submit(&self, jcl: &str, symbols: &BTreeMap<String, String>) -> Result<JobRecord> {
        self.system.submit(jcl, symbols)
    }

    fn jobs(&self) -> Result<Vec<JobRecord>> {
        self.system.jobs()
    }

    fn job(&self, id: &str) -> Result<JobRecord> {
        self.system.job(id)
    }

    fn spool(&self, id: &str, ddname: &str) -> Result<String> {
        self.system.spool(id, ddname)
    }

    fn datasets(&self, pattern: Option<&str>) -> Result<Vec<DatasetInfo>> {
        self.system.datasets(pattern)
    }

    fn read_dataset(&self, dsn: &str) -> Result<(DatasetEntry, Vec<u8>)> {
        self.system.read_dataset(dsn)
    }
}
