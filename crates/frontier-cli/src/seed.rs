//! Deterministic seed data for First Frontier Bank.
//!
//! Record layouts match `cobol/copybooks/ACCTLOAD.cpy` and `TRANFEED.cpy`.
//! Both files are fixed-length display records with no line terminators,
//! exactly as they would arrive from an upstream system.
//!
//! Money literals are cents written as `dollars_cents` (`2_500_00` is $2,500.00).
#![allow(clippy::inconsistent_digit_grouping)]

use anyhow::{Result, bail};
use frontier_jcl::System;

pub const ACCTLOAD_DSN: &str = "FFB.SEED.ACCTLOAD";
pub const TRANFEED_DSN: &str = "FFB.DAILY.TRANFEED";
const ACCTLOAD_LRECL: usize = 100;
const TRANFEED_LRECL: usize = 80;
const FIRST_ACCOUNT: u64 = 4_000_000_001;

/// SplitMix64: tiny, fast, and reproducible across platforms.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }

    fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len() as u64) as usize]
    }
}

const FIRST: &[&str] = &[
    "JAMES",
    "MARY",
    "ROBERT",
    "PATRICIA",
    "JOHN",
    "JENNIFER",
    "MICHAEL",
    "LINDA",
    "DAVID",
    "ELIZABETH",
    "WILLIAM",
    "BARBARA",
    "RICHARD",
    "SUSAN",
    "JOSEPH",
    "JESSICA",
    "THOMAS",
    "SARAH",
    "CARLOS",
    "KAREN",
    "WEI",
    "NANCY",
    "ANTHONY",
    "LISA",
    "MARK",
    "BETTY",
    "DONALD",
    "MARGARET",
    "AHMED",
    "SANDRA",
    "PRIYA",
    "ASHLEY",
    "KENJI",
    "KIMBERLY",
    "OLUWASEUN",
    "EMILY",
    "DMITRI",
    "DONNA",
    "MATEO",
    "MICHELLE",
];
const LAST: &[&str] = &[
    "SMITH",
    "JOHNSON",
    "WILLIAMS",
    "BROWN",
    "JONES",
    "GARCIA",
    "MILLER",
    "DAVIS",
    "RODRIGUEZ",
    "MARTINEZ",
    "HERNANDEZ",
    "LOPEZ",
    "GONZALEZ",
    "WILSON",
    "ANDERSON",
    "THOMAS",
    "TAYLOR",
    "MOORE",
    "JACKSON",
    "MARTIN",
    "LEE",
    "PEREZ",
    "THOMPSON",
    "WHITE",
    "HARRIS",
    "SANCHEZ",
    "CLARK",
    "RAMIREZ",
    "LEWIS",
    "ROBINSON",
    "NGUYEN",
    "PATEL",
    "KIM",
    "OKAFOR",
    "IVANOV",
    "TANAKA",
    "SCHMIDT",
    "COHEN",
    "HOPPER",
    "LOVELACE",
];

fn pad(s: &str, width: usize) -> String {
    format!("{s:<width$.width$}")
}

pub fn accounts(sys: &System, count: u32, seed: u64) -> Result<()> {
    let mut rng = Rng(seed);
    let mut out = Vec::with_capacity(count as usize * ACCTLOAD_LRECL);
    let (mut checking, mut savings) = (0, 0);

    for i in 0..count as u64 {
        let id = FIRST_ACCOUNT + i;
        let name = format!(
            "{} {}. {}",
            rng.pick(FIRST),
            (b'A' + rng.below(26) as u8) as char,
            rng.pick(LAST)
        );
        let is_savings = rng.chance(0.4);
        let status = match rng.below(100) {
            0..=93 => 'A',
            94..=97 => 'F',
            _ => 'C',
        };
        let open_date = format!(
            "{:04}{:02}{:02}",
            rng.range(1985, 2026),
            rng.range(1, 12),
            rng.range(1, 28)
        );
        // Balances in cents: a long tail, a few overdrawn checking accounts.
        let magnitude = [100_00, 2_500_00, 25_000_00, 250_000_00][rng.below(4) as usize];
        let mut balance = rng.below(magnitude) as i64;
        if !is_savings && rng.chance(0.03) {
            balance = -(rng.below(300_00) as i64);
        }
        let (rate_bp, od_limit) = if is_savings {
            savings += 1;
            (rng.range(50, 450), 0) // 0.50% - 4.50%
        } else {
            checking += 1;
            (0, [0, 500_00, 1_000_00][rng.below(3) as usize])
        };

        let rec = format!(
            "{:010}{}{}{}{}{}{:013}{:05}{:09}{}",
            id,
            pad(&name, 30),
            if is_savings { 'S' } else { 'C' },
            status,
            open_date,
            if balance < 0 { '-' } else { '+' },
            balance.unsigned_abs(),
            rate_bp,
            od_limit,
            pad("", 22),
        );
        debug_assert_eq!(rec.len(), ACCTLOAD_LRECL);
        out.extend_from_slice(rec.as_bytes());
    }

    sys.write_dataset(
        ACCTLOAD_DSN,
        "FB",
        Some(ACCTLOAD_LRECL as u32),
        &out,
        "SEED",
    )?;
    println!("{ACCTLOAD_DSN}: {count} accounts ({checking} checking, {savings} savings)");
    Ok(())
}

pub fn feed(sys: &System, date: &str, count: u32, bad_rate: f64, seed: Option<u64>) -> Result<()> {
    if date.len() != 8 || !date.bytes().all(|b| b.is_ascii_digit()) {
        bail!("--date must be YYYYMMDD");
    }
    let accounts = match sys.datasets(Some(ACCTLOAD_DSN))?.first() {
        Some(d) => d.records.unwrap_or(0),
        None => bail!("{ACCTLOAD_DSN} not found: run `frontier seed accounts` first"),
    };
    if accounts == 0 {
        bail!("{ACCTLOAD_DSN} is empty");
    }

    let mut rng = Rng(seed.unwrap_or_else(|| date.parse::<u64>().unwrap()));
    let mut out = Vec::with_capacity(count as usize * TRANFEED_LRECL);
    let mut bad = 0;

    for seq in 0..count as u64 {
        // A small share of transactions reference accounts that don't exist.
        let acct = if rng.chance(0.003) {
            FIRST_ACCOUNT + accounts + rng.below(1000)
        } else {
            FIRST_ACCOUNT + rng.below(accounts)
        };
        let time = format!(
            "{:02}{:02}{:02}",
            rng.below(24),
            rng.below(60),
            rng.below(60)
        );
        let (kind, cents, channel, desc) = match rng.below(100) {
            0..=19 => (
                "DP",
                rng.range(50_00, 3_000_00),
                rng.pick(&["ATM", "BRN", "ACH"]),
                rng.pick(&["PAYROLL DEPOSIT", "CASH DEPOSIT", "CHECK DEPOSIT"]),
            ),
            20..=69 => (
                "WD",
                rng.range(5_00, 500_00),
                rng.pick(&["ATM", "POS", "POS", "WEB"]),
                rng.pick(&[
                    "GROCERY",
                    "FUEL",
                    "ATM WITHDRAWAL",
                    "ONLINE PURCHASE",
                    "RESTAURANT",
                ]),
            ),
            70..=74 => (
                "FE",
                rng.range(1_00, 35_00),
                "SYS",
                rng.pick(&["MONTHLY SERVICE FEE", "ATM FEE", "WIRE FEE"]),
            ),
            75..=86 => (
                "TI",
                rng.range(10_00, 2_000_00),
                rng.pick(&["WEB", "ACH"]),
                "TRANSFER IN",
            ),
            _ => (
                "TO",
                rng.range(10_00, 2_000_00),
                rng.pick(&["WEB", "ACH"]),
                "TRANSFER OUT",
            ),
        };

        let mut acct_field = format!("{acct:010}");
        let mut ts = format!("{date}{time}");
        let mut kind_field = kind.to_string();
        let mut amount = format!("{cents:011}");
        if rng.chance(bad_rate) {
            bad += 1;
            match rng.below(6) {
                0 => acct_field = pad("", 10),               // V001 missing account
                1 => ts = format!("{}32{time}", &date[..6]), // V002 invalid date
                2 => ts = format!("{}{time}", previous_day(date)), // V003 not business date
                3 => kind_field = "ZZ".into(),               // V004 invalid type
                4 => amount = format!("{:08}X{:02}", cents / 1000, cents % 100), // V005 not numeric
                _ => amount = "0".repeat(11),                // V006 zero amount
            }
        }
        let txn_id = format!("{}{:06}", &date[2..], seq + 1);
        let rec = format!(
            "{acct_field}{ts}{txn_id}{kind_field}{amount}{channel}{}{}",
            pad(desc, 20),
            pad("", 8)
        );
        debug_assert_eq!(rec.len(), TRANFEED_LRECL, "{rec}");
        out.extend_from_slice(rec.as_bytes());
    }

    sys.write_dataset(
        TRANFEED_DSN,
        "FB",
        Some(TRANFEED_LRECL as u32),
        &out,
        "SEED",
    )?;
    println!("{TRANFEED_DSN}: {count} transactions for {date} ({bad} deliberately malformed)");
    Ok(())
}

fn previous_day(date: &str) -> String {
    let (y, m, d): (u32, u32, u32) = (
        date[..4].parse().unwrap(),
        date[4..6].parse().unwrap(),
        date[6..].parse().unwrap(),
    );
    if d > 1 {
        return format!("{y:04}{m:02}{:02}", d - 1);
    }
    let (y, m) = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let last = match m {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    format!("{y:04}{m:02}{last:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previous_day_crosses_month_and_year() {
        assert_eq!(previous_day("20260927"), "20260926");
        assert_eq!(previous_day("20260301"), "20260228");
        assert_eq!(previous_day("20240301"), "20240229");
        assert_eq!(previous_day("20260101"), "20251231");
    }

    #[test]
    fn rng_is_deterministic() {
        let (mut a, mut b) = (Rng(7), Rng(7));
        assert_eq!(
            (0..5).map(|_| a.next()).collect::<Vec<_>>(),
            (0..5).map(|_| b.next()).collect::<Vec<_>>()
        );
    }
}
