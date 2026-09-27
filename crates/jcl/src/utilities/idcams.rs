//! IDCAMS subset: catalog management for non-VSAM datasets and GDGs.
//!
//! ```text
//!   DEFINE GDG (NAME(base) LIMIT(n) [SCRATCH|NOSCRATCH] [EMPTY|NOEMPTY])
//!   DELETE name [GDG] [FORCE] [PURGE]
//!   LISTCAT [ENTRIES(name|pattern)] [ALL]
//!   SET MAXCC = n  /  SET LASTCC = n
//! ```
//!
//! A trailing `-` continues a command on the next line; `/* ... */` is a comment.

use anyhow::Result;

use super::UtilCtx;
use crate::catalog::{Catalog, GdgBase};
use crate::jes::Outcome;
use crate::system::dsn_matches;

pub(super) fn run(ctx: &mut UtilCtx) -> Result<Outcome> {
    let Some(text) = ctx.read_text("SYSIN")? else {
        ctx.print(
            "SYSPRINT",
            &["IDC3203I ITEM 'SYSIN' DOES NOT ADHERE TO RESTRICTIONS - NO SYSIN".into()],
        )?;
        return Ok(Outcome::Rc(12));
    };

    let mut out = vec!["IDCAMS  SYSTEM SERVICES".to_string(), String::new()];
    let mut maxcc = 0u32;
    for command in commands(&text) {
        out.push(format!("  {command}"));
        let tokens = tokenize(&command);
        let (lastcc, messages) = match tokens.first().map(String::as_str) {
            Some("DEFINE") | Some("DEF") => define(ctx, &tokens)?,
            Some("DELETE") | Some("DEL") => delete(ctx, &tokens)?,
            Some("LISTCAT") | Some("LISTC") => listcat(ctx, &tokens)?,
            Some("SET") => {
                // SET MAXCC = n / SET LASTCC = n
                let value = tokens
                    .iter()
                    .rev()
                    .find_map(|t| t.parse::<u32>().ok())
                    .unwrap_or(0);
                if tokens.get(1).is_some_and(|t| t == "MAXCC") {
                    maxcc = value;
                }
                continue;
            }
            Some(other) => (12, vec![format!("IDC3211I KEYWORD '{other}' IS IMPROPER")]),
            None => continue,
        };
        out.extend(messages);
        out.push(format!(
            "IDC0001I FUNCTION COMPLETED, HIGHEST CONDITION CODE WAS {lastcc}"
        ));
        out.push(String::new());
        maxcc = maxcc.max(lastcc);
    }
    out.push(format!(
        "IDC0002I IDCAMS PROCESSING COMPLETE. MAXIMUM CONDITION CODE WAS {maxcc}"
    ));
    ctx.print("SYSPRINT", &out)?;
    Ok(Outcome::Rc(maxcc))
}

/// Join continuation lines and strip comments.
fn commands(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for raw in text.lines() {
        let mut line = raw.chars().take(72).collect::<String>();
        if let Some(start) = line.find("/*") {
            let end = line.find("*/").map(|e| e + 2).unwrap_or(line.len());
            line.replace_range(start..end.max(start), "");
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(body) = line.strip_suffix('-') {
            current.push_str(body.trim_end());
            current.push(' ');
        } else {
            current.push_str(line);
            out.push(std::mem::take(&mut current).trim().to_string());
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out
}

fn tokenize(command: &str) -> Vec<String> {
    command
        .replace('(', " ( ")
        .replace(')', " ) ")
        .replace('=', " = ")
        .split_whitespace()
        .map(str::to_uppercase)
        .collect()
}

/// Value of `KEYWORD(value)`.
fn arg(tokens: &[String], keys: &[&str]) -> Option<String> {
    let i = tokens.iter().position(|t| keys.contains(&t.as_str()))?;
    match (tokens.get(i + 1), tokens.get(i + 2), tokens.get(i + 3)) {
        (Some(open), Some(v), Some(close)) if open == "(" && close == ")" => Some(v.clone()),
        _ => None,
    }
}

fn has(tokens: &[String], key: &str) -> bool {
    tokens.iter().any(|t| t == key)
}

fn define(ctx: &UtilCtx, tokens: &[String]) -> Result<(u32, Vec<String>)> {
    if !(has(tokens, "GDG") || has(tokens, "GENERATIONDATAGROUP")) {
        return Ok((12, vec!["IDC3211I ONLY DEFINE GDG IS SUPPORTED".into()]));
    }
    let Some(name) = arg(tokens, &["NAME"]) else {
        return Ok((12, vec!["IDC3202I REQUIRED KEYWORD NAME IS MISSING".into()]));
    };
    let Some(limit) = arg(tokens, &["LIMIT", "LIM"]).and_then(|l| l.parse::<u32>().ok()) else {
        return Ok((
            12,
            vec!["IDC3202I REQUIRED KEYWORD LIMIT IS MISSING OR INVALID".into()],
        ));
    };
    if !(1..=255).contains(&limit) {
        return Ok((
            12,
            vec![format!("IDC3226I LIMIT {limit} OUT OF RANGE 1-255")],
        ));
    }
    let mut cat = ctx.sys.catalog()?;
    if cat.gdgs.contains_key(&name) || cat.datasets.contains_key(&name) {
        return Ok((
            12,
            vec![
                format!("IDC3013I DUPLICATE DATA SET NAME {name}"),
                "IDC3009I ** VSAM CATALOG RETURN CODE IS 8 - REASON CODE IS IGG0CLEH-38".into(),
            ],
        ));
    }
    cat.gdgs.insert(
        name.clone(),
        GdgBase {
            limit,
            scratch: has(tokens, "SCRATCH") || has(tokens, "SCR"),
            empty: has(tokens, "EMPTY") || has(tokens, "EMP"),
            generations: Vec::new(),
        },
    );
    ctx.sys.save_catalog(&cat)?;
    Ok((0, Vec::new()))
}

fn delete(ctx: &UtilCtx, tokens: &[String]) -> Result<(u32, Vec<String>)> {
    let Some(name) = tokens.get(1).filter(|t| *t != "(").cloned() else {
        return Ok((12, vec!["IDC3202I ENTRY NAME REQUIRED".into()]));
    };
    let mut cat = ctx.sys.catalog()?;
    let mut messages = Vec::new();

    if let Some(gdg) = cat.gdgs.get(&name).cloned() {
        if !gdg.generations.is_empty() && !has(tokens, "FORCE") {
            return Ok((
                8,
                vec![
                    format!(
                        "IDC3014I CATALOG ERROR - GDG {name} HAS {} GENERATIONS, SPECIFY FORCE",
                        gdg.generations.len()
                    ),
                    "IDC3009I ** VSAM CATALOG RETURN CODE IS 12 - REASON CODE IS IGG0CLEG-8".into(),
                ],
            ));
        }
        for generation in &gdg.generations {
            let dsn = Catalog::generation_name(&name, *generation);
            std::fs::remove_file(ctx.sys.dataset_path(&dsn)).ok();
            cat.datasets.remove(&dsn);
            messages.push(format!("IDC0550I ENTRY (A) {dsn} DELETED"));
        }
        cat.gdgs.remove(&name);
        messages.push(format!("IDC0550I ENTRY (B) {name} DELETED"));
    } else if cat.datasets.contains_key(&name) {
        std::fs::remove_file(ctx.sys.dataset_path(&name)).ok();
        cat.uncatalog(&name);
        messages.push(format!("IDC0550I ENTRY (A) {name} DELETED"));
    } else {
        return Ok((
            8,
            vec![
                format!("IDC3012I ENTRY {name} NOT FOUND"),
                "IDC3009I ** VSAM CATALOG RETURN CODE IS 8 - REASON CODE IS IGG0CLEG-42".into(),
            ],
        ));
    }
    ctx.sys.save_catalog(&cat)?;
    Ok((0, messages))
}

fn listcat(ctx: &UtilCtx, tokens: &[String]) -> Result<(u32, Vec<String>)> {
    let filter = arg(tokens, &["ENTRIES", "ENT", "LEVEL", "LVL"]);
    let cat = ctx.sys.catalog()?;
    let matches = |n: &str| filter.as_deref().is_none_or(|f| dsn_matches(f, n));
    let mut lines = Vec::new();
    let mut entries = 0;
    for (name, gdg) in cat.gdgs.iter().filter(|(n, _)| matches(n)) {
        entries += 1 + gdg.generations.len();
        lines.push(format!("GDG BASE ------ {name}"));
        lines.push(format!(
            "     LIMIT------------{:<5} {} {}",
            gdg.limit,
            if gdg.scratch { "SCRATCH" } else { "NOSCRATCH" },
            if gdg.empty { "EMPTY" } else { "NOEMPTY" }
        ));
        for generation in &gdg.generations {
            lines.push(format!(
                "     NONVSAM ---- {}",
                Catalog::generation_name(name, *generation)
            ));
        }
    }
    for (name, entry) in cat.datasets.iter().filter(|(n, _)| matches(n)) {
        if crate::catalog::parse_generation_name(name)
            .is_some_and(|(b, _)| cat.gdgs.contains_key(&b))
        {
            continue;
        }
        entries += 1;
        let lrecl = entry
            .lrecl
            .map(|l| l.to_string())
            .unwrap_or_else(|| "-".into());
        lines.push(format!("NONVSAM ------- {name}"));
        lines.push(format!(
            "     RECFM-{:<4} LRECL-{lrecl:<6} CREATED-{}",
            entry.recfm, entry.created
        ));
    }
    if lines.is_empty() {
        return Ok((4, vec!["IDC3012I NO ENTRIES MATCH".into()]));
    }
    lines.push(String::new());
    lines.push(format!(
        "     THE NUMBER OF ENTRIES PROCESSED WAS {entries}"
    ));
    Ok((0, lines))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_continuations_and_strips_comments() {
        let cmds = commands(
            "  /* setup */\n  DEFINE GDG (NAME(A.B) -\n     LIMIT(5) SCRATCH)\n  SET MAXCC = 0\n",
        );
        assert_eq!(
            cmds,
            vec!["DEFINE GDG (NAME(A.B) LIMIT(5) SCRATCH)", "SET MAXCC = 0"]
        );
        let t = tokenize(&cmds[0]);
        assert_eq!(arg(&t, &["NAME"]).as_deref(), Some("A.B"));
        assert_eq!(arg(&t, &["LIMIT"]).as_deref(), Some("5"));
        assert!(has(&t, "SCRATCH"));
    }
}
