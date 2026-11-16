//! The platform job's arguments, state bucket and refusals; see
//! `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md`, step 4's amendment.

use backplane::platform_job::{Task, destroys, state_bucket};

fn argv(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn a_job_plans_or_applies_one_platform_unit() {
    assert_eq!(
        Task::parse(&argv(&["plan", "platform/blue"])),
        Ok(Task::Plan {
            unit: "platform/blue".into()
        })
    );
    assert_eq!(
        Task::parse(&argv(&["apply", "platform/blue", "p1"])),
        Ok(Task::Apply {
            unit: "platform/blue".into(),
            plan: "p1".into()
        })
    );
    for bad in [
        vec!["plan", "infra/pigeon"],
        vec!["plan", "platform/"],
        vec!["plan", "platform/Blue"],
        vec!["apply", "platform/blue"],
        vec!["destroy", "platform/blue"],
    ] {
        assert!(Task::parse(&argv(&bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn the_state_bucket_is_the_one_suffixed_bucket_of_its_name() {
    let names = argv(&[
        "pigeon-ci-x1",
        "pigeon-platform-state-a1b2c3d4",
        "pigeon-platform-statement",
    ]);
    assert_eq!(
        state_bucket(&names),
        Ok(Some("pigeon-platform-state-a1b2c3d4".into()))
    );
    assert_eq!(state_bucket(&argv(&["pigeon-ci-x1"])), Ok(None));
    assert!(
        state_bucket(&argv(&[
            "pigeon-platform-state-a",
            "pigeon-platform-state-b"
        ]))
        .is_err()
    );
}

#[test]
fn a_plan_that_destroys_anything_is_refused() {
    let change = |address: &str, action| backplane::cobot::Change {
        address: address.into(),
        action,
        attributes: Vec::new(),
    };
    let safe = [change(
        "module.d1.cloudflare_d1_database.this",
        backplane::cobot::Action::Update,
    )];
    assert_eq!(destroys(&safe), None);
    let gone = [
        change(
            "module.queue.cloudflare_queue.this",
            backplane::cobot::Action::Create,
        ),
        change(
            "module.buckets[\"pigeon-ci\"].cloudflare_r2_bucket.this",
            backplane::cobot::Action::Replace,
        ),
    ];
    let why = destroys(&gone).unwrap_or_default();
    assert!(why.contains("pigeon-ci"), "{why}");
}
