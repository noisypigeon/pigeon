//! Session tickets: a signed claim to a session label, so one session can't act on another's
//! changes by mistake; see
//! `.claude/decisions/2026-11-08-development-events-tickets-digest-mandate.md` §5.

use std::collections::BTreeMap;

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::push::{b64url, b64url_decode};
use crate::sessions::Declared;

/// How long a ticket lives, in seconds.
pub const TTL: u64 = 24 * 60 * 60;

/// How long a lapsed ticket is remembered, so eviction can tell how long its session has gone;
/// see `.claude/decisions/2026-11-10-development-change-slot-eviction-policy.md` §3.
pub const REMEMBER: u64 = 7 * 24 * 60 * 60;

fn mac(key: &[u8], text: &str) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).unwrap_or_else(|_| unreachable!());
    m.update(text.as_bytes());
    m.finalize().into_bytes().to_vec()
}

/// A ticket for `label`, generation `generation`, living until `expires`, signed with `key`:
/// `<label>.<generation>.<expires>.<signature>`, the label and signature base64url.
#[must_use]
pub fn sign(key: &[u8], label: &str, generation: u32, expires: u64) -> String {
    let body = format!("{}.{generation}.{expires}", b64url(label.as_bytes()));
    format!("{body}.{}", b64url(&mac(key, &body)))
}

/// The label and generation `ticket` claims, if `key` signed it and it lives at `now`.
///
/// # Errors
///
/// Why it isn't a ticket.
pub fn verify(key: &[u8], ticket: &str, now: u64) -> Result<(String, u32), String> {
    let mut parts = ticket.trim().rsplitn(2, '.');
    let (sig, body) = (
        parts.next().unwrap_or_default(),
        parts.next().unwrap_or_default(),
    );
    let mut fields = body.split('.');
    let (Some(label), Some(generation), Some(expires), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return Err("a ticket is <label>.<generation>.<expires>.<signature>".into());
    };
    let wanted = mac(key, body);
    let got = b64url_decode(sig).unwrap_or_default();
    let same = got.len() == wanted.len()
        && got
            .iter()
            .zip(&wanted)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0;
    if !same {
        return Err("the ticket's signature doesn't hold".into());
    }
    let expires: u64 = expires
        .parse()
        .map_err(|_| "a ticket's expiry is a number")?;
    if expires <= now {
        return Err("the ticket has expired; start the session again".into());
    }
    let label = String::from_utf8(b64url_decode(label)?).map_err(|_| "a ticket's label is text")?;
    let generation = generation
        .parse()
        .map_err(|_| "a ticket's generation is a number")?;
    Ok((label, generation))
}

/// One label's ticket as issued: its generation, expiry, and what its session declared.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issued {
    /// Rises each time the label is started, so an older ticket stops working.
    pub generation: u32,
    /// When it expires, in Unix seconds.
    pub expires: u64,
    /// Its class, planner and brief; see
    /// `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md` §1.
    #[serde(default)]
    pub declared: Declared,
    /// The label of the session that created it; see
    /// `.claude/decisions/2026-11-15-development-archive-cleanly-ended-sessions.md` §1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// How its session ended, once it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended: Option<Ended>,
    /// Its archiver's report, once made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<Archived>,
}

impl Issued {
    /// The session that archives it once it ends clean: its parent, else its planner.
    #[must_use]
    pub fn archiver(&self) -> Option<&str> {
        self.parent.as_deref().or(self.declared.planner.as_deref())
    }
}

/// How a session ended; see
/// `.claude/decisions/2026-11-15-development-archive-cleanly-ended-sessions.md` §2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ended {
    /// When, in Unix seconds.
    pub at: u64,
    /// Whether it left nothing but hand-offs.
    pub clean: bool,
    /// The session that archives it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archiver: Option<String>,
    /// Whether the control plane ended it as silent past [`IDLE`]; see the archive ADR's
    /// 2026-11-16 amendment.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lapsed: bool,
}

/// An archiver's report on an ended session; see the archive ADR §3.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Archived {
    /// When, in Unix seconds.
    pub at: u64,
    /// The archiver's label.
    pub by: String,
    /// `archived` or `skipped`.
    pub outcome: String,
    /// Why, in a line.
    pub why: String,
}

/// Whether `label`'s end is clean: each change it holds has an open, unclaimed `handoff` task
/// for another session; see the archive ADR §2.
#[must_use]
pub fn clean(label: &str, held: &[u32], tasks: &[crate::tasks::Task]) -> bool {
    held.iter().all(|n| {
        tasks.iter().any(|t| {
            t.kind == "handoff"
                && t.change == *n
                && !t.done
                && t.claimed_by.is_none()
                && t.session.as_deref().is_some_and(|to| to != label)
        })
    })
}

/// How long after a clean end the digest names a session not yet archived, and until when.
pub const UNARCHIVED: (u64, u64) = (60 * 60, 24 * 60 * 60);

/// How long a session holding nothing goes without polling before its ticket lapses, in
/// seconds; see the archive ADR's 2026-11-16 amendment.
pub const IDLE: u64 = 2 * 60 * 60;

/// Every label's ticket as issued.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tickets(pub BTreeMap<String, Issued>);

impl Tickets {
    /// Issues `label` a ticket at `now`, as an opus session: refused while a live one exists
    /// unless `replace`, which revokes it.
    ///
    /// # Errors
    ///
    /// When `label` holds a live ticket and `replace` is false.
    pub fn start(&mut self, label: &str, replace: bool, now: u64) -> Result<Issued, String> {
        self.start_as(label, replace, now, Declared::default())
    }

    /// As [`Tickets::start`], for a session that declared `declared`; a replacement may declare
    /// another class.
    ///
    /// # Errors
    ///
    /// When `label` holds a live ticket and `replace` is false.
    pub fn start_as(
        &mut self,
        label: &str,
        replace: bool,
        now: u64,
        declared: Declared,
    ) -> Result<Issued, String> {
        self.0.retain(|_, i| i.expires + REMEMBER > now);
        let live = self.live(label, now).is_some();
        let generation = match self.0.get(label) {
            Some(_) if live && !replace => {
                return Err(format!(
                    "session {label} holds a live ticket; start it with `replace` to revoke that one"
                ));
            }
            Some(i) if live => i.generation + 1,
            _ => 1,
        };
        let issued = Issued {
            generation,
            expires: now + TTL,
            declared,
            parent: None,
            ended: None,
            archived: None,
        };
        self.0.insert(label.to_string(), issued.clone());
        Ok(issued)
    }

    /// Ends `label`'s live ticket at `now`, if it holds one: whether it did. The ticket is kept,
    /// lapsed, for [`REMEMBER`], so the control plane can tell how long the session has gone.
    pub fn end(&mut self, label: &str, now: u64) -> bool {
        match self.0.get_mut(label) {
            Some(i) if i.expires > now => {
                i.expires = now;
                true
            }
            _ => false,
        }
    }

    /// Records `parent` as the session that created `label`'s.
    pub fn parented(&mut self, label: &str, parent: Option<String>) {
        if let Some(i) = self.0.get_mut(label) {
            i.parent = parent;
        }
    }

    /// Records how `label`'s session ended, judged by [`clean`]; its archiver is what it declared.
    pub fn record_end(&mut self, label: &str, clean: bool, now: u64) -> Option<Ended> {
        let i = self.0.get_mut(label)?;
        let ended = Ended {
            at: now,
            clean,
            archiver: i.archiver().map(str::to_string),
            lapsed: false,
        };
        i.ended = Some(ended.clone());
        Some(ended)
    }

    /// Each live label quiet past [`IDLE`] at `now`, from its last poll in `polled`, else its
    /// ticket's start, that isn't in `busy` (holding an open change or a claimed task).
    #[must_use]
    pub fn lapsing(
        &self,
        now: u64,
        polled: &BTreeMap<String, u64>,
        busy: &std::collections::BTreeSet<String>,
    ) -> Vec<String> {
        self.0
            .iter()
            .filter(|(l, i)| i.expires > now && !busy.contains(*l))
            .filter(|(l, i)| {
                let since = polled
                    .get(*l)
                    .copied()
                    .unwrap_or(0)
                    .max(i.expires.saturating_sub(TTL));
                now.saturating_sub(since) >= IDLE
            })
            .map(|(l, _)| l.clone())
            .collect()
    }

    /// Ends `label`'s live ticket at `now` as lapsed, holding nothing, so clean: its end, or none
    /// when it held no live ticket.
    pub fn lapse(&mut self, label: &str, now: u64) -> Option<Ended> {
        if !self.end(label, now) {
            return None;
        }
        let i = self.0.get_mut(label)?;
        let ended = Ended {
            at: now,
            clean: true,
            archiver: i.archiver().map(str::to_string),
            lapsed: true,
        };
        i.ended = Some(ended.clone());
        Some(ended)
    }

    /// Records `by`'s report on `label` at `now`, after `label` ended clean, once: from its
    /// archiver, or, when it has none, from any other session; see the archive ADR's 2026-11-16
    /// amendment.
    ///
    /// # Errors
    ///
    /// Why the report is refused.
    pub fn record_archive(
        &mut self,
        label: &str,
        by: &str,
        outcome: &str,
        why: &str,
        now: u64,
    ) -> Result<Archived, String> {
        if !["archived", "skipped"].contains(&outcome) {
            return Err("`outcome` is archived or skipped".into());
        }
        if by == label {
            return Err("a session never reports its own archive".into());
        }
        let Some(i) = self.0.get_mut(label) else {
            return Err(format!("session {label} isn't known"));
        };
        match &i.ended {
            Some(e) if e.clean && e.archiver.as_deref().is_none_or(|a| a == by) => {}
            Some(e) if e.clean => return Err(format!("session {label}'s archiver isn't {by}")),
            _ => return Err(format!("session {label} hasn't ended clean")),
        }
        if i.archived.is_some() {
            return Err(format!("session {label}'s archive is already recorded"));
        }
        let a = Archived {
            at: now,
            by: by.to_string(),
            outcome: outcome.to_string(),
            why: why.to_string(),
        };
        i.archived = Some(a.clone());
        Ok(a)
    }

    /// Each label that ended clean, not lapsed, within [`UNARCHIVED`] of `now` with no report, and
    /// its archiver.
    #[must_use]
    pub fn unarchived(&self, now: u64) -> Vec<(String, Option<String>)> {
        self.0
            .iter()
            .filter(|(_, i)| i.archived.is_none())
            .filter_map(|(l, i)| {
                let e = i.ended.as_ref().filter(|e| e.clean && !e.lapsed)?;
                let age = now.saturating_sub(e.at);
                (UNARCHIVED.0..UNARCHIVED.1)
                    .contains(&age)
                    .then(|| (l.clone(), e.archiver.clone()))
            })
            .collect()
    }

    /// The class `label` declared with its latest ticket, live or lapsed; none for a label
    /// never started.
    #[must_use]
    pub fn class(&self, label: &str) -> Option<crate::sessions::Class> {
        self.0.get(label).map(|i| i.declared.model)
    }

    /// `label`'s live ticket, if any.
    #[must_use]
    pub fn live(&self, label: &str, now: u64) -> Option<Issued> {
        self.0.get(label).filter(|i| i.expires > now).cloned()
    }

    /// How long `label` has had no live ticket at `now`: since its last one lapsed, while that is
    /// remembered; none while it is live, or when it isn't known.
    #[must_use]
    pub fn gone_for(&self, label: &str, now: u64) -> Option<u64> {
        self.0
            .get(label)
            .filter(|i| i.expires <= now)
            .map(|i| now - i.expires)
    }
}

/// The sessions whose ticket an agent's `operation` needs: its `session` argument, unless it
/// names a recipient, and the session of the change it acts on; see
/// `.claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md`, 2026-11-14.
#[must_use]
pub fn guarded(operation: &str, session: Option<&str>, change: Option<&str>) -> Vec<String> {
    let recipient = operation == "change.handoff";
    session
        .filter(|_| !recipient)
        .into_iter()
        .chain(change)
        .map(str::to_string)
        .collect()
}

/// Whether an agent's operation naming `labels` may go ahead with `ticket` at `now`: a label
/// holding a live ticket needs that ticket, of its current generation; others need none.
///
/// # Errors
///
/// Which label wants which ticket.
pub fn require(
    key: &[u8],
    tickets: &Tickets,
    labels: &[String],
    ticket: Option<&str>,
    now: u64,
) -> Result<(), String> {
    for label in labels {
        let Some(issued) = tickets.live(label, now) else {
            continue;
        };
        let Some(t) = ticket.filter(|t| !t.trim().is_empty()) else {
            return Err(format!(
                "session {label} holds a ticket; pass it as `ticket`"
            ));
        };
        let (got, generation) = verify(key, t, now)?;
        if &got != label {
            return Err(format!(
                "this ticket is session {got}'s; this change is session {label}'s"
            ));
        }
        if generation != issued.generation {
            return Err(format!(
                "this ticket was replaced; session {label}'s newer one is needed"
            ));
        }
    }
    Ok(())
}

/// The label of `ticket` when it is live: verified under `key`, its label holding a live ticket of the same generation.
///
/// # Errors
///
/// Why not, for git's `ng` line.
pub fn holder(
    key: &[u8],
    tickets: &Tickets,
    ticket: Option<&str>,
    now: u64,
) -> Result<String, String> {
    let Some(t) = ticket.filter(|t| !t.trim().is_empty()) else {
        return Err("a scratch ref takes its session's ticket (X-Pigeon-Ticket)".into());
    };
    let (label, generation) = verify(key, t, now)?;
    let Some(issued) = tickets.live(&label, now) else {
        return Err(format!("session {label}'s ticket has ended"));
    };
    if issued.generation != generation {
        return Err(format!(
            "this ticket was replaced; session {label}'s newer one is needed"
        ));
    }
    Ok(label)
}
