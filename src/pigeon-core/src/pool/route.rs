//! Routing a task to the subscription or a pool; pure.

use serde::{Deserialize, Serialize};

use super::Micros;
use super::config::{Config, Prefer};
use super::ledger::State;
use crate::sessions::Class;

/// Where a brief says it runs.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunsOn {
    /// Wherever the router picks.
    #[default]
    Any,
    /// A session.
    Session,
    /// A pool.
    Pool,
}

/// One pool as the router sees it, in `.config/pool.yaml`'s order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    /// Its name.
    pub name: String,
    /// Its harness.
    pub harness: String,
    /// Its state.
    pub state: State,
    /// Tasks running.
    pub running: u32,
    /// Tasks it may run.
    pub concurrency: u32,
    /// Whether the task's budget fits its admit lines.
    pub admits: bool,
    /// When its cycle ends.
    pub cycle_end: u64,
    /// What its cycle's admit line leaves.
    pub headroom: Micros,
}

/// Where a task goes, and why when it is not a pool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "route", content = "why")]
pub enum Route {
    /// This pool.
    Pool(String),
    /// A session.
    Session(String),
    /// Neither yet.
    Wait(String),
}

/// Where a task of `class` and `harness` runs.
#[must_use]
pub fn route(
    runs_on: RunsOn,
    class: Class,
    harness: &str,
    cfg: &Config,
    pools: &[View],
    executor_sessions: u32,
    now: u64,
) -> Route {
    if class == Class::Opus {
        return Route::Session("opus never runs on a pool".into());
    }
    if runs_on == RunsOn::Session {
        return Route::Session("the brief runs on a session".into());
    }
    let none = |why: &str| {
        if runs_on == RunsOn::Pool {
            Route::Wait(why.into())
        } else {
            Route::Session(why.into())
        }
    };
    let open: Vec<&View> = pools
        .iter()
        .filter(|p| p.harness == harness && p.state == State::Open)
        .collect();
    if open.is_empty() {
        return none("no pool is open");
    }
    let fit: Vec<&View> = open.into_iter().filter(|p| p.admits).collect();
    if fit.is_empty() {
        return none("no pool admits the task's budget");
    }
    let free: Vec<&View> = fit
        .into_iter()
        .filter(|p| p.running < p.concurrency)
        .collect();
    let Some(first) = free.first() else {
        return none("every pool is busy");
    };
    if runs_on == RunsOn::Pool
        || cfg.prefer == Prefer::Pool
        || executor_sessions >= cfg.subscription_slots
    {
        return Route::Pool(first.name.clone());
    }
    let horizon = now.saturating_add(u64::from(cfg.drain_days) * 86_400);
    match free
        .iter()
        .find(|p| p.cycle_end <= horizon && p.headroom > 0)
    {
        Some(p) => Route::Pool(p.name.clone()),
        None => Route::Session("the subscription has room".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::config::config;

    const NOW: u64 = 1_795_000_000;
    const DAY: u64 = 86_400;

    fn cfg() -> Config {
        let mut c = config().unwrap();
        c.prefer = Prefer::Subscription;
        c.subscription_slots = 3;
        c.drain_days = 7;
        c
    }

    fn view(name: &str) -> View {
        View {
            name: name.into(),
            harness: "claude-code".into(),
            state: State::Open,
            running: 0,
            concurrency: 2,
            admits: true,
            cycle_end: NOW + 3 * DAY,
            headroom: 1_000_000,
        }
    }

    fn go(runs_on: RunsOn, class: Class, c: &Config, pools: &[View], sessions: u32) -> Route {
        route(runs_on, class, "claude-code", c, pools, sessions, NOW)
    }

    fn s(why: &str) -> Route {
        Route::Session(why.into())
    }

    fn w(why: &str) -> Route {
        Route::Wait(why.into())
    }

    fn p(name: &str) -> Route {
        Route::Pool(name.into())
    }

    #[test]
    fn opus_never_runs_on_a_pool() {
        for r in [RunsOn::Any, RunsOn::Pool, RunsOn::Session] {
            assert_eq!(
                go(r, Class::Opus, &cfg(), &[view("a")], 9),
                s("opus never runs on a pool")
            );
        }
    }

    #[test]
    fn a_session_brief_stays_on_a_session() {
        assert_eq!(
            go(RunsOn::Session, Class::Sonnet, &cfg(), &[view("a")], 9),
            s("the brief runs on a session")
        );
    }

    #[test]
    fn no_open_pool() {
        let mut closed = view("a");
        closed.state = State::Full;
        let mut other = view("b");
        other.harness = "other".into();
        let pools = [closed, other];
        assert_eq!(
            go(RunsOn::Any, Class::Sonnet, &cfg(), &pools, 0),
            s("no pool is open")
        );
        assert_eq!(
            go(RunsOn::Pool, Class::Sonnet, &cfg(), &pools, 0),
            w("no pool is open")
        );
        assert_eq!(
            go(RunsOn::Any, Class::Sonnet, &cfg(), &[], 0),
            s("no pool is open")
        );
    }

    #[test]
    fn no_pool_admits() {
        let mut a = view("a");
        a.admits = false;
        let why = "no pool admits the task's budget";
        assert_eq!(
            go(RunsOn::Any, Class::Haiku, &cfg(), &[a.clone()], 0),
            s(why)
        );
        assert_eq!(go(RunsOn::Pool, Class::Haiku, &cfg(), &[a], 0), w(why));
    }

    #[test]
    fn every_pool_busy() {
        let mut a = view("a");
        a.running = 2;
        assert_eq!(
            go(RunsOn::Any, Class::Haiku, &cfg(), &[a.clone()], 0),
            s("every pool is busy")
        );
        assert_eq!(
            go(RunsOn::Pool, Class::Haiku, &cfg(), &[a], 0),
            w("every pool is busy")
        );
    }

    #[test]
    fn pool_brief_takes_the_first_free_pool() {
        let mut a = view("a");
        a.running = 2;
        let pools = [a, view("b"), view("c")];
        assert_eq!(go(RunsOn::Pool, Class::Sonnet, &cfg(), &pools, 0), p("b"));
    }

    #[test]
    fn prefer_pool_takes_the_first_free_pool() {
        let mut c = cfg();
        c.prefer = Prefer::Pool;
        let mut far = view("a");
        far.cycle_end = NOW + 30 * DAY;
        assert_eq!(go(RunsOn::Any, Class::Sonnet, &c, &[far], 0), p("a"));
    }

    #[test]
    fn full_subscription_overflows_to_a_pool() {
        let mut far = view("a");
        far.cycle_end = NOW + 30 * DAY;
        assert_eq!(
            go(RunsOn::Any, Class::Sonnet, &cfg(), &[far.clone()], 3),
            p("a")
        );
        assert_eq!(
            go(RunsOn::Any, Class::Sonnet, &cfg(), &[far], 2),
            s("the subscription has room")
        );
    }

    #[test]
    fn drain_inside_and_outside_the_days() {
        let mut a = view("a");
        a.cycle_end = NOW + 7 * DAY;
        assert_eq!(
            go(RunsOn::Any, Class::Sonnet, &cfg(), &[a.clone()], 0),
            p("a")
        );
        a.cycle_end = NOW + 7 * DAY + 1;
        assert_eq!(
            go(RunsOn::Any, Class::Sonnet, &cfg(), &[a], 0),
            s("the subscription has room")
        );
    }

    #[test]
    fn no_headroom_no_drain() {
        let mut a = view("a");
        a.headroom = 0;
        assert_eq!(
            go(RunsOn::Any, Class::Sonnet, &cfg(), &[a], 0),
            s("the subscription has room")
        );
    }

    #[test]
    fn the_first_free_pool_wins() {
        let mut a = view("a");
        a.running = 2;
        let pools = [a, view("b"), view("c")];
        assert_eq!(go(RunsOn::Any, Class::Sonnet, &cfg(), &pools, 0), p("b"));
        let pools = [view("c"), view("b")];
        assert_eq!(go(RunsOn::Any, Class::Sonnet, &cfg(), &pools, 0), p("c"));
    }

    #[test]
    fn drain_skips_pools_outside_the_days() {
        let mut a = view("a");
        a.cycle_end = NOW + 20 * DAY;
        let pools = [a, view("b")];
        assert_eq!(go(RunsOn::Any, Class::Sonnet, &cfg(), &pools, 0), p("b"));
    }

    #[test]
    fn route_serializes_tagged() {
        let j = serde_json::to_string(&p("a")).unwrap();
        assert_eq!(j, r#"{"route":"pool","why":"a"}"#);
        let r: RunsOn = serde_json::from_str("\"pool\"").unwrap();
        assert_eq!(r, RunsOn::Pool);
        assert_eq!(RunsOn::default(), RunsOn::Any);
    }
}
