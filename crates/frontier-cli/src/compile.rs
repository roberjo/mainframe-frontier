//! Compile COBOL source into the load library with GnuCOBOL.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use frontier_jcl::System;

/// Flags: IBM dialect for Enterprise COBOL compatibility, and `-debug` so
/// runtime checks (invalid packed data, subscripts) raise errors that the
/// executor maps to S0C7 / S0C4 abends instead of silently continuing.
const COBC_FLAGS: &[&str] = &["-x", "-std=ibm", "-debug"];

pub fn build(sys: &System, src: &Path, copybooks: &Path, only: &[String]) -> Result<bool> {
    let cobc = std::env::var("COBC").unwrap_or_else(|_| "cobc".into());
    let only: Vec<String> = only.iter().map(|p| p.to_uppercase()).collect();

    let mut sources: Vec<_> = std::fs::read_dir(src)
        .with_context(|| format!("reading {}", src.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("cbl")))
        .collect();
    sources.sort();

    let mut ok = true;
    let mut built = 0;
    for path in sources {
        let pgm = path.file_stem().unwrap().to_string_lossy().to_uppercase();
        if !only.is_empty() && !only.contains(&pgm) {
            continue;
        }
        let out = sys.loadlib().join(&pgm);
        let result = Command::new(&cobc)
            .args(COBC_FLAGS)
            .arg("-I")
            .arg(copybooks)
            .arg("-o")
            .arg(&out)
            .arg(&path)
            .output();
        let output = match result {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                bail!(
                    "`{cobc}` not found. Install GnuCOBOL 3.2 or run inside the dev container: `make shell`"
                )
            }
            Err(e) => return Err(e).context("running cobc"),
        };
        let diagnostics: Vec<String> = String::from_utf8_lossy(&output.stderr)
            .lines()
            .chain(String::from_utf8_lossy(&output.stdout).lines())
            .filter(|l| !l.starts_with("Warning: program compiled against libxml"))
            .map(str::to_string)
            .collect();
        let status = if output.status.success() {
            "OK"
        } else {
            "FAILED"
        };
        println!("  {pgm:<8} {status}");
        for d in &diagnostics {
            println!("    {d}");
        }
        if output.status.success() {
            built += 1;
        } else {
            ok = false;
        }
    }
    println!(
        "{built} program(s) compiled into {}",
        sys.loadlib().display()
    );
    Ok(ok)
}
