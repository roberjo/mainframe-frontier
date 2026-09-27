//! Frontier Sort: a DFSORT-compatible subset for fixed-length records.
//!
//! Supported control statements:
//!
//! ```text
//!   SORT FIELDS=(p,l,f,A|D,...)        f = CH | BI | ZD | PD
//!   SORT FIELDS=(p,l,A|D,...),FORMAT=f
//!   SORT FIELDS=COPY  /  OPTION COPY
//!   INCLUDE COND=(p,l,f,op,const[,AND|OR,...])
//!   OMIT    COND=(...)
//!   SUM FIELDS=NONE                     drop records with duplicate keys
//! ```
//!
//! Constants are `C'text'`, `X'hex'`, or signed decimal numbers (for ZD/PD).
//! The sort is stable, matching DFSORT's `EQUALS` option.

use std::cmp::Ordering;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortError(pub String);

impl fmt::Display for SortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SortError {}

fn err<T>(msg: impl Into<String>) -> Result<T, SortError> {
    Err(SortError(msg.into()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Character: unsigned byte-wise comparison.
    Ch,
    /// Binary: unsigned byte-wise comparison.
    Bi,
    /// Zoned decimal (display numeric, sign overpunch in the last byte).
    Zd,
    /// Packed decimal (COMP-3).
    Pd,
}

impl Format {
    fn parse(s: &str) -> Result<Self, SortError> {
        match s {
            "CH" => Ok(Format::Ch),
            "BI" => Ok(Format::Bi),
            "ZD" => Ok(Format::Zd),
            "PD" => Ok(Format::Pd),
            other => err(format!("ICE113A UNSUPPORTED FORMAT {other}")),
        }
    }

    fn is_numeric(self) -> bool {
        matches!(self, Format::Zd | Format::Pd)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    /// 1-based starting position, as written in the control statement.
    pub pos: usize,
    pub len: usize,
    pub format: Format,
    pub ascending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Gt,
    Ge,
    Lt,
    Le,
}

impl Op {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "EQ" => Op::Eq,
            "NE" => Op::Ne,
            "GT" => Op::Gt,
            "GE" => Op::Ge,
            "LT" => Op::Lt,
            "LE" => Op::Le,
            _ => return None,
        })
    }

    fn test(self, ord: Ordering) -> bool {
        match self {
            Op::Eq => ord == Ordering::Equal,
            Op::Ne => ord != Ordering::Equal,
            Op::Gt => ord == Ordering::Greater,
            Op::Ge => ord != Ordering::Less,
            Op::Lt => ord == Ordering::Less,
            Op::Le => ord != Ordering::Greater,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Constant {
    Bytes(Vec<u8>),
    Number(i128),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    pub pos: usize,
    pub len: usize,
    pub format: Format,
    pub op: Op,
    pub constant: Constant,
}

/// INCLUDE or OMIT condition in disjunctive normal form: OR of AND-groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub include: bool,
    pub any_of: Vec<Vec<Comparison>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SortSpec {
    /// Empty when copying.
    pub keys: Vec<Key>,
    pub copy: bool,
    pub filter: Option<Filter>,
    pub sum_none: bool,
    /// The control statements as written, for the ICE000I listing.
    pub statements: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SortStats {
    pub records_in: u64,
    pub records_out: u64,
    pub filtered: u64,
    pub duplicates_dropped: u64,
}

/// Parse DFSORT control statements (SYSIN).
pub fn parse_control(text: &str) -> Result<SortSpec, SortError> {
    let mut spec = SortSpec::default();
    let mut saw_sort = false;

    for stmt in join_statements(text) {
        spec.statements.push(stmt.clone());
        let (verb, operands) = match stmt.split_once(char::is_whitespace) {
            Some((v, rest)) => (v.to_string(), operand_field(rest.trim_start())),
            None => (stmt.clone(), String::new()),
        };
        let params = keyword_params(&operands)?;
        match verb.as_str() {
            "SORT" => {
                saw_sort = true;
                let fields = param(&params, "FIELDS")
                    .ok_or_else(|| SortError("ICE010A NO SORT FIELDS=".into()))?;
                if fields == "COPY" {
                    spec.copy = true;
                } else {
                    let default_format = param(&params, "FORMAT")
                        .as_deref()
                        .map(Format::parse)
                        .transpose()?;
                    spec.keys = parse_keys(&fields, default_format)?;
                }
            }
            "OPTION" => {
                if operands.split(',').any(|o| o.trim() == "COPY") {
                    saw_sort = true;
                    spec.copy = true;
                }
            }
            "INCLUDE" | "OMIT" => {
                if spec.filter.is_some() {
                    return err("ICE008A DUPLICATE INCLUDE/OMIT STATEMENT");
                }
                let cond = param(&params, "COND")
                    .ok_or_else(|| SortError(format!("ICE005A {verb} REQUIRES COND=")))?;
                let default_format = param(&params, "FORMAT")
                    .as_deref()
                    .map(Format::parse)
                    .transpose()?;
                spec.filter = Some(Filter {
                    include: verb == "INCLUDE",
                    any_of: parse_condition(&cond, default_format)?,
                });
            }
            "SUM" => match param(&params, "FIELDS").as_deref() {
                Some("NONE") | Some("(NONE)") => spec.sum_none = true,
                _ => return err("ICE111A ONLY SUM FIELDS=NONE IS SUPPORTED"),
            },
            other => return err(format!("ICE005A STATEMENT DEFINER ERROR: {other}")),
        }
    }

    if !saw_sort {
        return err("ICE010A NO SORT OR OPTION COPY STATEMENT");
    }
    if spec.sum_none && spec.copy {
        return err("ICE145A SUM IS NOT VALID WITH COPY");
    }
    Ok(spec)
}

/// Sort (or copy) `input`, a concatenation of `lrecl`-byte records.
pub fn run(spec: &SortSpec, input: &[u8], lrecl: usize) -> Result<(Vec<u8>, SortStats), SortError> {
    if lrecl == 0 {
        return err("ICE043A INVALID LRECL 0");
    }
    if !input.len().is_multiple_of(lrecl) {
        return err(format!(
            "ICE141A SORTIN LENGTH {} IS NOT A MULTIPLE OF LRECL {lrecl}",
            input.len()
        ));
    }
    for k in &spec.keys {
        check_bounds(k.pos, k.len, lrecl)?;
    }
    if let Some(f) = &spec.filter {
        for c in f.any_of.iter().flatten() {
            check_bounds(c.pos, c.len, lrecl)?;
        }
    }

    let mut stats = SortStats::default();
    let mut records: Vec<&[u8]> = input.chunks_exact(lrecl).collect();
    stats.records_in = records.len() as u64;

    if let Some(f) = &spec.filter {
        records.retain(|r| {
            let hit = f
                .any_of
                .iter()
                .any(|group| group.iter().all(|c| compare(r, c)));
            hit == f.include
        });
        stats.filtered = stats.records_in - records.len() as u64;
    }

    if !spec.copy {
        records.sort_by(|a, b| compare_keys(a, b, &spec.keys));
        if spec.sum_none {
            let before = records.len();
            records.dedup_by(|b, a| compare_keys(a, b, &spec.keys) == Ordering::Equal);
            stats.duplicates_dropped = (before - records.len()) as u64;
        }
    }

    stats.records_out = records.len() as u64;
    Ok((records.concat(), stats))
}

/// DFSORT-style messages for SYSOUT.
pub fn messages(spec: &SortSpec, stats: &SortStats) -> Vec<String> {
    let mut out = vec![
        "ICE000I 1 - CONTROL STATEMENTS FOR FRONTIER SORT (DFSORT-COMPATIBLE SUBSET)".to_string(),
    ];
    out.extend(spec.statements.iter().map(|s| format!("            {s}")));
    out.push(format!(
        "ICE143I 0 {} TECHNIQUE SELECTED",
        if spec.copy {
            "COPY"
        } else {
            "IN-MEMORY STABLE SORT"
        }
    ));
    if stats.filtered > 0 {
        let verb = if spec.filter.as_ref().is_some_and(|f| f.include) {
            "INCLUDE"
        } else {
            "OMIT"
        };
        out.push(format!(
            "ICE055I 0 {verb} DROPPED {} RECORDS",
            stats.filtered
        ));
    }
    if stats.duplicates_dropped > 0 {
        out.push(format!(
            "ICE056I 0 SUM DELETED {} DUPLICATE RECORDS",
            stats.duplicates_dropped
        ));
    }
    out.push(format!(
        "ICE054I 0 RECORDS - IN: {}, OUT: {}",
        stats.records_in, stats.records_out
    ));
    out.push("ICE052I 0 END OF SORT".to_string());
    out
}

fn check_bounds(pos: usize, len: usize, lrecl: usize) -> Result<(), SortError> {
    if pos == 0 || len == 0 || pos - 1 + len > lrecl {
        return err(format!("ICE027A FIELD ({pos},{len}) BEYOND LRECL {lrecl}"));
    }
    Ok(())
}

fn field(rec: &[u8], pos: usize, len: usize) -> &[u8] {
    &rec[pos - 1..pos - 1 + len]
}

fn compare_keys(a: &[u8], b: &[u8], keys: &[Key]) -> Ordering {
    for k in keys {
        let fa = field(a, k.pos, k.len);
        let fb = field(b, k.pos, k.len);
        let ord = match k.format {
            Format::Ch | Format::Bi => fa.cmp(fb),
            Format::Zd => decode_zoned(fa).cmp(&decode_zoned(fb)),
            Format::Pd => decode_packed(fa).cmp(&decode_packed(fb)),
        };
        let ord = if k.ascending { ord } else { ord.reverse() };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    Ordering::Equal
}

fn compare(rec: &[u8], c: &Comparison) -> bool {
    let f = field(rec, c.pos, c.len);
    let ord = match (&c.constant, c.format) {
        (Constant::Number(n), Format::Zd) => decode_zoned(f).cmp(n),
        (Constant::Number(n), Format::Pd) => decode_packed(f).cmp(n),
        (Constant::Bytes(b), _) => {
            // DFSORT pads a short character constant with blanks.
            let mut padded = b.clone();
            padded.resize(c.len, b' ');
            f.cmp(&padded[..c.len])
        }
        (Constant::Number(_), _) => Ordering::Less, // rejected at parse time
    };
    c.op.test(ord)
}

/// Decode packed decimal. Invalid nibbles decode as 0 so a bad record
/// sorts deterministically instead of aborting the whole sort.
pub fn decode_packed(bytes: &[u8]) -> i128 {
    let mut value: i128 = 0;
    for (i, b) in bytes.iter().enumerate() {
        let hi = (b >> 4) as i128;
        let lo = (b & 0x0f) as i128;
        if hi > 9 {
            return 0;
        }
        value = value * 10 + hi;
        if i + 1 < bytes.len() {
            if lo > 9 {
                return 0;
            }
            value = value * 10 + lo;
        } else if lo == 0x0d || lo == 0x0b {
            value = -value;
        }
    }
    value
}

/// Decode zoned decimal. Accepts EBCDIC-style overpunch (`{`, `A`-`I`,
/// `}`, `J`-`R`) and GnuCOBOL's ASCII negative overpunch (`p`-`y`).
pub fn decode_zoned(bytes: &[u8]) -> i128 {
    let mut value: i128 = 0;
    let mut negative = false;
    for (i, &b) in bytes.iter().enumerate() {
        let last = i + 1 == bytes.len();
        let digit = match b {
            b'0'..=b'9' => b - b'0',
            b'{' if last => 0,
            b'A'..=b'I' if last => b - b'A' + 1,
            b'}' if last => {
                negative = true;
                0
            }
            b'J'..=b'R' if last => {
                negative = true;
                b - b'J' + 1
            }
            b'p'..=b'y' if last => {
                negative = true;
                b - b'p'
            }
            b' ' => 0,
            _ => return 0,
        };
        value = value * 10 + digit as i128;
    }
    if negative { -value } else { value }
}

/// Split SYSIN into statements: drop comments, join lines ending in ','.
fn join_statements(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for raw in text.lines() {
        let line: String = raw.chars().take(71).collect();
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('*') {
            continue;
        }
        let field = if current.is_empty() {
            let (verb, rest) = trimmed
                .split_once(char::is_whitespace)
                .unwrap_or((trimmed, ""));
            format!("{verb} {}", operand_field(rest.trim_start()))
        } else {
            operand_field(trimmed)
        };
        current.push_str(field.trim_end());
        if !current.ends_with(',') {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// The operand field ends at the first blank outside quotes.
fn operand_field(s: &str) -> String {
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        match c {
            '\'' => quoted = !quoted,
            ' ' if !quoted => return s[..i].to_string(),
            _ => {}
        }
    }
    s.to_string()
}

/// Split on commas at parenthesis depth 0, outside quotes.
fn split_top(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut quoted = false;
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '\'' => quoted = !quoted,
            '(' if !quoted => depth += 1,
            ')' if !quoted => depth -= 1,
            ',' if !quoted && depth == 0 => {
                parts.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    if !cur.is_empty() || !parts.is_empty() {
        parts.push(cur);
    }
    parts
}

fn keyword_params(operands: &str) -> Result<Vec<(String, String)>, SortError> {
    split_top(operands)
        .into_iter()
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => Ok((k.to_string(), v.to_string())),
            None => Ok((p.clone(), String::new())),
        })
        .collect()
}

fn param(params: &[(String, String)], key: &str) -> Option<String> {
    params
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

fn unwrap_parens(s: &str) -> Result<&str, SortError> {
    s.strip_prefix('(')
        .and_then(|s| s.strip_suffix(')'))
        .ok_or_else(|| SortError(format!("ICE007A SYNTAX ERROR: EXPECTED (...) IN {s}")))
}

fn number(s: &str) -> Result<usize, SortError> {
    s.parse()
        .map_err(|_| SortError(format!("ICE007A SYNTAX ERROR: EXPECTED NUMBER, FOUND {s}")))
}

fn parse_keys(fields: &str, default_format: Option<Format>) -> Result<Vec<Key>, SortError> {
    let items = split_top(unwrap_parens(fields)?);
    let width = if default_format.is_some() { 3 } else { 4 };
    if items.is_empty() || !items.len().is_multiple_of(width) {
        return err(format!("ICE007A SYNTAX ERROR IN SORT FIELDS={fields}"));
    }
    items
        .chunks(width)
        .map(|c| {
            let format = match default_format {
                Some(f) => f,
                None => Format::parse(&c[2])?,
            };
            let ascending = match c[width - 1].as_str() {
                "A" => true,
                "D" => false,
                other => {
                    return err(format!(
                        "ICE007A SYNTAX ERROR: EXPECTED A OR D, FOUND {other}"
                    ));
                }
            };
            Ok(Key {
                pos: number(&c[0])?,
                len: number(&c[1])?,
                format,
                ascending,
            })
        })
        .collect()
}

fn parse_constant(s: &str, format: Format) -> Result<Constant, SortError> {
    if let Some(body) = s.strip_prefix("C'").and_then(|b| b.strip_suffix('\'')) {
        if format.is_numeric() {
            return err(format!(
                "ICE114A CHARACTER CONSTANT {s} INVALID FOR NUMERIC FIELD"
            ));
        }
        return Ok(Constant::Bytes(body.replace("''", "'").into_bytes()));
    }
    if let Some(hex) = s.strip_prefix("X'").and_then(|b| b.strip_suffix('\'')) {
        if hex.len() % 2 != 0 {
            return err(format!("ICE112A ODD-LENGTH HEX CONSTANT {s}"));
        }
        let bytes = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| SortError(format!("ICE112A INVALID HEX CONSTANT {s}")))?;
        return Ok(Constant::Bytes(bytes));
    }
    if !format.is_numeric() {
        return err(format!(
            "ICE114A NUMERIC CONSTANT {s} INVALID FOR {format:?} FIELD"
        ));
    }
    s.parse::<i128>()
        .map(Constant::Number)
        .map_err(|_| SortError(format!("ICE007A SYNTAX ERROR: INVALID CONSTANT {s}")))
}

fn parse_condition(
    cond: &str,
    default_format: Option<Format>,
) -> Result<Vec<Vec<Comparison>>, SortError> {
    let items = split_top(unwrap_parens(cond)?);
    let width = if default_format.is_some() { 4 } else { 5 };
    let mut groups: Vec<Vec<Comparison>> = vec![Vec::new()];
    let mut i = 0;
    loop {
        if i + width > items.len() {
            return err(format!("ICE007A SYNTAX ERROR IN COND={cond}"));
        }
        let c = &items[i..i + width];
        let format = match default_format {
            Some(f) => f,
            None => Format::parse(&c[2])?,
        };
        let op = Op::parse(&c[width - 2]).ok_or_else(|| {
            SortError(format!(
                "ICE007A SYNTAX ERROR: INVALID OPERATOR {}",
                c[width - 2]
            ))
        })?;
        groups.last_mut().unwrap().push(Comparison {
            pos: number(&c[0])?,
            len: number(&c[1])?,
            format,
            op,
            constant: parse_constant(&c[width - 1], format)?,
        });
        i += width;
        if i == items.len() {
            break;
        }
        match items[i].as_str() {
            "AND" | "&" => {}
            "OR" | "|" => groups.push(Vec::new()),
            other => {
                return err(format!(
                    "ICE007A SYNTAX ERROR: EXPECTED AND/OR, FOUND {other}"
                ));
            }
        }
        i += 1;
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recs(rows: &[&str]) -> Vec<u8> {
        rows.concat().into_bytes()
    }

    fn rows(out: &[u8], lrecl: usize) -> Vec<String> {
        out.chunks(lrecl)
            .map(|c| String::from_utf8_lossy(c).into_owned())
            .collect()
    }

    #[test]
    fn sorts_on_multiple_character_keys() {
        let spec = parse_control("  SORT FIELDS=(1,2,CH,A,3,2,CH,D)\n").unwrap();
        let (out, stats) = run(&spec, &recs(&["BB01", "AA01", "BB09", "AA05"]), 4).unwrap();
        assert_eq!(rows(&out, 4), ["AA05", "AA01", "BB09", "BB01"]);
        assert_eq!(stats.records_in, 4);
        assert_eq!(stats.records_out, 4);
    }

    #[test]
    fn sort_is_stable() {
        let spec = parse_control(" SORT FIELDS=(1,1,A),FORMAT=CH").unwrap();
        let (out, _) = run(&spec, &recs(&["B1", "A1", "B2", "A2"]), 2).unwrap();
        assert_eq!(rows(&out, 2), ["A1", "A2", "B1", "B2"]);
    }

    #[test]
    fn packed_decimal_keys_sort_numerically() {
        // +5, -12, +100 packed into 3 bytes each
        let input = [0x00, 0x00, 0x5c, 0x00, 0x01, 0x2d, 0x00, 0x10, 0x0c];
        let spec = parse_control(" SORT FIELDS=(1,3,PD,A)").unwrap();
        let (out, _) = run(&spec, &input, 3).unwrap();
        assert_eq!(out, [0x00, 0x01, 0x2d, 0x00, 0x00, 0x5c, 0x00, 0x10, 0x0c]);
    }

    #[test]
    fn include_with_or_and_continuation() {
        let spec = parse_control(
            "  OPTION COPY\n  INCLUDE COND=(1,2,CH,EQ,C'DP',OR,\n               1,2,CH,EQ,C'WD')\n",
        )
        .unwrap();
        let (out, stats) = run(&spec, &recs(&["DP1", "FE2", "WD3", "TI4"]), 3).unwrap();
        assert_eq!(rows(&out, 3), ["DP1", "WD3"]);
        assert_eq!(stats.filtered, 2);
    }

    #[test]
    fn omit_on_zoned_decimal() {
        let spec = parse_control(" SORT FIELDS=COPY\n OMIT COND=(1,3,ZD,LT,+10)").unwrap();
        let (out, _) = run(&spec, &recs(&["005", "010", "12}", "200"]), 3).unwrap();
        assert_eq!(rows(&out, 3), ["010", "200"]);
    }

    #[test]
    fn sum_fields_none_drops_duplicates() {
        let spec = parse_control(" SORT FIELDS=(1,1,CH,A)\n SUM FIELDS=NONE").unwrap();
        let (out, stats) = run(&spec, &recs(&["B1", "A1", "B2"]), 2).unwrap();
        assert_eq!(rows(&out, 2), ["A1", "B1"]);
        assert_eq!(stats.duplicates_dropped, 1);
    }

    #[test]
    fn rejects_short_input_and_bad_fields() {
        let spec = parse_control(" SORT FIELDS=(1,10,CH,A)").unwrap();
        assert!(run(&spec, b"abc", 2).is_err());
        assert!(run(&spec, b"abcd", 4).is_err());
        assert!(parse_control(" SORT FIELDS=(1,2,XX,A)").is_err());
        assert!(parse_control(" INCLUDE COND=(1,2,CH,EQ,C'A')").is_err());
    }

    #[test]
    fn decodes_zoned_and_packed() {
        assert_eq!(decode_packed(&[0x01, 0x23, 0x4d]), -1234);
        assert_eq!(decode_packed(&[0x99, 0x9f]), 999);
        assert_eq!(decode_zoned(b"123"), 123);
        assert_eq!(decode_zoned(b"12r"), -122);
        assert_eq!(decode_zoned(b"12J"), -121);
    }
}
