//! EBCDIC code pages and mainframe numeric formats.
//!
//! Two data "profiles" matter when reading COBOL records:
//!
//! | | [`Encoding::Local`] (GnuCOBOL on Linux/macOS) | [`Encoding::Ebcdic`] (z/OS) |
//! |---|---|---|
//! | Text | ISO-8859-1 | IBM-037 or IBM-1047 |
//! | Zoned sign | negative digit `d` stored as `0x70 + d` (`p`..`y`) | zone nibble `C`/`F` positive, `D` negative |
//! | `COMP` / `BINARY` | big-endian | big-endian |
//! | `COMP-5` | native (little-endian) | big-endian |
//! | `COMP-3` | identical | identical |
//!
//! Packed decimal is byte-identical on both, which is why it survives a
//! text-mode file transfer so badly: the transfer translates it as if it
//! were characters.

mod tables;

use std::fmt;

// ---------------------------------------------------------------------------
// Code pages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CodePage {
    /// IBM-037: US/Canada, the traditional z/OS default for MVS datasets.
    Cp037,
    /// IBM-1047: Latin-1 Open Systems, the z/OS UNIX default.
    Cp1047,
}

struct Tables {
    to_ebcdic: [u8; 256],
    to_latin1: [u8; 256],
}

const fn invert(t: &[u8; 256]) -> [u8; 256] {
    let mut out = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        out[t[i] as usize] = i as u8;
        i += 1;
    }
    out
}

static CP037: Tables = Tables {
    to_ebcdic: tables::LATIN1_TO_CP037,
    to_latin1: invert(&tables::LATIN1_TO_CP037),
};
static CP1047: Tables = Tables {
    to_ebcdic: tables::LATIN1_TO_CP1047,
    to_latin1: invert(&tables::LATIN1_TO_CP1047),
};

impl CodePage {
    /// Accepts `37`, `037`, `IBM-037`, `cp037`, `1047`, `IBM1047`, ...
    pub fn parse(s: &str) -> Option<Self> {
        let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
        match digits.trim_start_matches('0') {
            "37" => Some(CodePage::Cp037),
            "1047" => Some(CodePage::Cp1047),
            _ => None,
        }
    }

    pub fn ccsid(self) -> u16 {
        match self {
            CodePage::Cp037 => 37,
            CodePage::Cp1047 => 1047,
        }
    }

    fn tables(self) -> &'static Tables {
        match self {
            CodePage::Cp037 => &CP037,
            CodePage::Cp1047 => &CP1047,
        }
    }

    /// ISO-8859-1 byte → EBCDIC byte.
    pub fn encode_byte(self, b: u8) -> u8 {
        self.tables().to_ebcdic[b as usize]
    }

    /// EBCDIC byte → ISO-8859-1 byte.
    pub fn decode_byte(self, b: u8) -> u8 {
        self.tables().to_latin1[b as usize]
    }

    pub fn encode_in_place(self, bytes: &mut [u8]) {
        let t = &self.tables().to_ebcdic;
        bytes.iter_mut().for_each(|b| *b = t[*b as usize]);
    }

    pub fn decode_in_place(self, bytes: &mut [u8]) {
        let t = &self.tables().to_latin1;
        bytes.iter_mut().for_each(|b| *b = t[*b as usize]);
    }
}

impl fmt::Display for CodePage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IBM-{:03}", self.ccsid())
    }
}

/// Where record bytes came from; decides text, zoned signs and COMP-5 byte order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Encoding {
    /// GnuCOBOL data on this machine: ISO-8859-1, `0x70+d` negative zoned, little-endian COMP-5.
    Local,
    /// z/OS data.
    Ebcdic(CodePage),
}

impl Encoding {
    /// Decode text bytes to a string (both code pages cover exactly ISO-8859-1).
    pub fn text(self, bytes: &[u8]) -> String {
        match self {
            Encoding::Local => bytes.iter().map(|&b| b as char).collect(),
            Encoding::Ebcdic(cp) => bytes.iter().map(|&b| cp.decode_byte(b) as char).collect(),
        }
    }

    /// Encode a string into text bytes; characters outside ISO-8859-1 become `?`.
    pub fn encode_text(self, s: &str) -> Vec<u8> {
        s.chars()
            .map(|c| {
                let b = u8::try_from(u32::from(c)).unwrap_or(b'?');
                match self {
                    Encoding::Local => b,
                    Encoding::Ebcdic(cp) => cp.encode_byte(b),
                }
            })
            .collect()
    }

    pub fn space(self) -> u8 {
        match self {
            Encoding::Local => b' ',
            Encoding::Ebcdic(cp) => cp.encode_byte(b' '),
        }
    }

    /// COMP-5 is native binary: little-endian for local data, big-endian on z/OS.
    pub fn comp5_big_endian(self) -> bool {
        matches!(self, Encoding::Ebcdic(_))
    }
}

impl fmt::Display for Encoding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Encoding::Local => f.write_str("LOCAL (ISO-8859-1)"),
            Encoding::Ebcdic(cp) => write!(f, "EBCDIC {cp}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Decimal
// ---------------------------------------------------------------------------

/// An exact decimal: `value × 10^-scale`. Negative scale comes from `P`
/// scaling positions (`PIC 99PPP`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Decimal {
    pub value: i128,
    pub scale: i32,
}

impl Decimal {
    pub fn new(value: i128, scale: i32) -> Self {
        Self { value, scale }
    }

    /// The same number expressed at another scale (truncating extra digits).
    pub fn rescale(self, scale: i32) -> Self {
        let diff = scale - self.scale;
        let value = if diff >= 0 {
            self.value * 10i128.pow(diff as u32)
        } else {
            self.value / 10i128.pow((-diff) as u32)
        };
        Self { value, scale }
    }

    /// Parse `-123.45` / `+7` / `0.5` at its natural scale.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let (neg, body) = match s.as_bytes().first()? {
            b'-' => (true, &s[1..]),
            b'+' => (false, &s[1..]),
            _ => (false, s),
        };
        let (int, frac) = body.split_once('.').unwrap_or((body, ""));
        if int.is_empty() && frac.is_empty() {
            return None;
        }
        if !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit()) {
            return None;
        }
        let digits = format!("{int}{frac}");
        let value: i128 = if digits.is_empty() {
            0
        } else {
            digits.parse().ok()?
        };
        Some(Self {
            value: if neg { -value } else { value },
            scale: frac.len() as i32,
        })
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.scale <= 0 {
            return write!(f, "{}", self.value * 10i128.pow((-self.scale) as u32));
        }
        let scale = self.scale as usize;
        let digits = self.value.unsigned_abs().to_string();
        let padded = format!("{digits:0>width$}", width = scale + 1);
        let (int, frac) = padded.split_at(padded.len() - scale);
        let sign = if self.value < 0 { "-" } else { "" };
        write!(f, "{sign}{int}.{frac}")
    }
}

/// Why bytes are not a valid number. On z/OS most of these are an S0C7.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NumError {
    /// A digit position (byte offset within the field) holds a non-digit.
    InvalidDigit { offset: usize },
    /// The sign nibble / overpunch / separate sign character is invalid.
    InvalidSign { offset: usize },
    /// The value does not fit the target field.
    Overflow,
    /// The field length is wrong for the format.
    Length,
}

impl fmt::Display for NumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NumError::InvalidDigit { offset } => write!(f, "invalid digit at byte {offset}"),
            NumError::InvalidSign { offset } => write!(f, "invalid sign at byte {offset}"),
            NumError::Overflow => f.write_str("value does not fit"),
            NumError::Length => f.write_str("invalid length"),
        }
    }
}

impl std::error::Error for NumError {}

fn pow10(n: u32) -> i128 {
    10i128.pow(n)
}

// ---------------------------------------------------------------------------
// Packed decimal (COMP-3)
// ---------------------------------------------------------------------------

pub mod packed {
    use super::{Decimal, NumError, pow10};

    /// Bytes needed for `digits` digits.
    pub fn len(digits: u32) -> usize {
        digits as usize / 2 + 1
    }

    /// Strict decode: every digit nibble 0-9 and a sign nibble A-F, exactly
    /// the check that raises S0C7 on z/OS. `A`/`C`/`E`/`F` are positive,
    /// `B`/`D` negative.
    pub fn decode(bytes: &[u8], scale: i32) -> Result<Decimal, NumError> {
        if bytes.is_empty() || bytes.len() > 16 {
            return Err(NumError::Length);
        }
        let mut value: i128 = 0;
        let last = bytes.len() - 1;
        for (i, &b) in bytes.iter().enumerate() {
            let (hi, lo) = (b >> 4, b & 0x0f);
            if hi > 9 {
                return Err(NumError::InvalidDigit { offset: i });
            }
            value = value * 10 + hi as i128;
            if i < last {
                if lo > 9 {
                    return Err(NumError::InvalidDigit { offset: i });
                }
                value = value * 10 + lo as i128;
            } else {
                match lo {
                    0x0a | 0x0c | 0x0e | 0x0f => {}
                    0x0b | 0x0d => value = -value,
                    _ => return Err(NumError::InvalidSign { offset: i }),
                }
            }
        }
        Ok(Decimal::new(value, scale))
    }

    /// Encode `value` (already at the field's scale) into `out`, using
    /// sign `C`/`D` when signed and `F` when unsigned (the IBM preferred signs).
    pub fn encode(value: i128, signed: bool, out: &mut [u8]) -> Result<(), NumError> {
        if out.is_empty() || out.len() > 16 {
            return Err(NumError::Length);
        }
        if !signed && value < 0 {
            return Err(NumError::Overflow);
        }
        let digits = out.len() * 2 - 1;
        let mag = value.unsigned_abs();
        if mag >= pow10(digits as u32) as u128 {
            return Err(NumError::Overflow);
        }
        let sign = match (signed, value < 0) {
            (false, _) => 0x0f,
            (true, false) => 0x0c,
            (true, true) => 0x0d,
        };
        let text = format!("{mag:0>digits$}");
        let mut nibbles: Vec<u8> = text.bytes().map(|b| b - b'0').collect();
        nibbles.push(sign);
        for (i, pair) in nibbles.chunks(2).enumerate() {
            out[i] = (pair[0] << 4) | pair[1];
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Zoned decimal (DISPLAY numeric)
// ---------------------------------------------------------------------------

pub mod zoned {
    use super::{Decimal, Encoding, NumError, pow10};

    /// Where the sign lives in a signed DISPLAY numeric field.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Sign {
        pub leading: bool,
        pub separate: bool,
    }

    fn digit_of(enc: Encoding, b: u8) -> Option<u8> {
        match enc {
            Encoding::Local => b.is_ascii_digit().then(|| b - b'0'),
            Encoding::Ebcdic(_) => (b >> 4 == 0xf && b & 0x0f <= 9).then_some(b & 0x0f),
        }
    }

    /// The digit and sign carried by an overpunched sign byte.
    fn overpunch(enc: Encoding, b: u8) -> Option<(u8, bool)> {
        match enc {
            Encoding::Local => match b {
                b'0'..=b'9' => Some((b - b'0', false)),
                0x70..=0x79 => Some((b - 0x70, true)), // GnuCOBOL: 'p'..'y'
                b'{' => Some((0, false)),              // EBCDIC-style, as seen after text transfers
                b'A'..=b'I' => Some((b - b'A' + 1, false)),
                b'}' => Some((0, true)),
                b'J'..=b'R' => Some((b - b'J' + 1, true)),
                _ => None,
            },
            Encoding::Ebcdic(_) => {
                let (zone, d) = (b >> 4, b & 0x0f);
                if d > 9 {
                    return None;
                }
                match zone {
                    0xa | 0xc | 0xe | 0xf => Some((d, false)),
                    0xb | 0xd => Some((d, true)),
                    _ => None,
                }
            }
        }
    }

    fn separate_sign(enc: Encoding, b: u8) -> Option<bool> {
        let b = match enc {
            Encoding::Local => b,
            Encoding::Ebcdic(cp) => cp.decode_byte(b),
        };
        match b {
            b'+' => Some(false),
            b'-' => Some(true),
            _ => None,
        }
    }

    /// Decode a DISPLAY numeric field. `signed` = the PICTURE has an `S`.
    pub fn decode(
        bytes: &[u8],
        enc: Encoding,
        signed: bool,
        sign: Sign,
        scale: i32,
    ) -> Result<Decimal, NumError> {
        if bytes.is_empty() || bytes.len() > 38 {
            return Err(NumError::Length);
        }
        let mut negative = false;
        let (start, end) = if signed && sign.separate {
            if bytes.len() < 2 {
                return Err(NumError::Length);
            }
            let pos = if sign.leading { 0 } else { bytes.len() - 1 };
            negative =
                separate_sign(enc, bytes[pos]).ok_or(NumError::InvalidSign { offset: pos })?;
            if sign.leading {
                (1, bytes.len())
            } else {
                (0, bytes.len() - 1)
            }
        } else {
            (0, bytes.len())
        };
        let sign_pos = if signed && !sign.separate {
            Some(if sign.leading { start } else { end - 1 })
        } else {
            None
        };

        let mut value: i128 = 0;
        for (i, &b) in bytes.iter().enumerate().take(end).skip(start) {
            let d = if Some(i) == sign_pos {
                let (d, neg) = overpunch(enc, b).ok_or(NumError::InvalidSign { offset: i })?;
                negative = neg;
                d
            } else if !signed && i == end - 1 && matches!(enc, Encoding::Ebcdic(_)) {
                // Unsigned z/OS fields still carry a zone; accept C/F.
                match overpunch(enc, b) {
                    Some((d, false)) => d,
                    _ => return Err(NumError::InvalidDigit { offset: i }),
                }
            } else {
                digit_of(enc, b).ok_or(NumError::InvalidDigit { offset: i })?
            };
            value = value * 10 + d as i128;
        }
        Ok(Decimal::new(if negative { -value } else { value }, scale))
    }

    /// Encode `value` (at the field's scale) into `out`, the full field width.
    pub fn encode(
        value: i128,
        enc: Encoding,
        signed: bool,
        sign: Sign,
        out: &mut [u8],
    ) -> Result<(), NumError> {
        if !signed && value < 0 {
            return Err(NumError::Overflow);
        }
        let digits = if signed && sign.separate {
            out.len().checked_sub(1).ok_or(NumError::Length)?
        } else {
            out.len()
        };
        if digits == 0 || digits > 37 {
            return Err(NumError::Length);
        }
        let mag = value.unsigned_abs();
        if mag >= pow10(digits as u32) as u128 {
            return Err(NumError::Overflow);
        }
        let text = format!("{mag:0>digits$}");
        let negative = value < 0;
        let digit_byte = |d: u8| match enc {
            Encoding::Local => b'0' + d,
            Encoding::Ebcdic(_) => 0xf0 | d,
        };
        let mut body: Vec<u8> = text.bytes().map(|b| digit_byte(b - b'0')).collect();
        if signed && !sign.separate {
            let pos = if sign.leading { 0 } else { body.len() - 1 };
            let d = text.as_bytes()[pos] - b'0';
            body[pos] = match enc {
                Encoding::Local if negative => 0x70 + d,
                Encoding::Local => b'0' + d,
                Encoding::Ebcdic(_) => (if negative { 0xd0 } else { 0xc0 }) | d,
            };
        }
        if signed && sign.separate {
            let ch = if negative { b'-' } else { b'+' };
            let ch = match enc {
                Encoding::Local => ch,
                Encoding::Ebcdic(cp) => cp.encode_byte(ch),
            };
            if sign.leading {
                out[0] = ch;
                out[1..].copy_from_slice(&body);
            } else {
                let n = out.len();
                out[..n - 1].copy_from_slice(&body);
                out[n - 1] = ch;
            }
        } else {
            out.copy_from_slice(&body);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Binary (COMP, COMP-4, BINARY, COMP-5)
// ---------------------------------------------------------------------------

pub mod binary {
    use super::{Decimal, NumError};

    /// Storage bytes for `digits` digits: 1-4 → 2, 5-9 → 4, 10-18 → 8.
    pub fn len(digits: u32) -> usize {
        match digits {
            0..=4 => 2,
            5..=9 => 4,
            _ => 8,
        }
    }

    pub fn decode(
        bytes: &[u8],
        big_endian: bool,
        signed: bool,
        scale: i32,
    ) -> Result<Decimal, NumError> {
        if !matches!(bytes.len(), 1 | 2 | 4 | 8) {
            return Err(NumError::Length);
        }
        let mut buf = [0u8; 8];
        let n = bytes.len();
        if big_endian {
            buf[8 - n..].copy_from_slice(bytes);
        } else {
            let mut rev = bytes.to_vec();
            rev.reverse();
            buf[8 - n..].copy_from_slice(&rev);
        }
        let raw = u64::from_be_bytes(buf);
        let value = if signed {
            // Sign-extend from the field width.
            let shift = 64 - 8 * n as u32;
            (((raw << shift) as i64) >> shift) as i128
        } else {
            raw as i128
        };
        Ok(Decimal::new(value, scale))
    }

    pub fn encode(
        value: i128,
        big_endian: bool,
        signed: bool,
        out: &mut [u8],
    ) -> Result<(), NumError> {
        let n = out.len();
        if !matches!(n, 1 | 2 | 4 | 8) {
            return Err(NumError::Length);
        }
        let bits = 8 * n as u32;
        let fits = if signed {
            value >= -(1i128 << (bits - 1)) && value < (1i128 << (bits - 1))
        } else {
            value >= 0 && value < (1i128 << bits)
        };
        if !fits {
            return Err(NumError::Overflow);
        }
        let bytes = (value as i64).to_be_bytes();
        out.copy_from_slice(&bytes[8 - n..]);
        if !big_endian {
            out.reverse();
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Floating point (COMP-1, COMP-2)
// ---------------------------------------------------------------------------

pub mod float {
    /// IBM System/360 hexadecimal floating point (z/OS COMP-1/COMP-2):
    /// sign bit, 7-bit base-16 exponent excess 64, then the fraction.
    pub fn decode_ibm_hfp(bytes: &[u8]) -> Option<f64> {
        if !matches!(bytes.len(), 4 | 8) {
            return None;
        }
        let negative = bytes[0] & 0x80 != 0;
        let exponent = (bytes[0] & 0x7f) as i32 - 64;
        let mut fraction = 0f64;
        let mut scale = 1.0 / 256.0;
        for &b in &bytes[1..] {
            fraction += b as f64 * scale;
            scale /= 256.0;
        }
        let v = fraction * 16f64.powi(exponent);
        Some(if negative { -v } else { v })
    }

    /// IEEE 754 in native (little-endian) order, as GnuCOBOL stores it locally.
    pub fn decode_ieee_le(bytes: &[u8]) -> Option<f64> {
        match bytes.len() {
            4 => Some(f32::from_le_bytes(bytes.try_into().ok()?) as f64),
            8 => Some(f64::from_le_bytes(bytes.try_into().ok()?)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::zoned::Sign;
    use super::*;

    const E37: Encoding = Encoding::Ebcdic(CodePage::Cp037);

    #[test]
    fn code_pages_are_bijections_with_known_points() {
        for cp in [CodePage::Cp037, CodePage::Cp1047] {
            for b in 0..=255u8 {
                assert_eq!(cp.decode_byte(cp.encode_byte(b)), b, "{cp} {b:#04x}");
            }
            assert_eq!(cp.encode_byte(b' '), 0x40);
            assert_eq!(cp.encode_byte(b'A'), 0xC1);
            assert_eq!(cp.encode_byte(b'a'), 0x81);
            assert_eq!(cp.encode_byte(b'0'), 0xF0);
            assert_eq!(cp.encode_byte(b'{'), 0xC0);
            assert_eq!(cp.encode_byte(b'}'), 0xD0);
        }
        assert_eq!(CodePage::Cp037.encode_byte(b'['), 0xBA);
        assert_eq!(CodePage::Cp1047.encode_byte(b'['), 0xAD);
        let differing = (0..=255u8)
            .filter(|&b| CodePage::Cp037.encode_byte(b) != CodePage::Cp1047.encode_byte(b))
            .count();
        assert_eq!(differing, 6, "037 and 1047 differ only in [ ] ^ ¬ Ý ¨");
    }

    #[test]
    fn parses_code_page_names() {
        assert_eq!(CodePage::parse("IBM-037"), Some(CodePage::Cp037));
        assert_eq!(CodePage::parse("37"), Some(CodePage::Cp037));
        assert_eq!(CodePage::parse("cp1047"), Some(CodePage::Cp1047));
        assert_eq!(CodePage::parse("500"), None);
    }

    #[test]
    fn decimal_display_and_parse() {
        assert_eq!(Decimal::new(496900, 2).to_string(), "4969.00");
        assert_eq!(Decimal::new(-5, 2).to_string(), "-0.05");
        assert_eq!(Decimal::new(12, -3).to_string(), "12000");
        assert_eq!(Decimal::new(-598, 6).to_string(), "-0.000598");
        assert_eq!(Decimal::parse("-12.340"), Some(Decimal::new(-12340, 3)));
        assert_eq!(
            Decimal::parse("1.5").unwrap().rescale(2),
            Decimal::new(150, 2)
        );
        assert_eq!(Decimal::parse("x1"), None);
    }

    #[test]
    fn packed_matches_what_gnucobol_wrote() {
        // From the Phase 1 probe: MOVE -123.45 TO PIC S9(5)V99 COMP-3 → 00 12 34 5D
        assert_eq!(
            packed::decode(&[0x00, 0x12, 0x34, 0x5d], 2),
            Ok(Decimal::new(-12345, 2))
        );
        // From ACCTMAST: +4969.00 in S9(11)V99
        assert_eq!(
            packed::decode(&[0, 0, 0, 0x04, 0x96, 0x90, 0x0c], 2),
            Ok(Decimal::new(496900, 2))
        );
        let mut out = [0u8; 4];
        packed::encode(-12345, true, &mut out).unwrap();
        assert_eq!(out, [0x00, 0x12, 0x34, 0x5d]);
        packed::encode(7, false, &mut out[..2]).unwrap();
        assert_eq!(&out[..2], &[0x00, 0x7f]);
        assert_eq!(packed::len(13), 7);
    }

    #[test]
    fn packed_rejects_s0c7_data() {
        assert_eq!(
            packed::decode(&[0x5a, 0x5a], 0),
            Err(NumError::InvalidDigit { offset: 0 })
        );
        assert_eq!(
            packed::decode(&[0x12, 0x34], 0),
            Err(NumError::InvalidSign { offset: 1 })
        );
        assert_eq!(
            packed::decode(&[0x40, 0x40, 0x40], 0),
            Err(NumError::InvalidSign { offset: 2 })
        );
        assert!(packed::encode(1000, true, &mut [0u8; 2]).is_err());
    }

    #[test]
    fn zoned_local_matches_gnucobol() {
        // Probe: -123 in S9(3) → "12s"; +120 → "120"; -45 SIGN LEADING → "p45"; -6 TRAILING SEPARATE → "006-"
        let trailing = Sign::default();
        let leading = Sign {
            leading: true,
            separate: false,
        };
        let sep = Sign {
            leading: false,
            separate: true,
        };
        let l = Encoding::Local;
        assert_eq!(
            zoned::decode(b"12s", l, true, trailing, 0),
            Ok(Decimal::new(-123, 0))
        );
        assert_eq!(
            zoned::decode(b"120", l, true, trailing, 0),
            Ok(Decimal::new(120, 0))
        );
        assert_eq!(
            zoned::decode(b"p45", l, true, leading, 0),
            Ok(Decimal::new(-45, 0))
        );
        assert_eq!(
            zoned::decode(b"006-", l, true, sep, 0),
            Ok(Decimal::new(-6, 0))
        );
        assert_eq!(
            zoned::decode(
                b"+0000001234567",
                l,
                true,
                Sign {
                    leading: true,
                    separate: true
                },
                2
            ),
            Ok(Decimal::new(1234567, 2))
        );

        let mut out = [0u8; 3];
        zoned::encode(-123, l, true, trailing, &mut out).unwrap();
        assert_eq!(&out, b"12s");
        zoned::encode(-45, l, true, leading, &mut out).unwrap();
        assert_eq!(&out, b"p45");
        let mut out4 = [0u8; 4];
        zoned::encode(-6, l, true, sep, &mut out4).unwrap();
        assert_eq!(&out4, b"006-");
    }

    #[test]
    fn zoned_ebcdic() {
        let t = Sign::default();
        assert_eq!(
            zoned::decode(&[0xf1, 0xf2, 0xd3], E37, true, t, 0),
            Ok(Decimal::new(-123, 0))
        );
        assert_eq!(
            zoned::decode(&[0xf1, 0xf2, 0xc3], E37, true, t, 1),
            Ok(Decimal::new(123, 1))
        );
        assert_eq!(
            zoned::decode(&[0xf1, 0xf2, 0xf3], E37, false, t, 0),
            Ok(Decimal::new(123, 0))
        );
        assert_eq!(
            zoned::decode(&[0xf1, 0x40, 0xf3], E37, false, t, 0),
            Err(NumError::InvalidDigit { offset: 1 })
        );
        let mut out = [0u8; 3];
        zoned::encode(-123, E37, true, t, &mut out).unwrap();
        assert_eq!(out, [0xf1, 0xf2, 0xd3]);
        zoned::encode(7, E37, false, t, &mut out).unwrap();
        assert_eq!(out, [0xf0, 0xf0, 0xf7]);
        let mut sep = [0u8; 4];
        zoned::encode(
            -6,
            E37,
            true,
            Sign {
                leading: true,
                separate: true,
            },
            &mut sep,
        )
        .unwrap();
        assert_eq!(sep, [0x60, 0xf0, 0xf0, 0xf6]);
    }

    #[test]
    fn local_zoned_rejects_spaces() {
        assert_eq!(
            zoned::decode(b"1 3", Encoding::Local, false, Sign::default(), 0),
            Err(NumError::InvalidDigit { offset: 1 })
        );
    }

    #[test]
    fn binary_byte_orders() {
        // Probe: S9(4) COMP -2 → FF FE; S9(9) BINARY 258 → 00 00 01 02; S9(4) COMP-5 258 → 02 01 (local)
        assert_eq!(
            binary::decode(&[0xff, 0xfe], true, true, 0),
            Ok(Decimal::new(-2, 0))
        );
        assert_eq!(
            binary::decode(&[0, 0, 1, 2], true, true, 0),
            Ok(Decimal::new(258, 0))
        );
        assert_eq!(
            binary::decode(&[2, 1], false, true, 0),
            Ok(Decimal::new(258, 0))
        );
        assert_eq!(
            binary::decode(&[0xff, 0xfe], true, false, 0),
            Ok(Decimal::new(65534, 0))
        );
        let mut out = [0u8; 2];
        binary::encode(258, false, true, &mut out).unwrap();
        assert_eq!(out, [2, 1]);
        binary::encode(-2, true, true, &mut out).unwrap();
        assert_eq!(out, [0xff, 0xfe]);
        assert!(binary::encode(40000, true, true, &mut out).is_err());
        assert_eq!(binary::len(4), 2);
        assert_eq!(binary::len(9), 4);
        assert_eq!(binary::len(18), 8);
    }

    #[test]
    fn hex_float() {
        // 0x41100000 = 1.0 in IBM HFP; 0xC2640000 = -100.0
        assert_eq!(float::decode_ibm_hfp(&[0x41, 0x10, 0, 0]), Some(1.0));
        assert_eq!(float::decode_ibm_hfp(&[0xc2, 0x64, 0, 0]), Some(-100.0));
    }

    #[test]
    fn text_round_trip() {
        let bytes = E37.encode_text("FIRST FRONTIER BANK");
        assert_eq!(bytes[0], 0xC6);
        assert_eq!(E37.text(&bytes), "FIRST FRONTIER BANK");
    }
}
