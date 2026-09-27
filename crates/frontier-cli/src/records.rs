//! Viewing records: raw, hex, and copybook-driven field, table and JSON views.

use anyhow::Result;
use clap::ValueEnum;
use frontier_adapter::layouts::Layout;
use frontier_copybook::{DecodeOptions, Encoding, FlatField, decode, flatten};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Characters, one record per line
    Raw,
    /// Hex dump with characters
    Hex,
    /// One field per line (needs a layout)
    Fields,
    /// One record per row (needs a layout)
    Table,
    /// JSON array (needs a layout)
    Json,
    /// One JSON object per line (needs a layout)
    Jsonl,
}

pub struct View<'a> {
    pub title: String,
    pub data: &'a [u8],
    pub lrecl: usize,
    pub layout: Option<&'a Layout>,
    pub encoding: Encoding,
    pub skip: usize,
    pub limit: usize,
    pub format: Option<Format>,
    pub fields: Option<Vec<String>>,
    pub redefines: bool,
}

pub fn parse_encoding(s: &str) -> Result<Encoding, String> {
    if s.eq_ignore_ascii_case("local") || s.eq_ignore_ascii_case("ascii") {
        return Ok(Encoding::Local);
    }
    frontier_copybook::CodePage::parse(s)
        .map(Encoding::Ebcdic)
        .ok_or_else(|| format!("unknown encoding {s}: use local, 037 or 1047"))
}

fn printable(bytes: &[u8], enc: Encoding) -> String {
    enc.text(bytes)
        .chars()
        .map(|c| if (' '..='~').contains(&c) { c } else { '.' })
        .collect()
}

pub fn print(v: &View) -> Result<()> {
    let format = v.format.unwrap_or(if v.layout.is_some() {
        Format::Table
    } else {
        Format::Raw
    });
    let needs_layout = matches!(
        format,
        Format::Fields | Format::Table | Format::Json | Format::Jsonl
    );
    let layout = match (needs_layout, v.layout) {
        (true, None) => anyhow::bail!(
            "--format {format:?} needs a layout: pass --layout COPYBOOK or map the dataset in cobol/datasets.toml"
        ),
        (_, l) => l,
    };
    let total = v.data.len() / v.lrecl;
    let records = v
        .data
        .chunks(v.lrecl)
        .enumerate()
        .skip(v.skip)
        .take(v.limit);
    let opts = DecodeOptions {
        encoding: v.encoding,
        redefines: v.redefines,
        ..Default::default()
    };

    if !matches!(format, Format::Json | Format::Jsonl) {
        let using = layout
            .map(|l| format!("  LAYOUT={} {}", l.name(), l.record().display_name()))
            .unwrap_or_default();
        println!(
            "{}  LRECL={}  RECORDS={total}  ENCODING={}{using}",
            v.title, v.lrecl, v.encoding
        );
        if let Some(l) = layout
            && l.record().total_size() != v.lrecl
        {
            println!(
                "  WARNING: layout length {} differs from LRECL {}",
                l.record().total_size(),
                v.lrecl
            );
        }
    }

    match format {
        Format::Raw => {
            for (i, rec) in records {
                println!("{:>8} {}", i + 1, printable(rec, v.encoding));
            }
        }
        Format::Hex => {
            for (i, rec) in records {
                println!("{:>8}", i + 1);
                for (off, chunk) in rec.chunks(32).enumerate() {
                    let h: Vec<String> = chunk
                        .chunks(4)
                        .map(|w| w.iter().map(|b| format!("{b:02X}")).collect())
                        .collect();
                    println!(
                        "  +{:04} {:<72} |{}|",
                        off * 32,
                        h.join(" "),
                        printable(chunk, v.encoding)
                    );
                }
            }
        }
        Format::Fields => {
            let layout = layout.unwrap();
            for (i, rec) in records {
                println!("\nRECORD {} OF {total}", i + 1);
                println!(
                    "  {:>5} {:>4}  {:<30} {:<24} VALUE",
                    "POS", "LEN", "FIELD", "TYPE"
                );
                let fields = select(flatten(&decode(layout.record(), rec, &opts)), &v.fields);
                for f in fields {
                    let name = format!(
                        "{}{}",
                        "  ".repeat(f.depth.saturating_sub(1)),
                        f.path.rsplit('.').next().unwrap_or(&f.path)
                    );
                    let value = match &f.value {
                        Ok(s) => format!("{s}{}", conds(&f)),
                        Err(e) => format!("*** INVALID: {e}"),
                    };
                    println!(
                        "  {:>5} {:>4}  {:<30} {:<24} {value}",
                        f.offset + 1,
                        f.len,
                        name,
                        f.type_label
                    );
                }
            }
        }
        Format::Table => {
            let layout = layout.unwrap();
            let rows: Vec<(usize, Vec<FlatField>)> = records
                .map(|(i, rec)| {
                    (
                        i,
                        select(flatten(&decode(layout.record(), rec, &opts)), &v.fields),
                    )
                })
                .collect();
            let Some((_, first)) = rows.first() else {
                return Ok(());
            };
            let headers: Vec<&str> = first.iter().map(|f| f.path.as_str()).collect();
            let cell = |f: &FlatField| match &f.value {
                Ok(s) => s.clone(),
                Err(_) => "*INVALID*".to_string(),
            };
            let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
            for (_, row) in &rows {
                for (w, f) in widths.iter_mut().zip(row) {
                    *w = (*w).max(cell(f).chars().count()).min(40);
                }
            }
            let line = |cols: Vec<String>| -> String {
                cols.iter()
                    .zip(&widths)
                    .map(|(c, w)| format!("{:<w$.w$}", c, w = *w))
                    .collect::<Vec<_>>()
                    .join("  ")
            };
            println!(
                "{:>8}  {}",
                "REC",
                line(headers.iter().map(|h| h.to_string()).collect())
            );
            for (i, row) in &rows {
                println!("{:>8}  {}", i + 1, line(row.iter().map(cell).collect()));
            }
        }
        Format::Json => {
            let layout = layout.unwrap();
            let all: Vec<serde_json::Value> = records
                .map(|(_, rec)| decode(layout.record(), rec, &opts).to_json())
                .collect();
            println!("{}", serde_json::to_string_pretty(&all)?);
        }
        Format::Jsonl => {
            let layout = layout.unwrap();
            for (_, rec) in records {
                println!(
                    "{}",
                    serde_json::to_string(&decode(layout.record(), rec, &opts).to_json())?
                );
            }
        }
    }
    Ok(())
}

fn conds(f: &FlatField) -> String {
    if f.conditions.is_empty() {
        String::new()
    } else {
        format!("  [{}]", f.conditions.join(", "))
    }
}

/// Keep only the requested fields (by name or path), in the requested order.
fn select(fields: Vec<FlatField>, wanted: &Option<Vec<String>>) -> Vec<FlatField> {
    match wanted {
        None => fields,
        Some(names) => names
            .iter()
            .flat_map(|n| {
                fields
                    .iter()
                    .filter(move |f| {
                        f.path.eq_ignore_ascii_case(n) || f.name.eq_ignore_ascii_case(n)
                    })
                    .cloned()
            })
            .collect(),
    }
}

/// Data-quality scan: every field of every record decoded against the layout.
pub struct CheckResult {
    pub records: usize,
    pub bad_records: usize,
    pub bad_fields: usize,
    pub by_field: Vec<(String, usize)>,
    pub examples: Vec<String>,
}

pub fn check(
    layout: &Layout,
    data: &[u8],
    lrecl: usize,
    encoding: Encoding,
    max_examples: usize,
) -> CheckResult {
    let opts = DecodeOptions {
        encoding,
        ..Default::default()
    };
    let mut result = CheckResult {
        records: 0,
        bad_records: 0,
        bad_fields: 0,
        by_field: Vec::new(),
        examples: Vec::new(),
    };
    for (i, rec) in data.chunks(lrecl).enumerate() {
        result.records += 1;
        let bad: Vec<FlatField> = flatten(&decode(layout.record(), rec, &opts))
            .into_iter()
            .filter(|f| f.value.is_err())
            .collect();
        if bad.is_empty() {
            continue;
        }
        result.bad_records += 1;
        for f in bad {
            result.bad_fields += 1;
            match result.by_field.iter_mut().find(|(n, _)| *n == f.path) {
                Some((_, c)) => *c += 1,
                None => result.by_field.push((f.path.clone(), 1)),
            }
            if result.examples.len() < max_examples {
                result.examples.push(format!(
                    "record {:>7}  {:<24} pos {:>4} len {:>3}  {:<22} {}",
                    i + 1,
                    f.path,
                    f.offset + 1,
                    f.len,
                    f.type_label,
                    f.value.unwrap_err()
                ));
            }
        }
    }
    result
}
