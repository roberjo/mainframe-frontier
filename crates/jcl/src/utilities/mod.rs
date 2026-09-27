//! Built-in system utilities, dispatched by `PGM=` before the loadlib is searched.

mod idcams;

use std::fs;
use std::io::Write as _;

use anyhow::Result;

use crate::jes::{Alloc, Outcome};
use crate::system::System;

pub(crate) struct UtilCtx<'a> {
    pub sys: &'a System,
    pub allocs: &'a mut [Alloc],
}

impl UtilCtx<'_> {
    fn dd(&self, name: &str) -> Option<&Alloc> {
        self.allocs.iter().find(|a| a.ddname == name)
    }

    fn dd_mut(&mut self, name: &str) -> Option<&mut Alloc> {
        self.allocs.iter_mut().find(|a| a.ddname == name)
    }

    /// Append lines to a message DD (SYSPRINT, SYSOUT). Silently dropped when
    /// the DD is absent, as the real utilities do.
    fn print(&self, ddname: &str, lines: &[String]) -> Result<()> {
        if let Some(a) = self.dd(ddname) {
            let mut f = fs::OpenOptions::new().append(true).open(&a.path)?;
            for l in lines {
                writeln!(f, "{l}")?;
            }
        }
        Ok(())
    }

    fn read_text(&self, ddname: &str) -> Result<Option<String>> {
        match self.dd(ddname) {
            Some(a) => Ok(Some(fs::read_to_string(&a.path)?)),
            None => Ok(None),
        }
    }
}

/// Run `pgm` if it is a built-in utility; `None` means "not a utility".
pub(crate) fn run(pgm: &str, ctx: &mut UtilCtx) -> Result<Option<Outcome>> {
    let outcome = match pgm {
        "IEFBR14" => Outcome::Rc(0),
        "SORT" | "ICEMAN" | "DFSORT" | "SYNCSORT" => sort(ctx)?,
        "IEBGENER" | "ICEGENER" => iebgener(ctx)?,
        "IDCAMS" => idcams::run(ctx)?,
        _ => return Ok(None),
    };
    Ok(Some(outcome))
}

fn sort(ctx: &mut UtilCtx) -> Result<Outcome> {
    let fail = |ctx: &UtilCtx, msg: String| -> Result<Outcome> {
        ctx.print("SYSOUT", &[msg])?;
        Ok(Outcome::Rc(16))
    };

    let Some(control) = ctx.read_text("SYSIN")? else {
        return fail(ctx, "ICE010A 0 NO SYSIN DD STATEMENT".into());
    };
    let spec = match frontier_sort::parse_control(&control) {
        Ok(s) => s,
        Err(e) => return fail(ctx, e.to_string()),
    };
    let Some(sortin) = ctx.dd("SORTIN").cloned() else {
        return fail(ctx, "ICE080A 0 SORTIN DD STATEMENT MISSING".into());
    };
    let Some(lrecl) = sortin.lrecl else {
        return fail(
            ctx,
            "ICE088A 0 SORTIN LRECL UNKNOWN - SPECIFY LRECL= OR CATALOG IT".into(),
        );
    };
    if ctx.dd("SORTOUT").is_none() {
        return fail(ctx, "ICE080A 0 SORTOUT DD STATEMENT MISSING".into());
    }

    let input = fs::read(&sortin.path)?;
    let (output, stats) = match frontier_sort::run(&spec, &input, lrecl as usize) {
        Ok(r) => r,
        Err(e) => return fail(ctx, e.to_string()),
    };

    let sortout = ctx.dd_mut("SORTOUT").expect("checked above");
    fs::write(&sortout.path, output)?;
    sortout.lrecl = sortout.lrecl.or(Some(lrecl));
    sortout.recfm = sortout.recfm.clone().or(sortin.recfm.clone());

    ctx.print("SYSOUT", &frontier_sort::messages(&spec, &stats))?;
    Ok(Outcome::Rc(0))
}

fn iebgener(ctx: &mut UtilCtx) -> Result<Outcome> {
    let (Some(ut1), Some(_)) = (ctx.dd("SYSUT1").cloned(), ctx.dd("SYSUT2")) else {
        ctx.print(
            "SYSPRINT",
            &["IEB311I CONFLICTING DCB PARAMETERS - SYSUT1 AND SYSUT2 ARE REQUIRED".into()],
        )?;
        return Ok(Outcome::Rc(12));
    };
    let data = fs::read(&ut1.path)?;
    let ut2 = ctx.dd_mut("SYSUT2").expect("checked above");
    fs::write(&ut2.path, &data)?;
    ut2.lrecl = ut2.lrecl.or(ut1.lrecl);
    ut2.recfm = ut2.recfm.clone().or(ut1.recfm.clone());

    let records = match ut1.lrecl {
        Some(l) if l > 0 => format!("{} RECORDS", data.len() / l as usize),
        _ => format!("{} BYTES", data.len()),
    };
    ctx.print(
        "SYSPRINT",
        &[
            "DATA SET UTILITY - GENERATE".into(),
            format!(
                "COPIED {records} FROM {} TO {}",
                ut1.display,
                ctx.dd("SYSUT2").unwrap().display
            ),
            "PROCESSING ENDED AT EOD".into(),
        ],
    )?;
    Ok(Outcome::Rc(0))
}
