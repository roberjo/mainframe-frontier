//! Record decoding.

use std::collections::HashMap;

use ebcdic::{Decimal, Encoding, binary, float, packed, zoned};
use serde_json::{Map, Value};

use crate::{CondValue, Item, Literal, PicCategory, Usage};

#[derive(Debug, Clone, Copy)]
pub struct DecodeOptions {
    pub encoding: Encoding,
    /// Trim trailing spaces from text fields.
    pub trim: bool,
    /// Include REDEFINES views (they may not all be valid for a given record).
    pub redefines: bool,
    /// Include FILLER items.
    pub fillers: bool,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            encoding: Encoding::Local,
            trim: true,
            redefines: false,
            fillers: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    Text(String),
    Number(Decimal),
    Float(f64),
    /// Opaque bytes (pointers, indexes), as hex.
    Hex(String),
}

impl Scalar {
    pub fn display(&self) -> String {
        match self {
            Scalar::Text(s) | Scalar::Hex(s) => s.clone(),
            Scalar::Number(d) => d.to_string(),
            Scalar::Float(f) => f.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Leaf<'a> {
    pub item: &'a Item,
    /// Absolute byte offset in the record (0-based).
    pub offset: usize,
    pub len: usize,
    pub raw: &'a [u8],
    /// `Err` holds why the bytes are not valid for the PICTURE/USAGE.
    pub value: Result<Scalar, String>,
}

#[derive(Debug, Clone)]
pub enum Node<'a> {
    Group(Vec<(String, Node<'a>)>),
    Array(Vec<Node<'a>>),
    Leaf(Leaf<'a>),
}

struct Ctx<'a> {
    opts: DecodeOptions,
    bytes: &'a [u8],
    /// Numeric values seen so far, for OCCURS DEPENDING ON.
    counters: HashMap<&'a str, i128>,
}

/// Decode one record.
pub fn decode<'a>(record: &'a Item, bytes: &'a [u8], opts: &DecodeOptions) -> Node<'a> {
    let mut ctx = Ctx {
        opts: *opts,
        bytes,
        counters: HashMap::new(),
    };
    decode_item(record, 0, &mut ctx)
}

fn decode_item<'a>(item: &'a Item, shift: usize, ctx: &mut Ctx<'a>) -> Node<'a> {
    if !item.is_group() {
        return Node::Leaf(decode_leaf(item, item.offset + shift, ctx));
    }
    let mut fields: Vec<(String, Node<'a>)> = Vec::new();
    for child in &item.children {
        if child.redefines.is_some() && !ctx.opts.redefines {
            continue;
        }
        if child.name.is_none() && !ctx.opts.fillers {
            continue;
        }
        let node = match &child.occurs {
            Some(o) => {
                let count = match &o.depending_on {
                    Some(d) => {
                        let n = ctx
                            .counters
                            .get(d.as_str())
                            .copied()
                            .unwrap_or(o.max as i128);
                        n.clamp(o.min as i128, o.max as i128) as usize
                    }
                    None => o.max as usize,
                };
                Node::Array(
                    (0..count)
                        .map(|k| decode_item(child, shift + k * child.size, ctx))
                        .collect(),
                )
            }
            None => decode_item(child, shift, ctx),
        };
        let mut key = child.display_name().to_string();
        if fields.iter().any(|(k, _)| *k == key) {
            let n = fields.iter().filter(|(k, _)| k.starts_with(&key)).count() + 1;
            key = format!("{key}#{n}");
        }
        fields.push((key, node));
    }
    Node::Group(fields)
}

fn decode_leaf<'a>(item: &'a Item, offset: usize, ctx: &mut Ctx<'a>) -> Leaf<'a> {
    let len = item.size;
    let Some(raw) = ctx.bytes.get(offset..offset + len) else {
        return Leaf {
            item,
            offset,
            len,
            raw: &[],
            value: Err("record too short".into()),
        };
    };
    let value = decode_scalar(item, raw, &ctx.opts);
    if let (Ok(Scalar::Number(d)), Some(name)) = (&value, &item.name) {
        ctx.counters.insert(name.as_str(), d.rescale(0).value);
    }
    Leaf {
        item,
        offset,
        len,
        raw,
        value,
    }
}

fn decode_scalar(item: &Item, raw: &[u8], opts: &DecodeOptions) -> Result<Scalar, String> {
    let enc = opts.encoding;
    let pic = item.pic.as_ref();
    let (signed, scale) = pic.map_or((false, 0), |p| (p.signed, p.scale));
    let hex = || raw.iter().map(|b| format!("{b:02X}")).collect::<String>();
    let describe = |e: ebcdic::NumError| format!("{e} (hex {})", hex());
    match item.usage {
        Usage::Packed => packed::decode(raw, scale)
            .map(Scalar::Number)
            .map_err(describe),
        Usage::Binary => binary::decode(raw, true, signed, scale)
            .map(Scalar::Number)
            .map_err(describe),
        Usage::Comp5 => binary::decode(raw, enc.comp5_big_endian(), signed, scale)
            .map(Scalar::Number)
            .map_err(describe),
        Usage::Comp1 | Usage::Comp2 => {
            let v = match enc {
                Encoding::Local => float::decode_ieee_le(raw),
                Encoding::Ebcdic(_) => float::decode_ibm_hfp(raw),
            };
            v.map(Scalar::Float)
                .ok_or_else(|| format!("invalid float (hex {})", hex()))
        }
        Usage::Index | Usage::Pointer => Ok(Scalar::Hex(hex())),
        Usage::Display => match pic.map(|p| p.category) {
            Some(PicCategory::Numeric) => {
                let sign = item
                    .sign
                    .map(|s| zoned::Sign {
                        leading: s.leading,
                        separate: s.separate,
                    })
                    .unwrap_or_default();
                if item.blank_when_zero && raw.iter().all(|&b| b == enc.space()) {
                    return Ok(Scalar::Number(Decimal::new(0, scale)));
                }
                zoned::decode(raw, enc, signed, sign, scale)
                    .map(Scalar::Number)
                    .map_err(describe)
            }
            _ => {
                let text = enc.text(raw);
                Ok(Scalar::Text(if opts.trim {
                    text.trim_end_matches(' ').to_string()
                } else {
                    text
                }))
            }
        },
    }
}

impl Node<'_> {
    /// JSON with COBOL names as keys. Numbers with decimal places are strings
    /// (`"4969.00"`) so no precision is lost to IEEE doubles; integers are
    /// JSON numbers when they fit in 53 bits. Invalid fields are `null`.
    pub fn to_json(&self) -> Value {
        match self {
            Node::Group(fields) => {
                let mut map = Map::new();
                for (k, v) in fields {
                    map.insert(k.clone(), v.to_json());
                }
                Value::Object(map)
            }
            Node::Array(items) => Value::Array(items.iter().map(Node::to_json).collect()),
            Node::Leaf(leaf) => match &leaf.value {
                Ok(Scalar::Text(s)) | Ok(Scalar::Hex(s)) => Value::String(s.clone()),
                Ok(Scalar::Number(d)) if d.scale <= 0 => {
                    let v = d.rescale(0).value;
                    if v.unsigned_abs() < (1u128 << 53) {
                        Value::from(v as i64)
                    } else {
                        Value::String(v.to_string())
                    }
                }
                Ok(Scalar::Number(d)) => Value::String(d.to_string()),
                Ok(Scalar::Float(f)) => {
                    serde_json::Number::from_f64(*f).map_or(Value::Null, Value::Number)
                }
                Err(_) => Value::Null,
            },
        }
    }
}

/// One elementary field of a decoded record, in layout order.
#[derive(Debug, Clone)]
pub struct FlatField {
    /// `TXN(2).AMOUNT`
    pub path: String,
    pub name: String,
    pub depth: usize,
    pub offset: usize,
    pub len: usize,
    pub type_label: String,
    pub value: Result<String, String>,
    /// Level-88 condition names that are true for this value.
    pub conditions: Vec<String>,
    pub raw_hex: String,
}

/// Flatten a decoded record into its elementary fields.
pub fn flatten(node: &Node) -> Vec<FlatField> {
    let mut out = Vec::new();
    flatten_into(node, "", 0, &mut out);
    out
}

fn flatten_into(node: &Node, path: &str, depth: usize, out: &mut Vec<FlatField>) {
    match node {
        Node::Group(fields) => {
            for (k, v) in fields {
                let p = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                flatten_into(v, &p, depth + 1, out);
            }
        }
        Node::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                flatten_into(v, &format!("{path}({})", i + 1), depth, out);
            }
        }
        Node::Leaf(leaf) => out.push(FlatField {
            name: path.rsplit('.').next().unwrap_or(path).to_string(),
            path: path.to_string(),
            depth,
            offset: leaf.offset,
            len: leaf.len,
            type_label: leaf.item.type_label(),
            value: leaf
                .value
                .as_ref()
                .map(Scalar::display)
                .map_err(Clone::clone),
            conditions: true_conditions(leaf),
            raw_hex: leaf.raw.iter().map(|b| format!("{b:02X}")).collect(),
        }),
    }
}

fn true_conditions(leaf: &Leaf) -> Vec<String> {
    let Ok(value) = &leaf.value else {
        return Vec::new();
    };
    leaf.item
        .conditions
        .iter()
        .filter(|c| {
            c.values.iter().any(|v| match v {
                CondValue::Single(l) => {
                    compare(value, l, leaf.raw) == Some(std::cmp::Ordering::Equal)
                }
                CondValue::Range(lo, hi) => {
                    compare(value, lo, leaf.raw).is_some_and(|o| o.is_ge())
                        && compare(value, hi, leaf.raw).is_some_and(|o| o.is_le())
                }
            })
        })
        .map(|c| c.name.clone())
        .collect()
}

fn compare(value: &Scalar, lit: &Literal, raw: &[u8]) -> Option<std::cmp::Ordering> {
    match (value, lit) {
        (Scalar::Number(d), Literal::Number(n)) => {
            let n = Decimal::parse(n)?;
            let scale = d.scale.max(n.scale);
            Some(d.rescale(scale).value.cmp(&n.rescale(scale).value))
        }
        (Scalar::Number(d), Literal::Figurative(f)) if f == "ZERO" => Some(d.value.cmp(&0)),
        (Scalar::Text(s), Literal::Text(t)) => Some(s.trim_end().cmp(t.trim_end())),
        (Scalar::Text(s), Literal::Figurative(f)) => match f.as_str() {
            "SPACE" => Some(if s.trim().is_empty() {
                std::cmp::Ordering::Equal
            } else {
                std::cmp::Ordering::Greater
            }),
            "ZERO" => Some(if !s.is_empty() && s.chars().all(|c| c == '0') {
                std::cmp::Ordering::Equal
            } else {
                std::cmp::Ordering::Greater
            }),
            "HIGH-VALUE" => Some(if raw.iter().all(|&b| b == 0xff) {
                std::cmp::Ordering::Equal
            } else {
                std::cmp::Ordering::Less
            }),
            "LOW-VALUE" => Some(if raw.iter().all(|&b| b == 0) {
                std::cmp::Ordering::Equal
            } else {
                std::cmp::Ordering::Greater
            }),
            _ => None,
        },
        _ => None,
    }
}
