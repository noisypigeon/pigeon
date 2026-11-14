//! An instance's roles, memberships and invitations, and the check every operation takes; pure.
//! See `.claude/decisions/2026-11-14-security-instance-members-roles-invitations.md` §2 to §8.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

#[cfg(test)]
mod tests;

/// Every capability an operation may require.
pub const CAPABILITIES: [&str; 8] = [
    "read",
    "propose",
    "approve",
    "approve.owner",
    "auto",
    "mandates",
    "platform",
    "members",
];

/// The capabilities a service account may hold.
pub const SERVICE_CEILING: [&str; 2] = ["read", "propose"];

/// How long an invitation stays open, in seconds.
pub const INVITE_TTL: u64 = 72 * 3600;

/// The most invitations open or enrolled at once.
pub const MAX_OPEN_INVITES: usize = 10;

/// The `admitted_by` of the seeded administrator.
pub const SEED: &str = "seed";

/// A member's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Holds every capability.
    Administrator,
    /// Holds what `roles.yaml` grants.
    User,
    /// A machine account, held to the service ceiling.
    Service,
}

impl Role {
    /// The role's name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Administrator => "administrator",
            Self::User => "user",
            Self::Service => "service",
        }
    }

    /// Parses a role's name.
    ///
    /// # Errors
    /// An unknown name.
    pub fn parse(s: &str) -> Result<Role, String> {
        match s {
            "administrator" => Ok(Self::Administrator),
            "user" => Ok(Self::User),
            "service" => Ok(Self::Service),
            _ => Err(format!("unknown role `{s}`")),
        }
    }
}

/// `.config/roles.yaml`'s shape: each role's capabilities.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Roles {
    /// Capabilities by role name.
    pub roles: BTreeMap<String, Vec<String>>,
}

impl Roles {
    /// The role's capabilities, empty when absent.
    #[must_use]
    pub fn capabilities(&self, role: Role) -> &[String] {
        self.roles.get(role.name()).map_or(&[], Vec::as_slice)
    }
}

/// Every problem in the roles, empty when sound.
#[must_use]
pub fn check_roles(r: &Roles) -> Vec<String> {
    let mut out = Vec::new();
    for key in r.roles.keys() {
        if Role::parse(key).is_err() {
            out.push(format!("`{key}` is not a role"));
        }
    }
    for role in [Role::Administrator, Role::User, Role::Service] {
        let Some(caps) = r.roles.get(role.name()) else {
            out.push(format!("role `{}` is missing", role.name()));
            continue;
        };
        for (i, c) in caps.iter().enumerate() {
            if !CAPABILITIES.contains(&c.as_str()) {
                out.push(format!("`{}` lists unknown capability `{c}`", role.name()));
            }
            if caps[..i].contains(c) {
                out.push(format!("`{}` lists `{c}` twice", role.name()));
            }
        }
        if role == Role::Administrator {
            for c in CAPABILITIES {
                if !caps.iter().any(|x| x == c) {
                    out.push(format!("`administrator` must hold `{c}`"));
                }
            }
        }
        if role == Role::Service {
            for c in caps {
                if !SERVICE_CEILING.contains(&c.as_str()) {
                    out.push(format!("`service` may not hold `{c}`"));
                }
            }
        }
    }
    out
}

/// A membership's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// In force.
    Active,
    /// Ended.
    Ended,
}

/// An invitation's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InviteStatus {
    /// Awaiting the invitee.
    Open,
    /// The invitee enrolled; awaiting an administrator.
    Enrolled,
    /// Admitted as a member.
    Admitted,
    /// Declined by the invitee.
    Declined,
    /// Revoked by an administrator.
    Revoked,
    /// Past its expiry.
    Expired,
}

/// An account's membership of the instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership {
    /// The account.
    pub account: String,
    /// Its role.
    pub role: Role,
    /// Its state.
    pub status: Status,
    /// A service account's scopes.
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Who admitted it, or the last to change its role.
    pub admitted_by: String,
    /// When, in Unix seconds.
    pub at: u64,
    /// When it ended.
    #[serde(default)]
    pub ended: Option<u64>,
}

/// An invitation to join.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invitation {
    /// The token's SHA-256, hex.
    pub id: String,
    /// The invitee's email.
    pub email: String,
    /// The role offered.
    pub role: Role,
    /// Who invited.
    pub inviter: String,
    /// When it expires, in Unix seconds.
    pub expires: u64,
    /// Its state.
    pub status: InviteStatus,
    /// The enrolled account.
    #[serde(default)]
    pub account: Option<String>,
    /// The enrolled passkey.
    #[serde(default)]
    pub passkey: Option<String>,
    /// The code both sides compare.
    #[serde(default)]
    pub match_code: Option<String>,
}

/// Every membership and invitation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Members {
    /// Memberships, ended ones included.
    #[serde(default)]
    pub memberships: Vec<Membership>,
    /// Invitations, closed ones included.
    #[serde(default)]
    pub invitations: Vec<Invitation>,
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// An invitation's id: the token's SHA-256, hex.
#[must_use]
pub fn token_id(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// The six digits an invitee and an administrator compare.
#[must_use]
pub fn match_code(token: &str, public_key: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    h.update([0u8]);
    h.update(public_key);
    let d = h.finalize();
    let n = u32::from_be_bytes([d[0], d[1], d[2], d[3]]) % 1_000_000;
    format!("{n:06}")
}

fn open(s: InviteStatus) -> bool {
    matches!(s, InviteStatus::Open | InviteStatus::Enrolled)
}

impl Members {
    /// The account's active membership.
    #[must_use]
    pub fn active(&self, account: &str) -> Option<&Membership> {
        self.memberships
            .iter()
            .find(|m| m.account == account && m.status == Status::Active)
    }

    /// How many active administrators there are.
    #[must_use]
    pub fn administrators(&self) -> usize {
        self.memberships
            .iter()
            .filter(|m| m.status == Status::Active && m.role == Role::Administrator)
            .count()
    }

    fn admin(&self, by: &str) -> Result<(), String> {
        match self.active(by) {
            Some(m) if m.role == Role::Administrator => Ok(()),
            _ => Err("only an administrator may do this".to_owned()),
        }
    }

    fn invitation_mut(&mut self, id: &str) -> Result<&mut Invitation, String> {
        self.invitations
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(|| "no such invitation".to_owned())
    }

    /// Adds the first administrator when there are no memberships.
    #[must_use]
    pub fn seed(mut self, account: &str, now: u64) -> Self {
        if self.memberships.is_empty() {
            self.memberships.push(Membership {
                account: account.to_owned(),
                role: Role::Administrator,
                status: Status::Active,
                scopes: Vec::new(),
                admitted_by: SEED.to_owned(),
                at: now,
                ended: None,
            });
        }
        self
    }

    /// An administrator invites an email to a role.
    ///
    /// # Errors
    /// The inviter is no administrator, the role is service, the email is bad, or ten are open.
    pub fn invite(
        mut self,
        token: &str,
        email: &str,
        role: Role,
        inviter: &str,
        now: u64,
    ) -> Result<(Self, Invitation), String> {
        for i in &mut self.invitations {
            if open(i.status) && i.expires <= now {
                i.status = InviteStatus::Expired;
            }
        }
        self.admin(inviter)?;
        if role == Role::Service {
            return Err("a service account is created, not invited".to_owned());
        }
        let email = email.trim().to_lowercase();
        let mut parts = email.split('@');
        let ok = matches!(
            (parts.next(), parts.next(), parts.next()),
            (Some(a), Some(b), None) if !a.is_empty() && !b.is_empty()
        );
        if !ok || email.len() > 254 {
            return Err("not an email address".to_owned());
        }
        if self.invitations.iter().filter(|i| open(i.status)).count() >= MAX_OPEN_INVITES {
            return Err(format!("at most {MAX_OPEN_INVITES} invitations are open"));
        }
        let inv = Invitation {
            id: token_id(token),
            email,
            role,
            inviter: inviter.to_owned(),
            expires: now + INVITE_TTL,
            status: InviteStatus::Open,
            account: None,
            passkey: None,
            match_code: None,
        };
        self.invitations.push(inv.clone());
        Ok((self, inv))
    }

    /// The invitee enrols a passkey under the link's token.
    ///
    /// # Errors
    /// No such invitation, one not open, expired, or the account is already a member.
    pub fn enrol(
        mut self,
        token: &str,
        account: &str,
        passkey: &str,
        public_key: &[u8],
        now: u64,
    ) -> Result<(Self, Invitation), String> {
        let already = self.active(account).is_some();
        let inv = self.invitation_mut(&token_id(token))?;
        if inv.status != InviteStatus::Open {
            return Err("this link was already used".to_owned());
        }
        if inv.expires <= now {
            return Err("this link has expired".to_owned());
        }
        if already {
            return Err("already a member".to_owned());
        }
        inv.status = InviteStatus::Enrolled;
        inv.account = Some(account.to_owned());
        inv.passkey = Some(passkey.to_owned());
        inv.match_code = Some(match_code(token, public_key));
        let out = inv.clone();
        Ok((self, out))
    }

    /// An administrator admits an enrolled invitee.
    ///
    /// # Errors
    /// `by` is no administrator or is the invitee, or the invitation is not enrolled or has expired.
    pub fn admit(mut self, id: &str, by: &str, now: u64) -> Result<(Self, Membership), String> {
        self.admin(by)?;
        let inv = self.invitation_mut(id)?;
        if inv.status != InviteStatus::Enrolled {
            return Err("not enrolled".to_owned());
        }
        if inv.expires <= now {
            return Err("this link has expired".to_owned());
        }
        let account = inv.account.clone().unwrap_or_default();
        if account == by {
            return Err("no one admits themselves".to_owned());
        }
        inv.status = InviteStatus::Admitted;
        let m = Membership {
            account,
            role: inv.role,
            status: Status::Active,
            scopes: Vec::new(),
            admitted_by: by.to_owned(),
            at: now,
            ended: None,
        };
        self.memberships.push(m.clone());
        Ok((self, m))
    }

    /// The invitee declines.
    ///
    /// # Errors
    /// No such open invitation.
    pub fn decline(mut self, id: &str) -> Result<Self, String> {
        let inv = self.invitation_mut(id)?;
        if !open(inv.status) {
            return Err("no such invitation".to_owned());
        }
        inv.status = InviteStatus::Declined;
        Ok(self)
    }

    /// An administrator revokes an invitation.
    ///
    /// # Errors
    /// `by` is no administrator, or no such open invitation.
    pub fn revoke(mut self, id: &str, by: &str) -> Result<Self, String> {
        self.admin(by)?;
        let inv = self.invitation_mut(id)?;
        if !open(inv.status) {
            return Err("no such invitation".to_owned());
        }
        inv.status = InviteStatus::Revoked;
        Ok(self)
    }

    /// An administrator changes another member's role.
    ///
    /// # Errors
    /// `by` is no administrator or is the member, the member is absent, or a service role is involved.
    pub fn set_role(
        mut self,
        account: &str,
        role: Role,
        by: &str,
        now: u64,
    ) -> Result<(Self, Membership), String> {
        self.admin(by)?;
        if by == account {
            return Err("no one changes their own role".to_owned());
        }
        let m = self
            .memberships
            .iter_mut()
            .find(|m| m.account == account && m.status == Status::Active)
            .ok_or_else(|| "not a member".to_owned())?;
        if m.role == Role::Service || role == Role::Service {
            return Err("a service account's role is fixed".to_owned());
        }
        m.role = role;
        by.clone_into(&mut m.admitted_by);
        m.at = now;
        let out = m.clone();
        Ok((self, out))
    }

    fn end(mut self, account: &str, now: u64) -> Self {
        for m in &mut self.memberships {
            if m.account == account && m.status == Status::Active {
                m.status = Status::Ended;
                m.ended = Some(now);
            }
        }
        for i in &mut self.invitations {
            if open(i.status) && i.inviter == account {
                i.status = InviteStatus::Revoked;
            }
        }
        self
    }

    /// An administrator removes another member.
    ///
    /// # Errors
    /// `by` is no administrator or is the member, or the member is absent.
    pub fn remove(self, account: &str, by: &str, now: u64) -> Result<Self, String> {
        self.admin(by)?;
        if by == account {
            return Err("use leave to end your own membership".to_owned());
        }
        if self.active(account).is_none() {
            return Err("not a member".to_owned());
        }
        Ok(self.end(account, now))
    }

    /// A member ends their own membership.
    ///
    /// # Errors
    /// Not a member, or the last administrator.
    pub fn leave(self, account: &str, now: u64) -> Result<Self, String> {
        let Some(m) = self.active(account) else {
            return Err("not a member".to_owned());
        };
        if m.role == Role::Administrator && self.administrators() == 1 {
            return Err("the last administrator can't leave".to_owned());
        }
        Ok(self.end(account, now))
    }

    /// An administrator creates a service account.
    ///
    /// # Errors
    /// `by` is no administrator, the account is a member, or the scopes are empty or above the ceiling.
    pub fn create_service(
        mut self,
        account: &str,
        scopes: &[String],
        by: &str,
        now: u64,
    ) -> Result<(Self, Membership), String> {
        self.admin(by)?;
        if self.active(account).is_some() {
            return Err("already a member".to_owned());
        }
        if scopes.is_empty()
            || scopes
                .iter()
                .any(|s| !SERVICE_CEILING.contains(&s.as_str()))
        {
            return Err("a service account's scopes are read and propose only".to_owned());
        }
        let m = Membership {
            account: account.to_owned(),
            role: Role::Service,
            status: Status::Active,
            scopes: scopes.to_vec(),
            admitted_by: by.to_owned(),
            at: now,
            ended: None,
        };
        self.memberships.push(m.clone());
        Ok((self, m))
    }
}

/// What an operation asks of the check.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// The calling account.
    pub account: &'a str,
    /// The capability the operation needs.
    pub capability: &'a str,
    /// Whether it needs a passkey proof.
    pub needs_proof: bool,
    /// The account that proved with a passkey.
    pub proof: Option<&'a str>,
    /// The change's proposer.
    pub proposer: Option<&'a str>,
    /// The membership acted on.
    pub subject: Option<&'a str>,
}

/// Why the check refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// No active membership.
    NotMember,
    /// The role lacks the capability.
    Capability,
    /// No passkey proof from the same account.
    Proof,
    /// The account acts on its own change or membership.
    Separation,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotMember => "not a member of this instance",
            Self::Capability => "this role does not hold the capability",
            Self::Proof => "this needs a passkey proof from the same account",
            Self::Separation => "not on one's own change or membership",
        })
    }
}

/// §8's check, in order: membership, capability, scope, proof, then separation.
///
/// # Errors
/// The first [`Refusal`] that applies.
pub fn check<'a>(
    roles: &Roles,
    members: &'a Members,
    r: &Request<'_>,
) -> Result<&'a Membership, Refusal> {
    let m = members.active(r.account).ok_or(Refusal::NotMember)?;
    if !roles.capabilities(m.role).iter().any(|c| c == r.capability) {
        return Err(Refusal::Capability);
    }
    let service = m.role == Role::Service;
    if service
        && (!m.scopes.iter().any(|s| s == r.capability) || !SERVICE_CEILING.contains(&r.capability))
    {
        return Err(Refusal::Capability);
    }
    if r.needs_proof && (service || r.proof != Some(r.account)) {
        return Err(Refusal::Proof);
    }
    if r.proposer == Some(r.account) {
        return Err(Refusal::Separation);
    }
    if r.capability == "members" && r.subject == Some(r.account) {
        return Err(Refusal::Separation);
    }
    Ok(m)
}
