//! The fast lane's rules as tables: who a lane covers, what it waives, and its grant.

use super::{DEFAULT_MINUTES, Lanes, Line, MAX_MINUTES, NEVER_WAIVED, State, covers, waivable};
use crate::approval::tiers::{AUTO_COVERS, Tier, Tiered, Tiers};
use crate::change::Change;

const NOW: u64 = 1_800_000_000;
const SESSION: &str = "https://claude.ai/code/session_a";
const HEAD: &str = "1111111111111111111111111111111111111111";

fn change(n: u32, scope: &str, tier: Tier, rule: &str) -> Change {
    let mut c = Change::propose(n, "x", "fix: x", "p", vec![scope.into()], "agent");
    c.session = Some(SESSION.into());
    c.head = Some(HEAD.into());
    let t = |tier, rule: &str| Tiered {
        tier,
        rule: rule.into(),
    };
    c.tiers = Some(Tiers {
        authorize: t(Tier::Code, "rule"),
        merge: t(tier, rule),
        close: t(Tier::Confirm, "rule"),
    });
    c
}

fn open(line: Line) -> Lanes {
    let (lanes, asked) =
        Lanes::default().request(line, None, "pool config", SESSION, None, "tok", NOW);
    asked.expect("asked");
    let (lanes, granted) = lanes.grant("tok", "owner", NOW);
    granted.expect("granted");
    lanes
}

#[test]
fn a_lane_never_waives_the_bounds_policies_restack_mirror_or_merge() {
    for job in NEVER_WAIVED {
        assert!(!waivable(job), "{job}");
        assert!(!waivable(&format!("gate: {job}")), "gate: {job}");
    }
    for job in ["fmt", "clippy", "tests: pigeon-core", "gate: clippy"] {
        assert!(waivable(job), "{job}");
    }
}

#[test]
fn a_lane_covers_its_line_but_never_an_owner_guarded_judged_or_gate_head() {
    let lanes = open(Line::Paths(vec!["infra/pigeon/**".into()]));
    let lane = lanes.current(NOW).expect("open");
    let gate = "guarded: src/pigeon-core/src/approval/tiers.rs";
    let covered = format!("guarded: .config/x.yaml{AUTO_COVERS}");
    let table: Vec<(&str, Change, bool)> = vec![
        (
            "routine",
            change(1, "infra/pigeon/a.yaml", Tier::Confirm, "routine"),
            true,
        ),
        (
            "deploys",
            change(2, "infra/pigeon/b/**", Tier::Code, "deploys"),
            true,
        ),
        (
            "outside",
            change(3, "src/**", Tier::Confirm, "routine"),
            false,
        ),
        (
            "owner",
            change(4, "infra/pigeon/a.yaml", Tier::Owner, "secrets: a"),
            false,
        ),
        (
            "gate",
            change(5, "infra/pigeon/a.yaml", Tier::Code, gate),
            false,
        ),
        (
            "Auto-covered guarded",
            change(6, "infra/pigeon/a.yaml", Tier::Code, &covered),
            false,
        ),
        (
            "eviction",
            change(7, "infra/pigeon/a.yaml", Tier::Code, "eviction: x"),
            false,
        ),
    ];
    for (name, c, ok) in table {
        assert_eq!(covers(lane, &c, NOW).is_ok(), ok, "{name}");
    }
    let mut untiered = change(9, "infra/pigeon/a.yaml", Tier::Confirm, "routine");
    untiered.tiers = None;
    assert!(covers(lane, &untiered, NOW).is_err(), "no tiers yet");
    let mut briefed = change(10, "infra/pigeon/a.yaml", Tier::Confirm, "routine");
    briefed.brief = Some("b1".into());
    assert!(
        covers(lane, &briefed, NOW).is_err(),
        "a brief's head awaiting review"
    );
}

#[test]
fn a_lane_names_a_session_or_changes() {
    let c = change(7, "src/**", Tier::Confirm, "routine");
    let by_session = open(Line::Session(SESSION.into()));
    assert!(covers(by_session.current(NOW).unwrap(), &c, NOW).is_ok());
    let by_number = open(Line::Changes(vec![7]));
    assert!(covers(by_number.current(NOW).unwrap(), &c, NOW).is_ok());
    let other = open(Line::Changes(vec![8]));
    assert!(covers(other.current(NOW).unwrap(), &c, NOW).is_err());
}

#[test]
fn a_lane_lasts_its_minutes_and_settle_ends_it() {
    let lanes = open(Line::Changes(vec![1]));
    let lane = lanes.current(NOW).expect("open").clone();
    assert_eq!(lane.until, Some(NOW + u64::from(DEFAULT_MINUTES) * 60));
    let end = lane.until.unwrap();
    assert!(lanes.current(end - 1).is_some());
    assert!(lanes.current(end).is_none(), "closed at its end, unasked");
    let (settled, ended) = lanes.settle(end);
    assert_eq!(ended.len(), 1);
    assert_eq!(settled.get(lane.id).unwrap().state, State::Ended);
}

#[test]
fn a_request_opens_nothing_and_its_minutes_are_bounded() {
    let (lanes, asked) = Lanes::default().request(
        Line::Changes(vec![1]),
        Some(10),
        "why",
        SESSION,
        None,
        "tok",
        NOW,
    );
    assert_eq!(asked.expect("asked").state, State::Offered);
    assert!(lanes.current(NOW).is_none(), "asking opens nothing");
    for (minutes, reason, ok) in [
        (Some(0), "why", false),
        (Some(MAX_MINUTES), "why", true),
        (Some(MAX_MINUTES + 1), "why", false),
        (None, " ", false),
    ] {
        let (_, r) = Lanes::default().request(
            Line::Changes(vec![1]),
            minutes,
            reason,
            SESSION,
            None,
            "t",
            NOW,
        );
        assert_eq!(r.is_ok(), ok, "{minutes:?} {reason:?}");
    }
}

#[test]
fn a_grant_is_spent_once_and_a_second_lane_is_refused() {
    let lanes = open(Line::Changes(vec![1]));
    let (_, again) = lanes.grant("tok", "owner", NOW);
    assert!(again.is_err(), "once only");
    let (lanes, _) = lanes.request(
        Line::Changes(vec![2]),
        None,
        "more",
        SESSION,
        None,
        "t2",
        NOW,
    );
    let (lanes, second) = lanes.grant("t2", "owner", NOW);
    assert!(second.is_err(), "one lane at a time");
    assert_eq!(lanes.list.iter().filter(|l| l.open(NOW)).count(), 1);
}

#[test]
fn an_extension_widens_and_moves_the_open_lane_end() {
    let lanes = open(Line::Changes(vec![1]));
    let id = lanes.current(NOW).unwrap().id;
    let later = NOW + 600;
    let (lanes, _) = lanes.request(
        Line::Changes(vec![2]),
        Some(MAX_MINUTES),
        "more",
        SESSION,
        Some(id),
        "t2",
        later,
    );
    let (lanes, granted) = lanes.grant("t2", "owner", later);
    let lane = granted.expect("granted");
    assert_eq!(lane.id, id);
    assert_eq!(lane.line, Line::Changes(vec![1, 2]));
    assert_eq!(lane.until, Some(later + u64::from(MAX_MINUTES) * 60));
    assert_eq!(lanes.list.iter().filter(|l| l.open(later)).count(), 1);
    let (lanes, kind) = lanes.request(
        Line::Paths(vec!["a/**".into()]),
        None,
        "x",
        SESSION,
        Some(id),
        "t3",
        later,
    );
    kind.expect("asking is open");
    let (lanes, refused) = lanes.grant("t3", "owner", later);
    let why = refused.expect_err("another kind of line doesn't widen it");
    assert!(why.contains("same kind"), "{why}");
    assert_eq!(lanes.get(id).unwrap().line, Line::Changes(vec![1, 2]));
}

#[test]
fn a_waiver_is_recorded_only_for_a_covered_head_and_a_waivable_job() {
    let lanes = open(Line::Changes(vec![1]));
    let c = change(1, "a/**", Tier::Confirm, "routine");
    let (lanes, id) = lanes.waive(&c, "clippy", NOW).expect("waived");
    assert_eq!(lanes.get(id).unwrap().waived.len(), 1);
    assert!(lanes.waive(&c, "bound", NOW).is_err());
    assert!(lanes.waive(&c, "gate: policy.check", NOW).is_err());
    let outside = change(2, "a/**", Tier::Confirm, "routine");
    assert!(lanes.waive(&outside, "clippy", NOW).is_err());
    let (ended, _) = lanes.end_open("deploy failed", NOW);
    assert!(
        ended.waive(&c, "clippy", NOW).is_err(),
        "an ended lane waives nothing"
    );
}
