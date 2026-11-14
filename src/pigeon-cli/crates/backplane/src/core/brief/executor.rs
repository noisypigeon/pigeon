//! What an executor may propose: only its brief's change; see
//! `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §2, §4.

use super::Brief;
use crate::sessions::Class;

/// The parts of a proposal the check reads.
#[derive(Debug, Clone, Copy)]
pub struct Proposal<'a> {
    /// Its scope.
    pub scope: &'a [String],
    /// What its ADR will govern.
    pub affects: &'a [String],
    /// The mandate it names.
    pub mandate: Option<u32>,
    /// The brief it names.
    pub brief: Option<&'a str>,
}

/// What the proposing session declared and holds.
#[derive(Debug, Clone, Copy)]
pub struct Proposer<'a> {
    /// Its class.
    pub class: Class,
    /// The brief its ticket declared.
    pub declared: Option<&'a str>,
}

/// Every reason `who` may not propose `p`, given its brief (none when not found) and the open
/// change already carrying that brief, if any; none means it may.
#[must_use]
pub fn check(
    who: Proposer<'_>,
    p: Proposal<'_>,
    brief: Option<&Brief>,
    open: Option<u32>,
) -> Vec<String> {
    let mut out = Vec::new();
    let Some(id) = p.brief else {
        if who.class != Class::Opus {
            out.push(format!(
                "a {} session executes a brief: name it as `brief`",
                who.class.name()
            ));
        }
        return out;
    };
    let Some(b) = brief else {
        out.push(format!("no brief `{id}`"));
        return out;
    };
    if let Some(n) = open {
        out.push(format!("{} already has open change !{n}", b.id));
    }
    if who.class == Class::Opus {
        return out;
    }
    if who.declared.is_some_and(|d| d != b.id) {
        out.push(format!(
            "this session declared brief {}, not {}",
            who.declared.unwrap_or_default(),
            b.id
        ));
    }
    let draft = &b.latest().draft;
    if !who.class.covers(draft.model) {
        out.push(format!(
            "{} is for a {} session; a {} session can't carry it out",
            b.id,
            draft.model.name(),
            who.class.name()
        ));
    }
    if !p.affects.is_empty() {
        out.push("an executor's change decides nothing: no `affects`".into());
    }
    if p.mandate.is_some() {
        out.push("an executor works under its brief, not a mandate".into());
    }
    if let Some(adr) = p.scope.iter().find(|s| s.starts_with(".claude/decisions/")) {
        out.push(format!("an executor never writes an ADR: `{adr}`"));
    }
    let mut want: Vec<&String> = draft.scope.iter().collect();
    let mut got: Vec<&String> = p.scope.iter().collect();
    want.sort();
    got.sort();
    if !p.scope.is_empty() && want != got {
        out.push(format!(
            "the scope is {}'s: {}",
            b.id,
            draft.scope.join(", ")
        ));
    }
    out
}

/// The title and plan an executor's change carries: the brief's commit, citing its revision.
#[must_use]
pub fn commit(b: &Brief) -> (String, String) {
    let d = &b.latest().draft;
    let plan = format!(
        "{}\n\nCarries out brief {} r{} ({}), for {} step {}.",
        d.commit.plan.trim_end(),
        b.id,
        b.revision(),
        d.model.name(),
        d.adr,
        d.step
    );
    (d.commit.title.clone(), plan)
}
