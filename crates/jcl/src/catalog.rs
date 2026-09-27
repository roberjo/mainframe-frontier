//! The dataset catalog: cataloged datasets and generation data groups.
//!
//! Each dataset is one file under `$FRONTIER_HOME/datasets/<DSN>`. Record
//! format and length live here, because a fixed-block file on disk is just
//! bytes (exactly like a real FB dataset without its VTOC entry).

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatasetEntry {
    /// FB, VB, FBA, LS (line sequential: a text file with newline-delimited records)...
    pub recfm: String,
    pub lrecl: Option<u32>,
    pub created: String,
    /// Job ID (or tool) that created the dataset.
    pub created_by: String,
}

impl DatasetEntry {
    pub fn is_fixed(&self) -> bool {
        self.recfm.starts_with('F')
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GdgBase {
    pub limit: u32,
    /// Delete rolled-off generations (SCRATCH) or just uncatalog them.
    pub scratch: bool,
    /// Roll off every generation when the limit is hit (EMPTY) or just the oldest.
    pub empty: bool,
    /// Absolute generation numbers, oldest first.
    pub generations: Vec<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub datasets: BTreeMap<String, DatasetEntry>,
    #[serde(default)]
    pub gdgs: BTreeMap<String, GdgBase>,
}

/// A generation that fell off a GDG when a new one was cataloged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RolledOff {
    pub dsn: String,
    pub scratched: bool,
}

impl Catalog {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn generation_name(base: &str, generation: u32) -> String {
        format!("{base}.G{generation:04}V00")
    }

    /// Resolve a relative generation against the given generation list
    /// (the job's snapshot). `(0)` is the newest, `(-1)` the one before,
    /// `(+1)` the next new generation.
    pub fn resolve_relative(generations: &[u32], relative: i32) -> Option<u32> {
        if relative > 0 {
            let last = generations.last().copied().unwrap_or(0);
            // Generation numbers wrap from 9999 back to 1, as on z/OS.
            Some((last + relative as u32 - 1) % 9999 + 1)
        } else {
            let back = (-relative) as usize;
            generations
                .len()
                .checked_sub(back + 1)
                .map(|i| generations[i])
        }
    }

    /// Catalog a new generation and roll off any beyond the limit.
    pub fn add_generation(&mut self, base: &str, generation: u32) -> Vec<RolledOff> {
        let Some(gdg) = self.gdgs.get_mut(base) else {
            return Vec::new();
        };
        if !gdg.generations.contains(&generation) {
            gdg.generations.push(generation);
        }
        let mut rolled = Vec::new();
        if gdg.generations.len() as u32 > gdg.limit {
            let drop = if gdg.empty {
                gdg.generations.len() - 1
            } else {
                gdg.generations.len() - gdg.limit as usize
            };
            for g in gdg.generations.drain(..drop) {
                rolled.push(RolledOff {
                    dsn: Self::generation_name(base, g),
                    scratched: gdg.scratch,
                });
            }
        }
        for r in &rolled {
            self.datasets.remove(&r.dsn);
        }
        rolled
    }

    /// Remove a dataset entry, and its GDG membership if it is a generation.
    pub fn uncatalog(&mut self, dsn: &str) -> bool {
        if let Some((base, generation)) = parse_generation_name(dsn)
            && let Some(gdg) = self.gdgs.get_mut(&base)
        {
            gdg.generations.retain(|g| *g != generation);
        }
        self.datasets.remove(dsn).is_some()
    }
}

/// `BASE(+1)` → (`BASE`, 1). Non-numeric suffixes (PDS members) → None.
pub fn parse_relative(dsn: &str) -> Option<(String, i32)> {
    let open = dsn.find('(')?;
    let inner = dsn[open + 1..].strip_suffix(')')?;
    let rel: i32 = inner.strip_prefix('+').unwrap_or(inner).parse().ok()?;
    Some((dsn[..open].to_string(), rel))
}

/// `BASE.G0007V00` → (`BASE`, 7).
pub fn parse_generation_name(dsn: &str) -> Option<(String, u32)> {
    let (base, last) = dsn.rsplit_once('.')?;
    let digits = last.strip_prefix('G')?.strip_suffix("V00")?;
    if digits.len() != 4 {
        return None;
    }
    Some((base.to_string(), digits.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gdg(limit: u32, empty: bool) -> Catalog {
        let mut c = Catalog::default();
        c.gdgs.insert(
            "A.B".into(),
            GdgBase {
                limit,
                scratch: true,
                empty,
                generations: vec![],
            },
        );
        c
    }

    #[test]
    fn relative_resolution() {
        let g = [3, 4, 5];
        assert_eq!(Catalog::resolve_relative(&g, 0), Some(5));
        assert_eq!(Catalog::resolve_relative(&g, -2), Some(3));
        assert_eq!(Catalog::resolve_relative(&g, -3), None);
        assert_eq!(Catalog::resolve_relative(&g, 1), Some(6));
        assert_eq!(Catalog::resolve_relative(&[], 1), Some(1));
        assert_eq!(Catalog::resolve_relative(&[], 0), None);
        assert_eq!(Catalog::resolve_relative(&[9999], 1), Some(1));
    }

    #[test]
    fn rolls_off_oldest_generation() {
        let mut c = gdg(2, false);
        for g in 1..=2 {
            c.datasets
                .insert(Catalog::generation_name("A.B", g), entry());
            assert!(c.add_generation("A.B", g).is_empty());
        }
        c.datasets
            .insert(Catalog::generation_name("A.B", 3), entry());
        let rolled = c.add_generation("A.B", 3);
        assert_eq!(
            rolled,
            vec![RolledOff {
                dsn: "A.B.G0001V00".into(),
                scratched: true
            }]
        );
        assert_eq!(c.gdgs["A.B"].generations, vec![2, 3]);
        assert!(!c.datasets.contains_key("A.B.G0001V00"));
    }

    #[test]
    fn empty_rolls_off_everything_but_newest() {
        let mut c = gdg(2, true);
        c.add_generation("A.B", 1);
        c.add_generation("A.B", 2);
        assert_eq!(c.add_generation("A.B", 3).len(), 2);
        assert_eq!(c.gdgs["A.B"].generations, vec![3]);
    }

    #[test]
    fn name_parsing() {
        assert_eq!(parse_relative("FFB.X(+1)"), Some(("FFB.X".into(), 1)));
        assert_eq!(parse_relative("FFB.X(-1)"), Some(("FFB.X".into(), -1)));
        assert_eq!(parse_relative("FFB.X(MEMBER)"), None);
        assert_eq!(
            parse_generation_name("FFB.X.G0012V00"),
            Some(("FFB.X".into(), 12))
        );
        assert_eq!(parse_generation_name("FFB.X.Y"), None);
    }

    fn entry() -> DatasetEntry {
        DatasetEntry {
            recfm: "FB".into(),
            lrecl: Some(80),
            created: String::new(),
            created_by: String::new(),
        }
    }
}
