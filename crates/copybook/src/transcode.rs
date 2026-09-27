//! Copybook-aware conversion between local (ASCII) and EBCDIC records.
//!
//! A plain code-page conversion (what an FTP "ASCII mode" transfer does)
//! destroys packed and binary fields. Driven by the layout, each field is
//! converted by what it is:
//!
//! | Field | Conversion |
//! |-------|------------|
//! | text, edited, FILLER, bytes outside the layout | code-page translation |
//! | zoned decimal | decoded and re-encoded (the sign conventions differ) |
//! | COMP-3, COMP/BINARY | copied unchanged |
//! | COMP-5 | byte order swapped when the platforms differ |
//! | COMP-1/COMP-2 | copied unchanged and counted (IEEE ↔ IBM HFP is not converted) |
//!
//! REDEFINES views are ignored: the original definition drives conversion.

use ebcdic::{Encoding, zoned};

use crate::decode::{DecodeOptions, Node, decode};
use crate::{Item, PicCategory, Usage};

#[derive(Debug, Clone, Default)]
pub struct TranscodeReport {
    pub records: u64,
    /// Zoned fields that were not valid numbers and were translated as text.
    pub invalid_numeric: u64,
    /// The first few problems, as `record N FIELD: reason`.
    pub problems: Vec<String>,
    pub floats_unconverted: u64,
}

fn translate(b: u8, from: Encoding, to: Encoding) -> u8 {
    let latin1 = match from {
        Encoding::Local => b,
        Encoding::Ebcdic(cp) => cp.decode_byte(b),
    };
    match to {
        Encoding::Local => latin1,
        Encoding::Ebcdic(cp) => cp.encode_byte(latin1),
    }
}

/// Convert every `lrecl`-byte record in `data` from one encoding to another.
pub fn transcode(
    record: &Item,
    data: &[u8],
    lrecl: usize,
    from: Encoding,
    to: Encoding,
    report: &mut TranscodeReport,
) -> Result<Vec<u8>, String> {
    if lrecl == 0 || !data.len().is_multiple_of(lrecl) {
        return Err(format!(
            "{} bytes is not a multiple of LRECL {lrecl}",
            data.len()
        ));
    }
    let opts = DecodeOptions {
        encoding: from,
        trim: false,
        redefines: false,
        fillers: true,
    };
    let mut out = Vec::with_capacity(data.len());
    for (n, rec) in data.chunks(lrecl).enumerate() {
        let mut converted: Vec<u8> = rec.iter().map(|&b| translate(b, from, to)).collect();
        let tree = decode(record, rec, &opts);
        convert_node(&tree, rec, &mut converted, from, to, n as u64 + 1, report);
        out.extend_from_slice(&converted);
        report.records += 1;
    }
    Ok(out)
}

fn convert_node(
    node: &Node,
    rec: &[u8],
    out: &mut [u8],
    from: Encoding,
    to: Encoding,
    recno: u64,
    report: &mut TranscodeReport,
) {
    match node {
        Node::Group(fields) => fields
            .iter()
            .for_each(|(_, n)| convert_node(n, rec, out, from, to, recno, report)),
        Node::Array(items) => items
            .iter()
            .for_each(|n| convert_node(n, rec, out, from, to, recno, report)),
        Node::Leaf(leaf) => {
            if leaf.raw.is_empty() {
                return;
            }
            let range = leaf.offset..leaf.offset + leaf.len;
            let item: &Item = leaf.item;
            match item.usage {
                Usage::Packed | Usage::Binary | Usage::Index | Usage::Pointer => {
                    out[range.clone()].copy_from_slice(&rec[range])
                }
                Usage::Comp5 => {
                    out[range.clone()].copy_from_slice(&rec[range.clone()]);
                    if from.comp5_big_endian() != to.comp5_big_endian() {
                        out[range].reverse();
                    }
                }
                Usage::Comp1 | Usage::Comp2 => {
                    out[range.clone()].copy_from_slice(&rec[range]);
                    report.floats_unconverted += 1;
                }
                Usage::Display => {
                    let Some(pic) = item
                        .pic
                        .as_ref()
                        .filter(|p| p.category == PicCategory::Numeric)
                    else {
                        return; // text: already translated
                    };
                    let sign = item
                        .sign
                        .map(|s| zoned::Sign {
                            leading: s.leading,
                            separate: s.separate,
                        })
                        .unwrap_or_default();
                    let result =
                        zoned::decode(&rec[range.clone()], from, pic.signed, sign, pic.scale)
                            .and_then(|d| {
                                zoned::encode(
                                    d.value,
                                    to,
                                    pic.signed,
                                    sign,
                                    &mut out[range.clone()],
                                )
                            });
                    if let Err(e) = result {
                        // Keep the text translation already in `out`, and report it.
                        report.invalid_numeric += 1;
                        if report.problems.len() < 20 {
                            report.problems.push(format!(
                                "record {recno} {} at {}: {e}",
                                item.display_name(),
                                leaf.offset + 1
                            ));
                        }
                    }
                }
            }
        }
    }
}
