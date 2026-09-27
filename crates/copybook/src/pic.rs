//! PICTURE string analysis.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PicCategory {
    Alphabetic,
    Alphanumeric,
    AlphanumericEdited,
    Numeric,
    NumericEdited,
    National,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pic {
    /// As written, upper-cased: `S9(11)V99`.
    pub text: String,
    pub category: PicCategory,
    /// Numeric digit positions (`9`s; for edited pictures also `Z` and `*`).
    pub digits: u32,
    /// Digits right of the implied decimal point; negative with trailing `P`.
    pub scale: i32,
    /// Leading `S`.
    pub signed: bool,
    /// Bytes occupied in DISPLAY usage, not counting a SEPARATE sign.
    pub length: usize,
}

impl Pic {
    pub fn is_numeric(&self) -> bool {
        self.category == PicCategory::Numeric
    }
}

/// Parse a PICTURE character-string.
pub fn parse_pic(s: &str) -> Result<Pic, String> {
    let text = s.to_uppercase();
    let chars: Vec<char> = text.chars().collect();
    let mut symbols: Vec<(String, u32)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let sym = if (c == 'C' && chars.get(i + 1) == Some(&'R'))
            || (c == 'D' && chars.get(i + 1) == Some(&'B'))
        {
            i += 2;
            chars[i - 2..i].iter().collect::<String>()
        } else {
            i += 1;
            c.to_string()
        };
        let mut count = 1u32;
        if chars.get(i) == Some(&'(') {
            let close = chars[i..]
                .iter()
                .position(|&c| c == ')')
                .ok_or_else(|| format!("unbalanced ( in PICTURE {s}"))?;
            let n: String = chars[i + 1..i + close].iter().collect();
            count = n
                .trim()
                .parse()
                .map_err(|_| format!("invalid repeat count ({n}) in PICTURE {s}"))?;
            if count == 0 {
                return Err(format!("zero repeat count in PICTURE {s}"));
            }
            i += close + 1;
        }
        symbols.push((sym, count));
    }
    if symbols.is_empty() {
        return Err("empty PICTURE".into());
    }

    let has = |set: &[&str]| symbols.iter().any(|(s, _)| set.contains(&s.as_str()));
    let only = |set: &[&str]| symbols.iter().all(|(s, _)| set.contains(&s.as_str()));
    for (sym, _) in &symbols {
        if ![
            "A", "X", "9", "S", "V", "P", "B", "0", "/", "Z", "*", "+", "-", ".", ",", "$", "CR",
            "DB", "N", "E",
        ]
        .contains(&sym.as_str())
        {
            return Err(format!("invalid symbol {sym} in PICTURE {s}"));
        }
    }

    let category = if has(&["N"]) {
        PicCategory::National
    } else if only(&["9", "S", "V", "P"]) {
        PicCategory::Numeric
    } else if only(&["A"]) {
        PicCategory::Alphabetic
    } else if only(&["A", "X", "9"]) {
        PicCategory::Alphanumeric
    } else if has(&["A", "X"]) && only(&["A", "X", "9", "B", "0", "/"]) {
        PicCategory::AlphanumericEdited
    } else {
        PicCategory::NumericEdited
    };

    let mut length = 0usize;
    let mut digits = 0u32;
    let mut scale = 0i32;
    let mut signed = false;
    let mut after_point = false;
    let mut seen_digit = false;
    for (idx, (sym, count)) in symbols.iter().enumerate() {
        let n = *count;
        match sym.as_str() {
            "S" => {
                if idx != 0 || n != 1 {
                    return Err(format!("S must be the first symbol of PICTURE {s}"));
                }
                signed = true;
            }
            "V" => after_point = true,
            "P" => {
                if seen_digit && !after_point {
                    scale -= n as i32; // 99PPP: scaled up
                } else {
                    scale += n as i32; // VPP99 / PP99: scaled down
                    after_point = true;
                }
            }
            "9" | "Z" | "*" => {
                length += n as usize;
                if sym == "9" || category == PicCategory::NumericEdited {
                    digits += n;
                    seen_digit = true;
                    if after_point {
                        scale += n as i32;
                    }
                }
            }
            "." if category == PicCategory::NumericEdited => {
                length += n as usize;
                after_point = true;
            }
            "CR" | "DB" => length += 2 * n as usize,
            "N" => length += 2 * n as usize,
            _ => length += n as usize,
        }
    }
    if category == PicCategory::Numeric && digits > 38 {
        return Err(format!("PICTURE {s} has more than 38 digits"));
    }
    Ok(Pic {
        text,
        category,
        digits,
        scale,
        signed,
        length,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Pic {
        parse_pic(s).unwrap()
    }

    #[test]
    fn numeric_pictures() {
        let bal = p("S9(11)V99");
        assert_eq!(
            (bal.category, bal.digits, bal.scale, bal.signed, bal.length),
            (PicCategory::Numeric, 13, 2, true, 13)
        );
        let rate = p("9V9(4)");
        assert_eq!(
            (rate.digits, rate.scale, rate.signed, rate.length),
            (5, 4, false, 5)
        );
        assert_eq!(p("S9(07)V9(06)").scale, 6);
        assert_eq!(p("99PPP").scale, -3);
        assert_eq!(p("VPP99").scale, 4);
        assert_eq!(p("PP99").scale, 4);
        assert_eq!(p("9(8)").length, 8);
    }

    #[test]
    fn other_categories() {
        assert_eq!(p("X(10)").category, PicCategory::Alphanumeric);
        assert_eq!(p("X(10)").length, 10);
        assert_eq!(p("A(5)").category, PicCategory::Alphabetic);
        assert_eq!(p("XXBXX").category, PicCategory::AlphanumericEdited);
        let edited = p("-ZZZ,ZZZ,ZZ9.99");
        assert_eq!(edited.category, PicCategory::NumericEdited);
        assert_eq!(edited.length, 15);
        assert_eq!(p("ZZ9.99CR").length, 8);
        assert_eq!(p("9999/99/99").category, PicCategory::NumericEdited);
        assert_eq!(p("N(4)").length, 8);
    }

    #[test]
    fn rejects_bad_pictures() {
        assert!(parse_pic("9(").is_err());
        assert!(parse_pic("9S").is_err());
        assert!(parse_pic("Q9").is_err());
        assert!(parse_pic("9(0)").is_err());
    }
}
