use super::*;
use crate::brief::{Commit, Draft, Step};
use crate::pool::config::config;
use crate::pool::job::{self, Outcome, Pooled};
use crate::pool::ledger::State;
use crate::pool::route::RunsOn;
use crate::sessions::Declared;
use crate::tickets::{Ended, Issued, Tickets};
use serde_json::json;

const NOW: u64 = 1_795_000_000;

fn draft(usd: Option<&str>) -> Draft {
    Draft {
        title: "t".into(),
        adr: ".claude/decisions/a.md".into(),
        step: "1".into(),
        model: Class::Sonnet,
        base: "7bb3087".into(),
        scope: vec!["a".into()],
        decisions: vec![],
        steps: vec![Step::default()],
        checks: vec![],
        stop_if: vec![],
        budget: None,
        runs_on: RunsOn::Any,
        pool_usd: usd.map(str::to_string),
        pool_minutes: None,
        commit: Commit::default(),
    }
}

fn pool(enabled: bool) -> Pool {
    let c = config().unwrap();
    let mut p = c.pools.values().next().unwrap().clone();
    p.enabled = enabled;
    p
}

fn sonnet() -> Budget {
    config().unwrap().budget(Class::Sonnet, None, None).unwrap()
}

fn issued(model: Class, expires: u64) -> Issued {
    Issued {
        generation: 1,
        expires,
        declared: Declared {
            model,
            planner: None,
            brief: None,
        },
        parent: None,
        ended: None,
        archived: None,
    }
}

fn pooled(outcome: Outcome, pushed: Option<&str>, escalation: Option<serde_json::Value>) -> Pooled {
    Pooled {
        outcome,
        why: "why".into(),
        turns: 1,
        harness_cost: None,
        checks: vec![],
        escalation,
        pushed: pushed.map(str::to_string),
    }
}

#[test]
fn label_and_task_id_text() {
    assert_eq!(label("b5", 2), "pool:b5.2");
    assert_eq!(task_id("b5", 2), "b5.2");
}

#[test]
fn next_k_starts_at_one() {
    assert_eq!(next_k([], "b5"), 1);
    assert_eq!(next_k(["session_x", "pool:b6.4"], "b5"), 1);
}

#[test]
fn next_k_follows_the_largest() {
    let labels = [
        "pool:b5.1",
        "pool:b5.3",
        "pool:b6.9",
        "https://claude.ai/code/s",
    ];
    assert_eq!(next_k(labels, "b5"), 4);
}

#[test]
fn next_k_ignores_a_malformed_label() {
    assert_eq!(next_k(["pool:b5.x", "pool:b5.2.1", "pool:b5."], "b5"), 1);
    assert_eq!(next_k(["pool:b5.x", "pool:b5.2"], "b5"), 3);
}

#[test]
fn models_come_from_models_json() {
    assert_eq!(model(Class::Haiku).unwrap(), "claude-haiku-5-5");
    assert_eq!(model(Class::Sonnet).unwrap(), "claude-sonnet-5-5");
    assert_eq!(model(Class::Opus).unwrap(), "claude-opus-5-5");
}

#[test]
fn executors_count_live_haiku_and_sonnet() {
    let mut t = Tickets::default();
    t.0.insert("a".into(), issued(Class::Sonnet, NOW + 10));
    t.0.insert("b".into(), issued(Class::Haiku, NOW + 10));
    assert_eq!(executors(&t, NOW), 2);
}

#[test]
fn executors_skip_expired_ended_opus_and_pool_labels() {
    let mut t = Tickets::default();
    t.0.insert("live".into(), issued(Class::Sonnet, NOW + 10));
    t.0.insert("expired".into(), issued(Class::Sonnet, NOW));
    let mut ended = issued(Class::Sonnet, NOW + 10);
    ended.ended = Some(Ended {
        at: NOW,
        clean: true,
        archiver: None,
        lapsed: false,
    });
    t.0.insert("ended".into(), ended);
    t.0.insert("planner".into(), issued(Class::Opus, NOW + 10));
    t.0.insert("pool:b1.1".into(), issued(Class::Sonnet, NOW + 10));
    assert_eq!(executors(&t, NOW), 1);
}

#[test]
fn an_unused_enabled_pool_is_open_and_admits() {
    let v = view(&pool(true), None, Class::Sonnet, sonnet(), NOW);
    assert_eq!(v.state, State::Open);
    assert_eq!(v.running, 0);
    assert!(v.admits);
}

#[test]
fn a_disabled_pool_does_not_admit() {
    let v = view(&pool(false), None, Class::Sonnet, sonnet(), NOW);
    assert!(!v.admits);
}

#[test]
fn a_view_carries_the_pool_s_names() {
    let p = pool(true);
    let v = view(&p, None, Class::Sonnet, sonnet(), NOW);
    assert_eq!(v.name, p.name);
    assert_eq!(v.harness, p.harness);
    assert_eq!(v.concurrency, p.concurrency);
}

#[test]
fn budget_defaults_without_overrides() {
    let c = config().unwrap();
    assert_eq!(budget(&c, &draft(None)).unwrap(), sonnet());
}

#[test]
fn budget_above_the_maximum_is_refused() {
    assert!(budget(&config().unwrap(), &draft(Some("9"))).is_err());
}

#[test]
fn budget_takes_an_override() {
    let b = budget(&config().unwrap(), &draft(Some("5"))).unwrap();
    assert_eq!(b.usd, 5_000_000);
}

#[test]
fn budget_refuses_text_that_is_no_amount() {
    assert_eq!(
        budget(&config().unwrap(), &draft(Some("x"))),
        Err("`pool_usd` is not a dollar amount".into())
    );
}

#[test]
fn pool_url_text() {
    assert_eq!(pool_url("h.dev", "p"), "https://h.dev/v0/pool/p");
}

#[test]
fn expiry_and_loss() {
    let b = Budget {
        minutes: 40,
        ..sonnet()
    };
    assert_eq!(token_expiry(NOW, &b), NOW + 2400);
    assert_eq!(lost_at(NOW, &b), NOW + 2400 + LOST_AFTER);
}

#[test]
fn job_env_parses_as_a_job() {
    let s = Secrets {
        task_token: "t",
        access_id: "i",
        access_secret: "s",
    };
    let brief = json!({"steps": []});
    let env = job_env("b1.1", "https://h/v0/pool/p", "m", &brief, &sonnet(), &s);
    assert_eq!(env.len(), 10);
    assert_eq!(env[job::USD], "4.00");
    let j = job::parse(&env).unwrap();
    assert_eq!(j.task, "b1.1");
    assert_eq!(j.token, "t");
    assert_eq!(j.model, "m");
    assert_eq!(j.brief, brief);
}

#[test]
fn done_with_a_head_is_pushed() {
    let p = pooled(Outcome::Done, Some("abc"), None);
    assert_eq!(
        action(Some(&p)),
        (Action::Pushed { head: "abc".into() }, "done".into())
    );
}

fn esc(question: &str, tried: &str) -> Action {
    Action::Escalate {
        question: question.into(),
        tried: tried.into(),
    }
}

#[test]
fn done_without_a_head_escalates() {
    let p = pooled(Outcome::Done, None, None);
    assert_eq!(
        action(Some(&p)),
        (
            esc("the pool task finished but pushed nothing", "why"),
            "failed".into()
        )
    );
}

#[test]
fn escalated_carries_its_question() {
    let e = json!({"question": "q?", "tried": "t"});
    let p = pooled(Outcome::Escalated, None, Some(e));
    assert_eq!(action(Some(&p)), (esc("q?", "t"), "escalated".into()));
}

#[test]
fn escalated_without_a_question_uses_why() {
    let p = pooled(Outcome::Escalated, None, None);
    assert_eq!(action(Some(&p)), (esc("why", ""), "escalated".into()));
}

#[test]
fn stopped_escalates_with_its_reason() {
    let p = pooled(Outcome::Stopped, None, None);
    assert_eq!(
        action(Some(&p)),
        (
            esc("the pool task stopped: why", ""),
            "stopped (why)".into()
        )
    );
}

#[test]
fn failed_escalates_with_its_reason() {
    let p = pooled(Outcome::Failed, None, None);
    assert_eq!(
        action(Some(&p)),
        (esc("the pool task failed: why", ""), "failed".into())
    );
}

#[test]
fn a_lost_job_escalates() {
    assert_eq!(
        action(None),
        (esc("the pool task's job was lost", ""), "failed".into())
    );
}

#[test]
fn only_open_unclaimed_brief_tasks_wait() {
    let task = |id: u32, kind: &str, brief: Option<&str>| crate::tasks::Task {
        id,
        kind: kind.into(),
        change: 0,
        parent: None,
        session: None,
        note: String::new(),
        created: NOW,
        claimed_by: None,
        done: false,
        brief: brief.map(Into::into),
        model: Some(Class::Haiku),
    };
    let mut claimed = task(1, "brief", Some("b1"));
    claimed.claimed_by = Some("s".into());
    let mut done = task(2, "brief", Some("b2"));
    done.done = true;
    let t = crate::tasks::Tasks {
        last: 5,
        list: vec![
            claimed,
            done,
            task(3, "brief", Some("b3")),
            task(4, "review", None),
            task(5, "brief", Some("b5")),
        ],
    };
    assert_eq!(waiting(&t), vec![(3, "b3".into()), (5, "b5".into())]);
}

#[test]
fn a_job_is_lost_after_two_404s_a_sweep_apart() {
    assert_eq!(gone(None, true, NOW), (Some(NOW), false));
    assert_eq!(gone(Some(NOW), true, NOW + SWEEP - 1), (Some(NOW), false));
    assert_eq!(gone(Some(NOW), true, NOW + SWEEP), (Some(NOW), true));
}

#[test]
fn any_answer_but_404_clears_the_first() {
    assert_eq!(gone(Some(NOW), false, NOW + 10 * SWEEP), (None, false));
    assert_eq!(gone(None, false, NOW), (None, false));
}

#[test]
fn only_a_first_task_that_spent_nothing_runs_again() {
    assert!(runs_again("pool:b7.1", 0));
    assert!(!runs_again("pool:b7.1", 1));
    assert!(!runs_again("pool:b7.2", 0));
    assert!(!runs_again("pool:b7", 0));
}
