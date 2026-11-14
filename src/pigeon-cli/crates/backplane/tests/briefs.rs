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
