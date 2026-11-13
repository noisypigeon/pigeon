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
}

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
