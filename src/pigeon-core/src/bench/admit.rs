//! Admission of benches under the cap and budget.

use super::MAX_PER_SESSION;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One running bench.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    /// The owning session's ID.
    pub session: String,
    /// The bench's index within its session.
    pub k: u8,
    /// When it started, in seconds.
    pub since: u64,
    /// When it was last used, in seconds.
    pub used: u64,
    /// Whether a command is running on it.
    pub busy: bool,
}

/// Every bench, the queue for bench 0, and the day's Container-seconds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fleet {
    /// Running benches by name.
    pub slots: BTreeMap<String, Slot>,
    /// Sessions waiting for bench 0, oldest first.
    pub queue: Vec<String>,
    /// The day the budget counts, as days since the epoch.
    pub day: u64,
    /// Container-seconds used on `day`.
    pub seconds: u64,
    /// Whether the owner was asked about the budget today.
    pub asked: bool,
}

/// The fleet cap and the daily budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// Most benches at once.
    pub max: u32,
    /// Container-hours a day.
    pub hours: u32,
}

/// What admission decided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "admit")]
pub enum Admit {
    /// The bench already runs.
    Have,
    /// A new bench starts.
    Start,
    /// A new bench starts in place of an idle fan-out bench.
    Reclaim {
        /// The reclaimed bench's name.
        victim: String,
    },
    /// The session queues for bench 0.
    Wait {
        /// Sessions ahead of it.
        ahead: u32,
    },
    /// Not admitted.
    Refused {
        /// Why.
        why: String,
    },
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max: super::DEFAULT_MAX,
            hours: super::DEFAULT_HOURS,
        }
    }
}

/// Adds `seconds` to the day's use, rolling the day first.
#[must_use]
pub fn meter(f: &Fleet, seconds: u64, now: u64) -> Fleet {
    let mut g = f.clone();
    let d = now / 86_400;
    if d != g.day {
        g.day = d;
        g.seconds = 0;
        g.asked = false;
    }
    g.seconds = g.seconds.saturating_add(seconds);
    g
}

/// True when the day's use has reached the budget.
#[must_use]
pub fn over_budget(f: &Fleet, l: Limits) -> bool {
    f.seconds >= u64::from(l.hours) * 3_600
}

/// Records that the owner was asked today.
#[must_use]
pub fn mark_asked(f: &Fleet) -> Fleet {
    let mut g = f.clone();
    g.asked = true;
    g
}

fn refuse(f2: Fleet, why: String) -> (Fleet, Admit) {
    (f2, Admit::Refused { why })
}

fn insert(f: &mut Fleet, key: String, session: &str, k: u8, now: u64) {
    f.slots.insert(
        key,
        Slot {
            session: session.to_string(),
            k,
            since: now,
            used: now,
            busy: false,
        },
    );
}

/// Admits `session`'s bench `k` under the cap and budget.
#[must_use]
pub fn admit(f: &Fleet, l: Limits, session: &str, k: u8, now: u64) -> (Fleet, Admit) {
    let mut f2 = meter(f, 0, now);
    let key = match super::name(session, k) {
        Some(key) if k < MAX_PER_SESSION => key,
        _ => return refuse(f2, format!("bench: no bench {k} for {session}")),
    };
    if let Some(slot) = f2.slots.get_mut(&key) {
        slot.used = now;
        f2.queue.retain(|s| s != session);
        return (f2, Admit::Have);
    }
    let max = l.max as usize;
    if k > 0 {
        if over_budget(&f2, l) {
            let why = format!("wait: bench budget: {} Container-hours used today", l.hours);
            return refuse(f2, why);
        }
        if !f2.queue.is_empty() {
            let why = format!(
                "wait: bench capacity: {} sessions wait for a first bench",
                f2.queue.len()
            );
            return refuse(f2, why);
        }
        if f2.slots.len() >= max {
            let why = format!("wait: bench capacity: {} benches in use", l.max);
            return refuse(f2, why);
        }
        insert(&mut f2, key, session, k, now);
        return (f2, Admit::Start);
    }
    if f2.slots.len() < max && (f2.queue.is_empty() || f2.queue[0] == session) {
        insert(&mut f2, key, session, k, now);
        f2.queue.retain(|s| s != session);
        return (f2, Admit::Start);
    }
    if f2.slots.len() >= max {
        let victim = f2
            .slots
            .iter()
            .filter(|(_, s)| s.k > 0 && !s.busy)
            .min_by(|(ka, a), (kb, b)| a.used.cmp(&b.used).then_with(|| ka.cmp(kb)))
            .map(|(name, _)| name.clone());
        if let Some(v) = victim {
            f2.slots.remove(&v);
            insert(&mut f2, key, session, k, now);
            f2.queue.retain(|s| s != session);
            return (f2, Admit::Reclaim { victim: v });
        }
    }
    let ahead = if let Some(i) = f2.queue.iter().position(|s| s == session) {
        i
    } else {
        f2.queue.push(session.to_string());
        f2.queue.len() - 1
    };
    let ahead = u32::try_from(ahead).unwrap_or(u32::MAX);
    (f2, Admit::Wait { ahead })
}

/// Marks a bench used now, and busy or idle.
#[must_use]
pub fn touch(f: &Fleet, name: &str, busy: bool, now: u64) -> Fleet {
    let mut g = f.clone();
    if let Some(s) = g.slots.get_mut(name) {
        s.used = now;
        s.busy = busy;
    }
    g
}

/// Drops a bench.
#[must_use]
pub fn release(f: &Fleet, name: &str) -> Fleet {
    let mut g = f.clone();
    g.slots.remove(name);
    g
}

/// Drops every bench and the queue entry of `session`, returning the benches' names.
#[must_use]
pub fn end_session(f: &Fleet, session: &str) -> (Fleet, Vec<String>) {
    let mut g = f.clone();
    let removed: Vec<String> = g
        .slots
        .iter()
        .filter(|(_, s)| s.session == session)
        .map(|(k, _)| k.clone())
        .collect();
    for k in &removed {
        g.slots.remove(k);
    }
    g.queue.retain(|s| s != session);
    (g, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const L: Limits = Limits { max: 2, hours: 1 };

    fn fleet_with(f: &Fleet, session: &str, k: u8, now: u64) -> Fleet {
        let (g, a) = admit(f, L, session, k, now);
        assert!(matches!(a, Admit::Start), "{a:?}");
        g
    }

    #[test]
    fn default_limits() {
        assert_eq!(Limits::default(), Limits { max: 24, hours: 80 });
    }

    #[test]
    fn meter_rolls_day() {
        let f = Fleet {
            day: 0,
            seconds: 100,
            asked: true,
            ..Fleet::default()
        };
        let g = meter(&f, 5, 10);
        assert_eq!((g.day, g.seconds, g.asked), (0, 105, true));
        let g = meter(&f, 5, 86_400);
        assert_eq!((g.day, g.seconds, g.asked), (1, 5, false));
        assert_eq!(meter(&f, u64::MAX, 0).seconds, u64::MAX);
    }

    #[test]
    fn budget_bounds() {
        let mut f = Fleet {
            seconds: 3_599,
            ..Fleet::default()
        };
        assert!(!over_budget(&f, L));
        f.seconds = 3_600;
        assert!(over_budget(&f, L));
    }

    #[test]
    fn mark_asked_sets() {
        assert!(mark_asked(&Fleet::default()).asked);
    }

    #[test]
    fn rule1_no_bench() {
        let (_, a) = admit(&Fleet::default(), L, "session_a", 3, 0);
        assert_eq!(
            a,
            Admit::Refused {
                why: "bench: no bench 3 for session_a".to_string()
            }
        );
        let (_, a) = admit(&Fleet::default(), L, "bad", 0, 0);
        assert!(matches!(a, Admit::Refused { .. }));
    }

    #[test]
    fn rule2_have_removes_queue() {
        let f = fleet_with(&Fleet::default(), "session_a", 0, 1);
        let mut f = f;
        f.queue.push("session_a".to_string());
        let (g, a) = admit(&f, L, "session_a", 0, 9);
        assert_eq!(a, Admit::Have);
        assert_eq!(g.queue, Vec::<String>::new());
        assert_eq!(g.slots["session_a.0"].used, 9);
    }

    #[test]
    fn rule3_budget() {
        let f = Fleet {
            seconds: 3_600,
            ..Fleet::default()
        };
        let (_, a) = admit(&f, L, "session_a", 1, 0);
        assert_eq!(
            a,
            Admit::Refused {
                why: "wait: bench budget: 1 Container-hours used today".to_string()
            }
        );
    }

    #[test]
    fn rule4_queue_blocks_fan_out() {
        let f = Fleet {
            queue: vec!["session_b".to_string()],
            ..Fleet::default()
        };
        let (_, a) = admit(&f, L, "session_a", 1, 0);
        assert_eq!(
            a,
            Admit::Refused {
                why: "wait: bench capacity: 1 sessions wait for a first bench".to_string()
            }
        );
    }

    #[test]
    fn rule5_cap() {
        let f = fleet_with(&Fleet::default(), "session_a", 0, 0);
        let f = fleet_with(&f, "session_b", 0, 0);
        let (_, a) = admit(&f, L, "session_a", 1, 0);
        assert_eq!(
            a,
            Admit::Refused {
                why: "wait: bench capacity: 2 benches in use".to_string()
            }
        );
    }

    #[test]
    fn rule6_fan_out_starts() {
        let (g, a) = admit(&Fleet::default(), L, "session_a", 1, 7);
        assert_eq!(a, Admit::Start);
        let s = &g.slots["session_a.1"];
        assert_eq!((s.k, s.since, s.used, s.busy), (1, 7, 7, false));
    }

    #[test]
    fn rule7_first_bench_starts() {
        let (g, a) = admit(&Fleet::default(), L, "session_a", 0, 0);
        assert_eq!(a, Admit::Start);
        assert!(g.slots.contains_key("session_a.0"));
        let f = Fleet {
            queue: vec!["session_a".to_string()],
            ..Fleet::default()
        };
        let (g, a) = admit(&f, L, "session_a", 0, 0);
        assert_eq!(a, Admit::Start);
        assert_eq!(g.queue, Vec::<String>::new());
    }

    #[test]
    fn rule8_reclaims_lru_idle() {
        let f = fleet_with(&Fleet::default(), "session_a", 1, 5);
        let f = fleet_with(&f, "session_b", 1, 3);
        let (g, a) = admit(&f, L, "session_c", 0, 10);
        assert_eq!(
            a,
            Admit::Reclaim {
                victim: "session_b.1".to_string()
            }
        );
        assert!(g.slots.contains_key("session_c.0"));
        assert!(!g.slots.contains_key("session_b.1"));
    }

    #[test]
    fn rule8_skips_busy() {
        let f = fleet_with(&Fleet::default(), "session_a", 1, 5);
        let f = fleet_with(&f, "session_b", 1, 3);
        let f = touch(&f, "session_b.1", true, 3);
        let (_, a) = admit(&f, L, "session_c", 0, 10);
        assert_eq!(
            a,
            Admit::Reclaim {
                victim: "session_a.1".to_string()
            }
        );
    }

    #[test]
    fn rule9_waits() {
        let f = fleet_with(&Fleet::default(), "session_a", 0, 0);
        let f = fleet_with(&f, "session_b", 0, 0);
        let (g, a) = admit(&f, L, "session_c", 0, 1);
        assert_eq!(a, Admit::Wait { ahead: 0 });
        let (g, a) = admit(&g, L, "session_c", 0, 2);
        assert_eq!(a, Admit::Wait { ahead: 0 });
        assert_eq!(g.queue.len(), 1);
    }

    #[test]
    fn fifo_for_bench_zero() {
        let f = fleet_with(&Fleet::default(), "session_a", 0, 0);
        let f = fleet_with(&f, "session_b", 0, 0);
        let (f, a) = admit(&f, L, "session_c", 0, 1);
        assert_eq!(a, Admit::Wait { ahead: 0 });
        let (f, a) = admit(&f, L, "session_d", 0, 1);
        assert_eq!(a, Admit::Wait { ahead: 1 });
        let f = release(&f, "session_a.0");
        let (_, a) = admit(&f, L, "session_d", 0, 2);
        assert_eq!(a, Admit::Wait { ahead: 1 });
        let (f, a) = admit(&f, L, "session_c", 0, 2);
        assert_eq!(a, Admit::Start);
        let (_, a) = admit(&f, L, "session_d", 0, 3);
        assert_eq!(a, Admit::Wait { ahead: 0 });
    }

    #[test]
    fn fan_out_refused_while_session_waits() {
        let f = fleet_with(&Fleet::default(), "session_a", 0, 0);
        let f = fleet_with(&f, "session_b", 0, 0);
        let (f, _) = admit(&f, L, "session_c", 0, 1);
        let f = release(&f, "session_a.0");
        let (_, a) = admit(&f, L, "session_b", 1, 2);
        assert_eq!(
            a,
            Admit::Refused {
                why: "wait: bench capacity: 1 sessions wait for a first bench".to_string()
            }
        );
    }

    #[test]
    fn touch_missing_unchanged() {
        let f = fleet_with(&Fleet::default(), "session_a", 0, 0);
        assert_eq!(touch(&f, "nope.0", true, 9), f);
    }

    #[test]
    fn touch_sets() {
        let f = fleet_with(&Fleet::default(), "session_a", 0, 0);
        let g = touch(&f, "session_a.0", true, 9);
        assert!(g.slots["session_a.0"].busy);
        assert_eq!(g.slots["session_a.0"].used, 9);
    }

    #[test]
    fn end_session_removes_all() {
        let l = Limits { max: 8, hours: 1 };
        let mut f = Fleet::default();
        for (s, k) in [
            ("session_a", 2),
            ("session_a", 0),
            ("session_a", 1),
            ("session_b", 0),
        ] {
            f = admit(&f, l, s, k, 0).0;
        }
        f.queue.push("session_a".to_string());
        let (g, removed) = end_session(&f, "session_a");
        assert_eq!(removed, ["session_a.0", "session_a.1", "session_a.2"]);
        assert_eq!(g.queue, Vec::<String>::new());
        assert_eq!(g.slots.keys().collect::<Vec<_>>(), ["session_b.0"]);
    }
}
