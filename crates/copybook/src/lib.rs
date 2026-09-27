//! COBOL copybooks as data: parse a copybook into a byte-exact layout, then
//! decode records to JSON, list them field by field, generate JSON Schema,
//! and transcode records between local (ASCII) and EBCDIC representations.
//!
//! ```
//! use frontier_copybook::{parse, ParseOptions, DecodeOptions, decode};
//! let src = "       01  REC.\n           05  ID   PIC X(3).\n           05  AMT  PIC S9(3)V99 COMP-3.\n";
//! let book = parse("DEMO", src, &ParseOptions::default()).unwrap();
//! let rec = &book.records[0];
//! assert_eq!(rec.size, 6);
//! let json = decode(rec, b"ABC\x01\x23\x4d", &DecodeOptions::default()).to_json();
//! assert_eq!(json["AMT"], "-12.34");
//! ```

mod decode;
mod parse;
mod pic;
mod schema;
mod transcode;

use serde::Serialize;

pub use decode::{DecodeOptions, FlatField, Leaf, Node, Scalar, decode, flatten};
pub use ebcdic::{CodePage, Decimal, Encoding};
pub use parse::{CopybookError, ParseOptions, SourceFormat, parse};
pub use pic::{Pic, PicCategory, parse_pic};
pub use schema::json_schema;
pub use transcode::{TranscodeReport, transcode};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Usage {
    Display,
    /// COMP, COMP-4, BINARY: big-endian two's complement.
    Binary,
    /// COMP-5: native binary (byte order depends on the platform).
    Comp5,
    /// COMP-3, PACKED-DECIMAL.
    Packed,
    /// Single-precision float (IBM hex float on z/OS).
    Comp1,
    /// Double-precision float.
    Comp2,
    Index,
    Pointer,
}

impl Usage {
    pub fn label(self) -> &'static str {
        match self {
            Usage::Display => "DISPLAY",
            Usage::Binary => "COMP",
            Usage::Comp5 => "COMP-5",
            Usage::Packed => "COMP-3",
            Usage::Comp1 => "COMP-1",
            Usage::Comp2 => "COMP-2",
            Usage::Index => "INDEX",
            Usage::Pointer => "POINTER",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SignClause {
    pub leading: bool,
    pub separate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Occurs {
    pub min: u32,
    pub max: u32,
    pub depending_on: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Literal {
    Text(String),
    /// Numeric literal as written (`0.05`, `-1`).
    Number(String),
    /// ZERO, SPACE, HIGH-VALUE, LOW-VALUE, QUOTE, NULL.
    Figurative(String),
}

impl Literal {
    pub fn display(&self) -> String {
        match self {
            Literal::Text(s) => format!("'{s}'"),
            Literal::Number(n) => n.clone(),
            Literal::Figurative(f) => f.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CondValue {
    Single(Literal),
    Range(Literal, Literal),
}

/// A level-88 condition name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Condition {
    pub name: String,
    pub values: Vec<CondValue>,
}

/// One data description entry, with its computed layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Item {
    pub level: u8,
    /// `None` for FILLER.
    pub name: Option<String>,
    pub line: usize,
    pub pic: Option<Pic>,
    pub usage: Usage,
    #[serde(skip)]
    pub(crate) usage_explicit: bool,
    pub sign: Option<SignClause>,
    pub occurs: Option<Occurs>,
    pub redefines: Option<String>,
    pub value: Option<Literal>,
    pub conditions: Vec<Condition>,
    pub blank_when_zero: bool,
    pub justified: bool,
    pub sync: bool,
    /// A level 01 invented to wrap a copybook that starts at a lower level.
    pub synthetic: bool,
    /// Byte offset of the first occurrence from the start of the record (0-based).
    pub offset: usize,
    /// Bytes in one occurrence.
    pub size: usize,
    pub children: Vec<Item>,
}

impl Item {
    pub(crate) fn new(level: u8, name: Option<String>, line: usize) -> Self {
        Item {
            level,
            name,
            line,
            pic: None,
            usage: Usage::Display,
            usage_explicit: false,
            sign: None,
            occurs: None,
            redefines: None,
            value: None,
            conditions: Vec::new(),
            blank_when_zero: false,
            justified: false,
            sync: false,
            synthetic: false,
            offset: 0,
            size: 0,
            children: Vec::new(),
        }
    }

    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or("FILLER")
    }

    pub fn is_group(&self) -> bool {
        !self.children.is_empty()
    }

    /// Bytes for all occurrences (the maximum for OCCURS DEPENDING ON).
    pub fn total_size(&self) -> usize {
        self.size * self.occurs.as_ref().map_or(1, |o| o.max as usize)
    }

    /// `S9(11)V99 COMP-3`, `X(10)`, `GROUP`.
    pub fn type_label(&self) -> String {
        if self.is_group() {
            return "GROUP".into();
        }
        let pic = self
            .pic
            .as_ref()
            .map(|p| p.text.clone())
            .unwrap_or_default();
        let mut s = match self.usage {
            Usage::Display => pic,
            u if pic.is_empty() => u.label().to_string(),
            u => format!("{pic} {}", u.label()),
        };
        if let Some(sign) = self.sign
            && self.pic.as_ref().is_some_and(|p| p.signed)
            && self.usage == Usage::Display
        {
            s.push_str(if sign.leading {
                " LEADING"
            } else {
                " TRAILING"
            });
            if sign.separate {
                s.push_str(" SEP");
            }
        }
        s
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Copybook {
    pub name: String,
    /// Level 01 and 77 items.
    pub records: Vec<Item>,
}

impl Copybook {
    /// A record by name, or the first one.
    pub fn record(&self, name: Option<&str>) -> Option<&Item> {
        match name {
            Some(n) => self
                .records
                .iter()
                .find(|r| r.name.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(n))),
            None => self.records.first(),
        }
    }

    /// A compiler-style data map: level, name, picture/usage, offset, length.
    pub fn listing(&self) -> String {
        let mut out = String::new();
        for r in &self.records {
            out.push_str(&format!(
                "{} {}  LENGTH {}\n",
                self.name,
                r.display_name(),
                r.total_size()
            ));
            out.push_str(&format!(
                "{:<5} {:<34} {:<26} {:>6} {:>6} {:>6}  {}\n",
                "LVL", "NAME", "PICTURE / USAGE", "POS", "LEN", "OCCURS", "NOTES"
            ));
            listing_item(r, 0, &mut out);
            out.push('\n');
        }
        out
    }
}

fn listing_item(item: &Item, depth: usize, out: &mut String) {
    let indent = "  ".repeat(depth);
    let occurs = match &item.occurs {
        Some(o) if o.min != o.max => format!("{}-{}", o.min, o.max),
        Some(o) => o.max.to_string(),
        None => String::new(),
    };
    let mut notes = Vec::new();
    if let Some(r) = &item.redefines {
        notes.push(format!("REDEFINES {r}"));
    }
    if let Some(o) = &item.occurs
        && let Some(d) = &o.depending_on
    {
        notes.push(format!("DEPENDING ON {d}"));
    }
    if let Some(v) = &item.value {
        notes.push(format!("VALUE {}", v.display()));
    }
    let name = format!("{indent}{}", item.display_name());
    out.push_str(&format!(
        "{:<5} {:<34} {:<26} {:>6} {:>6} {:>6}  {}\n",
        format!("{:02}", item.level),
        name,
        item.type_label(),
        item.offset + 1,
        item.size,
        occurs,
        notes.join("; ")
    ));
    for c in &item.conditions {
        let values: Vec<String> = c
            .values
            .iter()
            .map(|v| match v {
                CondValue::Single(l) => l.display(),
                CondValue::Range(a, b) => format!("{} THRU {}", a.display(), b.display()),
            })
            .collect();
        out.push_str(&format!(
            "88    {indent}  {:<32} VALUE {}\n",
            c.name,
            values.join(" ")
        ));
    }
    for child in &item.children {
        listing_item(child, depth + 1, out);
    }
}
