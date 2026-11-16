//! A pool's ledger: windows, holds, task budgets, the admit and stop lines, signals; pure.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::config::{Budget, Pool};
use super::{Micros, at, civil, date, days_in, dollars, line};
use crate::sessions::Class;

/// Seconds a hold lives before the sweep charges it in full.
pub const HOLD_TTL: u64 = 900;
/// Seconds past its deadline before the sweep closes a task.
pub const DEADLINE_GRACE: u64 = 600;
/// Hours of per-model spend kept.
pub const KEEP_HOURS: u64 = 90 * 24;
/// Closed tasks kept.
pub const KEEP_CLOSED: usize = 20;
/// The model key charges without a response are filed under.
pub const UNRECONCILED: &str = "unreconciled";

/// What a pool can take now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum State {
    /// Takes tasks.
    Open,
    /// Past the admit line.
    Full,
    /// Past the stop line.
    Capped,
    /// The gateway says the credit is gone.
    Exhausted {
        /// Until when.
        until: u64,
    },
    /// The gateway says a spend limit is reached.
    Limited {
        /// Until when.
        until: u64,
    },
    /// A tripwire or the owner paused it.
    Paused {
        /// Why.
        why: String,
    },
    /// Not enabled.
    Disabled,
}

/// What a gateway response says about the pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Signal {
    /// Fine.
    Ok,
    /// A rate limit; not a spend signal.
    RateLimited,
    /// The credit is gone.
    Exhausted,
    /// A spend limit is reached.
    Limited,
    /// The key was refused.
    Unauthorized,
    /// A response named another workspace.
    WrongAccount,
    /// A cost passed its reservation.
    Overrun,
}

/// A spend window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    /// First second.
    pub start: u64,
    /// First second after it.
    pub end: u64,
    /// Its limit.
    pub limit: Micros,
    /// Charged.
    pub spent: Micros,
    /// Reserved.
    pub held: Micros,
}

/// A task and what it has used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    /// Its id.
    pub id: String,
    /// Its class.
    pub class: Class,
    /// Its bounds.
    pub budget: Budget,
    /// When it opened.
    pub opened: u64,
    /// When it must end.
    pub deadline: u64,
    /// Charged.
    pub spent: Micros,
    /// Reserved.
    pub held: Micros,
    /// Fresh tokens used.
    pub fresh_tokens: u64,
    /// Requests reserved.
    pub requests: u32,
    /// When it closed.
    pub closed: Option<u64>,
    /// How it ended.
    pub outcome: Option<String>,
}

/// A reservation for one request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hold {
    /// Its task.
    pub task: String,
    /// What it reserves.
    pub amount: Micros,
    /// When it was made.
    pub at: u64,
}

/// One pool's books.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ledger {
    /// The pool's name.
    pub pool: String,
    /// The credit cycle.
    pub cycle: Window,
    /// The calendar month.
    pub month: Window,
    /// Tasks by id.
    pub tasks: BTreeMap<String, Task>,
    /// Holds by id.
    pub holds: BTreeMap<String, Hold>,
    /// Why it is paused.
    pub paused: Option<String>,
    /// Until when the credit is gone.
    pub exhausted_until: Option<u64>,
    /// Until when a spend limit holds.
    pub limited_until: Option<u64>,
    /// Spend by `hour/model`.
    pub hours: BTreeMap<String, Micros>,
}

/// Why a call was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Not enabled.
    Disabled,
    /// Paused.
    Paused(String),
    /// Past the stop line.
    Capped,
    /// Past the admit line.
    Full {
        /// Until when.
        until: u64,
    },
    /// Credit gone.
    Exhausted {
        /// Until when.
        until: u64,
    },
    /// Spend limit reached.
    Limited {
        /// Until when.
        until: u64,
    },
    /// Every slot is taken.
    Busy {
        /// Tasks running.
        running: u32,
    },
    /// No such task.
    NoTask(String),
    /// The task is closed.
    TaskClosed(String),
    /// The task is past its deadline.
    Deadline(String),
    /// The task spent a bound.
    Spent {
        /// The task.
        task: String,
        /// The bound.
        bound: &'static str,
    },
    /// No such hold.
    NoHold(String),
    /// Already there.
    Duplicate(String),
}

/// A tripwire: the signal and why the pool pauses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trip {
    /// The signal.
    pub signal: Signal,
    /// Why.
    pub why: String,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => write!(f, "pool: disabled"),
            Self::Paused(w) => write!(f, "pool: paused: {w}"),
            Self::Capped => write!(f, "pool: capped"),
            Self::Full { until } => write!(f, "pool: full until {}", date(*until)),
            Self::Exhausted { until } => {
                write!(f, "pool: credit exhausted until {}", date(*until))
            }
            Self::Limited { until } => {
                write!(f, "pool: spend limit reached until {}", date(*until))
            }
            Self::Busy { running } => write!(f, "wait: pool: {running} tasks running"),
            Self::NoTask(t) => write!(f, "pool: no task {t}"),
            Self::TaskClosed(t) => write!(f, "pool: task {t} is closed"),
            Self::Deadline(t) => write!(f, "pool: task {t} passed its deadline"),
            Self::Spent { task, bound } => write!(f, "pool: task {task} spent its {bound}"),
            Self::NoHold(h) => write!(f, "pool: no hold {h}"),
            Self::Duplicate(x) => write!(f, "pool: {x} exists"),
        }
    }
}

fn next_month(y: u64, m: u64) -> (u64, u64) {
    if m == 12 { (y + 1, 1) } else { (y, m + 1) }
}

fn prev_month(y: u64, m: u64) -> (u64, u64) {
    if m == 1 { (y - 1, 12) } else { (y, m - 1) }
}

fn on(y: u64, m: u64, day: u64) -> u64 {
    at(y, m, day.min(days_in(y, m)))
}

/// The credit cycle holding `now`, starting on `day` of each month, clamped to short months.
#[must_use]
pub fn cycle_bounds(now: u64, day: u64) -> (u64, u64) {
    let (y, m, d) = civil(now);
    if d >= day.min(days_in(y, m)) {
        let (ny, nm) = next_month(y, m);
        (on(y, m, day), on(ny, nm, day))
    } else {
        let (py, pm) = prev_month(y, m);
        (on(py, pm, day), on(y, m, day))
    }
}

/// The calendar month holding `now`.
#[must_use]
pub fn month_bounds(now: u64) -> (u64, u64) {
    let (y, m, _) = civil(now);
    let (ny, nm) = next_month(y, m);
    (at(y, m, 1), at(ny, nm, 1))
}

fn cycle_of(pool: &Pool, now: u64) -> (u64, u64) {
    match pool.cycle_day {
        Some(d) => cycle_bounds(now, d),
        None => month_bounds(now),
    }
}

/// An empty ledger for `pool` at `now`.
#[must_use]
pub fn new(pool: &Pool, now: u64) -> Ledger {
    let (cs, ce) = cycle_of(pool, now);
    let (ms, me) = month_bounds(now);
    Ledger {
        pool: pool.name.clone(),
        cycle: Window {
            start: cs,
            end: ce,
            limit: pool.credit,
            spent: 0,
            held: 0,
        },
        month: Window {
            start: ms,
            end: me,
            limit: pool.month_limit,
            spent: 0,
            held: 0,
        },
        tasks: BTreeMap::new(),
        holds: BTreeMap::new(),
        paused: None,
        exhausted_until: None,
        limited_until: None,
        hours: BTreeMap::new(),
    }
}

/// `l` with expired windows, signals, hours and old tasks rolled off at `now`.
#[must_use]
pub fn roll(l: &Ledger, pool: &Pool, now: u64) -> Ledger {
    let mut n = l.clone();
    if now >= n.cycle.end {
        let (s, e) = cycle_of(pool, now);
        n.cycle.start = s;
        n.cycle.end = e;
        n.cycle.spent = 0;
        n.cycle.limit = pool.credit;
    }
    if now >= n.month.end {
        let (s, e) = month_bounds(now);
        n.month.start = s;
        n.month.end = e;
        n.month.spent = 0;
        n.month.limit = pool.month_limit;
    }
    if n.exhausted_until.is_some_and(|u| now >= u) {
        n.exhausted_until = None;
    }
    if n.limited_until.is_some_and(|u| now >= u) {
        n.limited_until = None;
    }
    n.hours.retain(|k, _| {
        k.split_once('/')
            .and_then(|(h, _)| h.parse::<u64>().ok())
            .is_none_or(|h| h + KEEP_HOURS * 3600 > now)
    });
    let mut closed: Vec<(u64, String)> = n
        .tasks
        .values()
        .filter_map(|t| t.closed.map(|c| (c, t.id.clone())))
        .collect();
    closed.sort_by(|a, b| b.cmp(a));
    for (_, id) in closed.into_iter().skip(KEEP_CLOSED) {
        n.tasks.remove(&id);
    }
    n
}

/// Tasks not closed.
#[must_use]
pub fn running(l: &Ledger) -> u32 {
    let n = l.tasks.values().filter(|t| t.closed.is_none()).count();
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// What open tasks may still spend, beyond what they spent and hold.
#[must_use]
pub fn committed(l: &Ledger) -> Micros {
    l.tasks
        .values()
        .filter(|t| t.closed.is_none())
        .map(|t| t.budget.usd.saturating_sub(t.spent).saturating_sub(t.held))
        .fold(0, Micros::saturating_add)
}

/// What the cycle's admit line leaves after spend, holds and commitments.
#[must_use]
pub fn headroom(l: &Ledger) -> Micros {
    line(l.cycle.limit, super::ADMIT_PERMILLE).saturating_sub(
        l.cycle
            .spent
            .saturating_add(l.cycle.held)
            .saturating_add(committed(l)),
    )
}

/// The pool's state at `now`.
#[must_use]
pub fn state(l: &Ledger, pool: &Pool, now: u64) -> State {
    if !pool.enabled {
        return State::Disabled;
    }
    if let Some(w) = &l.paused {
        return State::Paused { why: w.clone() };
    }
    if let Some(u) = l.exhausted_until.filter(|u| *u > now) {
        return State::Exhausted { until: u };
    }
    if let Some(u) = l.limited_until.filter(|u| *u > now) {
        return State::Limited { until: u };
    }
    let wins = [l.cycle, l.month];
    let used = |w: &Window| w.spent.saturating_add(w.held);
    if wins
        .iter()
        .any(|w| used(w) >= line(w.limit, super::STOP_PERMILLE))
    {
        return State::Capped;
    }
    let c = committed(l);
    if wins
        .iter()
        .any(|w| used(w).saturating_add(c) >= line(w.limit, super::ADMIT_PERMILLE))
    {
        return State::Full;
    }
    State::Open
}

fn gate(s: State) -> Result<(), Refusal> {
    match s {
        State::Disabled => Err(Refusal::Disabled),
        State::Paused { why } => Err(Refusal::Paused(why)),
        State::Exhausted { until } => Err(Refusal::Exhausted { until }),
        State::Limited { until } => Err(Refusal::Limited { until }),
        State::Capped => Err(Refusal::Capped),
        State::Full | State::Open => Ok(()),
    }
}

/// Opens task `id`, if the pool is open, has a slot and the budget fits both admit lines.
///
/// # Errors
///
/// A [`Refusal`] naming why not.
pub fn admit_task(
    l: &Ledger,
    pool: &Pool,
    id: &str,
    class: Class,
    budget: Budget,
    now: u64,
) -> Result<Ledger, Refusal> {
    let mut n = roll(l, pool, now);
    gate(state(&n, pool, now))?;
    if n.tasks.contains_key(id) {
        return Err(Refusal::Duplicate(id.to_string()));
    }
    let r = running(&n);
    if r >= pool.concurrency {
        return Err(Refusal::Busy { running: r });
    }
    let c = committed(&n);
    for w in [n.cycle, n.month] {
        let total = w
            .spent
            .saturating_add(w.held)
            .saturating_add(c)
            .saturating_add(budget.usd);
        if total > line(w.limit, super::ADMIT_PERMILLE) {
            return Err(Refusal::Full { until: w.end });
        }
    }
    n.tasks.insert(
        id.to_string(),
        Task {
            id: id.to_string(),
            class,
            budget,
            opened: now,
            deadline: now.saturating_add(u64::from(budget.minutes) * 60),
            spent: 0,
            held: 0,
            fresh_tokens: 0,
            requests: 0,
            closed: None,
            outcome: None,
        },
    );
    Ok(n)
}

/// Reserves `amount` under `hold` for `task`'s next request.
///
/// # Errors
///
/// A [`Refusal`] naming why not.
pub fn reserve(
    l: &Ledger,
    pool: &Pool,
    task: &str,
    hold: &str,
    amount: Micros,
    now: u64,
) -> Result<Ledger, Refusal> {
    let mut n = roll(l, pool, now);
    let t = n
        .tasks
        .get(task)
        .ok_or_else(|| Refusal::NoTask(task.to_string()))?;
    if t.closed.is_some() {
        return Err(Refusal::TaskClosed(task.to_string()));
    }
    if now >= t.deadline {
        return Err(Refusal::Deadline(task.to_string()));
    }
    gate(state(&n, pool, now))?;
    let t = &n.tasks[task];
    let spent = |bound| Refusal::Spent {
        task: task.to_string(),
        bound,
    };
    if t.requests >= t.budget.requests {
        return Err(spent("requests"));
    }
    if t.fresh_tokens >= t.budget.fresh_tokens {
        return Err(spent("fresh tokens"));
    }
    if t.spent.saturating_add(t.held).saturating_add(amount) > t.budget.usd {
        return Err(spent("dollars"));
    }
    for w in [n.cycle, n.month] {
        if w.spent.saturating_add(w.held).saturating_add(amount)
            > line(w.limit, super::STOP_PERMILLE)
        {
            return Err(Refusal::Capped);
        }
    }
    if n.holds.contains_key(hold) {
        return Err(Refusal::Duplicate(hold.to_string()));
    }
    if let Some(t) = n.tasks.get_mut(task) {
        t.requests += 1;
        t.held = t.held.saturating_add(amount);
    }
    n.cycle.held = n.cycle.held.saturating_add(amount);
    n.month.held = n.month.held.saturating_add(amount);
    n.holds.insert(
        hold.to_string(),
        Hold {
            task: task.to_string(),
            amount,
            at: now,
        },
    );
    Ok(n)
}

fn charge(
    l: &Ledger,
    hold: &str,
    cost: Micros,
    fresh_tokens: u64,
    model: &str,
    now: u64,
) -> Result<Ledger, Refusal> {
    let mut n = l.clone();
    let h = n
        .holds
        .remove(hold)
        .ok_or_else(|| Refusal::NoHold(hold.to_string()))?;
    if let Some(t) = n.tasks.get_mut(&h.task) {
        t.held = t.held.saturating_sub(h.amount);
        t.spent = t.spent.saturating_add(cost);
        t.fresh_tokens = t.fresh_tokens.saturating_add(fresh_tokens);
    }
    for w in [&mut n.cycle, &mut n.month] {
        w.held = w.held.saturating_sub(h.amount);
        w.spent = w.spent.saturating_add(cost);
    }
    let e = n
        .hours
        .entry(format!("{}/{model}", now / 3600 * 3600))
        .or_insert(0);
    *e = e.saturating_add(cost);
    Ok(n)
}

/// Charges `hold`'s real `cost`; a cost past the reservation trips and pauses the pool.
///
/// # Errors
///
/// [`Refusal::NoHold`] when there is no such hold.
pub fn reconcile(
    l: &Ledger,
    hold: &str,
    cost: Micros,
    fresh_tokens: u64,
    model: &str,
    now: u64,
) -> Result<(Ledger, Option<Trip>), Refusal> {
    let amount = l
        .holds
        .get(hold)
        .ok_or_else(|| Refusal::NoHold(hold.to_string()))?
        .amount;
    let mut n = charge(l, hold, cost, fresh_tokens, model, now)?;
    if cost > amount {
        let why = format!(
            "hold {hold} reserved {} and cost {}",
            dollars(amount),
            dollars(cost)
        );
        n.paused = Some(why.clone());
        return Ok((
            n,
            Some(Trip {
                signal: Signal::Overrun,
                why,
            }),
        ));
    }
    Ok((n, None))
}

fn close(l: &Ledger, task: &str, outcome: &str, now: u64) -> Result<Ledger, Refusal> {
    let t = l
        .tasks
        .get(task)
        .ok_or_else(|| Refusal::NoTask(task.to_string()))?;
    if t.closed.is_some() {
        return Ok(l.clone());
    }
    let mut n = l.clone();
    let ids: Vec<(String, Micros)> = l
        .holds
        .iter()
        .filter(|(_, h)| h.task == task)
        .map(|(id, h)| (id.clone(), h.amount))
        .collect();
    for (id, amount) in ids {
        n = charge(&n, &id, amount, 0, UNRECONCILED, now)?;
    }
    if let Some(t) = n.tasks.get_mut(task) {
        t.closed = Some(now);
        t.outcome = Some(outcome.to_string());
    }
    Ok(n)
}

/// Closes `task`, charging its open holds in full; closing a closed task changes nothing.
///
/// # Errors
///
/// [`Refusal::NoTask`] when there is no such task.
pub fn close_task(l: &Ledger, task: &str, outcome: &str, now: u64) -> Result<Ledger, Refusal> {
    close(l, task, outcome, now)
}

/// Charges stale holds in full and closes tasks past their deadline and grace.
#[must_use]
pub fn sweep(l: &Ledger, pool: &Pool, now: u64) -> Ledger {
    let mut n = roll(l, pool, now);
    let stale: Vec<String> = n
        .holds
        .iter()
        .filter(|(_, h)| h.at.saturating_add(HOLD_TTL) <= now)
        .map(|(id, _)| id.clone())
        .collect();
    for id in stale {
        if let Ok(m) = charge(&n, &id, n.holds[&id].amount, 0, UNRECONCILED, now) {
            n = m;
        }
    }
    let late: Vec<String> = n
        .tasks
        .values()
        .filter(|t| t.closed.is_none() && t.deadline.saturating_add(DEADLINE_GRACE) <= now)
        .map(|t| t.id.clone())
        .collect();
    for id in late {
        if let Ok(m) = close(&n, &id, "deadline", now) {
            n = m;
        }
    }
    n
}

/// Applies a gateway signal; a tripwire pauses the pool.
#[must_use]
pub fn signal(l: &Ledger, s: Signal) -> (Ledger, Option<Trip>) {
    let mut n = l.clone();
    let why = match s {
        Signal::Ok | Signal::RateLimited => return (n, None),
        Signal::Exhausted => {
            n.exhausted_until = Some(n.cycle.end);
            (n.cycle.spent.saturating_add(n.cycle.held) < n.cycle.limit / 2).then(|| {
                format!(
                    "the gateway says the credit is gone at {} of {} this cycle",
                    dollars(n.cycle.spent),
                    dollars(n.cycle.limit)
                )
            })
        }
        Signal::Limited => {
            n.limited_until = Some(n.month.end);
            (n.month.spent.saturating_add(n.month.held) < n.month.limit / 2).then(|| {
                format!(
                    "the gateway says a spend limit is reached at {} of {} this month",
                    dollars(n.month.spent),
                    dollars(n.month.limit)
                )
            })
        }
        Signal::Unauthorized => Some("the gateway refused the pool's key".to_string()),
        Signal::WrongAccount => Some("a response named another workspace".to_string()),
        Signal::Overrun => Some("a cost passed its reservation".to_string()),
    };
    match why {
        Some(why) => {
            n.paused = Some(why.clone());
            (n, Some(Trip { signal: s, why }))
        }
        None => (n, None),
    }
}

/// `l` paused for `why`.
#[must_use]
pub fn pause(l: &Ledger, why: &str) -> Ledger {
    let mut n = l.clone();
    n.paused = Some(why.to_string());
    n
}

/// `l` no longer paused.
#[must_use]
pub fn resume(l: &Ledger) -> Ledger {
    let mut n = l.clone();
    n.paused = None;
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_795_000_000; // 2026-11-18

    fn pool() -> Pool {
        Pool {
            name: "p".into(),
            enabled: true,
            gateway: "anthropic-messages".into(),
            harness: "claude-code".into(),
            credit: 100_000_000,
            month_limit: 50_000_000,
            cycle_day: Some(16),
            concurrency: 2,
            prices: BTreeMap::new(),
            long: BTreeMap::new(),
        }
    }

    fn budget(usd: Micros) -> Budget {
        Budget {
            usd,
            fresh_tokens: 1_000_000,
            requests: 100,
            minutes: 30,
        }
    }

    fn led() -> Ledger {
        new(&pool(), NOW)
    }

    fn with_task(usd: Micros) -> Ledger {
        admit_task(&led(), &pool(), "t", Class::Sonnet, budget(usd), NOW).unwrap()
    }

    fn day(y: u64, m: u64, d: u64) -> u64 {
        at(y, m, d)
    }

    #[test]
    fn cycle_around_the_start_day() {
        let (s, e) = cycle_bounds(day(2026, 11, 15) + 5, 16);
        assert_eq!((s, e), (day(2026, 10, 16), day(2026, 11, 16)));
        for d in [16, 17] {
            let (s, e) = cycle_bounds(day(2026, 11, d) + 5, 16);
            assert_eq!((s, e), (day(2026, 11, 16), day(2026, 12, 16)));
        }
    }

    #[test]
    fn cycle_day_31_clamps() {
        assert_eq!(
            cycle_bounds(day(2026, 2, 10), 31),
            (day(2026, 1, 31), day(2026, 2, 28))
        );
        assert_eq!(
            cycle_bounds(day(2026, 2, 28), 31),
            (day(2026, 2, 28), day(2026, 3, 31))
        );
        assert_eq!(
            cycle_bounds(day(2026, 4, 15), 31),
            (day(2026, 3, 31), day(2026, 4, 30))
        );
    }

    #[test]
    fn cycle_across_the_year() {
        assert_eq!(
            cycle_bounds(day(2026, 12, 20), 16),
            (day(2026, 12, 16), day(2027, 1, 16))
        );
        assert_eq!(
            cycle_bounds(day(2027, 1, 5), 16),
            (day(2026, 12, 16), day(2027, 1, 16))
        );
    }

    #[test]
    fn month_across_the_year() {
        assert_eq!(
            month_bounds(day(2026, 12, 15)),
            (day(2026, 12, 1), day(2027, 1, 1))
        );
        assert_eq!(
            month_bounds(day(2026, 11, 15)),
            (day(2026, 11, 1), day(2026, 12, 1))
        );
    }

    #[test]
    fn new_with_a_cycle_day() {
        let l = led();
        assert_eq!(
            (l.cycle.start, l.cycle.end),
            (day(2026, 11, 16), day(2026, 12, 16))
        );
        assert_eq!(
            (l.month.start, l.month.end),
            (day(2026, 11, 1), day(2026, 12, 1))
        );
        assert_eq!((l.cycle.limit, l.month.limit), (100_000_000, 50_000_000));
        assert!(l.tasks.is_empty() && l.holds.is_empty() && l.paused.is_none());
    }

    #[test]
    fn new_without_a_cycle_day() {
        let mut p = pool();
        p.cycle_day = None;
        let l = new(&p, NOW);
        assert_eq!((l.cycle.start, l.cycle.end), (l.month.start, l.month.end));
    }

    #[test]
    fn roll_resets_spent_and_keeps_held() {
        let mut l = led();
        l.cycle.spent = 5;
        l.cycle.held = 3;
        l.month.spent = 7;
        l.month.held = 2;
        let r = roll(&l, &pool(), l.cycle.end);
        assert_eq!((r.cycle.spent, r.cycle.held), (0, 3));
        assert_eq!(r.cycle.start, l.cycle.end);
        assert_eq!((r.month.spent, r.month.held), (0, 2));
        let r = roll(&l, &pool(), l.month.end);
        assert_eq!((r.month.spent, r.month.held), (0, 2));
        assert_eq!(r.month.start, l.month.end);
        assert_eq!(r.cycle.spent, 5);
    }

    #[test]
    fn roll_takes_the_limit_from_the_pool() {
        let mut p = pool();
        let l = led();
        p.credit = 80_000_000;
        assert_eq!(roll(&l, &p, l.cycle.end).cycle.limit, 80_000_000);
        assert_eq!(roll(&l, &p, NOW).cycle.limit, 100_000_000);
    }

    #[test]
    fn roll_clears_signals_once_passed() {
        let mut l = led();
        l.exhausted_until = Some(NOW + 10);
        l.limited_until = Some(NOW + 10);
        let r = roll(&l, &pool(), NOW + 9);
        assert!(r.exhausted_until.is_some() && r.limited_until.is_some());
        let r = roll(&l, &pool(), NOW + 10);
        assert!(r.exhausted_until.is_none() && r.limited_until.is_none());
    }

    #[test]
    fn roll_drops_old_hours() {
        let mut l = led();
        l.hours.insert("0/m".into(), 1);
        l.hours.insert(format!("{}/m", NOW / 3600 * 3600), 2);
        let r = roll(&l, &pool(), NOW);
        assert_eq!(r.hours.len(), 1);
        assert!(r.hours.contains_key(&format!("{}/m", NOW / 3600 * 3600)));
    }

    #[test]
    fn roll_keeps_twenty_closed_tasks() {
        let mut l = with_task(1_000_000);
        for i in 0..25u64 {
            let mut t = l.tasks["t"].clone();
            t.id = format!("c{i}");
            t.closed = Some(i);
            l.tasks.insert(t.id.clone(), t);
        }
        let r = roll(&l, &pool(), NOW);
        assert_eq!(r.tasks.len(), 21);
        assert!(r.tasks.contains_key("t"));
        assert!(r.tasks.contains_key("c24") && r.tasks.contains_key("c5"));
        assert!(!r.tasks.contains_key("c4"));
    }

    #[test]
    fn state_disabled() {
        let mut p = pool();
        p.enabled = false;
        assert_eq!(state(&led(), &p, NOW), State::Disabled);
    }

    #[test]
    fn state_paused() {
        let l = pause(&led(), "why");
        assert_eq!(state(&l, &pool(), NOW), State::Paused { why: "why".into() });
    }

    #[test]
    fn state_exhausted_and_limited() {
        let mut l = led();
        l.exhausted_until = Some(NOW + 1);
        assert_eq!(state(&l, &pool(), NOW), State::Exhausted { until: NOW + 1 });
        assert_eq!(state(&l, &pool(), NOW + 1), State::Open);
        l.exhausted_until = None;
        l.limited_until = Some(NOW + 1);
        assert_eq!(state(&l, &pool(), NOW), State::Limited { until: NOW + 1 });
    }

    #[test]
    fn state_capped() {
        let mut l = led();
        l.cycle.spent = 97_000_000;
        assert_eq!(state(&l, &pool(), NOW), State::Capped);
        let mut l = led();
        l.month.held = 48_500_000;
        assert_eq!(state(&l, &pool(), NOW), State::Capped);
    }

    #[test]
    fn state_full() {
        let mut l = led();
        l.cycle.spent = 90_000_000;
        assert_eq!(state(&l, &pool(), NOW), State::Full);
        let mut l = led();
        l.month.spent = 45_000_000;
        assert_eq!(state(&l, &pool(), NOW), State::Full);
        let l = with_task(46_000_000 - 1_000_000);
        assert_eq!(state(&l, &pool(), NOW), State::Full);
    }

    #[test]
    fn state_open() {
        assert_eq!(state(&led(), &pool(), NOW), State::Open);
    }

    #[test]
    fn running_committed_headroom() {
        let l = with_task(4_000_000);
        assert_eq!(running(&l), 1);
        assert_eq!(committed(&l), 4_000_000);
        assert_eq!(headroom(&l), 86_000_000);
        let l = reserve(&l, &pool(), "t", "h", 1_000_000, NOW).unwrap();
        assert_eq!(committed(&l), 3_000_000);
        assert_eq!(headroom(&l), 86_000_000);
        let l = close_task(&l, "t", "done", NOW).unwrap();
        assert_eq!((running(&l), committed(&l)), (0, 0));
        assert_eq!(headroom(&l), 89_000_000);
    }

    #[test]
    fn admit_ok() {
        let l = with_task(4_000_000);
        let t = &l.tasks["t"];
        assert_eq!(t.deadline, NOW + 1800);
        assert_eq!((t.opened, t.spent, t.held, t.requests), (NOW, 0, 0, 0));
        assert!(t.closed.is_none() && t.outcome.is_none());
        assert_eq!(t.class, Class::Sonnet);
    }

    #[test]
    fn admit_duplicate_and_busy() {
        let l = with_task(1_000_000);
        let e = admit_task(&l, &pool(), "t", Class::Sonnet, budget(1), NOW).unwrap_err();
        assert_eq!(e, Refusal::Duplicate("t".into()));
        let l = admit_task(&l, &pool(), "u", Class::Sonnet, budget(1_000_000), NOW).unwrap();
        let e = admit_task(&l, &pool(), "v", Class::Sonnet, budget(1), NOW).unwrap_err();
        assert_eq!(e, Refusal::Busy { running: 2 });
    }

    #[test]
    fn admit_full_by_cycle_and_month() {
        let e =
            admit_task(&led(), &pool(), "t", Class::Sonnet, budget(91_000_000), NOW).unwrap_err();
        assert_eq!(
            e,
            Refusal::Full {
                until: day(2026, 12, 16)
            }
        );
        let e =
            admit_task(&led(), &pool(), "t", Class::Sonnet, budget(46_000_000), NOW).unwrap_err();
        assert_eq!(
            e,
            Refusal::Full {
                until: day(2026, 12, 1)
            }
        );
        assert!(admit_task(&led(), &pool(), "t", Class::Sonnet, budget(45_000_000), NOW).is_ok());
    }

    #[test]
    fn admit_refused_states() {
        let p = pool();
        let go = |l: &Ledger, p: &Pool| {
            admit_task(l, p, "t", Class::Sonnet, budget(1), NOW).unwrap_err()
        };
        let mut off = pool();
        off.enabled = false;
        assert_eq!(go(&led(), &off), Refusal::Disabled);
        assert_eq!(go(&pause(&led(), "x"), &p), Refusal::Paused("x".into()));
        let mut l = led();
        l.exhausted_until = Some(NOW + 5);
        assert_eq!(go(&l, &p), Refusal::Exhausted { until: NOW + 5 });
        let mut l = led();
        l.limited_until = Some(NOW + 5);
        assert_eq!(go(&l, &p), Refusal::Limited { until: NOW + 5 });
        let mut l = led();
        l.cycle.spent = 97_000_000;
        assert_eq!(go(&l, &p), Refusal::Capped);
    }

    #[test]
    fn reserve_ok() {
        let l = reserve(&with_task(4_000_000), &pool(), "t", "h", 300_000, NOW).unwrap();
        assert_eq!(
            l.holds["h"],
            Hold {
                task: "t".into(),
                amount: 300_000,
                at: NOW
            }
        );
        assert_eq!((l.tasks["t"].held, l.tasks["t"].requests), (300_000, 1));
        assert_eq!((l.cycle.held, l.month.held), (300_000, 300_000));
    }

    #[test]
    fn reserve_task_refusals() {
        let l = with_task(4_000_000);
        let p = pool();
        assert_eq!(
            reserve(&l, &p, "x", "h", 1, NOW).unwrap_err(),
            Refusal::NoTask("x".into())
        );
        assert_eq!(
            reserve(&l, &p, "t", "h", 1, NOW + 1800).unwrap_err(),
            Refusal::Deadline("t".into())
        );
        let c = close_task(&l, "t", "done", NOW).unwrap();
        assert_eq!(
            reserve(&c, &p, "t", "h", 1, NOW).unwrap_err(),
            Refusal::TaskClosed("t".into())
        );
    }

    #[test]
    fn reserve_spent_bounds() {
        let p = pool();
        let spent = |bound| Refusal::Spent {
            task: "t".into(),
            bound,
        };
        let mut l = with_task(4_000_000);
        l.tasks.get_mut("t").unwrap().requests = 100;
        assert_eq!(
            reserve(&l, &p, "t", "h", 1, NOW).unwrap_err(),
            spent("requests")
        );
        let mut l = with_task(4_000_000);
        l.tasks.get_mut("t").unwrap().fresh_tokens = 1_000_000;
        assert_eq!(
            reserve(&l, &p, "t", "h", 1, NOW).unwrap_err(),
            spent("fresh tokens")
        );
        let l = with_task(4_000_000);
        assert_eq!(
            reserve(&l, &p, "t", "h", 4_000_001, NOW).unwrap_err(),
            spent("dollars")
        );
        assert!(reserve(&l, &p, "t", "h", 4_000_000, NOW).is_ok());
    }

    #[test]
    fn reserve_capped_by_the_stop_line() {
        let mut l = with_task(1_000_000);
        l.cycle.spent = 96_900_000;
        let e = reserve(&l, &pool(), "t", "h", 200_000, NOW).unwrap_err();
        assert_eq!(e, Refusal::Capped);
        let mut l = with_task(1_000_000);
        l.month.spent = 48_400_000;
        assert_eq!(
            reserve(&l, &pool(), "t", "h", 200_000, NOW).unwrap_err(),
            Refusal::Capped
        );
    }

    #[test]
    fn reserve_refused_states() {
        let l = pause(&with_task(1_000_000), "x");
        let e = reserve(&l, &pool(), "t", "h", 1, NOW).unwrap_err();
        assert_eq!(e, Refusal::Paused("x".into()));
        let mut l = with_task(1_000_000);
        l.exhausted_until = Some(NOW + 9);
        assert_eq!(
            reserve(&l, &pool(), "t", "h", 1, NOW).unwrap_err(),
            Refusal::Exhausted { until: NOW + 9 }
        );
    }

    #[test]
    fn reserve_duplicate_hold() {
        let l = reserve(&with_task(4_000_000), &pool(), "t", "h", 1, NOW).unwrap();
        assert_eq!(
            reserve(&l, &pool(), "t", "h", 1, NOW).unwrap_err(),
            Refusal::Duplicate("h".into())
        );
    }

    #[test]
    fn reserve_allowed_while_full() {
        let mut l = with_task(1_000_000);
        l.cycle.spent = 90_000_000;
        assert_eq!(state(&l, &pool(), NOW), State::Full);
        assert!(reserve(&l, &pool(), "t", "h", 100_000, NOW).is_ok());
    }

    fn held(amount: Micros) -> Ledger {
        reserve(&with_task(4_000_000), &pool(), "t", "h", amount, NOW).unwrap()
    }

    #[test]
    fn reconcile_below_and_equal() {
        for cost in [100_000, 300_000] {
            let (l, trip) = reconcile(&held(300_000), "h", cost, 50, "m", NOW).unwrap();
            assert!(trip.is_none() && l.paused.is_none());
            assert!(l.holds.is_empty());
            assert_eq!((l.tasks["t"].spent, l.tasks["t"].held), (cost, 0));
            assert_eq!(l.tasks["t"].fresh_tokens, 50);
            assert_eq!((l.cycle.spent, l.cycle.held), (cost, 0));
            assert_eq!((l.month.spent, l.month.held), (cost, 0));
        }
    }

    #[test]
    fn reconcile_above_trips_and_pauses() {
        let (l, trip) = reconcile(&held(300_000), "h", 400_000, 0, "m", NOW).unwrap();
        let t = trip.unwrap();
        assert_eq!(t.signal, Signal::Overrun);
        assert_eq!(t.why, "hold h reserved 0.30 and cost 0.40");
        assert_eq!(l.paused, Some(t.why));
        assert_eq!(l.cycle.spent, 400_000);
    }

    #[test]
    fn reconcile_no_hold() {
        assert_eq!(
            reconcile(&led(), "h", 1, 0, "m", NOW).unwrap_err(),
            Refusal::NoHold("h".into())
        );
    }

    #[test]
    fn reconcile_files_the_hour() {
        let at_t = NOW + 4000;
        let (l, _) = reconcile(&held(300_000), "h", 200_000, 0, "m", at_t).unwrap();
        assert_eq!(l.hours[&format!("{}/m", at_t / 3600 * 3600)], 200_000);
    }

    #[test]
    fn close_task_charges_holds_in_full() {
        let l = reserve(&held(300_000), &pool(), "t", "h2", 200_000, NOW).unwrap();
        let l = close_task(&l, "t", "done", NOW + 5).unwrap();
        let t = &l.tasks["t"];
        assert_eq!((t.spent, t.held), (500_000, 0));
        assert_eq!(
            (t.closed, t.outcome.as_deref()),
            (Some(NOW + 5), Some("done"))
        );
        assert!(l.holds.is_empty());
        assert_eq!((l.cycle.spent, l.cycle.held), (500_000, 0));
        let key = format!("{}/{UNRECONCILED}", (NOW + 5) / 3600 * 3600);
        assert_eq!(l.hours[&key], 500_000);
    }

    #[test]
    fn close_task_is_idempotent() {
        let l = close_task(&held(300_000), "t", "done", NOW).unwrap();
        assert_eq!(close_task(&l, "t", "again", NOW + 9).unwrap(), l);
        assert_eq!(
            close_task(&l, "x", "done", NOW).unwrap_err(),
            Refusal::NoTask("x".into())
        );
    }

    #[test]
    fn sweep_charges_a_stale_hold() {
        let l = held(300_000);
        let s = sweep(&l, &pool(), NOW + 899);
        assert_eq!(s.holds.len(), 1);
        let s = sweep(&l, &pool(), NOW + 900);
        assert!(s.holds.is_empty());
        assert_eq!(s.tasks["t"].spent, 300_000);
        assert!(s.tasks["t"].closed.is_none());
    }

    #[test]
    fn sweep_closes_a_late_task() {
        let l = with_task(1_000_000);
        assert!(sweep(&l, &pool(), NOW + 2399).tasks["t"].closed.is_none());
        let s = sweep(&l, &pool(), NOW + 2400);
        assert_eq!(s.tasks["t"].closed, Some(NOW + 2400));
        assert_eq!(s.tasks["t"].outcome.as_deref(), Some("deadline"));
    }

    #[test]
    fn signal_ok_and_rate_limited() {
        for s in [Signal::Ok, Signal::RateLimited] {
            assert_eq!(signal(&led(), s), (led(), None));
        }
    }

    #[test]
    fn signal_exhausted() {
        let (l, t) = signal(&led(), Signal::Exhausted);
        assert_eq!(l.exhausted_until, Some(l.cycle.end));
        let t = t.unwrap();
        assert_eq!(t.signal, Signal::Exhausted);
        assert_eq!(
            t.why,
            "the gateway says the credit is gone at 0.00 of 100.00 this cycle"
        );
        assert_eq!(l.paused, Some(t.why));
        let mut l = led();
        l.cycle.spent = 60_000_000;
        let (l, t) = signal(&l, Signal::Exhausted);
        assert!(t.is_none() && l.paused.is_none());
        assert_eq!(l.exhausted_until, Some(l.cycle.end));
    }

    #[test]
    fn signal_limited() {
        let (l, t) = signal(&led(), Signal::Limited);
        assert_eq!(l.limited_until, Some(l.month.end));
        assert_eq!(
            t.unwrap().why,
            "the gateway says a spend limit is reached at 0.00 of 50.00 this month"
        );
        let mut l = led();
        l.month.spent = 30_000_000;
        let (l, t) = signal(&l, Signal::Limited);
        assert!(t.is_none() && l.paused.is_none());
        assert_eq!(l.limited_until, Some(l.month.end));
    }

    #[test]
    fn signal_tripwires() {
        for (s, why) in [
            (Signal::Unauthorized, "the gateway refused the pool's key"),
            (Signal::WrongAccount, "a response named another workspace"),
            (Signal::Overrun, "a cost passed its reservation"),
        ] {
            let (l, t) = signal(&led(), s);
            let t = t.unwrap();
            assert_eq!((t.signal, t.why.as_str()), (s, why));
            assert_eq!(l.paused.as_deref(), Some(why));
        }
    }

    #[test]
    fn pause_and_resume() {
        let l = pause(&with_task(1_000_000), "x");
        assert_eq!(l.paused.as_deref(), Some("x"));
        let r = resume(&l);
        assert!(r.paused.is_none());
        assert_eq!(r.tasks, l.tasks);
    }

    #[test]
    fn refusal_display() {
        let u = day(2026, 12, 16);
        let cases = [
            (Refusal::Disabled, "pool: disabled"),
            (Refusal::Paused("x".into()), "pool: paused: x"),
            (Refusal::Capped, "pool: capped"),
            (Refusal::Full { until: u }, "pool: full until 2026-12-16"),
            (
                Refusal::Exhausted { until: u },
                "pool: credit exhausted until 2026-12-16",
            ),
            (
                Refusal::Limited { until: u },
                "pool: spend limit reached until 2026-12-16",
            ),
            (Refusal::Busy { running: 2 }, "wait: pool: 2 tasks running"),
            (Refusal::NoTask("t".into()), "pool: no task t"),
            (Refusal::TaskClosed("t".into()), "pool: task t is closed"),
            (
                Refusal::Deadline("t".into()),
                "pool: task t passed its deadline",
            ),
            (
                Refusal::Spent {
                    task: "t".into(),
                    bound: "requests",
                },
                "pool: task t spent its requests",
            ),
            (Refusal::NoHold("h".into()), "pool: no hold h"),
            (Refusal::Duplicate("h".into()), "pool: h exists"),
        ];
        for (r, s) in cases {
            assert_eq!(r.to_string(), s);
        }
    }

    #[test]
    fn state_serializes_tagged() {
        let j = serde_json::to_string(&State::Exhausted { until: 5 }).unwrap();
        assert_eq!(j, r#"{"state":"exhausted","until":5}"#);
        assert_eq!(
            serde_json::to_string(&State::Open).unwrap(),
            r#"{"state":"open"}"#
        );
    }

    #[test]
    fn the_stop_line_holds_under_load() {
        let mut p = pool();
        p.credit = 10_000_000;
        p.month_limit = 10_000_000;
        let mut b = budget(4_000_000);
        b.requests = 1000;
        let ok = |x: &Ledger| assert!(x.cycle.spent + x.cycle.held <= 9_700_000);
        let mut led = new(&p, NOW);
        for id in ["a", "b"] {
            led = admit_task(&led, &p, id, Class::Sonnet, b, NOW).unwrap();
            ok(&led);
        }
        let (mut count, mut refused) = (0, 0);
        while refused < 2 && count < 200 {
            let task = if count % 2 == 0 { "a" } else { "b" };
            match reserve(&led, &p, task, &format!("h{count}"), 300_000, NOW) {
                Ok(next) => {
                    led = next;
                    refused = 0;
                }
                Err(_) => refused += 1,
            }
            ok(&led);
            if count % 3 == 0 && led.holds.contains_key(&format!("h{count}")) {
                led = reconcile(&led, &format!("h{count}"), 300_000, 0, "m", NOW)
                    .unwrap()
                    .0;
                ok(&led);
            }
            count += 1;
        }
        assert!(refused >= 2 && count < 200);
    }
}
