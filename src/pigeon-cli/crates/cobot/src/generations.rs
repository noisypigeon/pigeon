//! Date-keyed token generations; see
//! `.claude/decisions/2026-11-02-runtime-cloudflare-provisioner-unit.md` §3.

/// A UTC day: days since 1970-01-01.
pub type Day = i64;

/// One generation: the days it is valid from and until.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Generation {
    /// First valid day.
    pub not_before: Day,
    /// Expiry day.
    pub expires_on: Day,
}

impl Generation {
    /// `<name>@<YYYY-MM-DD>..<YYYY-MM-DD>`.
    #[must_use]
    pub fn key(self, name: &str) -> String {
        format!(
            "{name}@{}..{}",
            date(self.not_before),
            date(self.expires_on)
        )
    }

    /// Parses a key made by [`Generation::key`] into its name and generation.
    #[must_use]
    pub fn parse(key: &str) -> Option<(String, Self)> {
        let (name, dates) = key.rsplit_once('@')?;
        let (from, to) = dates.split_once("..")?;
        Some((
            name.to_string(),
            Self {
                not_before: day(from)?,
                expires_on: day(to)?,
            },
        ))
    }

    /// `not_before` as RFC 3339 midnight UTC.
    #[must_use]
    pub fn not_before_rfc3339(self) -> String {
        format!("{}T00:00:00Z", date(self.not_before))
    }

    /// `expires_on` as RFC 3339 midnight UTC.
    #[must_use]
    pub fn expires_on_rfc3339(self) -> String {
        format!("{}T00:00:00Z", date(self.expires_on))
    }
}

/// The generations to render for one token on `today`, given those already in state.
///
/// The current generation reuses a same-start instance that lasts until the next
/// generation; every unexpired existing instance is retained; nothing else is kept.
#[must_use]
pub fn instances(
    today: Day,
    period_days: i64,
    overlap_days: i64,
    existing: &[Generation],
) -> Vec<Generation> {
    let period = period_days.max(1);
    let start = today.div_euclid(period) * period;
    let next = start + period;
    let current = existing
        .iter()
        .copied()
        .find(|g| g.not_before == start && g.expires_on >= next)
        .unwrap_or(Generation {
            not_before: start,
            expires_on: next + overlap_days.max(0),
        });
    let mut out: Vec<Generation> = existing
        .iter()
        .copied()
        .filter(|g| g.expires_on > today)
        .collect();
    out.push(current);
    out.sort();
    out.dedup();
    out
}

/// `YYYY-MM-DD` for a day (proleptic Gregorian, UTC).
#[must_use]
pub fn date(day: Day) -> String {
    let z = day + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// The day for `YYYY-MM-DD`, if valid.
#[must_use]
pub fn day(text: &str) -> Option<Day> {
    let mut parts = text.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let result = era * 146_097 + doe - 719_468;
    (date(result) == text).then_some(result)
}

/// Today, as a [`Day`], from the system clock.
#[must_use]
pub fn today() -> Day {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    i64::try_from(secs / 86_400).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{Generation, date, day, instances};

    fn d(s: &str) -> i64 {
        day(s).expect("date")
    }

    fn keys(gs: &[Generation]) -> Vec<String> {
        gs.iter().map(|g| g.key("t")).collect()
    }

    #[test]
    fn dates_round_trip() {
        for s in ["1970-01-01", "2000-02-29", "2026-11-02", "2100-12-31"] {
            assert_eq!(date(d(s)), s);
        }
        assert_eq!(day("2026-02-30"), None);
        assert_eq!(day("x"), None);
    }

    #[test]
    fn keys_round_trip() {
        let g = Generation {
            not_before: d("2026-11-02"),
            expires_on: d("2027-02-07"),
        };
        assert_eq!(g.key("blog-deploy"), "blog-deploy@2026-11-02..2027-02-07");
        assert_eq!(
            Generation::parse("blog-deploy@2026-11-02..2027-02-07"),
            Some(("blog-deploy".to_string(), g))
        );
        assert_eq!(g.not_before_rfc3339(), "2026-11-02T00:00:00Z");
    }

    #[test]
    fn fresh_unit_renders_only_the_current_generation() {
        let today = d("2026-11-02");
        let got = instances(today, 90, 7, &[]);
        assert_eq!(got.len(), 1);
        assert!(got[0].not_before <= today && got[0].expires_on - got[0].not_before == 97);
    }

    #[test]
    fn overlap_retains_the_previous_generation_then_drops_it() {
        let period = 90;
        let start = (d("2026-11-02") / period) * period;
        let prev = Generation {
            not_before: start - period,
            expires_on: start + 7,
        };
        let inside = instances(start + 3, period, 7, &[prev]);
        assert_eq!(inside.len(), 2, "{:?}", keys(&inside));
        let after = instances(start + 8, period, 7, &[prev]);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].not_before, start);
    }

    #[test]
    fn period_change_keeps_live_tokens_and_adds_the_new_current() {
        let today = d("2026-11-02");
        let old = instances(today, 90, 7, &[]);
        let new = instances(today, 30, 7, &old);
        assert!(new.contains(&old[0]));
        assert!(new.len() <= 2);
    }

    #[test]
    fn overlap_change_mid_generation_adds_nothing() {
        let today = d("2026-11-02");
        let current = instances(today, 90, 7, &[]);
        assert_eq!(instances(today, 90, 14, &current), current);
    }

    #[test]
    fn longer_period_with_same_start_mints_a_longer_lived_instance() {
        let short = Generation {
            not_before: 0,
            expires_on: 37,
        };
        let got = instances(10, 60, 7, &[short]);
        assert_eq!(
            got,
            vec![
                short,
                Generation {
                    not_before: 0,
                    expires_on: 67
                }
            ]
        );
    }
}
