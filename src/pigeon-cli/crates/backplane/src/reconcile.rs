//! Reconciliation planned by the control plane: for a change whose restack conflicts with
//! `main`, and for an open change in the area an ADR just merged decides; see
//! `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §4.

use std::fmt::Write as _;

use serde::{Deserialize, Serialize};

use crate::changes::{Change, Restacked, State};

/// The ADR that decides reconciliation, which every plan cites.
pub const ADR: &str = ".claude/decisions/2026-11-08-development-concurrent-agent-sessions.md";

/// Where ADRs live.
pub const DECISIONS: &str = ".claude/decisions/";

/// Who proposes a reconciliation.
pub const PROPOSER: &str = "pigeon";

/// A change the control plane proposes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Proposal {
    /// Its slug.
    pub slug: String,
    /// Its title.
    pub title: String,
    /// Its plan.
    pub plan: String,
    /// Its scope.
    pub scope: Vec<String>,
}

/// The ADRs `text` cites, in order of their names, which is their dates.
#[must_use]
pub fn adrs(text: &str) -> Vec<String> {
    let mut found: Vec<String> = text
        .match_indices(DECISIONS)
        .filter_map(|(i, _)| {
            let rest = &text[i..];
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || "._-/".contains(c)))
                .unwrap_or(rest.len());
            let path = rest[..end].trim_end_matches('.');
            let name = path.strip_prefix(DECISIONS)?;
            let md = std::path::Path::new(name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"));
            let ok = md && !name.contains('/') && name != "README.md";
            ok.then(|| path.to_string())
        })
        .collect();
    found.sort();
    found.dedup();
    found
}

/// The plan for change `c`, whose restack onto `main` conflicted as `r` says.
#[must_use]
pub fn conflict(c: &Change, r: &Restacked) -> Proposal {
    let mut cited = adrs(&c.plan);
    cited.extend(adrs(&r.messages));
    cited.retain(|a| a != ADR);
    cited.sort();
    cited.dedup();
    let later = cited.last().cloned();
    let in_decisions = r.conflicts.iter().any(|p| p.starts_with(DECISIONS));
    let mut scope = r.conflicts.clone();
    if let Some(a) = &later
        && !scope.contains(a)
    {
        scope.push(a.clone());
    }
    let session = c
        .session
        .as_deref()
        .map(|s| format!(", session {s}"))
        .unwrap_or_default();
    let mut plan = format!(
        "Reconciles !{} ({}{session}) with main at {}, per {ADR} §4.\n\n",
        c.number,
        c.title,
        short(&r.onto)
    );
    let _ = writeln!(plan, "Conflicting paths:");
    for x in &r.context {
        let by = if x.commits.is_empty() {
            "main".to_string()
        } else {
            x.commits.join("; ")
        };
        let _ = writeln!(plan, "- {}: main changed it in {by}.", x.path);
    }
    if r.context.is_empty() {
        for p in &r.conflicts {
            let _ = writeln!(plan, "- {p}");
        }
    }
    plan.push('\n');
    match &later {
        Some(a) if in_decisions || !cited.is_empty() => {
            let _ = writeln!(
                plan,
                "Rule: append an amendment to {a}, the later ADR either side cites, saying what \
                 was reconciled and which side's decision stands, then make the paths agree with it."
            );
        }
        _ => {
            let _ = writeln!(
                plan,
                "Rule: fix the paths so both sides' intent holds; main's side is settled."
            );
        }
    }
    if !cited.is_empty() {
        let _ = writeln!(plan, "\nADRs cited: {}.", cited.join(", "));
    }
    let _ = writeln!(
        plan,
        "\nThen rebase !{} onto main, or close it if this change carries its intent.",
        c.number
    );
    for x in &r.context {
        let _ = write!(
            plan,
            "\n{} on main:\n```diff\n{}\n```\n{} on !{}:\n```diff\n{}\n```\n",
            x.path,
            x.base_diff.trim_end(),
            x.path,
            c.number,
            x.change_diff.trim_end()
        );
    }
    Proposal {
        slug: format!("reconcile-{}", c.number),
        title: format!("fix: reconcile !{} with main", c.number),
        plan,
        scope,
    }
}

/// The review planned for an open change `number`, citing `open_plan`, after change `merged`
/// with `merged_plan` merged decisions over `globs` that the open change's scope meets: none
/// when the open change cites no ADR the merged one doesn't.
#[must_use]
pub fn review(
    number: u32,
    open_plan: &str,
    merged: u32,
    merged_plan: &str,
    globs: &[String],
) -> Option<Proposal> {
    let theirs: Vec<String> = adrs(merged_plan).into_iter().filter(|a| a != ADR).collect();
    let mine: Vec<String> = adrs(open_plan)
        .into_iter()
        .filter(|a| a != ADR && !theirs.contains(a))
        .collect();
    if mine.is_empty() || theirs.is_empty() {
        return None;
    }
    let mut all: Vec<&String> = mine.iter().chain(&theirs).collect();
    all.sort();
    let later = (*all.last()?).clone();
    let earlier: Vec<&&String> = all.iter().filter(|a| ***a != later).collect();
    let plan = format!(
        "Reviews !{number} against !{merged}, per {ADR} §4.\n\n\
         !{merged} merged decisions in {} over {}; !{number} works there and cites {}.\n\n\
         Rule: append an amendment to {later}, the later ADR, citing {}: say whether the two \
         agree, and if not, what !{number} changes.\n",
        theirs.join(", "),
        globs.join(", "),
        mine.join(", "),
        earlier
            .iter()
            .map(|a| a.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );
    Some(Proposal {
        slug: format!("review-{number}-{merged}"),
        title: format!("docs(adr): review !{number} against !{merged}"),
        plan,
        scope: vec![later],
    })
}

/// The open change already carrying a generated change's `slug`, the lowest-numbered when
/// several do, so a change that conflicts again reuses it; see
/// `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §4 F1.
#[must_use]
pub fn existing(open: &[crate::sessions::Open], slug: &str) -> Option<u32> {
    open.iter()
        .filter(|o| o.slug == slug)
        .map(|o| o.number)
        .min()
}

/// Whether change `after`'s generated changes lost their reason since `before`: `Some(true)`
/// when it merged or closed, which retires its reconciliation and its reviews, `Some(false)`
/// when only its conflict cleared, which retires its reconciliation; see
/// `.claude/decisions/2026-11-12-development-grants-applies-rescue-yield.md` §4 F2.
#[must_use]
pub fn gone(before: Option<&Change>, after: &Change) -> Option<bool> {
    let done = |c: &Change| matches!(c.state, State::Merged | State::Closed);
    let before = before?;
    if !done(before) && done(after) {
        return Some(true);
    }
    (before.conflict.is_some() && after.conflict.is_none() && !done(after)).then_some(false)
}

/// The open generated changes of change `number` to close, never pushed: its reconciliation,
/// and with `ended` the reviews of it.
#[must_use]
pub fn retired(open: &[crate::sessions::Open], number: u32, ended: bool) -> Vec<u32> {
    let reconcile = format!("reconcile-{number}");
    let review = format!("review-{number}-");
    open.iter()
        .filter(|o| matches!(o.state.as_str(), "proposed" | "planned"))
        .filter(|o| o.slug == reconcile || (ended && o.slug.starts_with(&review)))
        .map(|o| o.number)
        .collect()
}

fn short(sha: &str) -> &str {
    sha.get(..12).unwrap_or(sha)
}
