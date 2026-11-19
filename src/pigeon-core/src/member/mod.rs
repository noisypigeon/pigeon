//! An instance's roles, memberships and invitations, and the check every operation takes; pure.
//! See `.claude/decisions/2026-11-14-security-instance-members-roles-invitations.md` §2 to §8.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

#[cfg(test)]
mod tests;

pub mod bind;
pub mod grant;
pub mod handover;
pub mod join;
pub mod listing;
pub mod shadow;

/// Every level permission an operation may require.
pub const PERMISSIONS: [&str; 8] = [
    "read",
    "propose",
    "approve",
    "approve.owner",
    "auto",
    "mandates",
    "platform",
    "members",
];

/// The capabilities the Administrator grants per member.
pub const CAPABILITIES: [&str; 6] = ["subdomain", "domain", "blog", "icbc", "sites", "instances"];

/// The capabilities that can be granted yet; each other one waits for its feature's ADR.
pub const GRANTABLE: [&str; 1] = ["icbc"];

/// The permissions a standard member may hold.
pub const STANDARD_CEILING: [&str; 0] = [];

/// The `granted_by` of a grant the learner migration made.
pub const MIGRATION: &str = "migration";

/// The permissions a service account may hold.
pub const SERVICE_CEILING: [&str; 2] = ["read", "propose"];

/// How long an invitation stays open, in seconds.
pub const INVITE_TTL: u64 = 72 * 3600;

/// The most invitations open or enrolled at once.
pub const MAX_OPEN_INVITES: usize = 10;

/// The `admitted_by` of the seeded administrator.
pub const SEED: &str = "seed";

const ONE_ADMINISTRATOR: &str = "an instance's administrator is seeded, never invited or promoted";

/// The `Instance` storage key the members are kept under.
pub const STORE: &str = "members";

/// A member's role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Holds every capability.
    Administrator,
    /// A person holding only what the Administrator grants.
    #[serde(alias = "user")]
    Standard,
    /// A machine account, held to the service ceiling.
    Service,
}

impl Role {
    /// The role's name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Administrator => "administrator",
            Self::Standard => "standard",
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
            "standard" | "user" => Ok(Self::Standard),
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
    pub fn permissions(&self, role: Role) -> &[String] {
        self.roles.get(role.name()).map_or(&[], Vec::as_slice)
    }

    /// `.config/roles.yaml`'s text, in the one shape its eviction policy allows: a comment,
    /// `roles:`, then `  <role>: [<cap>, ...]` lines.
    ///
    /// # Errors
    /// A line in any other shape.
    pub fn parse_file(text: &str) -> Result<Self, String> {
        let mut roles = BTreeMap::new();
        for (i, line) in text.lines().enumerate() {
            if line.starts_with('#') || line == "roles:" || line.trim().is_empty() {
                continue;
            }
            let bad = || format!("line {}: not `  <role>: [<capability>, ...]`", i + 1);
            let (name, list) = line
                .strip_prefix("  ")
                .and_then(|l| l.split_once(": "))
                .ok_or_else(bad)?;
            let list = list
                .strip_prefix('[')
                .and_then(|l| l.strip_suffix(']'))
                .ok_or_else(bad)?;
            let caps = list
                .split(',')
                .map(str::trim)
                .filter(|c| !c.is_empty())
                .map(str::to_owned)
                .collect();
            roles.insert(name.to_owned(), caps);
        }
        Ok(Self { roles })
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
    for role in [Role::Administrator, Role::Standard, Role::Service] {
        let Some(caps) = r.roles.get(role.name()) else {
            out.push(format!("role `{}` is missing", role.name()));
            continue;
        };
        for (i, c) in caps.iter().enumerate() {
            if !PERMISSIONS.contains(&c.as_str()) {
                out.push(format!("`{}` lists unknown capability `{c}`", role.name()));
            }
            if caps[..i].contains(c) {
                out.push(format!("`{}` lists `{c}` twice", role.name()));
            }
        }
        if role == Role::Administrator {
            for c in PERMISSIONS {
                if !caps.iter().any(|x| x == c) {
                    out.push(format!("`administrator` must hold `{c}`"));
                }
            }
        }
        if role == Role::Standard {
            for c in caps {
                if !STANDARD_CEILING.contains(&c.as_str()) {
                    out.push(format!("`standard` may not hold `{c}`"));
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

/// A capability the Administrator granted a standard member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// The capability, one of [`CAPABILITIES`].
    pub capability: String,
    /// What it is limited to, when it takes a scope.
    #[serde(default)]
    pub scope: Option<String>,
    /// Who granted it.
    pub granted_by: String,
    /// When, in Unix seconds.
    pub at: u64,
    /// When it lapses; never when absent.
    #[serde(default)]
    pub expires: Option<u64>,
    /// When it was revoked.
    #[serde(default)]
    pub revoked: Option<u64>,
}

impl Grant {
    /// Whether it is in force at `now`.
    #[must_use]
    pub fn active(&self, now: u64) -> bool {
        self.revoked.is_none() && self.expires.is_none_or(|e| now < e)
    }
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
    /// A standard member's capability grants, revoked and expired ones included.
    #[serde(default)]
    pub grants: Vec<Grant>,
    /// A service account's allowed operations, `<area>.*` or a name; every one within its scopes when empty.
    #[serde(default)]
    pub tools: Vec<String>,
    /// A service account's refused operations, `<area>.*` or a name.
    #[serde(default)]
    pub denied: Vec<String>,
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
    /// Service accounts' bound credentials, revoked ones included.
    #[serde(default)]
    pub credentials: Vec<Credential>,
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

/// What an account may see of the members: its own row, and an administrator's whole roster.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct View {
    /// The caller's active membership.
    pub me: Option<Membership>,
    /// Every active membership, for an administrator.
    pub members: Vec<Membership>,
    /// Every open or enrolled invitation, for an administrator.
    pub invitations: Vec<Invitation>,
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

    /// What `account` may see: its own row, and for an administrator every active member and
    /// open invitation.
    #[must_use]
    pub fn view(&self, account: &str) -> View {
        let me = self.active(account).cloned();
        if me.as_ref().is_some_and(|m| m.role == Role::Administrator) {
            View {
                me,
                members: self
                    .memberships
                    .iter()
                    .filter(|m| m.status == Status::Active)
                    .cloned()
                    .collect(),
                invitations: self
                    .invitations
                    .iter()
                    .filter(|i| open(i.status))
                    .cloned()
                    .collect(),
            }
        } else {
            View {
                me,
                members: Vec::new(),
                invitations: Vec::new(),
            }
        }
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
                grants: Vec::new(),
                tools: Vec::new(),
                denied: Vec::new(),
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
        if role == Role::Administrator {
            return Err(ONE_ADMINISTRATOR.to_owned());
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
        if inv.role == Role::Administrator {
            return Err(ONE_ADMINISTRATOR.to_owned());
        }
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
            grants: Vec::new(),
            tools: Vec::new(),
            denied: Vec::new(),
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
        if role == Role::Administrator {
            return Err(ONE_ADMINISTRATOR.to_owned());
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
                for g in &mut m.grants {
                    if g.active(now) {
                        g.revoked = Some(now);
                    }
                }
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
            grants: Vec::new(),
            tools: Vec::new(),
            denied: Vec::new(),
            admitted_by: by.to_owned(),
            at: now,
            ended: None,
        };
        self.memberships.push(m.clone());
        Ok((self, m))
    }

    /// An administrator grants a standard member a capability, replacing any active grant of the
    /// same capability and scope.
    ///
    /// # Errors
    /// `by` is no administrator, the capability is unknown or not grantable yet, the member is no
    /// standard member, the scope breaks the capability's rule, or the expiry is not in the future.
    pub fn grant(
        mut self,
        account: &str,
        capability: &str,
        scope: Option<&str>,
        expires: Option<u64>,
        by: &str,
        now: u64,
    ) -> Result<(Self, Grant), String> {
        self.admin(by)?;
        if !CAPABILITIES.contains(&capability) {
            return Err(format!("`{capability}` is not a capability"));
        }
        if !GRANTABLE.contains(&capability) {
            return Err(format!("`{capability}` can't be granted yet"));
        }
        if !self
            .active(account)
            .is_some_and(|m| m.role == Role::Standard)
        {
            return Err("only a standard member is granted a capability".to_owned());
        }
        if let Some(c) = crate::capability::get(capability) {
            use crate::capability::Capability as _;
            c.check_scope(scope).map_err(|r| r.0)?;
        }
        if expires.is_some_and(|t| t <= now) {
            return Err("a grant must expire in the future".to_owned());
        }
        let grant = Grant {
            capability: capability.to_owned(),
            scope: scope.map(str::to_owned),
            granted_by: by.to_owned(),
            at: now,
            expires,
            revoked: None,
        };
        if let Some(m) = self
            .memberships
            .iter_mut()
            .find(|m| m.account == account && m.status == Status::Active)
        {
            for g in &mut m.grants {
                if g.active(now) && g.capability == capability && g.scope.as_deref() == scope {
                    g.revoked = Some(now);
                }
            }
            m.grants.push(grant.clone());
        }
        Ok((self, grant))
    }

    /// An administrator revokes every active grant of a capability from a member.
    ///
    /// # Errors
    /// `by` is no administrator, the account is no member, or it holds no such grant.
    pub fn revoke_grant(
        mut self,
        account: &str,
        capability: &str,
        by: &str,
        now: u64,
    ) -> Result<Self, String> {
        self.admin(by)?;
        let m = self
            .memberships
            .iter_mut()
            .find(|m| m.account == account && m.status == Status::Active)
            .ok_or_else(|| "not a member".to_owned())?;
        let mut any = false;
        for g in &mut m.grants {
            if g.capability == capability && g.active(now) {
                g.revoked = Some(now);
                any = true;
            }
        }
        if any {
            Ok(self)
        } else {
            Err("no such grant".to_owned())
        }
    }

    /// Whether `account` holds `capability` (for `scope`, when given) at `now`.
    #[must_use]
    pub fn holds(&self, account: &str, capability: &str, scope: Option<&str>, now: u64) -> bool {
        match self.active(account) {
            Some(m) if m.role == Role::Administrator => true,
            Some(m) if m.role == Role::Standard => m.grants.iter().any(|g| {
                g.capability == capability
                    && g.active(now)
                    && scope.is_none_or(|s| g.scope.as_deref() == Some(s))
            }),
            _ => false,
        }
    }

    /// On a learner instance, gives each standard member with no `icbc` record the grant; true
    /// when any was added.
    #[must_use]
    pub fn migrate(mut self, learner: bool, now: u64) -> (Self, bool) {
        if !learner {
            return (self, false);
        }
        let mut added = false;
        for m in &mut self.memberships {
            if m.status == Status::Active
                && m.role == Role::Standard
                && !m.grants.iter().any(|g| g.capability == "icbc")
            {
                m.grants.push(Grant {
                    capability: "icbc".to_owned(),
                    scope: None,
                    granted_by: MIGRATION.to_owned(),
                    at: now,
                    expires: None,
                    revoked: None,
                });
                added = true;
            }
        }
        (self, added)
    }
}

/// What an operation requires of the caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requires {
    /// A level permission, by name in [`PERMISSIONS`].
    Permission(&'static str),
    /// A granted capability, and the argument naming its scope.
    Capability {
        /// The capability's name.
        name: &'static str,
        /// The argument naming the scope.
        scope_arg: Option<&'static str>,
    },
    /// The caller's own credentials or membership.
    Own,
}

/// What an operation asks of the check.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// The calling account.
    pub account: &'a str,
    /// The operation called.
    pub operation: &'a str,
    /// What the operation needs.
    pub requires: Requires,
    /// The scope argument's value.
    pub scope: Option<&'a str>,
    /// The time, in Unix seconds.
    pub now: u64,
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
    /// The level or its grants lack what is required.
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
            Self::Capability => "this level or its grants do not allow it",
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
    let service = m.role == Role::Service;
    let allowed = match r.requires {
        Requires::Permission(p) => {
            roles.permissions(m.role).iter().any(|c| c == p)
                && (m.role != Role::Standard || STANDARD_CEILING.contains(&p))
                && (!service || (m.scopes.iter().any(|s| s == p) && SERVICE_CEILING.contains(&p)))
        }
        Requires::Capability { name, .. } => members.holds(r.account, name, r.scope, r.now),
        Requires::Own => r.subject == Some(r.account),
    };
    if !allowed {
        return Err(Refusal::Capability);
    }
    if service
        && ((!m.tools.is_empty() && !m.tools.iter().any(|t| matches(t, r.operation)))
            || m.denied.iter().any(|d| matches(d, r.operation)))
    {
        return Err(Refusal::Capability);
    }
    if r.needs_proof && (service || r.proof != Some(r.account)) {
        return Err(Refusal::Proof);
    }
    if r.proposer == Some(r.account) {
        return Err(Refusal::Separation);
    }
    if r.requires == Requires::Permission("members") && r.subject == Some(r.account) {
        return Err(Refusal::Separation);
    }
    Ok(m)
}

/// Whether `pattern`, an operation's name or `<area>.*`, covers `operation`.
#[must_use]
pub fn matches(pattern: &str, operation: &str) -> bool {
    pattern
        .strip_suffix('*')
        .map_or(pattern == operation, |area| {
            area.ends_with('.') && operation.starts_with(area)
        })
}

/// The bound credential that stands for both agent service users; a ticket's class picks one.
pub const AGENT: &str = "agent";

/// The operations an agent executor or a pool runner may not call.
pub const EXECUTOR_DENIED: [&str; 7] = [
    "brief.create",
    "brief.amend",
    "brief.review",
    "owner.*",
    "mandate.*",
    "lane.*",
    "grant.*",
];

/// The seeded service users: name, scopes, `tools` and whether the executor's denials apply; see
/// .claude/decisions/2026-11-18-security-users-levels-capability-grants.md, step 9's shape.
pub const SERVICE_USERS: [(&str, &[&str], &[&str], bool); 5] = [
    ("agent-planner", &["read", "propose"], &[], false),
    ("agent-executor", &["read", "propose"], &[], true),
    ("pool", &["read", "propose"], &[], true),
    ("jobs", &["read"], &["build.*"], false),
    (
        "ci",
        &["read", "propose"],
        &["build.*", "release.*", "ci.*"],
        false,
    ),
];

/// The agent service user a session of model class `model` acts as: a planner class (`opus`,
/// `fable`) plans; any other class, or none, executes.
#[must_use]
pub fn agent_user(model: Option<&str>) -> &'static str {
    let plans = model.is_some_and(|m| {
        crate::sessions::Class::parse(Some(m)).is_ok_and(|c| c.role() == "planner")
    });
    if plans {
        "agent-planner"
    } else {
        "agent-executor"
    }
}

/// A credential bound to a service account; only the client ID's SHA-256 is kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    /// The service account, or [`AGENT`].
    pub account: String,
    /// Its kind: `access`.
    pub kind: String,
    /// The client ID's SHA-256, hex.
    pub client_hash: String,
    /// Who bound it.
    pub bound_by: String,
    /// When, in Unix seconds.
    pub at: u64,
    /// When it was revoked.
    #[serde(default)]
    pub revoked: Option<u64>,
}

impl Members {
    /// Adds each seeded service user that has no active membership.
    #[must_use]
    pub fn seed_services(mut self, now: u64) -> Self {
        for (name, scopes, tools, executor) in SERVICE_USERS {
            if self.active(name).is_some() {
                continue;
            }
            self.memberships.push(Membership {
                account: name.to_owned(),
                role: Role::Service,
                status: Status::Active,
                scopes: scopes.iter().map(|s| (*s).to_owned()).collect(),
                grants: Vec::new(),
                tools: tools.iter().map(|s| (*s).to_owned()).collect(),
                denied: if executor {
                    EXECUTOR_DENIED.iter().map(|s| (*s).to_owned()).collect()
                } else {
                    Vec::new()
                },
                admitted_by: SEED.to_owned(),
                at: now,
                ended: None,
            });
        }
        self
    }

    /// The administrator binds an Access client ID to a service account, or to [`AGENT`],
    /// replacing any active binding of the same ID.
    ///
    /// # Errors
    /// As [`Self::bind_hash`], or the client ID is empty.
    pub fn bind(
        self,
        account: &str,
        client_id: &str,
        by: &str,
        now: u64,
    ) -> Result<(Self, Credential), String> {
        if client_id.trim().is_empty() {
            return Err("no client ID".to_owned());
        }
        self.bind_hash(account, &token_id(client_id), by, now)
    }

    /// The administrator binds the client ID whose SHA-256 is `client_hash` to a service
    /// account, or to [`AGENT`], replacing any active binding of the same hash.
    ///
    /// # Errors
    /// `by` is no administrator, `client_hash` is no SHA-256, or the account is neither
    /// [`AGENT`] nor an active service account.
    pub fn bind_hash(
        mut self,
        account: &str,
        client_hash: &str,
        by: &str,
        now: u64,
    ) -> Result<(Self, Credential), String> {
        self.admin(by)?;
        if !bind::is_hash(client_hash) {
            return Err("not a client ID's SHA-256".to_owned());
        }
        if account != AGENT
            && !self
                .active(account)
                .is_some_and(|m| m.role == Role::Service)
        {
            return Err("only a service account is bound a credential".to_owned());
        }
        for c in &mut self.credentials {
            if c.client_hash == client_hash && c.revoked.is_none() {
                c.revoked = Some(now);
            }
        }
        let c = Credential {
            account: account.to_owned(),
            kind: "access".to_owned(),
            client_hash: client_hash.to_owned(),
            bound_by: by.to_owned(),
            at: now,
            revoked: None,
        };
        self.credentials.push(c.clone());
        Ok((self, c))
    }

    /// The administrator revokes the binding of an Access client ID.
    ///
    /// # Errors
    /// As [`Self::unbind_hash`].
    pub fn unbind(self, client_id: &str, by: &str, now: u64) -> Result<Self, String> {
        self.unbind_hash(&token_id(client_id), by, now)
    }

    /// The administrator revokes the binding of the client ID whose SHA-256 is `client_hash`.
    ///
    /// # Errors
    /// `by` is no administrator, or the hash has no active binding.
    pub fn unbind_hash(mut self, client_hash: &str, by: &str, now: u64) -> Result<Self, String> {
        self.admin(by)?;
        let mut any = false;
        for c in &mut self.credentials {
            if c.client_hash == client_hash && c.revoked.is_none() {
                c.revoked = Some(now);
                any = true;
            }
        }
        if any {
            Ok(self)
        } else {
            Err("no such credential".to_owned())
        }
    }

    /// The service account an Access client ID is bound to: [`AGENT`] resolves through
    /// [`agent_user`] with the session's model class.
    #[must_use]
    pub fn service_for(&self, client_id: &str, model: Option<&str>) -> Option<&str> {
        let client_hash = token_id(client_id);
        let c = self
            .credentials
            .iter()
            .find(|c| c.client_hash == client_hash && c.revoked.is_none())?;
        let name = if c.account == AGENT {
            agent_user(model)
        } else {
            c.account.as_str()
        };
        self.active(name)
            .filter(|m| m.role == Role::Service)
            .map(|m| m.account.as_str())
    }
}
