//! `lease.flip` and `promote.hold`'s offers: the owner's switches over the lease, granted with a passkey on `/lease-switch/<id>`; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md, its 2026-11-20 amendment.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The operation that flips the lease.
pub const FLIP: &str = "lease.flip";
/// The operation that holds or releases automatic promotion.
pub const HOLD: &str = "promote.hold";
/// The operation a grant's proof is bound to.
pub const GRANT: &str = "lease.switch";
/// The page's path prefix.
pub const PAGE: &str = "/lease-switch/";
/// Where the `Instance` keeps the offers.
pub const STORE: &str = "lease-switch-offers";
/// The colours a flip may name.
pub const COLOURS: &[&str] = &["pigeon-control", "pigeon-green", "pigeon-blue"];

/// What the owner switches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Switch {
    /// Flip the lease to this colour.
    Flip(String),
    /// Hold (true) or release (false) automatic promotion.
    Hold(bool),
}

/// The switch `op` asks for with `args`.
///
/// # Errors
///
/// When `op` isn't a switch, or its argument isn't one it takes.
pub fn switch(
    op: &str,
    args: &serde_json::Map<String, serde_json::Value>,
) -> Result<Switch, String> {
    match op {
        FLIP => {
            let c = args
                .get("color")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .ok_or("`color` names the colour to flip to")?;
            if COLOURS.contains(&c) {
                Ok(Switch::Flip(c.to_string()))
            } else {
                Err(format!("`{c}` is not a colour"))
            }
        }
        HOLD => args
            .get("hold")
            .and_then(serde_json::Value::as_bool)
            .map(Switch::Hold)
            .ok_or_else(|| "`hold` is true or false".to_string()),
        _ => Err(format!("`{op}` is not a lease switch")),
    }
}

/// A switch waiting for the owner's passkey.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offer {
    /// Its number, for the page's path.
    pub id: u32,
    /// What it switches.
    pub switch: Switch,
    /// When the page's link expires, in Unix seconds.
    pub expires: u64,
}

impl Offer {
    /// The page's path, with `token`.
    #[must_use]
    pub fn path(&self, token: &str) -> String {
        format!("{PAGE}{}?grant={token}", self.id)
    }

    /// What the page asks the owner.
    #[must_use]
    pub fn title(&self) -> String {
        match &self.switch {
            Switch::Flip(c) => format!("Flip the lease to {c}"),
            Switch::Hold(true) => "Hold automatic promotion".into(),
            Switch::Hold(false) => "Release automatic promotion".into(),
        }
    }

    /// What the switch does, as the page says it.
    #[must_use]
    pub fn what(&self) -> String {
        match &self.switch {
            Switch::Flip(c) => format!(
                "The lease moves to {c} through the drain, adopt and parity, then a 10-minute watch that flips back on a failure. Started here, it isn't held by promote.hold."
            ),
            Switch::Hold(true) => "While held, a deploy still ships and checks the standby, but MainRun doesn't promote it. lease.flip still flips.".into(),
            Switch::Hold(false) => "MainRun promotes a standby that passed its checks again.".into(),
        }
    }
}

/// The offers, kept by the `Instance`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Offers {
    /// The last offer's number.
    #[serde(default)]
    pub seq: u32,
    /// Offers by their token's key.
    #[serde(default)]
    pub offers: BTreeMap<String, Offer>,
}

impl Offers {
    /// The offers with one more, switching `switch` under `token` until `expires`, less the expired ones; and that offer.
    #[must_use]
    pub fn offer(self, switch: Switch, token: &str, now: u64, expires: u64) -> (Self, Offer) {
        let seq = self.seq + 1;
        let offer = Offer {
            id: seq,
            switch,
            expires,
        };
        let mut offers: BTreeMap<String, Offer> = self
            .offers
            .into_iter()
            .filter(|(_, o)| o.expires > now)
            .collect();
        offers.insert(crate::approval::grants::key(token), offer.clone());
        (Self { seq, offers }, offer)
    }

    /// The offer `token` names, without using it.
    ///
    /// # Errors
    ///
    /// When the token names none, or it has expired.
    pub fn peek(&self, token: &str, now: u64) -> Result<&Offer, String> {
        let o = self
            .offers
            .get(&crate::approval::grants::key(token))
            .ok_or("this link is not valid; ask for a new one")?;
        if o.expires <= now {
            return Err("this link has expired; ask for a new one".into());
        }
        Ok(o)
    }

    /// The offers without the one `token` names, and that offer.
    ///
    /// # Errors
    ///
    /// When the token names no live offer.
    pub fn take(self, token: &str, now: u64) -> Result<(Self, Offer), String> {
        let o = self.peek(token, now)?.clone();
        let key = crate::approval::grants::key(token);
        let offers = self.offers.into_iter().filter(|(k, _)| *k != key).collect();
        Ok((
            Self {
                seq: self.seq,
                offers,
            },
            o,
        ))
    }
}

/// The offer number in a page path `/lease-switch/<id>`.
#[must_use]
pub fn parse(path: &str) -> Option<u32> {
    let rest = path.strip_prefix(PAGE)?;
    let id = rest.split_once('?').map_or(rest, |(p, _)| p);
    (!id.starts_with('0')).then(|| id.parse().ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const NOW: u64 = 1_795_000_000;

    fn args(v: &serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        v.as_object().cloned().unwrap_or_default()
    }

    #[test]
    fn a_switch_is_checked_and_its_offer_is_taken_once() {
        assert_eq!(
            switch(FLIP, &args(&json!({ "color": " pigeon-green " }))),
            Ok(Switch::Flip("pigeon-green".into()))
        );
        assert!(switch(FLIP, &args(&json!({ "color": "pigeon-red" }))).is_err());
        assert!(switch(FLIP, &args(&json!({}))).is_err());
        assert_eq!(
            switch(HOLD, &args(&json!({ "hold": false }))),
            Ok(Switch::Hold(false))
        );
        assert!(switch(HOLD, &args(&json!({ "hold": "yes" }))).is_err());
        assert!(switch("lease.other", &args(&json!({}))).is_err());

        let (o, offer) =
            Offers::default().offer(Switch::Flip("pigeon-green".into()), "t", NOW, NOW + 60);
        assert_eq!(offer.path("t"), "/lease-switch/1?grant=t");
        assert_eq!(parse("/lease-switch/1?grant=t"), Some(1));
        assert_eq!(o.peek("t", NOW), Ok(&offer));
        assert!(o.peek("t", NOW + 61).is_err());
        let (o, taken) = o.take("t", NOW).expect("live");
        assert_eq!(taken, offer);
        assert!(o.take("t", NOW).is_err());

        assert_eq!(offer.title(), "Flip the lease to pigeon-green");
        let hold = |h| Offer {
            id: 2,
            switch: Switch::Hold(h),
            expires: NOW,
        };
        assert_eq!(hold(true).title(), "Hold automatic promotion");
        assert_eq!(hold(false).title(), "Release automatic promotion");
    }
}
