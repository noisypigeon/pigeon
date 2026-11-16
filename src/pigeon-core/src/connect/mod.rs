//! A provider connection's flow: states, transitions and checks, pure; see
//! `.claude/decisions/2026-11-14-security-provider-connect-browser-grants.md` §1, §4 and §5.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Seconds an oauth flow may take.
pub const OAUTH_TTL: u64 = 600;
/// Seconds a key flow may take.
pub const KEY_TTL: u64 = 1800;
/// Seconds an ended flow is kept.
pub const RETAIN: u64 = 86_400;

/// How the provider is connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Shape {
    /// A browser redirect and callback.
    Oauth,
    /// A pasted key.
    Key,
}

/// What the connection is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Purpose {
    /// The credential that mints grants.
    Granter,
    /// A credential held for the instance.
    Held,
}

/// Where a flow is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    /// Created, not yet opened.
    Started,
    /// Waiting for the provider's redirect.
    AwaitingCallback,
    /// Waiting for the pasted key.
    AwaitingKey,
    /// Verifying the credential with the provider.
    Exchanging,
    /// Waiting for the owner to pick an account.
    Choosing,
    /// Granting the chosen account.
    Granting,
    /// Done.
    Granted,
    /// Ended with a reason.
    Failed,
    /// Past its deadline.
    Expired,
    /// Cancelled or superseded.
    Cancelled,
}

impl Status {
    /// Whether the flow has ended.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Granted | Self::Failed | Self::Expired | Self::Cancelled
        )
    }
}

/// A member's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// May connect providers.
    Administrator,
    /// An ordinary member.
    User,
    /// A machine member.
    ServiceAccount,
}

/// A provider account a credential reaches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAccount {
    /// The provider's ID for it.
    pub id: String,
    /// Its display name.
    pub name: String,
}

/// One connection flow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    /// The flow's ID.
    pub id: String,
    /// The instance connecting.
    pub instance: String,
    /// The provider's slug.
    pub provider: String,
    /// Oauth or key.
    pub shape: Shape,
    /// What the connection is for.
    pub purpose: Purpose,
    /// The digest of the ceiling the owner approved.
    pub ceiling_digest: String,
    /// The oauth `state` parameter.
    pub state_param: String,
    /// The PKCE verifier.
    pub verifier: String,
    /// The passkey credential that started the flow.
    pub credential: String,
    /// When it started.
    pub created: u64,
    /// When it expires.
    pub deadline: u64,
    /// Where it is.
    pub status: Status,
    /// The provider accounts the credential reaches.
    pub accounts: Vec<ProviderAccount>,
    /// The chosen account's ID.
    pub chosen: Option<String>,
    /// Why it failed.
    pub reason: Option<String>,
    /// When it ended.
    pub ended: Option<u64>,
}

/// The inputs of a new flow.
pub struct Start {
    /// The flow's ID.
    pub id: String,
    /// The instance connecting.
    pub instance: String,
    /// The provider's slug.
    pub provider: String,
    /// Oauth or key.
    pub shape: Shape,
    /// What the connection is for.
    pub purpose: Purpose,
    /// The digest of the approved ceiling.
    pub ceiling_digest: String,
    /// The oauth `state` parameter.
    pub state_param: String,
    /// The PKCE verifier.
    pub verifier: String,
    /// The starting passkey credential.
    pub credential: String,
    /// The current time.
    pub now: u64,
}

/// Why a transition is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// An input is malformed.
    Invalid(String),
    /// No live flow has that state.
    UnknownState,
    /// The flow is in another status.
    WrongStatus {
        /// The status required.
        expected: Status,
        /// The status found.
        found: Status,
    },
    /// The flow is past its deadline.
    Expired,
    /// The proof is from another passkey.
    OtherCredential,
    /// The account is not one the credential reaches.
    UnknownAccount,
    /// The account is bound to another instance.
    BoundElsewhere(String),
    /// The credential reaches no account.
    NoAccount,
    /// A key reached several accounts.
    ManyAccounts,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(m) => write!(f, "{m}"),
            Self::UnknownState => write!(f, "no live flow has that state"),
            Self::WrongStatus { expected, found } => {
                write!(f, "the flow is {found:?}, not {expected:?}")
            }
            Self::Expired => write!(f, "the flow has expired"),
            Self::OtherCredential => {
                write!(
                    f,
                    "the proof is from another passkey than the one that started the flow"
                )
            }
            Self::UnknownAccount => {
                write!(f, "that provider account is not one the credential reaches")
            }
            Self::BoundElsewhere(i) => write!(f, "that provider account belongs to instance {i}"),
            Self::NoAccount => write!(f, "the credential reaches no provider account"),
            Self::ManyAccounts => write!(f, "a key reaches exactly one provider account"),
        }
    }
}

/// A provider account as the page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublicAccount {
    /// Its display name.
    pub name: String,
    /// The last four characters of its ID.
    pub last4: String,
    /// The instance it belongs to, when not this one.
    pub bound_to: Option<String>,
}

/// A flow as the page shows it, without state, verifier or credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublicFlow {
    /// The flow's ID.
    pub id: String,
    /// The instance connecting.
    pub instance: String,
    /// The provider's slug.
    pub provider: String,
    /// Oauth or key.
    pub shape: Shape,
    /// Where it is.
    pub status: Status,
    /// The accounts to show.
    pub accounts: Vec<PublicAccount>,
    /// The chosen account's ID.
    pub chosen: Option<String>,
    /// Why it failed.
    pub reason: Option<String>,
    /// When it expires.
    pub deadline: u64,
}

fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.split('-').all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_verifier(s: &str) -> bool {
    (43..=128).contains(&s.len())
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

fn eq_const(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// Applies the deadline: a live flow past it is refused.
fn live(flow: &Flow, now: u64) -> Result<(), Refusal> {
    if !flow.status.is_terminal() && now >= flow.deadline {
        return Err(Refusal::Expired);
    }
    Ok(())
}

/// Requires `flow` live and in `expected`.
fn require(flow: &Flow, expected: Status, now: u64) -> Result<(), Refusal> {
    live(flow, now)?;
    if flow.status == expected {
        Ok(())
    } else {
        Err(Refusal::WrongStatus {
            expected,
            found: flow.status,
        })
    }
}

fn with(flow: &Flow, status: Status) -> Flow {
    Flow {
        status,
        ..flow.clone()
    }
}

/// Whether a role may connect a provider.
#[must_use]
pub fn may_connect(role: Role) -> bool {
    role == Role::Administrator
}

/// A new flow, refusing malformed inputs.
///
/// # Errors
/// `Invalid` naming the field.
pub fn start(s: Start) -> Result<Flow, Refusal> {
    let bad = |m: &str| Err(Refusal::Invalid(m.to_string()));
    if s.id.is_empty() {
        return bad("the flow id is empty");
    }
    if !is_slug(&s.instance) {
        return bad("the instance is not a slug");
    }
    if !is_slug(&s.provider) {
        return bad("the provider is not a slug");
    }
    if !is_hex64(&s.state_param) {
        return bad("the state is not 64 lowercase hex characters");
    }
    if !is_verifier(&s.verifier) {
        return bad("the verifier is not 43 to 128 unreserved characters");
    }
    if !is_hex64(&s.ceiling_digest) {
        return bad("the ceiling digest is not 64 lowercase hex characters");
    }
    if s.credential.is_empty() {
        return bad("the credential is empty");
    }
    let ttl = match s.shape {
        Shape::Oauth => OAUTH_TTL,
        Shape::Key => KEY_TTL,
    };
    Ok(Flow {
        id: s.id,
        instance: s.instance,
        provider: s.provider,
        shape: s.shape,
        purpose: s.purpose,
        ceiling_digest: s.ceiling_digest,
        state_param: s.state_param,
        verifier: s.verifier,
        credential: s.credential,
        created: s.now,
        deadline: s.now + ttl,
        status: Status::Started,
        accounts: Vec::new(),
        chosen: None,
        reason: None,
        ended: None,
    })
}

/// The page was opened: await the callback or the key.
///
/// # Errors
/// `Expired` or `WrongStatus`.
pub fn opened(flow: &Flow, now: u64) -> Result<Flow, Refusal> {
    require(flow, Status::Started, now)?;
    Ok(with(
        flow,
        match flow.shape {
            Shape::Oauth => Status::AwaitingCallback,
            Shape::Key => Status::AwaitingKey,
        },
    ))
}

/// The provider redirected back with `state_param`.
///
/// # Errors
/// `Expired`, `WrongStatus` or `UnknownState`.
pub fn callback(flow: &Flow, state_param: &str, now: u64) -> Result<Flow, Refusal> {
    require(flow, Status::AwaitingCallback, now)?;
    if !eq_const(&flow.state_param, state_param) {
        return Err(Refusal::UnknownState);
    }
    Ok(with(flow, Status::Exchanging))
}

/// A key was pasted under `credential`.
///
/// # Errors
/// `Expired`, `WrongStatus` or `OtherCredential`.
pub fn pasted(flow: &Flow, credential: &str, now: u64) -> Result<Flow, Refusal> {
    require(flow, Status::AwaitingKey, now)?;
    if flow.credential != credential {
        return Err(Refusal::OtherCredential);
    }
    Ok(with(flow, Status::Exchanging))
}

/// The provider answered with the accounts the credential reaches.
///
/// # Errors
/// `Expired`, `WrongStatus`, `NoAccount`, `ManyAccounts` or `BoundElsewhere`.
pub fn verified(
    flow: &Flow,
    accounts: Vec<ProviderAccount>,
    bound: &BTreeMap<String, String>,
    now: u64,
) -> Result<Flow, Refusal> {
    require(flow, Status::Exchanging, now)?;
    match accounts.as_slice() {
        [] => Err(Refusal::NoAccount),
        [one] => {
            if let Some(i) = bound.get(&one.id).filter(|i| **i != flow.instance) {
                return Err(Refusal::BoundElsewhere(i.clone()));
            }
            let chosen = Some(one.id.clone());
            Ok(Flow {
                status: Status::Granting,
                chosen,
                accounts,
                ..flow.clone()
            })
        }
        _ if flow.shape == Shape::Key => Err(Refusal::ManyAccounts),
        _ => Ok(Flow {
            status: Status::Choosing,
            accounts,
            ..flow.clone()
        }),
    }
}

/// The owner picked `account` under `credential`.
///
/// # Errors
/// `Expired`, `WrongStatus`, `OtherCredential`, `UnknownAccount` or `BoundElsewhere`.
pub fn pick(
    flow: &Flow,
    account: &str,
    credential: &str,
    bound: &BTreeMap<String, String>,
    now: u64,
) -> Result<Flow, Refusal> {
    require(flow, Status::Choosing, now)?;
    if flow.credential != credential {
        return Err(Refusal::OtherCredential);
    }
    if !flow.accounts.iter().any(|a| a.id == account) {
        return Err(Refusal::UnknownAccount);
    }
    if let Some(i) = bound.get(account).filter(|i| **i != flow.instance) {
        return Err(Refusal::BoundElsewhere(i.clone()));
    }
    Ok(Flow {
        status: Status::Granting,
        chosen: Some(account.to_string()),
        ..flow.clone()
    })
}

/// The grant was made.
///
/// # Errors
/// `Expired` or `WrongStatus`.
pub fn granted(flow: &Flow, now: u64) -> Result<Flow, Refusal> {
    require(flow, Status::Granting, now)?;
    Ok(Flow {
        status: Status::Granted,
        ended: Some(now),
        ..flow.clone()
    })
}

/// Ends a live flow as failed; an ended flow is unchanged.
#[must_use]
pub fn fail(flow: &Flow, reason: &str, now: u64) -> Flow {
    if flow.status.is_terminal() {
        return flow.clone();
    }
    Flow {
        status: Status::Failed,
        reason: Some(reason.to_string()),
        ended: Some(now),
        ..flow.clone()
    }
}

/// Ends a live flow as cancelled; an ended flow is unchanged.
#[must_use]
pub fn cancel(flow: &Flow, now: u64) -> Flow {
    if flow.status.is_terminal() {
        return flow.clone();
    }
    Flow {
        status: Status::Cancelled,
        reason: None,
        ended: Some(now),
        ..flow.clone()
    }
}

/// Expires a live flow past its deadline.
#[must_use]
pub fn tick(flow: &Flow, now: u64) -> Flow {
    if !flow.status.is_terminal() && now >= flow.deadline {
        return Flow {
            status: Status::Expired,
            ended: Some(now),
            ..flow.clone()
        };
    }
    flow.clone()
}

/// Whether an ended flow has been kept long enough.
#[must_use]
pub fn removable(flow: &Flow, now: u64) -> bool {
    flow.status.is_terminal() && flow.ended.is_some_and(|e| e + RETAIN <= now)
}

/// The live flows a new one for `instance` and `provider` replaces, cancelled.
#[must_use]
pub fn superseded(live: &[Flow], instance: &str, provider: &str, now: u64) -> Vec<Flow> {
    live.iter()
        .filter(|f| !f.status.is_terminal() && f.instance == instance && f.provider == provider)
        .map(|f| cancel(f, now))
        .collect()
}

/// Whether `requested` stays within `ceiling`.
///
/// # Errors
/// The requested names outside the ceiling, sorted.
pub fn within_ceiling(
    requested: &BTreeSet<String>,
    ceiling: &BTreeSet<String>,
) -> Result<(), Vec<String>> {
    let out: Vec<String> = requested.difference(ceiling).cloned().collect();
    if out.is_empty() { Ok(()) } else { Err(out) }
}

/// The PKCE S256 challenge of `verifier`.
#[must_use]
pub fn pkce_challenge(verifier: &str) -> String {
    crate::mirroring::base64url(&Sha256::digest(verifier.as_bytes()))
}

/// The last four characters of `id`, or all of it when shorter.
#[must_use]
pub fn last4(id: &str) -> String {
    let n = id.chars().count();
    id.chars().skip(n.saturating_sub(4)).collect()
}

/// The page's view of a flow.
#[must_use]
pub fn public(flow: &Flow, bound: &BTreeMap<String, String>) -> PublicFlow {
    PublicFlow {
        id: flow.id.clone(),
        instance: flow.instance.clone(),
        provider: flow.provider.clone(),
        shape: flow.shape,
        status: flow.status,
        accounts: flow
            .accounts
            .iter()
            .map(|a| PublicAccount {
                name: a.name.clone(),
                last4: last4(&a.id),
                bound_to: bound.get(&a.id).filter(|i| **i != flow.instance).cloned(),
            })
            .collect(),
        chosen: flow.chosen.clone(),
        reason: flow.reason.clone(),
        deadline: flow.deadline,
    }
}

/// The event a status is announced as.
#[must_use]
pub const fn event_name(status: Status) -> &'static str {
    match status {
        Status::Started => "connect.started",
        Status::AwaitingCallback | Status::AwaitingKey => "connect.opened",
        Status::Exchanging => "connect.callback",
        Status::Choosing => "connect.choosing",
        Status::Granting => "connect.granting",
        Status::Granted => "connect.granted",
        Status::Failed => "connect.failed",
        Status::Expired => "connect.expired",
        Status::Cancelled => "connect.cancelled",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    const VER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

    fn input(shape: Shape) -> Start {
        Start {
            id: "f1".into(),
            instance: "acme".into(),
            provider: "cloudflare".into(),
            shape,
            purpose: Purpose::Granter,
            ceiling_digest: HEX.into(),
            state_param: HEX.into(),
            verifier: VER.into(),
            credential: "cred-a".into(),
            now: 1000,
        }
    }

    fn flow(shape: Shape) -> Flow {
        start(input(shape)).unwrap()
    }

    fn at(shape: Shape, status: Status) -> Flow {
        Flow {
            status,
            ..flow(shape)
        }
    }

    fn acct(id: &str) -> ProviderAccount {
        ProviderAccount {
            id: id.into(),
            name: format!("name-{id}"),
        }
    }

    fn none() -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn bound(id: &str, inst: &str) -> BTreeMap<String, String> {
        BTreeMap::from([(id.to_string(), inst.to_string())])
    }

    fn set(v: &[&str]) -> BTreeSet<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    fn invalid(s: Start) {
        assert!(matches!(start(s), Err(Refusal::Invalid(_))));
    }

    #[test]
    fn start_refuses_a_bad_instance() {
        for i in ["", "Acme", "-a", "a-", "a--b", "a_b", &"a".repeat(41)] {
            invalid(Start {
                instance: i.into(),
                ..input(Shape::Oauth)
            });
        }
    }

    #[test]
    fn start_refuses_a_bad_provider() {
        invalid(Start {
            provider: "Cloud Flare".into(),
            ..input(Shape::Oauth)
        });
    }

    #[test]
    fn start_refuses_a_bad_state() {
        invalid(Start {
            state_param: "abc".into(),
            ..input(Shape::Oauth)
        });
        invalid(Start {
            state_param: HEX.to_uppercase(),
            ..input(Shape::Oauth)
        });
    }

    #[test]
    fn start_refuses_a_bad_verifier() {
        invalid(Start {
            verifier: "short".into(),
            ..input(Shape::Oauth)
        });
        invalid(Start {
            verifier: "a".repeat(129),
            ..input(Shape::Oauth)
        });
        invalid(Start {
            verifier: format!("{}!", "a".repeat(43)),
            ..input(Shape::Oauth)
        });
    }

    #[test]
    fn start_refuses_a_bad_ceiling_digest() {
        invalid(Start {
            ceiling_digest: "xyz".into(),
            ..input(Shape::Oauth)
        });
    }

    #[test]
    fn start_refuses_empty_id_and_credential() {
        invalid(Start {
            id: String::new(),
            ..input(Shape::Oauth)
        });
        invalid(Start {
            credential: String::new(),
            ..input(Shape::Oauth)
        });
    }

    #[test]
    fn start_sets_the_deadlines() {
        let o = flow(Shape::Oauth);
        assert_eq!(o.deadline, 1000 + OAUTH_TTL);
        assert_eq!(o.status, Status::Started);
        assert!(o.accounts.is_empty() && o.chosen.is_none() && o.reason.is_none());
        assert!(o.ended.is_none());
        assert_eq!(flow(Shape::Key).deadline, 1000 + KEY_TTL);
    }

    #[test]
    fn opened_by_shape() {
        assert_eq!(
            opened(&flow(Shape::Oauth), 1001).unwrap().status,
            Status::AwaitingCallback
        );
        assert_eq!(
            opened(&flow(Shape::Key), 1001).unwrap().status,
            Status::AwaitingKey
        );
    }

    #[test]
    fn opened_refuses_other_status_and_expiry() {
        let f = at(Shape::Oauth, Status::Exchanging);
        assert!(matches!(opened(&f, 1001), Err(Refusal::WrongStatus { .. })));
        assert_eq!(
            opened(&flow(Shape::Oauth), 1000 + OAUTH_TTL),
            Err(Refusal::Expired)
        );
    }

    #[test]
    fn callback_with_the_right_state() {
        let f = at(Shape::Oauth, Status::AwaitingCallback);
        assert_eq!(callback(&f, HEX, 1001).unwrap().status, Status::Exchanging);
    }

    #[test]
    fn callback_with_a_wrong_state() {
        let f = at(Shape::Oauth, Status::AwaitingCallback);
        let other = "f".repeat(64);
        assert_eq!(callback(&f, &other, 1001), Err(Refusal::UnknownState));
        assert_eq!(callback(&f, "short", 1001), Err(Refusal::UnknownState));
    }

    #[test]
    fn callback_on_a_used_flow() {
        let f = at(Shape::Oauth, Status::Exchanging);
        assert!(matches!(
            callback(&f, HEX, 1001),
            Err(Refusal::WrongStatus { .. })
        ));
    }

    #[test]
    fn callback_after_the_deadline() {
        let f = at(Shape::Oauth, Status::AwaitingCallback);
        assert_eq!(
            callback(&f, HEX, 1000 + OAUTH_TTL + 1),
            Err(Refusal::Expired)
        );
    }

    #[test]
    fn pasted_with_the_starting_credential() {
        let f = at(Shape::Key, Status::AwaitingKey);
        assert_eq!(
            pasted(&f, "cred-a", 1001).unwrap().status,
            Status::Exchanging
        );
    }

    #[test]
    fn pasted_with_another_credential() {
        let f = at(Shape::Key, Status::AwaitingKey);
        assert_eq!(pasted(&f, "cred-b", 1001), Err(Refusal::OtherCredential));
    }

    #[test]
    fn verified_with_no_account() {
        let f = at(Shape::Oauth, Status::Exchanging);
        assert_eq!(verified(&f, vec![], &none(), 1001), Err(Refusal::NoAccount));
    }

    #[test]
    fn verified_with_one_free_account() {
        let f = at(Shape::Oauth, Status::Exchanging);
        let g = verified(&f, vec![acct("a1")], &none(), 1001).unwrap();
        assert_eq!(g.status, Status::Granting);
        assert_eq!(g.chosen.as_deref(), Some("a1"));
        assert_eq!(g.accounts, vec![acct("a1")]);
    }

    #[test]
    fn verified_with_one_account_bound_elsewhere() {
        let f = at(Shape::Oauth, Status::Exchanging);
        let r = verified(&f, vec![acct("a1")], &bound("a1", "other"), 1001);
        assert_eq!(r, Err(Refusal::BoundElsewhere("other".into())));
    }

    #[test]
    fn verified_with_one_account_bound_here() {
        let f = at(Shape::Oauth, Status::Exchanging);
        let g = verified(&f, vec![acct("a1")], &bound("a1", "acme"), 1001).unwrap();
        assert_eq!(g.status, Status::Granting);
    }

    #[test]
    fn verified_with_many_under_oauth() {
        let f = at(Shape::Oauth, Status::Exchanging);
        let g = verified(&f, vec![acct("a1"), acct("a2")], &none(), 1001).unwrap();
        assert_eq!(g.status, Status::Choosing);
        assert_eq!(g.accounts.len(), 2);
        assert!(g.chosen.is_none());
    }

    #[test]
    fn verified_with_many_under_key() {
        let f = at(Shape::Key, Status::Exchanging);
        let r = verified(&f, vec![acct("a1"), acct("a2")], &none(), 1001);
        assert_eq!(r, Err(Refusal::ManyAccounts));
    }

    fn choosing() -> Flow {
        Flow {
            accounts: vec![acct("a1"), acct("a2")],
            ..at(Shape::Oauth, Status::Choosing)
        }
    }

    #[test]
    fn pick_with_another_credential() {
        assert_eq!(
            pick(&choosing(), "a1", "cred-b", &none(), 1001),
            Err(Refusal::OtherCredential)
        );
    }

    #[test]
    fn pick_an_unknown_account() {
        assert_eq!(
            pick(&choosing(), "zz", "cred-a", &none(), 1001),
            Err(Refusal::UnknownAccount)
        );
    }

    #[test]
    fn pick_a_bound_account() {
        let r = pick(&choosing(), "a1", "cred-a", &bound("a1", "other"), 1001);
        assert_eq!(r, Err(Refusal::BoundElsewhere("other".into())));
    }

    #[test]
    fn pick_a_free_account() {
        let g = pick(&choosing(), "a2", "cred-a", &none(), 1001).unwrap();
        assert_eq!(g.status, Status::Granting);
        assert_eq!(g.chosen.as_deref(), Some("a2"));
    }

    #[test]
    fn granted_ends_the_flow() {
        let g = granted(&at(Shape::Oauth, Status::Granting), 1005).unwrap();
        assert_eq!((g.status, g.ended), (Status::Granted, Some(1005)));
        assert!(matches!(
            granted(&at(Shape::Oauth, Status::Choosing), 1005),
            Err(Refusal::WrongStatus { .. })
        ));
    }

    #[test]
    fn fail_and_cancel_on_live_flows() {
        let f = flow(Shape::Oauth);
        let x = fail(&f, "boom", 1002);
        assert_eq!(
            (x.status, x.reason.as_deref(), x.ended),
            (Status::Failed, Some("boom"), Some(1002))
        );
        let c = cancel(&f, 1003);
        assert_eq!(
            (c.status, c.reason, c.ended),
            (Status::Cancelled, None, Some(1003))
        );
    }

    #[test]
    fn fail_cancel_and_tick_leave_terminal_flows() {
        let t = granted(&at(Shape::Oauth, Status::Granting), 1005).unwrap();
        assert_eq!(fail(&t, "x", 2000), t);
        assert_eq!(cancel(&t, 2000), t);
        assert_eq!(tick(&t, 999_999), t);
    }

    #[test]
    fn tick_expires_a_live_flow_past_its_deadline() {
        let f = flow(Shape::Oauth);
        assert_eq!(tick(&f, 1000 + OAUTH_TTL - 1), f);
        let e = tick(&f, 1000 + OAUTH_TTL);
        assert_eq!(
            (e.status, e.ended),
            (Status::Expired, Some(1000 + OAUTH_TTL))
        );
    }

    #[test]
    fn removable_after_retain() {
        let f = cancel(&flow(Shape::Oauth), 2000);
        assert!(!removable(&f, 2000 + RETAIN - 1));
        assert!(removable(&f, 2000 + RETAIN));
        assert!(!removable(&flow(Shape::Oauth), 999_999_999));
    }

    #[test]
    fn superseded_across_two_instances() {
        let a = flow(Shape::Oauth);
        let b = Flow {
            id: "f2".into(),
            instance: "beta".into(),
            ..flow(Shape::Oauth)
        };
        let done = Flow {
            id: "f3".into(),
            ..cancel(&flow(Shape::Oauth), 1001)
        };
        let out = superseded(&[a, b, done], "acme", "cloudflare", 1010);
        assert_eq!(out.len(), 1);
        assert_eq!(
            (out[0].id.as_str(), out[0].status, out[0].ended),
            ("f1", Status::Cancelled, Some(1010))
        );
        assert_eq!(
            superseded(&[flow(Shape::Oauth)], "acme", "github", 1010).len(),
            0
        );
    }

    #[test]
    fn within_ceiling_lists_the_excess_sorted() {
        assert_eq!(within_ceiling(&set(&["a"]), &set(&["a", "b"])), Ok(()));
        assert_eq!(within_ceiling(&set(&[]), &set(&[])), Ok(()));
        assert_eq!(
            within_ceiling(&set(&["z", "a", "b"]), &set(&["b"])),
            Err(vec!["a".to_string(), "z".to_string()])
        );
    }

    #[test]
    fn pkce_matches_rfc_7636() {
        assert_eq!(
            pkce_challenge(VER),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn last4_takes_the_tail() {
        assert_eq!(last4("abcdef"), "cdef");
        assert_eq!(last4("abc"), "abc");
        assert_eq!(last4(""), "");
    }

    #[test]
    fn public_never_carries_secrets() {
        let f = Flow {
            accounts: vec![acct("account-1234"), acct("account-9999")],
            ..at(Shape::Oauth, Status::Choosing)
        };
        let p = public(&f, &bound("account-9999", "other"));
        let json = serde_json::to_string(&p).unwrap();
        assert!(!json.contains(HEX) && !json.contains(VER) && !json.contains("cred-a"));
        assert_eq!(p.accounts[0].last4, "1234");
        assert_eq!(p.accounts[0].bound_to, None);
        assert_eq!(p.accounts[1].bound_to.as_deref(), Some("other"));
        let same = public(&f, &bound("account-9999", "acme"));
        assert_eq!(same.accounts[1].bound_to, None);
    }

    #[test]
    fn may_connect_only_administrators() {
        assert!(may_connect(Role::Administrator));
        assert!(!may_connect(Role::User));
        assert!(!may_connect(Role::ServiceAccount));
    }

    #[test]
    fn is_terminal_statuses() {
        for s in [
            Status::Granted,
            Status::Failed,
            Status::Expired,
            Status::Cancelled,
        ] {
            assert!(s.is_terminal());
        }
        for s in [
            Status::Started,
            Status::AwaitingCallback,
            Status::AwaitingKey,
            Status::Exchanging,
            Status::Choosing,
            Status::Granting,
        ] {
            assert!(!s.is_terminal());
        }
    }

    #[test]
    fn refusal_messages() {
        assert_eq!(Refusal::Invalid("m".into()).to_string(), "m");
        assert_eq!(
            Refusal::UnknownState.to_string(),
            "no live flow has that state"
        );
        assert_eq!(
            Refusal::WrongStatus {
                expected: Status::Started,
                found: Status::Granted
            }
            .to_string(),
            "the flow is Granted, not Started"
        );
        assert_eq!(
            Refusal::BoundElsewhere("x".into()).to_string(),
            "that provider account belongs to instance x"
        );
        assert_eq!(
            Refusal::ManyAccounts.to_string(),
            "a key reaches exactly one provider account"
        );
    }

    #[test]
    fn event_names() {
        let all = [
            (Status::Started, "connect.started"),
            (Status::AwaitingCallback, "connect.opened"),
            (Status::AwaitingKey, "connect.opened"),
            (Status::Exchanging, "connect.callback"),
            (Status::Choosing, "connect.choosing"),
            (Status::Granting, "connect.granting"),
            (Status::Granted, "connect.granted"),
            (Status::Failed, "connect.failed"),
            (Status::Expired, "connect.expired"),
            (Status::Cancelled, "connect.cancelled"),
        ];
        for (s, n) in all {
            assert_eq!(event_name(s), n);
        }
    }
}
