//! Dataset → copybook registry (`cobol/datasets.toml`).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use frontier_copybook::{Copybook, Item, ParseOptions, parse};
use frontier_jcl::system::dsn_matches;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct LayoutEntry {
    pub pattern: String,
    pub copybook: String,
    pub record: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RegistryFile {
    #[serde(default)]
    dataset: Vec<LayoutEntry>,
}

/// A parsed copybook plus the record chosen for a dataset.
#[derive(Debug, Clone)]
pub struct Layout {
    pub copybook: Copybook,
    record_index: usize,
}

impl Layout {
    pub fn record(&self) -> &Item {
        &self.copybook.records[self.record_index]
    }

    pub fn name(&self) -> &str {
        &self.copybook.name
    }
}

#[derive(Debug, Clone)]
pub struct LayoutRegistry {
    copybook_dir: PathBuf,
    entries: Vec<LayoutEntry>,
}

impl LayoutRegistry {
    /// Load `registry` (may be missing: then nothing is mapped) with copybooks under `copybook_dir`.
    pub fn load(registry: &Path, copybook_dir: &Path) -> Result<Self> {
        let entries = match std::fs::read_to_string(registry) {
            Ok(text) => {
                toml::from_str::<RegistryFile>(&text)
                    .with_context(|| format!("parsing {}", registry.display()))?
                    .dataset
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", registry.display())),
        };
        Ok(Self {
            copybook_dir: copybook_dir.to_path_buf(),
            entries,
        })
    }

    pub fn entries(&self) -> &[LayoutEntry] {
        &self.entries
    }

    /// The registered layout for a dataset (first matching pattern wins).
    pub fn for_dsn(&self, dsn: &str) -> Result<Option<Layout>> {
        match self.entries.iter().find(|e| dsn_matches(&e.pattern, dsn)) {
            Some(e) => self.load_layout(&e.copybook, e.record.as_deref()).map(Some),
            None => Ok(None),
        }
    }

    /// Load a copybook by name (`ACCTREC`) or by path, choosing a record.
    pub fn load_layout(&self, copybook: &str, record: Option<&str>) -> Result<Layout> {
        let path = if copybook.contains('/') || copybook.contains('.') {
            PathBuf::from(copybook)
        } else {
            self.copybook_dir
                .join(format!("{}.cpy", copybook.to_uppercase()))
        };
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_uppercase())
            .unwrap_or_default();
        let source = std::fs::read_to_string(&path)
            .with_context(|| format!("reading copybook {}", path.display()))?;
        let copybook = parse(&name, &source, &ParseOptions::default())
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let record_index = match record {
            Some(r) => match copybook
                .records
                .iter()
                .position(|i| i.name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(r)))
            {
                Some(i) => i,
                None => bail!("copybook {name} has no record {r}"),
            },
            None => 0,
        };
        Ok(Layout {
            copybook,
            record_index,
        })
    }
}
