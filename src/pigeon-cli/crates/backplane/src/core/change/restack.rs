//! A change's head after `main` moves: the restack job, its report, and the approval it carries;
//! see `.claude/decisions/2026-11-08-development-concurrent-agent-sessions.md` §3.

use super::lifecycle::Step;
use super::types::{
    Approval, BOUND, Carried, Change, Conflicted, Effect, Job, JobStatus, MERGE, RESTACK, Relation,
    Restacked, State, refuse,
};

impl Change {
    /// `main` moved: a checked or approved head is restacked, keeping its approval to carry.
    pub(super) fn behind(mut self, main: String) -> Step {
        let Some(head) = self.head.clone() else {
            return refuse(format!("{} has no head to be behind", self.branch()));
        };
        self.behind = Some(main);
        Ok(self.start_restack(head))
    }

    /// A checked or approved head that is `behind`, with no restack or merge running, restacks;
    /// otherwise nothing; see the 2026-11-12 amendment to the concurrent-sessions ADR.
    pub(super) fn restack_if_behind(self) -> (Change, Vec<Effect>) {
        match self.head.clone() {
            Some(head) if self.behind.is_some() => self.start_restack(head),
            _ => (self, Vec::new()),
        }
    }

    /// The restack job for `head`, keeping an approval to carry.
    fn start_restack(mut self, head: String) -> (Change, Vec<Effect>) {
        if !matches!(self.state, State::Checked | State::Approved)
            || self.job(RESTACK).is_some_and(Job::open)
            || self.job(MERGE).is_some_and(Job::open)
        {
            return (self, Vec::new());
        }
        self.carried = match (&self.approval, &self.tiers) {
            (Some(a), Some(t)) if self.state == State::Approved => Some(Carried {
                by: a.by.clone(),
                at: a.at,
                from: head,
                merge: t.merge.tier,
                to: None,
            }),
            _ => None,
        };
        self.jobs.retain(|j| j.name != RESTACK);
        self.jobs.push(Job::queued(RESTACK, Vec::new()));
        (self, vec![Effect::Start(RESTACK.into())])
    }

    /// The restack job's report on head `sha`, then a head held while it ran, if another.
    pub(super) fn restacked(mut self, sha: &str, r: &Restacked) -> Step {
        let held = self.held.take();
        let (mut c, mut effects) = self.restack_report(sha, r)?;
        if let Some(h) = held
            && !c.on_head(&h)
        {
            c.carried = None;
            c.merge_after = None;
            let (next, more) = c.push_held(h);
            c = next;
            effects.extend(more);
        }
        Ok((c, effects))
    }

    /// The restack job's report on head `sha`.
    fn restack_report(mut self, sha: &str, r: &Restacked) -> Step {
        let pushed_first = r.to.is_some() && self.head == r.to;
        if !self.on_head(sha) && !pushed_first {
            return Ok((self, Vec::new()));
        }
        self = self.set_status(RESTACK, JobStatus::Passed);
        let carries = r.relation == Relation::Disjoint && r.same_patch;
        if r.landed {
            self.behind = None;
            self.carried = None;
            self.merge_after = None;
            self.landed.clone_from(&self.head);
            self.violations = vec![format!(
                "{}'s patch is already on main at {}; it closes once the close is approved",
                self.branch(),
                r.onto.get(..12).unwrap_or(&r.onto)
            )];
            return Ok((self, Vec::new()));
        }
        match (&r.relation, r.to.clone()) {
            (Relation::Current, _) => {
                self.behind = None;
                self.carried = None;
                match self.merge_after.take() {
                    Some(by) => Ok(self.start_merge_or_drop(&by)),
                    None => Ok((self, Vec::new())),
                }
            }
            (Relation::Conflicting, _) | (_, None) => {
                self.carried = None;
                self.merge_after = None;
                if r.relation == Relation::Conflicting {
                    let proposal = crate::reconcile::conflict(&self, r);
                    self.conflict = Some(Conflicted {
                        onto: r.onto.clone(),
                        paths: r.conflicts.clone(),
                        proposal: Some(proposal),
                        reconcile: None,
                    });
                }
                self.violations = vec![format!(
                    "{} conflicts with main in {}; rebase it onto main and push",
                    self.branch(),
                    r.conflicts.join(", ")
                )];
                Ok((self, Vec::new()))
            }
            (_, Some(to)) => {
                self.carried = match self.carried.take() {
                    Some(mut c) if carries && c.from == sha => {
                        c.to = Some(to.clone());
                        Some(c)
                    }
                    _ => None,
                };
                if pushed_first {
                    return Ok(self.carry());
                }
                self.tiers = None;
                self.behind = None;
                let kept = if carries {
                    self.passed_code()
                } else {
                    Vec::new()
                };
                if !carries {
                    self.gate = None;
                    self.gate_full = Some(format!("main changed its files at {}", r.onto));
                }
                let (mut c, effects) = self.pushed(to)?;
                c.kept = kept;
                Ok((c, effects))
            }
        }
    }

    /// The head's passed code checks and gate jobs, which a disjoint, same-patch restack keeps.
    fn passed_code(&self) -> Vec<Job> {
        self.jobs
            .iter()
            .filter(|j| {
                j.status == JobStatus::Passed
                    && j.gates()
                    && j.name != BOUND
                    && j.name != MERGE
                    && j.name != RESTACK
            })
            .cloned()
            .collect()
    }

    /// Approves a restacked head that reached `checked` with the approval carried to it, when
    /// its merge tier is no higher than the one approved, then starts a merge asked for
    /// meanwhile; a merge whose approval didn't carry is dropped.
    pub(super) fn carry(mut self) -> (Change, Vec<Effect>) {
        if self.state != State::Checked {
            return (self, Vec::new());
        }
        let (Some(c), Some(t)) = (&self.carried, &self.tiers) else {
            self.carried = None;
            self.merge_after = None;
            return (self, Vec::new());
        };
        if c.to.is_none() {
            return (self, Vec::new());
        }
        let carries = c.to == self.head && t.merge.tier <= c.merge;
        if carries {
            self.approval = Some(Approval {
                by: c.by.clone(),
                sha: self.head.clone(),
                at: c.at,
            });
            self.state = State::Approved;
        }
        self.carried = None;
        match self.merge_after.take() {
            Some(by) if carries => self.start_merge_or_drop(&by),
            _ => (self, Vec::new()),
        }
    }
}
