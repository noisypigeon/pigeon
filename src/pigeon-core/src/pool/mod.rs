//! Capacity pools: their config, ledger and routing, pure; see
//! `.claude/decisions/2026-11-15-development-capacity-pools-hard-spend-cap.md`.

pub mod aigateway;
pub mod anthropic;
pub mod config;
pub mod gateway;
pub mod job;
pub mod ledger;
pub mod meter;
pub mod route;

/// Money, in micro-dollars.
pub type Micros = u64;

/// The share of a window's limit, per mille, past which no new task is admitted: §2's admit line.
pub const ADMIT_PERMILLE: u64 = 900;
/// The share of a window's limit, per mille, past which no request is reserved: §2's stop line.
pub const STOP_PERMILLE: u64 = 970;

/// `text`, a decimal number of dollars, in micro-dollars.
#[must_use]
pub fn micros(text: &str) -> Option<Micros> {
    let (whole, frac) = match text.split_once('.') {
        Some((w, f)) => (w, f),
        None => (text, ""),
    };
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() || !digits(whole) || !digits(frac) {
        return None;
    }
    if text.contains('.') && (frac.is_empty() || frac.len() > 6) {
        return None;
    }
    let whole: u64 = whole.parse().ok()?;
    let frac: u64 = if frac.is_empty() {
        0
    } else {
        format!("{frac:0<6}").parse().ok()?
    };
    whole.checked_mul(1_000_000)?.checked_add(frac)
}

/// `m` in dollars with two decimals, rounded up to the cent, as `12.40`.
#[must_use]
pub fn dollars(m: Micros) -> String {
    let cents = m.div_ceil(10_000);
    format!("{}.{:02}", cents / 100, cents % 100)
}

/// `tokens` at `per_million` micro-dollars per million tokens, rounded up.
#[must_use]
pub fn cost(tokens: u64, per_million: Micros) -> Micros {
    let c = (u128::from(tokens) * u128::from(per_million)).div_ceil(1_000_000);
    u64::try_from(c).unwrap_or(u64::MAX)
}

/// `permille` of `limit`, rounded down.
#[must_use]
pub fn line(limit: Micros, permille: u64) -> Micros {
    let l = u128::from(limit) * u128::from(permille) / 1000;
    u64::try_from(l).unwrap_or(u64::MAX)
}

/// The UTC (year, month, day) holding Unix second `t`.
#[must_use]
#[allow(clippy::many_single_char_names)]
pub fn civil(t: u64) -> (u64, u64, u64) {
    let z = t / 86_400 + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    (y, m, d)
}

/// Unix seconds at 00:00 UTC on (`y`, `m`, `d`).
#[must_use]
#[allow(clippy::many_single_char_names)]
pub fn at(y: u64, m: u64, d: u64) -> u64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y / 400;
    let yoe = y - era * 400;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    (era * 146_097 + doe - 719_468) * 86_400
}

/// The days in month `m` of year `y`.
#[must_use]
pub fn days_in(y: u64, m: u64) -> u64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400)) => 29,
        _ => 28,
    }
}

/// `t`'s UTC date as `YYYY-MM-DD`.
#[must_use]
pub fn date(t: u64) -> String {
    let (y, m, d) = civil(t);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn micros_accepts_decimals() {
        assert_eq!(micros("2"), Some(2_000_000));
        assert_eq!(micros("0.125"), Some(125_000));
        assert_eq!(micros("0.01"), Some(10_000));
        assert_eq!(micros("0.000001"), Some(1));
    }

    #[test]
    fn micros_refuses_the_rest() {
        for bad in ["", "-1", "1.2345678", "1e3", "1.", ".5", "a", "1.2.3", " 1"] {
            assert_eq!(micros(bad), None, "{bad}");
        }
    }

    #[test]
    fn micros_overflow() {
        assert_eq!(micros("18446744073709"), Some(18_446_744_073_709_000_000));
        assert_eq!(micros("18446744073710"), None);
        assert_eq!(micros("99999999999999999999999"), None);
    }

    #[test]
    fn dollars_rounds_up() {
        assert_eq!(dollars(0), "0.00");
        assert_eq!(dollars(1), "0.01");
        assert_eq!(dollars(9_999), "0.01");
        assert_eq!(dollars(10_000), "0.01");
        assert_eq!(dollars(12_400_000), "12.40");
    }

    #[test]
    fn cost_rounds_up_and_saturates() {
        assert_eq!(cost(1_000_000, 2_000_000), 2_000_000);
        assert_eq!(cost(1, 125_000), 1);
        assert_eq!(cost(0, 125_000), 0);
        assert_eq!(cost(u64::MAX, u64::MAX), u64::MAX);
    }

    #[test]
    fn line_rounds_down() {
        assert_eq!(line(100_000_000, ADMIT_PERMILLE), 90_000_000);
        assert_eq!(line(100_000_000, STOP_PERMILLE), 97_000_000);
        assert_eq!(line(1, 900), 0);
        assert_eq!(line(u64::MAX, 5000), u64::MAX);
    }

    #[test]
    fn civil_and_at_round_trip() {
        for (y, m, d) in [(2026, 11, 15), (2024, 2, 29), (2027, 1, 1)] {
            let t = at(y, m, d);
            assert_eq!(civil(t), (y, m, d));
            assert_eq!(civil(t + 86_399), (y, m, d));
        }
    }

    #[test]
    fn days_in_months() {
        assert_eq!(days_in(2024, 2), 29);
        assert_eq!(days_in(2100, 2), 28);
        assert_eq!(days_in(2000, 2), 29);
        assert_eq!(days_in(2026, 2), 28);
        assert_eq!(days_in(2026, 4), 30);
        assert_eq!(days_in(2026, 12), 31);
    }

    #[test]
    fn date_formats() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(at(2026, 11, 15)), "2026-11-15");
    }
}
