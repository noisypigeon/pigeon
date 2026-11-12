//! The owner's CI window: for a bounded time, every change's code checks run on the `Checker`;
//! see `.claude/decisions/2026-11-10-development-code-checks-github-actions.md`, its 2026-11-11
//! amendment.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The operation that asks for a window, and the event recording each open and close.
pub const OPERATION: &str = "ci.window";

/// The longest window, in hours.
pub const MAX_HOURS: u32 = 24;

/// An open window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Window {
    /// When it closes, in Unix seconds.
    pub until: u64,
    /// Who opened it.
    pub by: String,
}

impl Window {
    /// Whether it is open at `now`; one past its end is closed without anyone closing it.
    #[must_use]
    pub fn open(&self, now: u64) -> bool {
        now < self.until
    }
}

/// A window asked for, waiting for the owner's code on its page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    /// Its number, for the page's path.
    pub id: u32,
    /// How long the window lasts from its grant; 0 closes it.
    pub hours: u32,
    /// When the page's link expires, in Unix seconds.
    pub expires: u64,
}

/// What `ci.window`'s `hours` asks for.
///
/// # Errors
///
/// When it isn't a whole number from 0 to [`MAX_HOURS`].
pub fn hours(text: Option<&str>) -> Result<u32, String> {
    let h = text
        .map(str::trim)
        .and_then(|t| t.parse::<u32>().ok())
        .ok_or("`hours` is a whole number: 1 to 24 opens the window, 0 closes it")?;
    if h > MAX_HOURS {
        return Err(format!("a window lasts at most {MAX_HOURS} hours"));
    }
    Ok(h)
}

/// The instance's window and the offers waiting for the owner, kept by the `Instance` object.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiWindow {
    /// The window, open or past its end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<Window>,
    /// The last offer's number.
    #[serde(default)]
    pub seq: u32,
    /// Offers by their token's key.
    #[serde(default)]
    pub offers: BTreeMap<String, Offer>,
}

impl CiWindow {
    /// The window, when it is open at `now`.
    #[must_use]
    pub fn current(&self, now: u64) -> Option<&Window> {
        self.window.as_ref().filter(|w| w.open(now))
    }

    /// Offers a window of `hours` under `token` until `expires`, dropping expired offers.
    pub fn offer(&mut self, hours: u32, token: &str, now: u64, expires: u64) -> Offer {
        self.offers.retain(|_, o| o.expires > now);
        self.seq += 1;
        let offer = Offer {
            id: self.seq,
            hours,
            expires,
        };
        self.offers.insert(crate::grants::key(token), offer.clone());
        offer
    }

    /// The offer `token` names, without using it.
    ///
    /// # Errors
    ///
    /// When the token names none, or it has expired.
    pub fn peek(&self, token: &str, now: u64) -> Result<&Offer, String> {
        let o = self
            .offers
            .get(&crate::grants::key(token))
            .ok_or("this link is not valid; ask for a new one")?;
        if o.expires <= now {
            return Err("this link has expired; ask for a new one".into());
        }
        Ok(o)
    }

    /// Grants the offer `token` names, as `by` at `now`: the window opens for its hours,
    /// replacing any end, or closes for 0. The offer is spent.
    ///
    /// # Errors
    ///
    /// When the token names no live offer.
    pub fn grant(&mut self, token: &str, by: &str, now: u64) -> Result<Option<Window>, String> {
        let hours = self.peek(token, now)?.hours;
        self.offers.remove(&crate::grants::key(token));
        self.window = (hours > 0).then(|| Window {
            until: now + u64::from(hours) * 3600,
            by: by.to_string(),
        });
        Ok(self.window.clone())
    }
}
