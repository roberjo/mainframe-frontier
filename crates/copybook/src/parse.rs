//! Copybook source → item tree with byte offsets and sizes.

use std::fmt;

use crate::pic::{PicCategory, parse_pic};
use crate::{CondValue, Condition, Copybook, Item, Literal, Occurs, SignClause, Usage};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SourceFormat {
    /// Columns 1-6 sequence, 7 indicator, 8-72 code (the z/OS default).
    #[default]
    Fixed,
    /// Whole line is code; `*>` starts a comment.
    Free,
}

#[derive(Debug, Clone)]
pub struct ParseOptions {
    pub format: SourceFormat,
    /// `COPY X REPLACING ==from== BY ==to==` pairs, applied as text.
    pub replacing: Vec<(String, String)>,
    /// With no `replacing`, drop `:TAG:` prefixes (`:AR:-BALANCE` → `BALANCE`).
    pub strip_tags: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            format: SourceFormat::Fixed,
            replacing: Vec::new(),
            strip_tags: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopybookError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for CopybookError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for CopybookError {}

fn err<T>(line: usize, message: impl Into<String>) -> Result<T, CopybookError> {
    Err(CopybookError {
        line,
        message: message.into(),
    })
}

// ---------------------------------------------------------------------------
// Source lines
// ---------------------------------------------------------------------------

fn source_lines(source: &str, opts: &ParseOptions) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    for (i, raw) in source.lines().enumerate() {
        let lineno = i + 1;
        let raw = raw.replace('\t', "    ");
        let (indicator, mut text) = match opts.format {
            SourceFormat::Fixed => {
                let chars: Vec<char> = raw.chars().collect();
                if chars.len() < 7 {
                    continue;
                }
                let end = chars.len().min(72);
                (chars[6], chars[7.min(end)..end].iter().collect::<String>())
            }
            SourceFormat::Free => (' ', raw.clone()),
        };
        if matches!(indicator, '*' | '/')
            || (opts.format == SourceFormat::Free && raw.trim_start().starts_with('*'))
        {
            continue;
        }
        if let Some(pos) = text.find("*>") {
            text.truncate(pos);
        }
        text = apply_replacing(&text, opts);

        if indicator == '-' {
            // Continuation: a literal resumes after its opening quote; other text is appended.
            if let Some((_, prev)) = out.last_mut() {
                let cont = text.trim_start();
                if let Some(rest) = cont.strip_prefix(['\'', '"']) {
                    let padded = format!("{prev:<65}");
                    *prev = format!("{padded}{rest}");
                } else {
                    prev.push_str(cont);
                }
                continue;
            }
        }
        out.push((lineno, text));
    }
    out
}

fn apply_replacing(text: &str, opts: &ParseOptions) -> String {
    if !opts.replacing.is_empty() {
        let mut s = text.to_string();
        for (from, to) in &opts.replacing {
            s = s.replace(from.as_str(), to.as_str());
        }
        return s;
    }
    if !opts.strip_tags {
        return text.to_string();
    }
    // Remove :TAG: and a following hyphen.
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == ':'
            && let Some(len) = chars[i + 1..].iter().position(|&c| c == ':')
            && len > 0
            && chars[i + 1..i + 1 + len]
                .iter()
                .all(|c| c.is_ascii_alphanumeric() || *c == '-')
        {
            i += len + 2;
            if chars.get(i) == Some(&'-') {
                i += 1;
            }
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Lit(String),
    Period,
}

#[derive(Debug, Clone)]
struct Token {
    tok: Tok,
    line: usize,
}

fn tokenize(lines: &[(usize, String)]) -> Result<Vec<Token>, CopybookError> {
    let mut tokens = Vec::new();
    for (lineno, text) in lines {
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c.is_whitespace() {
                i += 1;
                continue;
            }
            // Literal, including X'..' hex literals.
            let hex = (c == 'X' || c == 'x') && matches!(chars.get(i + 1), Some('\'') | Some('"'));
            if c == '\'' || c == '"' || hex {
                let start = if hex { i + 1 } else { i };
                let quote = chars[start];
                let mut j = start + 1;
                let mut value = String::new();
                loop {
                    if j >= chars.len() {
                        return err(*lineno, "unterminated literal");
                    }
                    if chars[j] == quote {
                        if chars.get(j + 1) == Some(&quote) {
                            value.push(quote);
                            j += 2;
                            continue;
                        }
                        break;
                    }
                    value.push(chars[j]);
                    j += 1;
                }
                if hex {
                    let bytes = (0..value.len())
                        .step_by(2)
                        .map(|k| {
                            value
                                .get(k..k + 2)
                                .and_then(|h| u8::from_str_radix(h, 16).ok())
                        })
                        .collect::<Option<Vec<u8>>>()
                        .ok_or(CopybookError {
                            line: *lineno,
                            message: format!("invalid hex literal X'{value}'"),
                        })?;
                    value = bytes.into_iter().map(|b| b as char).collect();
                }
                tokens.push(Token {
                    tok: Tok::Lit(value),
                    line: *lineno,
                });
                i = j + 1;
                continue;
            }
            let mut j = i;
            while j < chars.len() && !chars[j].is_whitespace() {
                j += 1;
            }
            let mut word: String = chars[i..j].iter().collect();
            let mut period = false;
            if word.ends_with('.') {
                word.pop();
                period = true;
            }
            while word.ends_with(',') || word.ends_with(';') {
                word.pop();
            }
            if !word.is_empty() {
                tokens.push(Token {
                    tok: Tok::Word(word.to_uppercase()),
                    line: *lineno,
                });
            }
            if period {
                tokens.push(Token {
                    tok: Tok::Period,
                    line: *lineno,
                });
            }
            i = j;
        }
    }
    Ok(tokens)
}

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

const CLAUSE_WORDS: &[&str] = &[
    "PIC",
    "PICTURE",
    "USAGE",
    "REDEFINES",
    "OCCURS",
    "VALUE",
    "VALUES",
    "SIGN",
    "LEADING",
    "TRAILING",
    "SYNC",
    "SYNCHRONIZED",
    "JUST",
    "JUSTIFIED",
    "BLANK",
    "EXTERNAL",
    "GLOBAL",
    "RENAMES",
    "IS",
];

fn usage_word(w: &str) -> Option<Usage> {
    Some(match w {
        "DISPLAY" => Usage::Display,
        "COMP" | "COMPUTATIONAL" | "COMP-4" | "COMPUTATIONAL-4" | "BINARY" => Usage::Binary,
        "COMP-5" | "COMPUTATIONAL-5" => Usage::Comp5,
        "COMP-3" | "COMPUTATIONAL-3" | "PACKED-DECIMAL" => Usage::Packed,
        "COMP-1" | "COMPUTATIONAL-1" => Usage::Comp1,
        "COMP-2" | "COMPUTATIONAL-2" => Usage::Comp2,
        "INDEX" => Usage::Index,
        "POINTER" | "PROCEDURE-POINTER" | "FUNCTION-POINTER" => Usage::Pointer,
        "NATIONAL" => Usage::Display,
        _ => return None,
    })
}

fn is_clause_start(w: &str) -> bool {
    CLAUSE_WORDS.contains(&w) || usage_word(w).is_some()
}

fn literal_of(tok: &Tok) -> Option<Literal> {
    match tok {
        Tok::Lit(s) => Some(Literal::Text(s.clone())),
        Tok::Word(w) => {
            let fig = match w.as_str() {
                "ZERO" | "ZEROS" | "ZEROES" => Some("ZERO"),
                "SPACE" | "SPACES" => Some("SPACE"),
                "HIGH-VALUE" | "HIGH-VALUES" => Some("HIGH-VALUE"),
                "LOW-VALUE" | "LOW-VALUES" => Some("LOW-VALUE"),
                "QUOTE" | "QUOTES" => Some("QUOTE"),
                "NULL" | "NULLS" => Some("NULL"),
                _ => None,
            };
            if let Some(f) = fig {
                return Some(Literal::Figurative(f.into()));
            }
            ebcdic::Decimal::parse(w).map(|_| Literal::Number(w.clone()))
        }
        Tok::Period => None,
    }
}

struct Entry {
    item: Item,
}

fn parse_entry(
    sentence: &[Token],
) -> Result<Option<(u8, Entry, Option<Condition>)>, CopybookError> {
    let line = sentence[0].line;
    let words = |i: usize| match sentence.get(i).map(|t| &t.tok) {
        Some(Tok::Word(w)) => Some(w.as_str()),
        _ => None,
    };
    let level: u8 = match words(0).and_then(|w| w.parse().ok()) {
        Some(l) => l,
        None => {
            let first = words(0).unwrap_or("literal");
            if matches!(first, "EJECT" | "SKIP1" | "SKIP2" | "SKIP3") {
                return Ok(None);
            }
            return err(line, format!("expected a level number, found {first}"));
        }
    };
    if !(matches!(level, 1..=49 | 66 | 77 | 88)) {
        return err(line, format!("invalid level number {level:02}"));
    }
    if level == 66 {
        return err(line, "level 66 RENAMES is not supported");
    }

    let mut i = 1;
    let name = match words(1) {
        Some("FILLER") => {
            i += 1;
            None
        }
        Some(w) if !is_clause_start(w) => {
            i += 1;
            Some(w.to_string())
        }
        _ => None,
    };

    // Level 88: condition-name VALUE[S] [IS|ARE] v [THRU v] ...
    if level == 88 {
        let Some(name) = name else {
            return err(line, "level 88 needs a condition name");
        };
        if !matches!(words(i), Some("VALUE") | Some("VALUES")) {
            return err(line, format!("88 {name} has no VALUE clause"));
        }
        i += 1;
        let mut values = Vec::new();
        while i < sentence.len() {
            if matches!(words(i), Some("IS") | Some("ARE")) {
                i += 1;
                continue;
            }
            let lo = literal_of(&sentence[i].tok).ok_or(CopybookError {
                line,
                message: format!("invalid VALUE in 88 {name}"),
            })?;
            i += 1;
            if matches!(words(i), Some("THRU") | Some("THROUGH")) {
                let hi =
                    sentence
                        .get(i + 1)
                        .and_then(|t| literal_of(&t.tok))
                        .ok_or(CopybookError {
                            line,
                            message: format!("invalid THRU in 88 {name}"),
                        })?;
                values.push(CondValue::Range(lo, hi));
                i += 2;
            } else {
                values.push(CondValue::Single(lo));
            }
        }
        return Ok(Some((
            88,
            Entry {
                item: Item::new(88, None, line),
            },
            Some(Condition { name, values }),
        )));
    }

    let mut item = Item::new(level, name, line);
    let mut pic_text = None;
    while i < sentence.len() {
        let Some(w) = words(i) else {
            return err(sentence[i].line, "unexpected literal");
        };
        i += 1;
        match w {
            "PIC" | "PICTURE" => {
                if words(i) == Some("IS") {
                    i += 1;
                }
                pic_text = Some(
                    words(i)
                        .ok_or(CopybookError {
                            line,
                            message: "PICTURE needs a string".into(),
                        })?
                        .to_string(),
                );
                i += 1;
            }
            "USAGE" => {
                if words(i) == Some("IS") {
                    i += 1;
                }
                item.usage = words(i).and_then(usage_word).ok_or(CopybookError {
                    line,
                    message: format!("invalid USAGE {}", words(i).unwrap_or("")),
                })?;
                item.usage_explicit = true;
                i += 1;
            }
            "REDEFINES" => {
                item.redefines = Some(
                    words(i)
                        .ok_or(CopybookError {
                            line,
                            message: "REDEFINES needs a name".into(),
                        })?
                        .to_string(),
                );
                i += 1;
            }
            "OCCURS" => {
                let n: u32 = words(i).and_then(|w| w.parse().ok()).ok_or(CopybookError {
                    line,
                    message: "OCCURS needs a count".into(),
                })?;
                i += 1;
                let mut occurs = Occurs {
                    min: n,
                    max: n,
                    depending_on: None,
                };
                if words(i) == Some("TO") {
                    occurs.max =
                        words(i + 1)
                            .and_then(|w| w.parse().ok())
                            .ok_or(CopybookError {
                                line,
                                message: "OCCURS TO needs a count".into(),
                            })?;
                    i += 2;
                }
                if words(i) == Some("TIMES") {
                    i += 1;
                }
                if words(i) == Some("DEPENDING") {
                    i += 1;
                    if words(i) == Some("ON") {
                        i += 1;
                    }
                    occurs.depending_on = Some(
                        words(i)
                            .ok_or(CopybookError {
                                line,
                                message: "DEPENDING ON needs a name".into(),
                            })?
                            .to_string(),
                    );
                    i += 1;
                }
                // ASCENDING/DESCENDING KEY IS a b / INDEXED BY x y: names we don't need.
                while let Some(w) = words(i) {
                    if matches!(
                        w,
                        "ASCENDING" | "DESCENDING" | "KEY" | "IS" | "INDEXED" | "BY"
                    ) || !is_clause_start(w)
                    {
                        i += 1;
                    } else {
                        break;
                    }
                }
                if occurs.max == 0 || occurs.min > occurs.max {
                    return err(line, "invalid OCCURS bounds");
                }
                item.occurs = Some(occurs);
            }
            "VALUE" | "VALUES" => {
                if matches!(words(i), Some("IS") | Some("ARE")) {
                    i += 1;
                }
                if words(i) == Some("ALL") {
                    i += 1;
                }
                item.value = sentence.get(i).and_then(|t| literal_of(&t.tok));
                i += 1;
            }
            "SIGN" => {
                if words(i) == Some("IS") {
                    i += 1;
                }
                continue;
            }
            "LEADING" | "TRAILING" => {
                let mut sign = SignClause {
                    leading: w == "LEADING",
                    separate: false,
                };
                if words(i) == Some("SEPARATE") {
                    sign.separate = true;
                    i += 1;
                    if words(i) == Some("CHARACTER") {
                        i += 1;
                    }
                }
                item.sign = Some(sign);
            }
            "SYNC" | "SYNCHRONIZED" => {
                item.sync = true;
                if matches!(words(i), Some("LEFT") | Some("RIGHT")) {
                    i += 1;
                }
            }
            "JUST" | "JUSTIFIED" => {
                item.justified = true;
                if words(i) == Some("RIGHT") {
                    i += 1;
                }
            }
            "BLANK" => {
                if words(i) == Some("WHEN") {
                    i += 1;
                }
                i += 1; // ZERO
                item.blank_when_zero = true;
            }
            "EXTERNAL" | "GLOBAL" | "IS" => {}
            "RENAMES" => return err(line, "RENAMES is not supported"),
            other => match usage_word(other) {
                Some(u) => {
                    item.usage = u;
                    item.usage_explicit = true;
                }
                None => return err(line, format!("unexpected {other} in data description")),
            },
        }
    }
    if let Some(p) = pic_text {
        item.pic = Some(parse_pic(&p).map_err(|m| CopybookError { line, message: m })?);
    }
    Ok(Some((level, Entry { item }, None)))
}

/// Parse copybook source into records (level 01/77 items) with layout computed.
pub fn parse(name: &str, source: &str, opts: &ParseOptions) -> Result<Copybook, CopybookError> {
    let lines = source_lines(source, opts);
    let tokens = tokenize(&lines)?;

    let mut entries: Vec<Item> = Vec::new();
    for sentence in tokens.split(|t| t.tok == Tok::Period) {
        if sentence.is_empty() {
            continue;
        }
        if let Some(Tok::Word(w)) = sentence.first().map(|t| &t.tok)
            && (w == "COPY" || w == "REPLACE")
        {
            return err(sentence[0].line, format!("nested {w} is not supported"));
        }
        let Some((level, entry, condition)) = parse_entry(sentence)? else {
            continue;
        };
        if let Some(c) = condition {
            let target = entries.last_mut().ok_or(CopybookError {
                line: sentence[0].line,
                message: "88 before any data item".into(),
            })?;
            target.conditions.push(c);
            continue;
        }
        debug_assert_eq!(level, entry.item.level);
        entries.push(entry.item);
    }
    if entries.is_empty() {
        return err(1, "no data descriptions found");
    }
    if entries[0].level != 1 && entries[0].level != 77 {
        // Copybooks often start at 05 for inclusion under a caller's 01; wrap them.
        let mut root = Item::new(1, Some(name.to_uppercase()), entries[0].line);
        root.synthetic = true;
        entries.insert(0, root);
    }

    let mut pos = 0;
    let mut records = build(&entries, &mut pos, 0)?;
    for r in &mut records {
        inherit(r, Usage::Display, None, false);
        layout(r, 0)?;
        check_odo(r, true)?;
    }
    Ok(Copybook {
        name: name.to_uppercase(),
        records,
    })
}

fn build(entries: &[Item], pos: &mut usize, parent_level: u8) -> Result<Vec<Item>, CopybookError> {
    let mut out = Vec::new();
    while *pos < entries.len() {
        let e = &entries[*pos];
        let top = e.level == 1 || e.level == 77;
        if parent_level != 0 && (top || e.level <= parent_level) {
            break;
        }
        let mut item = e.clone();
        *pos += 1;
        if item.level != 77 {
            item.children = build(entries, pos, item.level)?;
        }
        if !item.children.is_empty() && item.pic.is_some() {
            return err(
                item.line,
                format!("group item {} cannot have a PICTURE", item.display_name()),
            );
        }
        out.push(item);
    }
    Ok(out)
}

/// Group USAGE and SIGN clauses apply to their elementary items.
fn inherit(item: &mut Item, usage: Usage, sign: Option<SignClause>, explicit: bool) {
    if !item.usage_explicit && explicit {
        item.usage = usage;
        item.usage_explicit = true;
    }
    if item.sign.is_none() {
        item.sign = sign;
    }
    let (u, s, e) = (item.usage, item.sign, item.usage_explicit);
    for c in &mut item.children {
        inherit(c, u, s, e);
    }
}

fn elementary_size(item: &Item) -> Result<usize, CopybookError> {
    let line = item.line;
    let name = item.display_name();
    match (item.usage, &item.pic) {
        (Usage::Comp1, _) => Ok(4),
        (Usage::Comp2, _) => Ok(8),
        (Usage::Index, _) | (Usage::Pointer, _) => Ok(4),
        (_, None) => err(line, format!("elementary item {name} has no PICTURE")),
        (Usage::Display, Some(p)) => {
            let separate = item.sign.is_some_and(|s| s.separate);
            if separate && !(p.is_numeric() && p.signed) {
                return err(
                    line,
                    format!("SIGN SEPARATE on {name} requires a signed numeric PICTURE"),
                );
            }
            Ok(p.length + usize::from(separate))
        }
        (Usage::Packed, Some(p)) => {
            if !p.is_numeric() {
                return err(
                    line,
                    format!("COMP-3 item {name} needs a numeric PICTURE, not {}", p.text),
                );
            }
            Ok(ebcdic::packed::len(p.digits))
        }
        (Usage::Binary | Usage::Comp5, Some(p)) => {
            if !p.is_numeric() {
                return err(
                    line,
                    format!("binary item {name} needs a numeric PICTURE, not {}", p.text),
                );
            }
            if p.digits > 18 {
                return err(line, format!("binary item {name} has more than 18 digits"));
            }
            Ok(ebcdic::binary::len(p.digits))
        }
    }
}

fn layout(item: &mut Item, offset: usize) -> Result<(), CopybookError> {
    item.offset = offset;
    if item.children.is_empty() {
        item.size = elementary_size(item)?;
        if item
            .pic
            .as_ref()
            .is_some_and(|p| p.category == PicCategory::National)
            && item.usage != Usage::Display
        {
            return err(item.line, "NATIONAL items must be USAGE DISPLAY/NATIONAL");
        }
        return Ok(());
    }
    let mut cur = offset;
    let mut end = offset;
    for i in 0..item.children.len() {
        if let Some(target) = item.children[i].redefines.clone() {
            let base = item.children[..i]
                .iter()
                .find(|c| c.name.as_deref() == Some(target.as_str()))
                .map(|c| c.offset)
                .ok_or(CopybookError {
                    line: item.children[i].line,
                    message: format!("REDEFINES target {target} not found"),
                })?;
            layout(&mut item.children[i], base)?;
            end = end.max(base + item.children[i].total_size());
        } else {
            layout(&mut item.children[i], cur)?;
            cur += item.children[i].total_size();
            end = end.max(cur);
        }
    }
    item.size = end - offset;
    Ok(())
}

/// OCCURS DEPENDING ON is only supported at the end of a record.
fn check_odo(item: &Item, last: bool) -> Result<(), CopybookError> {
    if item
        .occurs
        .as_ref()
        .is_some_and(|o| o.depending_on.is_some())
        && !last
    {
        return err(
            item.line,
            format!(
                "OCCURS DEPENDING ON item {} must be the last item in the record",
                item.display_name()
            ),
        );
    }
    let n = item.children.len();
    for (i, c) in item.children.iter().enumerate() {
        check_odo(c, last && i + 1 == n)?;
    }
    Ok(())
}
