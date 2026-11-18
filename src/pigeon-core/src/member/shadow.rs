//! R3's shadow: `check` beside today's decision, which it never changes; see
//! `.claude/decisions/2026-11-17-runtime-mcp-feed-view-contract.md`, its R3 amendment.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{Members, Request, Role, Roles, check, token_id};
use crate::wire::Operation;

/// Who calls, as the door names them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Caller {
    /// A person signed in to Access, who is the Administrator.
    Person,
    /// A device, acting as its account.
    Device {
        /// The device's account.
        account: String,
    },
    /// A service token, and its ticket's model class.
    Service {
        /// The token's client ID.
        client_id: String,
        /// Its ticket's class, when it has one.
        model: Option<String>,
    },
}

/// The Administrator's account: the one active Administrator membership.
#[must_use]
pub fn administrator(members: &Members) -> Option<&str> {
    members
        .memberships
        .iter()
        .find(|m| m.status == super::Status::Active && m.role == Role::Administrator)
        .map(|m| m.account.as_str())
}

/// One difference from today's decision, as `member.shadow` logs it; never a client ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shadow {
    /// The operation.
    pub operation: String,
    /// The member checked, or `unbound`.
    pub account: String,
    /// An unbound service token's `token_id`.
    pub client_hash: Option<String>,
    /// Whether today's check allowed it.
    pub today: bool,
    /// Why `check` refuses, when it does.
    pub refusal: Option<String>,
}

impl Shadow {
    /// The key one difference is logged under, once.
    #[must_use]
    pub fn key(&self) -> String {
        format!(
            "{}|{}|{}|{}",
            self.operation,
            self.client_hash.as_deref().unwrap_or(&self.account),
            self.today,
            self.refusal.as_deref().unwrap_or("allowed")
        )
    }
}

/// `check` for `caller` on `op`, compared with `today`; a difference when they disagree.
#[must_use]
pub fn compare(
    roles: &Roles,
    members: &Members,
    caller: &Caller,
    op: &Operation,
    args: &Map<String, Value>,
    today: bool,
    now: u64,
) -> Option<Shadow> {
    let account = match caller {
        Caller::Person => administrator(members).map(str::to_string),
        Caller::Device { account } => Some(account.clone()),
        Caller::Service { client_id, model } => members
            .service_for(client_id, model.as_deref())
            .map(str::to_string),
    };
    let Some(account) = account else {
        let client_hash = match caller {
            Caller::Service { client_id, .. } => Some(token_id(client_id)),
            _ => None,
        };
        return Some(Shadow {
            operation: op.name.to_string(),
            account: "unbound".into(),
            client_hash,
            today,
            refusal: Some(super::Refusal::NotMember.to_string()),
        });
    };
    let requires = op.requires();
    let scope = match requires {
        super::Requires::Capability {
            scope_arg: Some(a), ..
        } => args.get(a).and_then(Value::as_str),
        _ => None,
    };
    let subject = args
        .get("account")
        .and_then(Value::as_str)
        .or_else(|| (requires == super::Requires::Own).then_some(account.as_str()));
    let request = Request {
        account: &account,
        operation: op.name,
        requires,
        scope,
        now,
        needs_proof: false,
        proof: None,
        proposer: None,
        subject,
    };
    let refusal = check(roles, members, &request).err().map(|r| r.to_string());
    (refusal.is_none() != today).then(|| Shadow {
        operation: op.name.to_string(),
        account,
        client_hash: None,
        today,
        refusal,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROLES: &str = r#"{"roles": {"administrator": ["read", "propose", "approve", "approve.owner", "auto", "mandates", "platform", "members"], "standard": [], "service": ["read", "propose"]}}"#;

    fn roles() -> Roles {
        serde_json::from_str(ROLES).unwrap()
    }

    fn op(name: &str) -> &'static Operation {
        crate::wire::catalog()
            .iter()
            .find(|o| o.name == name)
            .unwrap()
    }

    fn members() -> Members {
        Members::default().seed("root", 100).seed_services(100)
    }

    fn agent(model: &str) -> Caller {
        Caller::Service {
            client_id: "client-1".into(),
            model: Some(model.into()),
        }
    }

    #[test]
    fn the_person_is_the_administrator_and_agrees_with_today() {
        let m = members();
        assert_eq!(administrator(&m), Some("root"));
        let got = compare(
            &roles(),
            &m,
            &Caller::Person,
            op("change.status"),
            &Map::new(),
            true,
            200,
        );
        assert_eq!(got, None);
    }

    #[test]
    fn an_unbound_token_is_logged_by_its_hash_never_its_id() {
        let got = compare(
            &roles(),
            &members(),
            &agent("opus"),
            op("change.status"),
            &Map::new(),
            true,
            200,
        )
        .unwrap();
        assert_eq!(got.account, "unbound");
        assert_eq!(got.client_hash, Some(token_id("client-1")));
        assert!(!got.key().contains("client-1"));
    }

    #[test]
    fn a_bound_agent_token_acts_as_its_tickets_class() {
        let (m, _) = members()
            .bind(super::super::AGENT, "client-1", "root", 150)
            .unwrap();
        let read = compare(
            &roles(),
            &m,
            &agent("opus"),
            op("change.status"),
            &Map::new(),
            true,
            200,
        );
        assert_eq!(read, None);
        let brief = compare(
            &roles(),
            &m,
            &agent("sonnet"),
            op("brief.create"),
            &Map::new(),
            true,
            200,
        )
        .unwrap();
        assert_eq!(brief.account, "agent-executor");
        assert!(brief.refusal.is_some());
    }

    #[test]
    fn an_own_operation_acts_only_on_the_callers_account() {
        let m = members();
        let device = Caller::Device {
            account: "root".into(),
        };
        let own = op("device.show");
        assert_eq!(
            compare(&roles(), &m, &device, own, &Map::new(), true, 200),
            None
        );
        let mut args = Map::new();
        args.insert("account".into(), Value::from("bob"));
        let other = compare(&roles(), &m, &device, own, &args, true, 200).unwrap();
        assert!(other.refusal.is_some());
    }

    #[test]
    fn a_standard_device_differs_where_today_let_it_act_as_the_owner() {
        let mut m = members();
        m.memberships.push(super::super::Membership {
            account: "bob".into(),
            ..m.memberships[0].clone()
        });
        m.memberships.last_mut().unwrap().role = Role::Standard;
        let caller = Caller::Device {
            account: "bob".into(),
        };
        let got = compare(
            &roles(),
            &m,
            &caller,
            op("change.propose"),
            &Map::new(),
            true,
            200,
        )
        .unwrap();
        assert_eq!(got.account, "bob");
        assert!(got.today);
    }
}
