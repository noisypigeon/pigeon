//! The pure decision before a provider call goes out, and the document `policy.check` reads.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use super::registry::{Credential, Effect, Registry};

/// The state of the caller's ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ticket {
    /// Valid now.
    Live,
    /// None was presented.
    Missing,
    /// Past its expiry.
    Lapsed,
    /// Ended or revoked.
    Revoked,
}

/// The state of the change a call names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeState {
    /// Still open.
    Open,
    /// Evicted by the control plane.
    Evicted,
    /// Closed.
    Closed,
    /// Merged.
    Merged,
}

/// Who is calling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caller {
    /// The caller's account.
    pub account: String,
    /// The control plane calling itself: no ticket, holds every change.
    pub platform: bool,
    /// The capabilities the caller holds.
    pub capabilities: BTreeSet<String>,
    /// The caller's ticket state.
    pub ticket: Ticket,
    /// The changes the caller's session holds.
    pub holds: BTreeSet<u64>,
}

/// A call to a provider's tool.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    /// The registry server.
    pub server: String,
    /// The tool's name.
    pub tool: String,
    /// The tool's arguments.
    pub arguments: Value,
    /// The change the call is for, and its state.
    pub change: Option<(u64, ChangeState)>,
}

/// What approvals cover the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grants {
    /// A window, mandate, policy or change grant covers a write.
    pub write: bool,
    /// The owner's passkey proof for this call was verified.
    pub proof: bool,
}

/// The check that refused a call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The tool is not registered.
    Registry,
    /// The live schema differs from the pinned one.
    Drift,
    /// The caller lacks the capability.
    Capability,
    /// The ticket is not live.
    Ticket,
    /// The change is not open or not held.
    Change,
    /// An argument carries a credential.
    Arguments,
    /// The tier's approval is missing.
    Tier,
}

impl Step {
    /// The lowercase word for the step.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Registry => "registry",
            Self::Drift => "drift",
            Self::Capability => "capability",
            Self::Ticket => "ticket",
            Self::Change => "change",
            Self::Arguments => "arguments",
            Self::Tier => "tier",
        }
    }
}

/// What to do with a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Send it, with this credential and scope.
    Forward {
        /// The tool's effect.
        effect: Effect,
        /// The credential to attach.
        credential: Credential,
        /// The credential's scopes.
        scope: Vec<String>,
    },
    /// Hold it until an approval arrives.
    Pending {
        /// What is awaited.
        reason: String,
    },
    /// Refuse it.
    Refused {
        /// The check that failed.
        step: Step,
        /// Why.
        reason: String,
    },
}

const SECRET_MARKS: [&str; 9] = [
    "ghp_",
    "gho_",
    "ghs_",
    "ghu_",
    "github_pat_",
    "-----BEGIN ",
    "Bearer ",
    "xoxb-",
    "AKIA",
];

fn carries_credential(v: &Value) -> bool {
    match v {
        Value::String(s) => SECRET_MARKS.iter().any(|m| s.contains(m)),
        Value::Array(a) => a.iter().any(carries_credential),
        Value::Object(o) => o.values().any(carries_credential),
        _ => false,
    }
}

fn refused(step: Step, reason: String) -> Decision {
    Decision::Refused { step, reason }
}

/// The caller checks every brokered call makes: capability, ticket and change; the capacity pool's route runs them per model request.
///
/// # Errors
/// The first failing step and its reason.
pub fn admit(
    caller: &Caller,
    capability: &str,
    change: Option<(u64, ChangeState)>,
) -> Result<(), (Step, String)> {
    if !caller.platform && !caller.capabilities.contains(capability) {
        return Err((Step::Capability, format!("caller lacks `{capability}`")));
    }
    if !caller.platform {
        match caller.ticket {
            Ticket::Live => {}
            Ticket::Missing => return Err((Step::Ticket, "ticket is missing".into())),
            Ticket::Lapsed => return Err((Step::Ticket, "ticket has lapsed".into())),
            Ticket::Revoked => return Err((Step::Ticket, "ticket was revoked".into())),
        }
    }
    if let Some((n, state)) = change {
        let word = match state {
            ChangeState::Open => None,
            ChangeState::Evicted => Some("evicted"),
            ChangeState::Closed => Some("closed"),
            ChangeState::Merged => Some("merged"),
        };
        if let Some(w) = word {
            return Err((Step::Change, format!("!{n} is {w}")));
        }
        if !caller.platform && !caller.holds.contains(&n) {
            return Err((Step::Change, format!("!{n} is not this session's")));
        }
    }
    Ok(())
}

/// Decides a call, checking in order and stopping at the first refusal.
///
/// # Panics
/// Never for a registry `registry::parse` accepted.
#[must_use]
pub fn verdict(
    registry: &Registry,
    call: &Call,
    caller: &Caller,
    live_schema: Option<&str>,
    grants: Grants,
) -> Decision {
    let (server, tool) = (&call.server, &call.tool);
    let Some((srv, t)) = registry.tool(server, tool) else {
        return refused(
            Step::Registry,
            format!("tool `{server}/{tool}` is not in the registry"),
        );
    };
    if live_schema.is_some_and(|l| l != t.schema) {
        return refused(
            Step::Drift,
            format!("tool `{server}/{tool}`'s live schema changed; the registry pins another"),
        );
    }
    if let Err((step, reason)) = admit(caller, &t.capability, call.change) {
        return refused(step, reason);
    }
    if carries_credential(&call.arguments) {
        return refused(Step::Arguments, "an argument carries a credential".into());
    }
    let forward = || Decision::Forward {
        effect: t.effect,
        credential: Credential::parse(&srv.credential).expect("parse checked the credential"),
        scope: t.scope.clone(),
    };
    match t.effect {
        Effect::Read => forward(),
        Effect::Write if grants.write => forward(),
        Effect::Write => Decision::Pending {
            reason: "a write needs a window, mandate, policy or change grant".into(),
        },
        Effect::Apply if grants.proof => forward(),
        Effect::Apply => Decision::Pending {
            reason: "apply needs the owner's passkey proof".into(),
        },
    }
}

/// The call document `policy.check` reads.
#[must_use]
pub fn document(call: &Call, caller: &Caller, effect: Effect) -> Value {
    json!({
        "server": call.server,
        "tool": call.tool,
        "effect": effect.name(),
        "arguments": call.arguments,
        "caller": caller.account,
        "platform": caller.platform,
        "change": call.change.map(|(n, _)| n),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::registry::parse;

    const H: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn registry() -> Registry {
        let tool = |e: &str| {
            format!(
                "effect: {e}\n        capability: cap.{e}\n        scope: [s1]\n        schema: {H}\n"
            )
        };
        parse(&format!(
            "servers:\n  p:\n    url: https://p.example/mcp\n    credential: secret:P_TOKEN\n    tools:\n      r:\n        {}      w:\n        {}      a:\n        {}",
            tool("read"),
            tool("write"),
            tool("apply")
        ))
        .expect("test registry")
    }

    fn caller() -> Caller {
        Caller {
            account: "acct".into(),
            platform: false,
            capabilities: ["cap.read", "cap.write", "cap.apply"]
                .map(String::from)
                .into(),
            ticket: Ticket::Live,
            holds: [7].into(),
        }
    }

    fn call(tool: &str) -> Call {
        Call {
            server: "p".into(),
            tool: tool.into(),
            arguments: json!({"x": 1}),
            change: None,
        }
    }

    const NONE: Grants = Grants {
        write: false,
        proof: false,
    };

    fn step(d: Decision) -> (Step, String) {
        match d {
            Decision::Refused { step, reason } => (step, reason),
            other => panic!("not refused: {other:?}"),
        }
    }

    #[test]
    fn an_unregistered_tool_is_refused() {
        let (s, r) = step(verdict(&registry(), &call("nope"), &caller(), None, NONE));
        assert_eq!(s, Step::Registry);
        assert_eq!(r, "tool `p/nope` is not in the registry");
    }

    #[test]
    fn a_changed_schema_is_refused() {
        let (s, _) = step(verdict(&registry(), &call("r"), &caller(), Some("c"), NONE));
        assert_eq!(s, Step::Drift);
        assert!(matches!(
            verdict(&registry(), &call("r"), &caller(), Some(H), NONE),
            Decision::Forward { .. }
        ));
    }

    #[test]
    fn a_missing_capability_is_refused() {
        let mut c = caller();
        c.capabilities.clear();
        let (s, r) = step(verdict(&registry(), &call("r"), &c, None, NONE));
        assert_eq!(s, Step::Capability);
        assert_eq!(r, "caller lacks `cap.read`");
    }

    #[test]
    fn each_dead_ticket_is_refused() {
        for (t, want) in [
            (Ticket::Missing, "ticket is missing"),
            (Ticket::Lapsed, "ticket has lapsed"),
            (Ticket::Revoked, "ticket was revoked"),
        ] {
            let mut c = caller();
            c.ticket = t;
            let (s, r) = step(verdict(&registry(), &call("r"), &c, None, NONE));
            assert_eq!((s, r.as_str()), (Step::Ticket, want));
        }
    }

    #[test]
    fn a_change_that_is_not_open_is_refused() {
        for (st, want) in [
            (ChangeState::Evicted, "!7 is evicted"),
            (ChangeState::Closed, "!7 is closed"),
            (ChangeState::Merged, "!7 is merged"),
        ] {
            let mut k = call("r");
            k.change = Some((7, st));
            let (s, r) = step(verdict(&registry(), &k, &caller(), None, NONE));
            assert_eq!((s, r.as_str()), (Step::Change, want));
        }
    }

    #[test]
    fn a_change_not_held_is_refused() {
        let mut k = call("r");
        k.change = Some((8, ChangeState::Open));
        let (s, r) = step(verdict(&registry(), &k, &caller(), None, NONE));
        assert_eq!(s, Step::Change);
        assert_eq!(r, "!8 is not this session's");
    }

    #[test]
    fn a_nested_credential_is_refused_without_echoing_it() {
        let mut k = call("r");
        k.arguments = json!({"a": {"b": [1, {"c": "x ghp_abc123"}]}});
        let (s, r) = step(verdict(&registry(), &k, &caller(), None, NONE));
        assert_eq!(s, Step::Arguments);
        assert_eq!(r, "an argument carries a credential");
        assert!(!r.contains("ghp_"));
    }

    #[test]
    fn the_platform_needs_no_ticket_capability_or_hold() {
        let c = Caller {
            account: "platform".into(),
            platform: true,
            capabilities: BTreeSet::new(),
            ticket: Ticket::Missing,
            holds: BTreeSet::new(),
        };
        let mut k = call("r");
        k.change = Some((99, ChangeState::Open));
        assert!(matches!(
            verdict(&registry(), &k, &c, None, NONE),
            Decision::Forward { .. }
        ));
    }

    #[test]
    fn read_forwards_with_its_credential_and_scope() {
        assert_eq!(
            verdict(&registry(), &call("r"), &caller(), None, NONE),
            Decision::Forward {
                effect: Effect::Read,
                credential: Credential::Secret("P_TOKEN".into()),
                scope: vec!["s1".into()],
            }
        );
    }

    #[test]
    fn write_waits_without_a_grant_and_forwards_with_one() {
        let d = verdict(&registry(), &call("w"), &caller(), None, NONE);
        assert!(matches!(d, Decision::Pending { .. }));
        let g = Grants {
            write: true,
            proof: false,
        };
        assert!(matches!(
            verdict(&registry(), &call("w"), &caller(), None, g),
            Decision::Forward {
                effect: Effect::Write,
                ..
            }
        ));
    }

    #[test]
    fn apply_waits_without_a_proof_and_forwards_with_one() {
        let g = Grants {
            write: true,
            proof: false,
        };
        let d = verdict(&registry(), &call("a"), &caller(), None, g);
        assert!(matches!(d, Decision::Pending { .. }));
        let g = Grants {
            write: false,
            proof: true,
        };
        assert!(matches!(
            verdict(&registry(), &call("a"), &caller(), None, g),
            Decision::Forward {
                effect: Effect::Apply,
                ..
            }
        ));
    }

    #[test]
    fn checks_run_in_order() {
        let mut c = caller();
        c.capabilities.clear();
        c.ticket = Ticket::Revoked;
        let (s, _) = step(verdict(&registry(), &call("r"), &c, None, NONE));
        assert_eq!(s, Step::Capability);
        assert_eq!(s.name(), "capability");
    }

    #[test]
    fn the_document_names_the_call() {
        let mut k = call("w");
        k.change = Some((7, ChangeState::Open));
        assert_eq!(
            document(&k, &caller(), Effect::Write),
            json!({
                "server": "p", "tool": "w", "effect": "write",
                "arguments": {"x": 1}, "caller": "acct",
                "platform": false, "change": 7
            })
        );
        assert_eq!(
            document(&call("r"), &caller(), Effect::Read)["change"],
            Value::Null
        );
    }

    #[test]
    fn admit_refuses_as_verdict_does() {
        let mut c = caller();
        c.capabilities.clear();
        let want = step(verdict(&registry(), &call("r"), &c, None, NONE));
        assert_eq!(admit(&c, "cap.read", None), Err(want));

        let mut c = caller();
        c.ticket = Ticket::Lapsed;
        let want = step(verdict(&registry(), &call("r"), &c, None, NONE));
        assert_eq!(admit(&c, "cap.read", None), Err(want));

        let mut k = call("r");
        k.change = Some((7, ChangeState::Evicted));
        let want = step(verdict(&registry(), &k, &caller(), None, NONE));
        assert_eq!(admit(&caller(), "cap.read", k.change), Err(want));
    }

    #[test]
    fn admit_admits_a_live_caller_holding_its_change() {
        assert_eq!(
            admit(&caller(), "cap.read", Some((7, ChangeState::Open))),
            Ok(())
        );
    }
}
