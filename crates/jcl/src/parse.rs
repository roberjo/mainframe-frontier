//! JCL-lite parser.
//!
//! Supported: `JOB`, `EXEC PGM=`, `DD`, `SET`, `//*` comments, `/*`
//! delimiters, the `//` null statement, operand continuation (a trailing
//! comma), in-stream data (`DD *` / `DD DATA`), and `&SYMBOL` substitution
//! (`&&NAME` is a temporary dataset, not a symbol). Columns 72-80 are
//! ignored, as on the real thing.
//!
//! Not supported (rejected with a JCL error): cataloged/in-stream PROCs,
//! `IF/THEN/ELSE`, DD concatenation, `INCLUDE`/`JCLLIB`.

use std::collections::BTreeMap;
use std::fmt;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JclError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for JclError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LINE {}: {}", self.line, self.message)
    }
}

impl std::error::Error for JclError {}

fn jcl_err<T>(line: usize, message: impl Into<String>) -> Result<T, JclError> {
    Err(JclError {
        line,
        message: message.into(),
    })
}

// ---------------------------------------------------------------------------
// Operands
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Value {
    Atom(String),
    List(Vec<Operand>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Operand {
    pub key: Option<String>,
    pub value: Value,
}

impl Value {
    pub fn as_atom(&self) -> Option<&str> {
        match self {
            Value::Atom(s) => Some(s),
            Value::List(_) => None,
        }
    }

    /// Positional items of a list, or the atom itself as a one-item list.
    pub fn items(&self) -> Vec<&Value> {
        match self {
            Value::Atom(_) => vec![self],
            Value::List(ops) => ops.iter().map(|o| &o.value).collect(),
        }
    }

    pub fn keywords(&self) -> Vec<&Operand> {
        match self {
            Value::Atom(_) => Vec::new(),
            Value::List(ops) => ops.iter().filter(|o| o.key.is_some()).collect(),
        }
    }
}

/// Parse an operand field such as `DSN=A.B(+1),DISP=(NEW,CATLG,DELETE)`.
pub fn parse_operands(text: &str) -> Result<Vec<Operand>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    if chars.is_empty() {
        return Ok(Vec::new());
    }
    let ops = parse_list(&chars, &mut i, false)?;
    if i != chars.len() {
        return Err(format!("UNEXPECTED '{}' IN OPERANDS", chars[i]));
    }
    Ok(ops)
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '#' | '@' | '$')
}

fn parse_list(chars: &[char], i: &mut usize, in_parens: bool) -> Result<Vec<Operand>, String> {
    let mut ops = Vec::new();
    loop {
        // KEY= prefix?
        let mut j = *i;
        while j < chars.len() && is_name_char(chars[j]) {
            j += 1;
        }
        let key = if j > *i && j < chars.len() && chars[j] == '=' {
            let k: String = chars[*i..j].iter().collect();
            *i = j + 1;
            Some(k)
        } else {
            None
        };
        let value = parse_value(chars, i)?;
        ops.push(Operand { key, value });

        if *i == chars.len() {
            if in_parens {
                return Err("UNBALANCED PARENTHESES".into());
            }
            return Ok(ops);
        }
        match chars[*i] {
            ',' => *i += 1,
            ')' if in_parens => return Ok(ops),
            c => return Err(format!("UNEXPECTED '{c}' IN OPERANDS")),
        }
    }
}

fn parse_value(chars: &[char], i: &mut usize) -> Result<Value, String> {
    if *i < chars.len() && chars[*i] == '(' {
        *i += 1;
        if *i < chars.len() && chars[*i] == ')' {
            *i += 1;
            return Ok(Value::List(Vec::new()));
        }
        let items = parse_list(chars, i, true)?;
        // parse_list stops on the closing ')'
        *i += 1;
        return Ok(Value::List(items));
    }
    let mut atom = String::new();
    while *i < chars.len() {
        match chars[*i] {
            ',' | ')' => break,
            '\'' => {
                *i += 1;
                loop {
                    if *i >= chars.len() {
                        return Err("UNBALANCED APOSTROPHES".into());
                    }
                    if chars[*i] == '\'' {
                        if *i + 1 < chars.len() && chars[*i + 1] == '\'' {
                            atom.push('\'');
                            *i += 2;
                            continue;
                        }
                        *i += 1;
                        break;
                    }
                    atom.push(chars[*i]);
                    *i += 1;
                }
            }
            '(' => {
                // Parenthesized suffix inside an atom: DSN=BASE(+1), LIB(MEMBER)
                while *i < chars.len() && chars[*i] != ')' {
                    atom.push(chars[*i]);
                    *i += 1;
                }
                if *i == chars.len() {
                    return Err("UNBALANCED PARENTHESES".into());
                }
                atom.push(')');
                *i += 1;
            }
            c => {
                atom.push(c);
                *i += 1;
            }
        }
    }
    Ok(Value::Atom(atom))
}

// ---------------------------------------------------------------------------
// Job model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DispStatus {
    New,
    Old,
    Shr,
    Mod,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum DispAction {
    Delete,
    Keep,
    Pass,
    Catlg,
    Uncatlg,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Disp {
    pub status: DispStatus,
    pub normal: Option<DispAction>,
    pub abnormal: Option<DispAction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum DdKind {
    Dataset { dsn: String, disp: Disp },
    Sysout { class: String },
    Dummy,
    Instream { data: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Dd {
    pub name: String,
    pub line: usize,
    pub kind: DdKind,
    pub recfm: Option<String>,
    pub lrecl: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum CondOp {
    Gt,
    Ge,
    Eq,
    Ne,
    Lt,
    Le,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CondTest {
    pub code: u32,
    pub op: CondOp,
    pub step: Option<String>,
}

impl CondTest {
    /// True when the step should be *bypassed*: `code OP rc`.
    pub fn bypasses(&self, rc: u32) -> bool {
        match self.op {
            CondOp::Gt => self.code > rc,
            CondOp::Ge => self.code >= rc,
            CondOp::Eq => self.code == rc,
            CondOp::Ne => self.code != rc,
            CondOp::Lt => self.code < rc,
            CondOp::Le => self.code <= rc,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Cond {
    pub tests: Vec<CondTest>,
    pub even: bool,
    pub only: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Step {
    pub name: String,
    pub line: usize,
    pub pgm: String,
    pub parm: Option<String>,
    pub cond: Cond,
    pub dds: Vec<Dd>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Job {
    pub name: String,
    pub accounting: Option<String>,
    pub programmer: Option<String>,
    pub class: String,
    pub msgclass: String,
    pub steps: Vec<Step>,
    pub symbols: BTreeMap<String, String>,
    /// Numbered statement listing for the JESJCL spool file.
    pub listing: Vec<String>,
}

// ---------------------------------------------------------------------------
// Statement assembly
// ---------------------------------------------------------------------------

/// Columns 1-71 hold the statement; 72 is the continuation column and
/// 73-80 are sequence numbers.
fn statement_columns(line: &str) -> &str {
    match line.char_indices().nth(71) {
        Some((idx, _)) => &line[..idx],
        None => line,
    }
}

/// The operand field ends at the first blank outside apostrophes.
fn operand_end(s: &str) -> usize {
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        match c {
            '\'' => quoted = !quoted,
            ' ' if !quoted => return i,
            _ => {}
        }
    }
    s.len()
}

struct RawStatement {
    line: usize,
    name: Option<String>,
    op: String,
    operands: String,
}

/// Replace `&NAME` / `&NAME.` with symbol values. `&&` is left alone.
pub fn substitute(
    text: &str,
    symbols: &BTreeMap<String, String>,
) -> Result<(String, bool), String> {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut changed = false;
    while i < chars.len() {
        if chars[i] != '&' {
            out.push(chars[i]);
            i += 1;
            continue;
        }
        if i + 1 < chars.len() && chars[i + 1] == '&' {
            out.push_str("&&");
            i += 2;
            continue;
        }
        let start = i + 1;
        let mut j = start;
        while j < chars.len() && j - start < 8 && is_name_char(chars[j]) {
            j += 1;
        }
        if j == start {
            out.push('&');
            i += 1;
            continue;
        }
        let name: String = chars[start..j].iter().collect();
        let value = symbols
            .get(&name)
            .ok_or_else(|| format!("IEFC657I THE SYMBOL {name} WAS NOT DEFINED"))?;
        out.push_str(value);
        changed = true;
        i = j;
        if i < chars.len() && chars[i] == '.' {
            i += 1;
        }
    }
    Ok((out, changed))
}

/// Parse a job. `overrides` take precedence over `SET` statements in the JCL.
pub fn parse_job(source: &str, overrides: &BTreeMap<String, String>) -> Result<Job, JclError> {
    let lines: Vec<&str> = source.lines().collect();
    let mut symbols: BTreeMap<String, String> = overrides.clone();
    let mut listing = Vec::new();
    let mut job: Option<Job> = None;
    let mut stmt_no = 0usize;
    let mut i = 0usize;
    let mut temp_seq = 0u32;

    while i < lines.len() {
        let lineno = i + 1;
        let line = statement_columns(lines[i]);

        if line.starts_with("//*") {
            listing.push(format!("{:>10} {}", "", line.trim_end()));
            i += 1;
            continue;
        }
        if line.trim_end() == "//" {
            break; // null statement: end of job
        }
        if line.starts_with("/*") || line.trim().is_empty() {
            i += 1;
            continue;
        }
        if !line.starts_with("//") {
            return jcl_err(
                lineno,
                "IEFC605I UNIDENTIFIED OPERATION FIELD (DATA WITHOUT DD *)",
            );
        }

        // Fields: //NAME OP OPERANDS COMMENTS
        let body = &line[2..];
        let (name, rest) = if body.starts_with(' ') {
            (None, body)
        } else {
            let end = body.find(' ').unwrap_or(body.len());
            (Some(body[..end].to_string()), &body[end..])
        };
        let rest = rest.trim_start();
        let op_end = rest.find(' ').unwrap_or(rest.len());
        let op = rest[..op_end].to_string();
        let after_op = rest[op_end..].trim_start();
        let mut operands = after_op[..operand_end(after_op)].to_string();
        let mut raw_lines = vec![line.trim_end().to_string()];

        // Continuation: operand field ending in a comma continues on the next //  line.
        i += 1;
        while operands.ends_with(',') {
            let Some(next) = lines.get(i) else {
                return jcl_err(lineno, "IEFC621I EXPECTED CONTINUATION NOT RECEIVED");
            };
            let next = statement_columns(next);
            if !next.starts_with("// ") || next.starts_with("//*") {
                return jcl_err(i + 1, "IEFC621I EXPECTED CONTINUATION NOT RECEIVED");
            }
            let cont = next[2..].trim_start();
            operands.push_str(&cont[..operand_end(cont)]);
            raw_lines.push(next.trim_end().to_string());
            i += 1;
        }

        stmt_no += 1;
        for (k, raw) in raw_lines.iter().enumerate() {
            if k == 0 {
                listing.push(format!("{stmt_no:>10} {raw}"));
            } else {
                listing.push(format!("{:>10} {raw}", ""));
            }
        }

        let raw = RawStatement {
            line: lineno,
            name,
            op,
            operands,
        };

        // SET is processed before substitution of its own operands' symbols.
        if raw.op == "SET" {
            let (text, _) = substitute(&raw.operands, &symbols).map_err(|m| JclError {
                line: lineno,
                message: m,
            })?;
            for o in parse_operands(&text).map_err(|m| JclError {
                line: lineno,
                message: m,
            })? {
                let (Some(key), Value::Atom(v)) = (o.key, o.value) else {
                    return jcl_err(lineno, "IEFC019I SET REQUIRES SYMBOL=VALUE");
                };
                // Command-line overrides win over SET.
                if !overrides.contains_key(&key) {
                    symbols.insert(key, v);
                }
            }
            continue;
        }

        let (text, changed) = substitute(&raw.operands, &symbols).map_err(|m| JclError {
            line: lineno,
            message: m,
        })?;
        if changed {
            listing.push(format!("{:>10} IEFC653I SUBSTITUTION JCL - {text}", ""));
        }
        let ops = parse_operands(&text).map_err(|m| JclError {
            line: lineno,
            message: format!("IEFC662I {m}"),
        })?;

        match raw.op.as_str() {
            "JOB" => {
                if job.is_some() {
                    return jcl_err(lineno, "IEFC607I JOB HAS MULTIPLE JOB STATEMENTS");
                }
                let name = raw.name.clone().ok_or(JclError {
                    line: lineno,
                    message: "IEFC601I JOB STATEMENT REQUIRES A JOB NAME".into(),
                })?;
                if name.len() > 8 {
                    return jcl_err(
                        lineno,
                        format!("IEFC624I JOB NAME {name} EXCEEDS 8 CHARACTERS"),
                    );
                }
                let positional: Vec<String> = ops
                    .iter()
                    .filter(|o| o.key.is_none())
                    .map(|o| render_value(&o.value))
                    .collect();
                job = Some(Job {
                    name,
                    accounting: positional.first().cloned().filter(|s| !s.is_empty()),
                    programmer: positional.get(1).cloned().filter(|s| !s.is_empty()),
                    class: keyword(&ops, "CLASS").unwrap_or_else(|| "A".into()),
                    msgclass: keyword(&ops, "MSGCLASS").unwrap_or_else(|| "X".into()),
                    steps: Vec::new(),
                    symbols: BTreeMap::new(),
                    listing: Vec::new(),
                });
            }
            "EXEC" => {
                let j = job.as_mut().ok_or(JclError {
                    line: lineno,
                    message: "IEFC604I EXEC BEFORE JOB STATEMENT".into(),
                })?;
                let step = parse_exec(&raw, &ops)?;
                if j.steps
                    .iter()
                    .any(|s| s.name == step.name && !s.name.is_empty())
                {
                    return jcl_err(
                        lineno,
                        format!("IEFC613I DUPLICATE STEP NAME {}", step.name),
                    );
                }
                j.steps.push(step);
            }
            "DD" => {
                let j = job.as_mut().ok_or(JclError {
                    line: lineno,
                    message: "IEFC604I DD BEFORE JOB STATEMENT".into(),
                })?;
                let step = j.steps.last_mut().ok_or(JclError {
                    line: lineno,
                    message: "IEFC604I DD BEFORE EXEC STATEMENT".into(),
                })?;
                let mut dd = parse_dd(&raw, &ops, &mut temp_seq)?;
                if step.dds.iter().any(|d| d.name == dd.name) {
                    return jcl_err(lineno, format!("IEFC636I DUPLICATE DDNAME {}", dd.name));
                }
                if let DdKind::Instream { data } = &mut dd.kind {
                    let is_data = ops
                        .iter()
                        .any(|o| o.key.is_none() && o.value.as_atom() == Some("DATA"));
                    while i < lines.len() {
                        let l = lines[i];
                        if l.starts_with("/*") || (!is_data && l.starts_with("//")) {
                            break;
                        }
                        data.push(l.trim_end().to_string());
                        i += 1;
                    }
                    if i < lines.len() && lines[i].starts_with("/*") {
                        i += 1;
                    }
                }
                step.dds.push(dd);
            }
            "PROC" | "PEND" | "JCLLIB" | "INCLUDE" => {
                return jcl_err(
                    lineno,
                    format!("IEFC605I {} IS NOT SUPPORTED BY JCL-LITE", raw.op),
                );
            }
            "IF" | "ELSE" | "ENDIF" => {
                return jcl_err(
                    lineno,
                    "IEFC605I IF/THEN/ELSE IS NOT SUPPORTED BY JCL-LITE (USE COND=)",
                );
            }
            other => {
                return jcl_err(
                    lineno,
                    format!("IEFC605I UNIDENTIFIED OPERATION FIELD {other}"),
                );
            }
        }
    }

    let mut job = job.ok_or(JclError {
        line: 1,
        message: "IEFC601I NO JOB STATEMENT FOUND".into(),
    })?;
    if job.steps.is_empty() {
        return jcl_err(1, "IEFC605I JOB HAS NO STEPS");
    }
    job.symbols = symbols;
    job.listing = listing;
    Ok(job)
}

fn render_value(v: &Value) -> String {
    match v {
        Value::Atom(s) => s.clone(),
        Value::List(ops) => {
            let inner: Vec<String> = ops
                .iter()
                .map(|o| match &o.key {
                    Some(k) => format!("{k}={}", render_value(&o.value)),
                    None => render_value(&o.value),
                })
                .collect();
            format!("({})", inner.join(","))
        }
    }
}

fn keyword(ops: &[Operand], key: &str) -> Option<String> {
    ops.iter()
        .find(|o| o.key.as_deref() == Some(key))
        .map(|o| render_value(&o.value))
}

fn keyword_value<'a>(ops: &'a [Operand], key: &str) -> Option<&'a Value> {
    ops.iter()
        .find(|o| o.key.as_deref() == Some(key))
        .map(|o| &o.value)
}

fn parse_exec(raw: &RawStatement, ops: &[Operand]) -> Result<Step, JclError> {
    let line = raw.line;
    let name = raw.name.clone().unwrap_or_default();
    if name.len() > 8 {
        return jcl_err(
            line,
            format!("IEFC624I STEP NAME {name} EXCEEDS 8 CHARACTERS"),
        );
    }
    let pgm = match keyword(ops, "PGM") {
        Some(p) => p,
        None => {
            let proc = ops
                .first()
                .map(|o| render_value(&o.value))
                .unwrap_or_default();
            return jcl_err(
                line,
                format!("IEFC605I EXEC OF PROCEDURE {proc} IS NOT SUPPORTED BY JCL-LITE"),
            );
        }
    };
    let cond = match keyword_value(ops, "COND") {
        Some(v) => parse_cond(v).map_err(|m| JclError { line, message: m })?,
        None => Cond::default(),
    };
    Ok(Step {
        name,
        line,
        pgm,
        parm: keyword(ops, "PARM"),
        cond,
        dds: Vec::new(),
    })
}

fn parse_cond_op(s: &str) -> Option<CondOp> {
    Some(match s {
        "GT" => CondOp::Gt,
        "GE" => CondOp::Ge,
        "EQ" => CondOp::Eq,
        "NE" => CondOp::Ne,
        "LT" => CondOp::Lt,
        "LE" => CondOp::Le,
        _ => return None,
    })
}

fn parse_cond(v: &Value) -> Result<Cond, String> {
    let mut cond = Cond::default();
    let bad = || "IEFC662I INVALID COND PARAMETER".to_string();

    let test_from = |items: &[&Value]| -> Result<CondTest, String> {
        let atoms: Vec<&str> = items.iter().map(|v| v.as_atom().unwrap_or("")).collect();
        if !(2..=3).contains(&atoms.len()) {
            return Err(bad());
        }
        let code: u32 = atoms[0].parse().map_err(|_| bad())?;
        if code > 4095 {
            return Err(bad());
        }
        let op = parse_cond_op(atoms[1]).ok_or_else(bad)?;
        Ok(CondTest {
            code,
            op,
            step: atoms.get(2).map(|s| s.to_string()),
        })
    };

    match v {
        Value::Atom(a) if a == "EVEN" => cond.even = true,
        Value::Atom(a) if a == "ONLY" => cond.only = true,
        Value::Atom(_) => return Err(bad()),
        Value::List(_) => {
            let items = v.items();
            if items.first().is_some_and(|i| matches!(i, Value::List(_))) {
                for item in items {
                    match item {
                        Value::Atom(a) if a == "EVEN" => cond.even = true,
                        Value::Atom(a) if a == "ONLY" => cond.only = true,
                        Value::List(_) => cond.tests.push(test_from(&item.items())?),
                        _ => return Err(bad()),
                    }
                }
            } else {
                cond.tests.push(test_from(&items)?);
            }
        }
    }
    if cond.even && cond.only {
        return Err("IEFC662I COND CANNOT SPECIFY BOTH EVEN AND ONLY".into());
    }
    Ok(cond)
}

fn parse_disp_action(s: &str) -> Result<Option<DispAction>, String> {
    Ok(Some(match s {
        "" => return Ok(None),
        "DELETE" => DispAction::Delete,
        "KEEP" => DispAction::Keep,
        "PASS" => DispAction::Pass,
        "CATLG" => DispAction::Catlg,
        "UNCATLG" => DispAction::Uncatlg,
        other => return Err(format!("IEFC662I INVALID DISP DISPOSITION {other}")),
    }))
}

fn parse_disp(v: Option<&Value>) -> Result<Disp, String> {
    let items: Vec<&str> = match v {
        None => Vec::new(),
        Some(v) => v
            .items()
            .iter()
            .map(|i| i.as_atom().unwrap_or("?"))
            .collect(),
    };
    let status = match items.first().copied().unwrap_or("") {
        "" | "NEW" => DispStatus::New,
        "OLD" => DispStatus::Old,
        "SHR" => DispStatus::Shr,
        "MOD" => DispStatus::Mod,
        other => return Err(format!("IEFC662I INVALID DISP STATUS {other}")),
    };
    Ok(Disp {
        status,
        normal: parse_disp_action(items.get(1).copied().unwrap_or(""))?,
        abnormal: parse_disp_action(items.get(2).copied().unwrap_or(""))?,
    })
}

fn parse_dd(raw: &RawStatement, ops: &[Operand], temp_seq: &mut u32) -> Result<Dd, JclError> {
    let line = raw.line;
    let name = match &raw.name {
        Some(n) => n.clone(),
        None => {
            return jcl_err(
                line,
                "IEFC605I DD CONCATENATION IS NOT SUPPORTED BY JCL-LITE",
            );
        }
    };
    let e = |m: String| JclError { line, message: m };

    // DCB attributes may be given directly or inside DCB=(...).
    let mut recfm = keyword(ops, "RECFM");
    let mut lrecl = keyword(ops, "LRECL");
    if let Some(dcb) = keyword_value(ops, "DCB") {
        for o in dcb.keywords() {
            match o.key.as_deref() {
                Some("RECFM") => recfm = recfm.or(o.value.as_atom().map(str::to_string)),
                Some("LRECL") => lrecl = lrecl.or(o.value.as_atom().map(str::to_string)),
                _ => {}
            }
        }
    }
    let lrecl = match lrecl {
        Some(l) => Some(
            l.parse::<u32>()
                .map_err(|_| e(format!("IEFC662I INVALID LRECL {l}")))?,
        ),
        None => None,
    };

    let positional = ops
        .iter()
        .find(|o| o.key.is_none())
        .and_then(|o| o.value.as_atom());
    let kind = match positional {
        Some("*") | Some("DATA") => DdKind::Instream { data: Vec::new() },
        Some("DUMMY") => DdKind::Dummy,
        Some(other) if !other.is_empty() => {
            return jcl_err(line, format!("IEFC662I INVALID DD POSITIONAL {other}"));
        }
        _ => {
            if let Some(sysout) = keyword_value(ops, "SYSOUT") {
                let class = sysout
                    .items()
                    .first()
                    .and_then(|v| v.as_atom())
                    .unwrap_or("*")
                    .to_string();
                DdKind::Sysout { class }
            } else {
                let disp = parse_disp(keyword_value(ops, "DISP")).map_err(e)?;
                let dsn = match keyword(ops, "DSN").or_else(|| keyword(ops, "DSNAME")) {
                    Some(d) => d,
                    None if keyword_value(ops, "DISP").is_some()
                        || keyword_value(ops, "SPACE").is_some() =>
                    {
                        *temp_seq += 1;
                        format!("&&SYS{:05}", *temp_seq)
                    }
                    None => {
                        return jcl_err(
                            line,
                            format!("IEFC605I DD {name} HAS NO DSN, SYSOUT, DUMMY OR *"),
                        );
                    }
                };
                if dsn == "NULLFILE" {
                    DdKind::Dummy
                } else {
                    validate_dsn(&dsn).map_err(e)?;
                    DdKind::Dataset { dsn, disp }
                }
            }
        }
    };
    Ok(Dd {
        name,
        line,
        kind,
        recfm,
        lrecl,
    })
}

fn validate_dsn(dsn: &str) -> Result<(), String> {
    let base = dsn.strip_prefix("&&").unwrap_or(dsn);
    let base = base.split('(').next().unwrap_or(base);
    if base.is_empty() || base.len() > 44 {
        return Err(format!("IEFC662I INVALID DATA SET NAME {dsn}"));
    }
    for q in base.split('.') {
        let ok = !q.is_empty()
            && q.len() <= 8
            && q.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '#' | '@' | '$'))
            && q.chars().all(|c| is_name_char(c) || c == '-');
        if !ok {
            return Err(format!("IEFC662I INVALID DATA SET NAME {dsn}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(s: &str) -> Value {
        Value::Atom(s.into())
    }

    #[test]
    fn operands_with_lists_quotes_and_gdg_suffix() {
        let ops = parse_operands("DSN=FFB.ACCTMAST(+1),DISP=(NEW,CATLG,DELETE),PARM='A,B ''C'''")
            .unwrap();
        assert_eq!(ops[0].value, atom("FFB.ACCTMAST(+1)"));
        assert_eq!(
            ops[1].value.items(),
            vec![&atom("NEW"), &atom("CATLG"), &atom("DELETE")]
        );
        assert_eq!(ops[2].value, atom("A,B 'C'"));
    }

    #[test]
    fn omitted_positional_in_list() {
        let ops = parse_operands("DISP=(,PASS)").unwrap();
        assert_eq!(ops[0].value.items(), vec![&atom(""), &atom("PASS")]);
    }

    #[test]
    fn substitution_leaves_temp_names_alone() {
        let mut syms = BTreeMap::new();
        syms.insert("DATE".to_string(), "20260927".to_string());
        let (out, changed) = substitute("PARM='&DATE',DSN=&&TEMP,X=&DATE..A", &syms).unwrap();
        assert!(changed);
        assert_eq!(out, "PARM='20260927',DSN=&&TEMP,X=20260927.A");
        assert!(substitute("&NOPE", &syms).is_err());
    }

    const SAMPLE: &str = "\
//NIGHTLY  JOB (FFB),'NIGHTLY BATCH',CLASS=A,MSGCLASS=X
//         SET BUSDATE=20260927
//* comment line
//SORTTRN  EXEC PGM=SORT
//SORTIN   DD DSN=FFB.DAILY.TRANFEED,DISP=SHR
//SORTOUT  DD DSN=&&SORTED,DISP=(NEW,PASS),
//            RECFM=FB,LRECL=80                                         00010000
//SYSIN    DD *
  SORT FIELDS=(1,10,CH,A)
/*
//VALIDATE EXEC PGM=TRNVALID,PARM='&BUSDATE',COND=(4,LT)
//TRANIN   DD DSN=&&SORTED,DISP=(OLD,DELETE)
//OUT      DD SYSOUT=*
//NOTHING  DD DUMMY
//ARCHIVE  EXEC PGM=IEFBR14,COND=((8,LE),EVEN)
//
//IGNORED  EXEC PGM=NEVER
";

    #[test]
    fn parses_a_job() {
        let job = parse_job(SAMPLE, &BTreeMap::new()).unwrap();
        assert_eq!(job.name, "NIGHTLY");
        assert_eq!(job.accounting.as_deref(), Some("(FFB)"));
        assert_eq!(job.programmer.as_deref(), Some("NIGHTLY BATCH"));
        assert_eq!(job.steps.len(), 3);

        let sort = &job.steps[0];
        assert_eq!(sort.pgm, "SORT");
        assert_eq!(sort.dds[1].lrecl, Some(80));
        assert_eq!(sort.dds[1].recfm.as_deref(), Some("FB"));
        assert_eq!(
            sort.dds[2].kind,
            DdKind::Instream {
                data: vec!["  SORT FIELDS=(1,10,CH,A)".into()]
            }
        );

        let validate = &job.steps[1];
        assert_eq!(validate.parm.as_deref(), Some("20260927"));
        assert_eq!(
            validate.cond.tests,
            vec![CondTest {
                code: 4,
                op: CondOp::Lt,
                step: None
            }]
        );
        assert_eq!(validate.dds[1].kind, DdKind::Sysout { class: "*".into() });
        assert_eq!(validate.dds[2].kind, DdKind::Dummy);

        let archive = &job.steps[2];
        assert!(archive.cond.even);
        assert_eq!(archive.cond.tests.len(), 1);
        assert!(
            job.listing
                .iter()
                .any(|l| l.contains("IEFC653I SUBSTITUTION JCL - PGM=TRNVALID,PARM='20260927'"))
        );
    }

    #[test]
    fn overrides_beat_set() {
        let mut o = BTreeMap::new();
        o.insert("BUSDATE".to_string(), "20260930".to_string());
        let job = parse_job(SAMPLE, &o).unwrap();
        assert_eq!(job.steps[1].parm.as_deref(), Some("20260930"));
    }

    #[test]
    fn reports_errors_with_line_numbers() {
        let e = parse_job(
            "//J JOB\n//S EXEC PGM=X\n//D DD DSN=BAD..NAME,DISP=SHR\n",
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert_eq!(e.line, 3);
        let e = parse_job("//J JOB\n//S EXEC MYPROC\n", &BTreeMap::new()).unwrap_err();
        assert!(e.message.contains("PROCEDURE"));
        let e = parse_job(
            "//J JOB\n//S EXEC PGM=X\n//A DD DSN=A.B,\n",
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert!(e.message.contains("CONTINUATION"));
    }

    #[test]
    fn cond_tests() {
        let t = CondTest {
            code: 4,
            op: CondOp::Lt,
            step: None,
        };
        assert!(!t.bypasses(4));
        assert!(t.bypasses(8));
    }
}
