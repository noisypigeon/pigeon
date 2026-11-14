//! Briefs: a planner's explicit work orders for an executor session, validated and kept
//! append-only; see `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §3.

use serde::{Deserialize, Serialize};

use crate::sessions::Class;

pub mod executor;

/// The most paths a brief may touch.
pub const MAX_SCOPE: usize = 12;

/// The most paths a `haiku` brief may touch.
pub const MAX_SCOPE_HAIKU: usize = 5;

/// The largest brief, serialized, in bytes.
pub const MAX_BYTES: usize = 16 * 1024;

/// The most heads a budget may allow.
pub const MAX_HEADS: u32 = 8;

/// The most hours a budget may allow.
pub const MAX_HOURS: u32 = 8;

/// Words that leave a choice to the executor, refused in a step's `do` and `done_when`.
pub const OPEN_CHOICE: [&str; 9] = [
    "decide",
    "choose",
    "consider",
    "as needed",
    "if appropriate",
    "figure out",
    "tbd",
    "or similar",
    "etc.",
];

/// One step of a brief.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// What to do, in one imperative sentence.
    #[serde(rename = "do")]
    pub act: String,
    /// The paths it touches, each in the brief's scope.
    pub files: Vec<String>,
    /// The literal text, diff or signature; required for `haiku`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// A command and its expected result, or one observable fact.
    pub done_when: String,
}

/// How much a brief may spend before it returns to its planner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    /// Heads pushed.
    pub heads: u32,
    /// Hours since the brief was claimed.
    pub hours: u32,
}

impl Budget {
    /// The default budget for `model`'s briefs.
    #[must_use]
    pub const fn default_for(model: Class) -> Self {
        Self {
            heads: match model {
                Class::Haiku => 3,
                Class::Sonnet | Class::Opus => 4,
            },
            hours: 2,
        }
    }
}

/// The commit an executor's change carries, written by the planner.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    /// A Conventional Commit title.
    pub title: String,
    /// The change's plan.
    pub plan: String,
}

/// A brief's content: one revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    /// What it does, in a line.
    pub title: String,
    /// The ADR it carries out.
    pub adr: String,
    /// The ADR's execution step.
    pub step: String,
    /// The least capable class that may take it: `haiku` or `sonnet`.
    pub model: Class,
    /// The `main` commit it was written against.
    pub base: String,
    /// The exact paths it may touch.
    pub scope: Vec<String>,
    /// What the planner settled, one line each.
    pub decisions: Vec<String>,
    /// What to do, in order.
    pub steps: Vec<Step>,
    /// The commands to run before each push.
    pub checks: Vec<String>,
    /// When to escalate rather than continue.
    pub stop_if: Vec<String>,
    /// Its budget; the class's default when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget: Option<Budget>,
    /// The commit its change carries.
    pub commit: Commit,
}

impl Draft {
    /// Its budget, or its class's default.
    #[must_use]
    pub fn budget(&self) -> Budget {
        self.budget.unwrap_or(Budget::default_for(self.model))
    }
}

/// One revision of a brief: its draft, when, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision {
    /// The draft.
    pub draft: Draft,
    /// When, in Unix seconds.
    pub at: u64,
    /// Why it was written: `created`, or the amendment's reason.
    pub reason: String,
}

/// A brief and every revision of it, oldest first; no revision is ever rewritten.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Brief {
    /// `b<n>`.
    pub id: String,
    /// The planner session's label.
    pub planner: String,
    /// Its revisions.
    pub revisions: Vec<Revision>,
    /// The changes proposed to carry it out, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<u32>,
}

impl Brief {
    /// A new brief `b<number>` by `planner` at `at`.
    ///
    /// # Errors
    ///
    /// Every problem with `draft`, or a missing planner.
    pub fn create(number: u32, planner: &str, draft: Draft, at: u64) -> Result<Self, Vec<String>> {
        let mut problems = validate(&draft);
        if planner.trim().is_empty() {
            problems.push("a brief names its planner".into());
        }
        if !problems.is_empty() {
            return Err(problems);
        }
        Ok(Self {
            id: format!("b{number}"),
            planner: planner.trim().to_string(),
            revisions: vec![Revision {
                draft,
                at,
                reason: "created".into(),
            }],
            changes: Vec::new(),
        })
    }

    /// The brief with `draft` appended as its next revision, for `reason`, by `by`.
    ///
    /// # Errors
    ///
    /// When `by` isn't its planner, `reason` is empty, or `draft` is invalid.
    pub fn amend(
        &self,
        by: &str,
        draft: Draft,
        reason: &str,
        at: u64,
    ) -> Result<Self, Vec<String>> {
        let mut problems = validate(&draft);
        if by.trim() != self.planner {
            problems.push(format!(
                "only its planner, {}, amends {}",
                self.planner, self.id
            ));
        }
        if reason.trim().is_empty() {
            problems.push("an amendment says why".into());
        }
        if !problems.is_empty() {
            return Err(problems);
        }
        let mut next = self.clone();
        next.revisions.push(Revision {
            draft,
            at,
            reason: reason.trim().to_string(),
        });
        Ok(next)
    }

    /// The latest revision.
    #[must_use]
    pub fn latest(&self) -> &Revision {
        self.revisions
            .last()
            .unwrap_or_else(|| unreachable!("a brief has a revision"))
    }

    /// Its revision number, from 1.
    #[must_use]
    pub fn revision(&self) -> usize {
        self.revisions.len()
    }
}

/// Every problem with `draft`; none means it is explicit enough to hand over.
#[must_use]
pub fn validate(draft: &Draft) -> Vec<String> {
    let mut out = Vec::new();
    let empty = |s: &str| s.trim().is_empty();
    for (name, value) in [
        ("title", &draft.title),
        ("step", &draft.step),
        ("commit.plan", &draft.commit.plan),
    ] {
        if empty(value) {
            out.push(format!("`{name}` is empty"));
        }
    }
    let md = std::path::Path::new(&draft.adr)
        .extension()
        .is_some_and(|e| e == "md");
    if !(draft.adr.starts_with(".claude/decisions/") && md) {
        out.push("`adr` is the ADR's path, `.claude/decisions/<file>.md`".into());
    }
    if !is_commit(&draft.base) {
        out.push("`base` is a commit on `main`: 7 to 40 hex digits".into());
    }
    if draft.model == Class::Opus {
        out.push("a brief is for `haiku` or `sonnet`; an opus session plans".into());
    }
    if !is_conventional(&draft.commit.title) {
        out.push("`commit.title` is a Conventional Commit title".into());
    }
    scope(draft, &mut out);
    for (name, list) in [
        ("decisions", &draft.decisions),
        ("checks", &draft.checks),
        ("stop_if", &draft.stop_if),
    ] {
        if list.iter().all(|l| empty(l)) {
            out.push(format!("`{name}` holds at least one line"));
        }
    }
    if draft.steps.is_empty() {
        out.push("a brief has at least one step".into());
    }
    for (i, s) in draft.steps.iter().enumerate() {
        step(draft, i + 1, s, &mut out);
    }
    let b = draft.budget();
    if b.heads == 0 || b.heads > MAX_HEADS || b.hours == 0 || b.hours > MAX_HOURS {
        out.push(format!(
            "`budget` is 1 to {MAX_HEADS} heads and 1 to {MAX_HOURS} hours"
        ));
    }
    if serde_json::to_vec(draft).map_or(0, |v| v.len()) > MAX_BYTES {
        out.push(format!("a brief is at most {MAX_BYTES} bytes"));
    }
    out
}

fn scope(draft: &Draft, out: &mut Vec<String>) {
    let max = if draft.model == Class::Haiku {
        MAX_SCOPE_HAIKU
    } else {
        MAX_SCOPE
    };
    if draft.scope.is_empty() || draft.scope.len() > max {
        out.push(format!(
            "`scope` holds 1 to {max} paths for a {} brief",
            draft.model.name()
        ));
    }
    for p in &draft.scope {
        let bad = p.trim() != p
            || p.is_empty()
            || p.starts_with('/')
            || p.split('/').any(|seg| seg == ".." || seg.is_empty())
            || p.contains(['*', '?', '[', '{']);
        if bad {
            out.push(format!(
                "`scope` path `{p}` is an exact relative path, with no glob"
            ));
        }
        if p.starts_with(".claude/decisions/") {
            out.push(format!(
                "`scope` path `{p}` is an ADR; the planner writes ADRs"
            ));
        }
    }
}

fn step(draft: &Draft, n: usize, s: &Step, out: &mut Vec<String>) {
    if s.act.trim().is_empty() {
        out.push(format!("step {n}: `do` is empty"));
    }
    if s.done_when.trim().is_empty() {
        out.push(format!("step {n}: `done_when` is empty"));
    }
    if s.files.is_empty() {
        out.push(format!("step {n}: `files` names at least one path"));
    }
    for f in &s.files {
        if !draft.scope.contains(f) {
            out.push(format!("step {n}: `{f}` isn't in `scope`"));
        }
    }
    if draft.model == Class::Haiku && s.content.as_deref().is_none_or(|c| c.trim().is_empty()) {
        out.push(format!(
            "step {n}: a haiku step carries its `content`, the exact text or diff"
        ));
    }
    for (field, text) in [("do", &s.act), ("done_when", &s.done_when)] {
        if let Some(w) = open_choice(text) {
            out.push(format!(
                "step {n}: `{field}` leaves a choice (`{w}`); the planner makes it"
            ));
        }
    }
}

/// The first [`OPEN_CHOICE`] phrase in `text`, matched as whole words, ignoring case.
#[must_use]
pub fn open_choice(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    OPEN_CHOICE.into_iter().find(|w| {
        lower.match_indices(w).any(|(i, _)| {
            let before = lower[..i].chars().next_back();
            let after = lower[i + w.len()..].chars().next();
            let edge = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
            edge(before) && (w.ends_with('.') || edge(after))
        })
    })
}

fn is_commit(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_conventional(title: &str) -> bool {
    const TYPES: [&str; 10] = [
        "feat", "fix", "docs", "chore", "ci", "refactor", "test", "build", "perf", "revert",
    ];
    let Some((head, rest)) = title.split_once(": ") else {
        return false;
    };
    let head = head.strip_suffix('!').unwrap_or(head);
    let (kind, scope) = match head.split_once('(') {
        Some((k, s)) => match s.strip_suffix(')') {
            Some(s) => (k, Some(s)),
            None => return false,
        },
        None => (head, None),
    };
    let scope_ok = scope.is_none_or(|s| {
        !s.is_empty()
            && s.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    });
    TYPES.contains(&kind) && scope_ok && !rest.trim().is_empty()
}
