//! Briefs; see `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §3.

use backplane::core::brief::{Brief, Budget, Commit, Draft, Step, open_choice, validate};
use backplane::sessions::Class;

const PLANNER: &str = "https://claude.ai/code/planner";

fn step(files: &[&str], content: Option<&str>) -> Step {
    Step {
        act: "Add `Class::covers` after `Class::role`.".into(),
        files: files.iter().map(|f| (*f).to_string()).collect(),
        content: content.map(str::to_string),
        done_when: "`cargo test -p pigeon-backplane --test sessions` passes".into(),
    }
}

fn draft(model: Class) -> Draft {
    Draft {
        title: "Class::covers".into(),
        adr: ".claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md".into(),
        step: "1".into(),
        model,
        base: "7bb3087".into(),
        scope: vec!["src/sessions.rs".into(), "tests/sessions.rs".into()],
        decisions: vec!["`covers` compares classes by capability".into()],
        steps: vec![step(
            &["src/sessions.rs"],
            Some("pub fn covers(self, n: Self) -> bool { self >= n }"),
        )],
        checks: vec!["CHANGE_BASE=artifacts/main mise run ci-scoped".into()],
        stop_if: vec!["a test outside tests/sessions.rs fails".into()],
        budget: None,
        commit: Commit {
            title: "feat(backplane): Class::covers".into(),
            plan: "Adds Class::covers, per the brief.".into(),
        },
    }
}

fn has(problems: &[String], text: &str) -> bool {
    problems.iter().any(|p| p.contains(text))
}

#[test]
fn an_explicit_brief_passes_for_haiku_and_sonnet() {
    assert_eq!(validate(&draft(Class::Haiku)), Vec::<String>::new());
    assert_eq!(validate(&draft(Class::Sonnet)), Vec::<String>::new());
}

#[test]
fn a_brief_is_never_for_opus() {
    assert!(has(&validate(&draft(Class::Opus)), "opus session plans"));
}

#[test]
fn a_haiku_step_carries_its_content_and_a_sonnet_step_need_not() {
    let mut d = draft(Class::Haiku);
    d.steps = vec![step(&["src/sessions.rs"], None)];
    assert!(has(&validate(&d), "carries its `content`"));
    d.model = Class::Sonnet;
    assert_eq!(validate(&d), Vec::<String>::new());
}

#[test]
fn a_haiku_brief_touches_at_most_five_paths_and_any_brief_twelve() {
    let mut d = draft(Class::Haiku);
    d.scope = (0..6).map(|i| format!("src/f{i}.rs")).collect();
    d.steps = vec![step(&["src/f0.rs"], Some("x"))];
    assert!(has(&validate(&d), "1 to 5 paths"));
    d.model = Class::Sonnet;
    assert_eq!(validate(&d), Vec::<String>::new());
    d.scope = (0..13).map(|i| format!("src/f{i}.rs")).collect();
    assert!(has(&validate(&d), "1 to 12 paths"));
}

#[test]
fn scope_paths_are_exact_and_steps_stay_inside_them() {
    let mut d = draft(Class::Sonnet);
    for bad in [
        "src/**", "src/*.rs", "/abs.rs", "../up.rs", "a//b.rs", " lead.rs",
    ] {
        d.scope = vec![bad.into()];
        d.steps = vec![step(&[bad], None)];
        assert!(has(&validate(&d), "exact relative path"), "{bad}");
    }
    let mut d = draft(Class::Sonnet);
    d.steps = vec![step(&["src/other.rs"], None)];
    assert!(has(&validate(&d), "isn't in `scope`"));
}

#[test]
fn a_step_that_leaves_a_choice_is_refused() {
    for text in [
        "Choose a suitable name for the helper.",
        "Add tests as needed.",
        "Decide where the type lives.",
        "Refactor if appropriate.",
        "Handle errors, logging, etc.",
        "Name it TBD.",
        "Use `Vec` or similar.",
    ] {
        let mut d = draft(Class::Sonnet);
        d.steps[0].act = text.into();
        assert!(has(&validate(&d), "leaves a choice"), "{text}");
    }
    let mut d = draft(Class::Sonnet);
    d.steps[0].done_when = "Consider whether it compiles.".into();
    assert!(has(&validate(&d), "`done_when` leaves a choice"));
}

#[test]
fn open_choice_matches_whole_words_only() {
    assert_eq!(open_choice("Choose one"), Some("choose"));
    assert_eq!(open_choice("the decided order stays"), None);
    assert_eq!(open_choice("reconsidered"), None);
    assert_eq!(open_choice("chooser.rs keeps its name"), None);
    assert_eq!(open_choice("lists, etc."), Some("etc."));
    // Content is literal text, so a `choose` there is the planner's own words.
    let mut d = draft(Class::Haiku);
    d.steps[0].content = Some("fn choose() {}".into());
    assert_eq!(validate(&d), Vec::<String>::new());
}

#[test]
fn required_lines_title_base_and_commit_are_checked() {
    let mut d = draft(Class::Sonnet);
    d.decisions = vec![" ".into()];
    d.checks.clear();
    d.stop_if.clear();
    d.steps.clear();
    d.base = "main".into();
    d.adr = "docs/adr.md".into();
    d.commit.title = "Add covers".into();
    let p = validate(&d);
    for want in [
        "`decisions` holds",
        "`checks` holds",
        "`stop_if` holds",
        "at least one step",
        "`base` is a commit",
        "`adr` is the ADR's path",
        "Conventional Commit",
    ] {
        assert!(has(&p, want), "{want}: {p:?}");
    }
}

#[test]
fn a_budget_defaults_by_class_and_is_bounded() {
    assert_eq!(draft(Class::Haiku).budget(), Budget { heads: 3, hours: 2 });
    assert_eq!(draft(Class::Sonnet).budget(), Budget { heads: 4, hours: 2 });
    let mut d = draft(Class::Sonnet);
    d.budget = Some(Budget { heads: 0, hours: 2 });
    assert!(has(&validate(&d), "`budget`"));
    d.budget = Some(Budget { heads: 4, hours: 9 });
    assert!(has(&validate(&d), "`budget`"));
}

#[test]
fn a_brief_is_append_only_and_only_its_planner_amends_it() {
    let b = Brief::create(7, PLANNER, draft(Class::Haiku), 10).expect("valid");
    assert_eq!((b.id.as_str(), b.revision()), ("b7", 1));
    let mut next = draft(Class::Haiku);
    next.decisions.push("errors are `String`s".into());
    assert!(
        b.amend("https://claude.ai/code/other", next.clone(), "why", 11)
            .is_err()
    );
    assert!(b.amend(PLANNER, next.clone(), " ", 11).is_err());
    let a = b
        .amend(PLANNER, next, "the executor asked about errors", 11)
        .expect("amends");
    assert_eq!(a.revision(), 2);
    assert_eq!(
        a.revisions[0], b.revisions[0],
        "the first revision is untouched"
    );
    assert_eq!(a.latest().reason, "the executor asked about errors");
    let mut bad = draft(Class::Haiku);
    bad.steps[0].content = None;
    assert!(a.amend(PLANNER, bad, "drop content", 12).is_err());
}

#[test]
fn a_brief_round_trips_with_do_as_its_step_field() {
    let b = Brief::create(1, PLANNER, draft(Class::Sonnet), 1).expect("valid");
    let v = serde_json::to_value(&b).expect("json");
    assert!(v["revisions"][0]["draft"]["steps"][0]["do"].is_string());
    assert_eq!(v["revisions"][0]["draft"]["model"], "sonnet");
    let back: Brief = serde_json::from_value(v).expect("reads back");
    assert_eq!(back, b);
}

#[test]
fn the_store_numbers_briefs_and_amends_only_by_their_planner() {
    use backplane::briefs::{Briefs, number};
    let mut all = Briefs::default();
    let a = all.create(PLANNER, draft(Class::Haiku), 1).expect("b1");
    let b = all
        .create("https://claude.ai/code/other", draft(Class::Sonnet), 2)
        .expect("b2");
    assert_eq!((a.id.as_str(), b.id.as_str()), ("b1", "b2"));
    assert!(all.create(PLANNER, draft(Class::Opus), 3).is_err());
    assert_eq!(all.last, 2, "a refused brief takes no number");
    assert_eq!(
        (number("b7"), number("7"), number("b0"), number("x")),
        (Some(7), Some(7), None, None)
    );
    assert!(
        all.amend(
            "b1",
            "https://claude.ai/code/other",
            draft(Class::Haiku),
            "why",
            4
        )
        .is_err()
    );
    assert!(
        all.amend("b9", PLANNER, draft(Class::Haiku), "why", 4)
            .is_err()
    );
    let r2 = all
        .amend("1", PLANNER, draft(Class::Haiku), "tighter content", 4)
        .expect("amends");
    assert_eq!(r2.revision(), 2);
    assert_eq!(all.get("b1").map(Brief::revision), Some(2));
    let mine: Vec<&str> = all
        .list(Some(PLANNER))
        .iter()
        .map(|b| b.id.as_str())
        .collect();
    assert_eq!(mine, ["b1"]);
    let every: Vec<&str> = all.list(None).iter().map(|b| b.id.as_str()).collect();
    assert_eq!(every, ["b2", "b1"], "newest first");
}

#[test]
fn a_brief_never_touches_an_adr() {
    let mut d = draft(Class::Sonnet);
    let adr = ".claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md";
    d.scope.push(adr.into());
    assert!(has(&validate(&d), "the planner writes ADRs"));
}

mod executor {
    use super::{Class, PLANNER, draft};
    use backplane::core::brief::Brief;
    use backplane::core::brief::executor::{Proposal, Proposer, check, commit};

    fn brief(model: Class) -> Brief {
        Brief::create(4, PLANNER, draft(model), 1).expect("valid")
    }

    fn p<'a>(scope: &'a [String], brief: Option<&'a str>) -> Proposal<'a> {
        Proposal {
            scope,
            affects: &[],
            mandate: None,
            brief,
        }
    }

    fn who(class: Class, declared: Option<&str>) -> Proposer<'_> {
        Proposer { class, declared }
    }

    #[test]
    fn a_planner_proposes_freely_and_an_executor_only_its_brief() {
        let none: Vec<String> = Vec::new();
        assert_eq!(
            check(who(Class::Opus, None), p(&[], None), None, None),
            none
        );
        let refused = check(who(Class::Haiku, Some("b4")), p(&[], None), None, None);
        assert!(refused[0].contains("executes a brief"), "{refused:?}");
        let b = brief(Class::Haiku);
        let own = check(
            who(Class::Haiku, Some("b4")),
            p(&[], Some("b4")),
            Some(&b),
            None,
        );
        assert_eq!(own, none);
    }

    #[test]
    fn an_executor_is_refused_what_its_brief_doesnt_allow() {
        let b = brief(Class::Sonnet);
        let has = |v: &[String], s: &str| v.iter().any(|x| x.contains(s));
        let v = check(who(Class::Haiku, None), p(&[], Some("b4")), Some(&b), None);
        assert!(has(&v, "a haiku session can't carry it out"), "{v:?}");
        let v = check(
            who(Class::Sonnet, Some("b9")),
            p(&[], Some("b4")),
            Some(&b),
            None,
        );
        assert!(has(&v, "declared brief b9"), "{v:?}");
        let other = vec!["src/other.rs".to_string()];
        let v = check(
            who(Class::Sonnet, None),
            p(&other, Some("b4")),
            Some(&b),
            None,
        );
        assert!(has(&v, "the scope is b4's"), "{v:?}");
        let adr = vec![".claude/decisions/x.md".to_string()];
        let v = check(
            who(Class::Sonnet, None),
            p(&adr, Some("b4")),
            Some(&b),
            None,
        );
        assert!(has(&v, "never writes an ADR"), "{v:?}");
        let affects = vec!["src/**".to_string()];
        let q = Proposal {
            scope: &[],
            affects: &affects,
            mandate: Some(3),
            brief: Some("b4"),
        };
        let v = check(who(Class::Sonnet, None), q, Some(&b), None);
        assert!(has(&v, "no `affects`") && has(&v, "not a mandate"), "{v:?}");
        let v = check(
            who(Class::Sonnet, None),
            p(&[], Some("b4")),
            Some(&b),
            Some(410),
        );
        assert!(has(&v, "already has open change !410"), "{v:?}");
        let v = check(who(Class::Sonnet, None), p(&[], Some("b5")), None, None);
        assert!(has(&v, "no brief `b5`"), "{v:?}");
    }

    #[test]
    fn an_executors_change_carries_the_briefs_commit_and_revision() {
        let b = brief(Class::Haiku);
        let (title, plan) = commit(&b);
        assert_eq!(title, "feat(backplane): Class::covers");
        assert!(plan.contains("Carries out brief b4 r1 (haiku)"), "{plan}");
    }
}

#[test]
fn a_briefs_change_is_refused_only_the_planners_tiers() {
    use backplane::approvals::{Tier, Tiered, Tiers};
    use backplane::core::brief::executor::refused;
    let t = |authorize: &str, merge: &str| Tiers {
        authorize: Tiered {
            tier: Tier::Confirm,
            rule: authorize.into(),
        },
        merge: Tiered {
            tier: Tier::Code,
            rule: merge.into(),
        },
        close: Tiered {
            tier: Tier::Confirm,
            rule: "close".into(),
        },
    };
    assert_eq!(refused(&t("routine", "deploys")), Vec::<String>::new());
    assert_eq!(refused(&t("routine", "routine")), Vec::<String>::new());
    for rule in [
        "guarded: .mcp.json",
        "secrets: a/secrets.config.yaml",
        "sentinel: s.rs",
        "adds a secret",
    ] {
        let r = refused(&t("routine", rule));
        assert_eq!(r.len(), 1, "{rule}: {r:?}");
        assert!(r[0].contains(rule), "{r:?}");
    }
    let both = refused(&t("guarded: src/approve.rs", "guarded: src/approve.rs"));
    assert_eq!(both.len(), 1, "one rule named on both tiers is said once");
}

#[test]
fn an_executor_takes_only_tasks_on_its_briefs_changes() {
    use backplane::core::brief::executor::claim_refused;
    let code = vec!["src/sessions.rs".to_string()];
    let adr = vec![
        "src/sessions.rs".to_string(),
        ".claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md".to_string(),
    ];
    assert_eq!(
        claim_refused("brief", None, &[]),
        None,
        "a brief task is gated by class"
    );
    assert_eq!(
        claim_refused("yield-ended", Some("b4"), &code),
        None,
        "its own brief's change"
    );
    assert_eq!(claim_refused("restack-failed", Some("b4"), &code), None);
    let review = claim_refused("brief-review", Some("b4"), &code).expect("the planner's");
    assert!(review.contains("planner"), "{review}");
    let asked = claim_refused("escalation", Some("b4"), &code).expect("the planner's");
    assert!(asked.contains("planner"), "{asked}");
    let planner = claim_refused("handoff", None, &code).expect("a planner's change");
    assert!(planner.contains("carries no brief"), "{planner}");
    let review = claim_refused("review", None, &adr[1..]).expect("a review");
    assert!(review.contains("names an ADR"), "{review}");
    let reconcile = claim_refused("reconcile", Some("b4"), &adr).expect("an ADR amendment");
    assert!(reconcile.contains("names an ADR"), "{reconcile}");
}
