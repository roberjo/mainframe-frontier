//! The local "system": a directory tree that plays the part of DASD,
//! the catalog, the JES spool and the load library.
//!
//! ```text
//! $FRONTIER_HOME/
//!   catalog.json        dataset + GDG catalog
//!   jes.json            next job number
//!   datasets/<DSN>      one file per dataset
//!   loadlib/<PGM>       compiled COBOL programs
//!   spool/JOBnnnnn/     job output: job.json, JESMSGLG, JESJCL, JESYSMSG, step SYSOUTs
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::catalog::{Catalog, DatasetEntry, parse_relative};
use crate::jes::JobRecord;

pub struct System {
    home: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct DatasetInfo {
    pub dsn: String,
    pub entry: DatasetEntry,
    pub bytes: u64,
    pub records: Option<u64>,
    /// GDG base this dataset is a generation of.
    pub gdg: Option<String>,
}

#[derive(Serialize, Deserialize, Default)]
struct JesState {
    next_job: u32,
}

pub(crate) fn now_iso() -> String {
    chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
        .to_string()
}

impl System {
    pub fn open(home: impl Into<PathBuf>) -> Result<Self> {
        let home = home.into();
        for dir in ["datasets", "loadlib", "spool"] {
            std::fs::create_dir_all(home.join(dir))
                .with_context(|| format!("creating {}", home.join(dir).display()))?;
        }
        Ok(Self { home })
    }

    /// `$FRONTIER_HOME`, or `./var`.
    pub fn from_env() -> Result<Self> {
        Self::open(
            std::env::var_os("FRONTIER_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("var")),
        )
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn loadlib(&self) -> PathBuf {
        self.home.join("loadlib")
    }

    pub fn spool_root(&self) -> PathBuf {
        self.home.join("spool")
    }

    fn catalog_path(&self) -> PathBuf {
        self.home.join("catalog.json")
    }

    pub fn catalog(&self) -> Result<Catalog> {
        Catalog::load(&self.catalog_path())
    }

    pub fn save_catalog(&self, catalog: &Catalog) -> Result<()> {
        catalog.save(&self.catalog_path())
    }

    pub fn dataset_path(&self, dsn: &str) -> PathBuf {
        self.home.join("datasets").join(dsn)
    }

    /// Allocate the next job number (JOB00001...).
    pub(crate) fn next_job_id(&self) -> Result<String> {
        let path = self.home.join("jes.json");
        let mut state: JesState = match std::fs::read_to_string(&path) {
            Ok(t) => serde_json::from_str(&t)?,
            Err(_) => JesState { next_job: 1 },
        };
        let id = format!("JOB{:05}", state.next_job);
        state.next_job = if state.next_job >= 99_999 {
            1
        } else {
            state.next_job + 1
        };
        std::fs::write(&path, serde_json::to_string(&state)?)?;
        Ok(id)
    }

    /// Resolve `BASE(0)` / `BASE(-1)` to an absolute generation name using
    /// the current catalog. Plain names pass through.
    pub fn resolve_dsn(&self, dsn: &str) -> Result<String> {
        let Some((base, rel)) = parse_relative(dsn) else {
            return Ok(dsn.to_string());
        };
        let cat = self.catalog()?;
        let Some(gdg) = cat.gdgs.get(&base) else {
            bail!("GDG base {base} is not defined");
        };
        match Catalog::resolve_relative(&gdg.generations, rel) {
            Some(g) if rel <= 0 => Ok(Catalog::generation_name(&base, g)),
            _ => bail!("generation {dsn} does not exist"),
        }
    }

    /// Create or replace a cataloged dataset (used by tools such as the seeder).
    pub fn write_dataset(
        &self,
        dsn: &str,
        recfm: &str,
        lrecl: Option<u32>,
        data: &[u8],
        created_by: &str,
    ) -> Result<()> {
        if let Some(l) = lrecl
            && recfm.starts_with('F')
            && !data.len().is_multiple_of(l as usize)
        {
            bail!("{dsn}: {} bytes is not a multiple of LRECL {l}", data.len());
        }
        std::fs::write(self.dataset_path(dsn), data)?;
        let mut cat = self.catalog()?;
        cat.datasets.insert(
            dsn.to_string(),
            DatasetEntry {
                recfm: recfm.to_string(),
                lrecl,
                created: now_iso(),
                created_by: created_by.to_string(),
            },
        );
        self.save_catalog(&cat)
    }

    pub fn read_dataset(&self, dsn: &str) -> Result<(DatasetEntry, Vec<u8>)> {
        let dsn = self.resolve_dsn(dsn)?;
        let cat = self.catalog()?;
        let entry = cat
            .datasets
            .get(&dsn)
            .cloned()
            .with_context(|| format!("dataset {dsn} not cataloged"))?;
        let data =
            std::fs::read(self.dataset_path(&dsn)).with_context(|| format!("reading {dsn}"))?;
        Ok((entry, data))
    }

    pub fn datasets(&self, pattern: Option<&str>) -> Result<Vec<DatasetInfo>> {
        let cat = self.catalog()?;
        let mut out = Vec::new();
        for (dsn, entry) in &cat.datasets {
            if pattern.is_some_and(|p| !dsn_matches(p, dsn)) {
                continue;
            }
            let bytes = std::fs::metadata(self.dataset_path(dsn))
                .map(|m| m.len())
                .unwrap_or(0);
            let records = match entry.lrecl {
                Some(l) if entry.is_fixed() && l > 0 => Some(bytes / l as u64),
                _ => None,
            };
            let gdg = crate::catalog::parse_generation_name(dsn)
                .map(|(b, _)| b)
                .filter(|b| cat.gdgs.contains_key(b));
            out.push(DatasetInfo {
                dsn: dsn.clone(),
                entry: entry.clone(),
                bytes,
                records,
                gdg,
            });
        }
        Ok(out)
    }

    pub fn jobs(&self) -> Result<Vec<JobRecord>> {
        let mut jobs = Vec::new();
        for entry in std::fs::read_dir(self.spool_root())? {
            let path = entry?.path().join("job.json");
            if let Ok(text) = std::fs::read_to_string(&path) {
                jobs.push(
                    serde_json::from_str::<JobRecord>(&text)
                        .with_context(|| format!("parsing {}", path.display()))?,
                );
            }
        }
        jobs.sort_by(|a, b| a.submitted.cmp(&b.submitted).then(a.id.cmp(&b.id)));
        Ok(jobs)
    }

    /// Accepts `JOB00042`, `J42` or `42`.
    pub fn normalize_job_id(id: &str) -> String {
        let digits: String = id.chars().filter(|c| c.is_ascii_digit()).collect();
        match digits.parse::<u32>() {
            Ok(n) => format!("JOB{n:05}"),
            Err(_) => id.to_uppercase(),
        }
    }

    pub fn job(&self, id: &str) -> Result<JobRecord> {
        let id = Self::normalize_job_id(id);
        let path = self.spool_root().join(&id).join("job.json");
        let text = std::fs::read_to_string(&path).with_context(|| format!("job {id} not found"))?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Read one spool file (`JESMSGLG`, `STEPNAME.DDNAME`, ...).
    pub fn spool(&self, id: &str, ddname: &str) -> Result<String> {
        let job = self.job(id)?;
        let want = ddname.to_uppercase();
        let file = job
            .spool
            .iter()
            .find(|f| f.ddname == want)
            .with_context(|| format!("{} has no spool file {want}", job.id))?;
        Ok(std::fs::read_to_string(
            self.spool_root().join(&job.id).join(&file.ddname),
        )?)
    }
}

/// z/OS-style dataset name pattern: `*` matches within a qualifier,
/// `**` across qualifiers, `%` one character. A bare prefix matches
/// everything below it (`FFB` ≈ `FFB.**`).
pub fn dsn_matches(pattern: &str, dsn: &str) -> bool {
    let pattern = pattern.to_uppercase();
    if !pattern.contains(['*', '%']) {
        return dsn == pattern || dsn.starts_with(&format!("{pattern}."));
    }
    fn go(p: &[u8], s: &[u8]) -> bool {
        match (p.first(), s.first()) {
            (None, None) => true,
            (Some(b'*'), _) if p.get(1) == Some(&b'*') => {
                let rest = &p[2..];
                let rest = rest
                    .strip_prefix(b".")
                    .filter(|_| p.len() > 2)
                    .unwrap_or(rest);
                (0..=s.len()).any(|i| go(rest, &s[i..]))
            }
            (Some(b'*'), _) => (0..=s.len())
                .take_while(|&i| i == 0 || s[i - 1] != b'.')
                .any(|i| go(&p[1..], &s[i..])),
            (Some(b'%'), Some(c)) if *c != b'.' => go(&p[1..], &s[1..]),
            (Some(a), Some(b)) if a == b => go(&p[1..], &s[1..]),
            _ => false,
        }
    }
    go(pattern.as_bytes(), dsn.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::dsn_matches;

    #[test]
    fn patterns() {
        assert!(dsn_matches("FFB", "FFB.ACCTMAST.G0001V00"));
        assert!(!dsn_matches("FFB", "FFBX.A"));
        assert!(dsn_matches("FFB.*", "FFB.ACCTMAST"));
        assert!(!dsn_matches("FFB.*", "FFB.ACCTMAST.G0001V00"));
        assert!(dsn_matches("FFB.**", "FFB.ACCTMAST.G0001V00"));
        assert!(dsn_matches("FFB.DAILY.*", "FFB.DAILY.TRANFEED"));
        assert!(dsn_matches("FFB.%AILY.*", "FFB.DAILY.TRANFEED"));
        assert!(dsn_matches("ffb.**.G0001V00", "FFB.ACCTMAST.G0001V00"));
    }
}
