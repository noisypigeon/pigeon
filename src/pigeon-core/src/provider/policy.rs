//! The broker's policy step: each forwarded call judged by `.config/policies/provider-call-*.yaml`; see .claude/decisions/2026-11-16-security-provider-calls-mcp-broker.md's 2026-11-17 step 5b amendment.

use crate::provider::registry::Effect;
use crate::provider::verdict::Call;

/// The provider-call policies, by name; a test keeps this list and the directory equal.
pub const POLICIES: &[(&str, &str)] = &[];

/// The call document §4 step 4 judges: server, tool, effect, caller, change and arguments.
#[must_use]
pub fn document(call: &Call, effect: Effect, caller: &str) -> eviction::Call {
    let arguments = serde_json::to_string(&call.arguments).unwrap_or_default();
    [
        ("server", call.server.clone()),
        ("tool", call.tool.clone()),
        ("effect", effect.name().to_string()),
        ("caller", caller.to_string()),
        (
            "change",
            call.change.map(|(n, _)| n.to_string()).unwrap_or_default(),
        ),
        ("arguments", arguments),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v))
    .collect()
}

/// The first policy that refuses `document`, with its first violation, or none.
///
/// # Errors
///
/// A policy in [`POLICIES`] that doesn't parse, as `(name, message)`.
pub fn judge(document: &eviction::Call) -> Result<Option<(String, String)>, (String, String)> {
    judge_with(POLICIES, document)
}

fn judge_with(
    policies: &[(&str, &str)],
    document: &eviction::Call,
) -> Result<Option<(String, String)>, (String, String)> {
    let mut loaded = Vec::with_capacity(policies.len());
    for (name, text) in policies {
        loaded.push(eviction::load_str(name, text).map_err(|m| ((*name).to_string(), m))?);
    }
    Ok(eviction::judge_call(&loaded, document)
        .into_iter()
        .find_map(|v| v.violations.first().map(|m| (v.policy.clone(), m.clone()))))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const MIRROR: &[(&str, &str)] = &[(
        "provider-call-mirror",
        r#"description: A GitHub write names only the mirror repository.
target: provider-call
when:
  - field: { name: server, equals: github }
  - field: { name: effect, equals: write }
rules:
  - field: { name: arguments, matches: '"repo":"noisypigeon/pigeon"' }
"#,
    )];

    fn call(server: &str, arguments: serde_json::Value) -> Call {
        Call {
            server: server.into(),
            tool: "create_issue".into(),
            arguments,
            change: Some((7, crate::provider::verdict::ChangeState::Open)),
        }
    }

    #[test]
    fn the_list_is_the_directory() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../.config/policies");
        let mut on_disk: Vec<String> = std::fs::read_dir(dir)
            .expect("policies")
            .filter_map(|e| {
                let path = e.ok()?.path();
                let yaml = path
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("yaml"));
                let stem = path.file_stem()?.to_str()?.to_string();
                (yaml && stem.starts_with("provider-call-")).then_some(stem)
            })
            .collect();
        on_disk.sort();
        let listed: Vec<String> = POLICIES.iter().map(|(n, _)| (*n).to_string()).collect();
        assert_eq!(listed, on_disk);
    }

    #[test]
    fn every_policy_parses() {
        let doc = document(&call("github", json!({})), Effect::Read, "a");
        assert!(judge(&doc).is_ok());
    }

    #[test]
    fn a_github_write_names_only_the_mirror() {
        let ok = document(
            &call("github", json!({"repo": "noisypigeon/pigeon"})),
            Effect::Write,
            "a",
        );
        assert_eq!(judge_with(MIRROR, &ok), Ok(None));
        let other = document(
            &call("github", json!({"repo": "someone/else"})),
            Effect::Write,
            "a",
        );
        let (policy, why) = judge_with(MIRROR, &other)
            .expect("parses")
            .expect("refused");
        assert_eq!(policy, "provider-call-mirror");
        assert!(!why.contains("someone/else"), "{why}");
    }

    #[test]
    fn reads_and_other_servers_are_not_the_mirrors() {
        let read = document(
            &call("github", json!({"repo": "someone/else"})),
            Effect::Read,
            "a",
        );
        assert_eq!(judge_with(MIRROR, &read), Ok(None));
        let other = document(
            &call("buildkite", json!({"repo": "someone/else"})),
            Effect::Write,
            "a",
        );
        assert_eq!(judge_with(MIRROR, &other), Ok(None));
    }

    #[test]
    fn the_document_carries_every_field() {
        let doc = document(
            &call("github", json!({"b": 1, "a": 2})),
            Effect::Write,
            "acct",
        );
        assert_eq!(doc["server"], "github");
        assert_eq!(doc["effect"], "write");
        assert_eq!(doc["caller"], "acct");
        assert_eq!(doc["change"], "7");
        assert_eq!(doc["arguments"], r#"{"a":2,"b":1}"#);
    }
}
