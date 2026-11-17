//! [`super::capabilities`] and the manifest's `capabilities`; see
//! `.claude/decisions/2026-11-17-security-platform-run-minted-token.md` §1–§2.

use std::collections::BTreeMap;

use serde_json::json;

use super::capabilities;
use crate::infra::config::module_capabilities;

fn schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "cs": { "type": "array", "x-pigeon": { "module": "cloudflare/c", "for_each": "$item.name", "inputs": {} } }
        },
        "x-pigeon": {
            "singletons": { "a": { "module": "cloudflare/a", "inputs": {} } },
            "each": { "b": { "module": "cloudflare/b", "over": "bs", "for_each": "$item.name", "inputs": {} } }
        }
    })
}

fn manifests() -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "cloudflare/a".to_string(),
            "version: 0.1.0\ncapabilities: [x.read]\n".to_string(),
        ),
        (
            "cloudflare/b".to_string(),
            "version: 0.1.0\ncapabilities: [y.write]\n".to_string(),
        ),
        (
            "cloudflare/c".to_string(),
            "version: 0.1.0\ncapabilities: [z.write, x.read]\n".to_string(),
        ),
    ])
}

#[test]
fn a_unit_takes_its_singletons_its_non_empty_lists_and_the_state_bucket() {
    let config = json!({ "bs": [], "cs": [{ "name": "n" }] });
    assert_eq!(
        capabilities(&schema(), &config, &manifests()),
        Ok(vec![
            "r2.write".to_string(),
            "x.read".to_string(),
            "z.write".to_string()
        ])
    );
    let config = json!({ "bs": [{ "name": "m" }] });
    assert_eq!(
        capabilities(&schema(), &config, &manifests()),
        Ok(vec![
            "r2.write".to_string(),
            "x.read".to_string(),
            "y.write".to_string()
        ])
    );
}

#[test]
fn a_module_without_capabilities_or_a_manifest_is_refused() {
    let mut m = manifests();
    m.insert("cloudflare/a".into(), "version: 0.1.0\n".into());
    let why = capabilities(&schema(), &json!({}), &m).expect_err("refused");
    assert!(why.contains("declares no `capabilities`"), "{why}");
    m.remove("cloudflare/a");
    let why = capabilities(&schema(), &json!({}), &m).expect_err("refused");
    assert!(why.contains("manifest wasn't read"), "{why}");
}

#[test]
fn a_manifest_declares_a_list_or_nothing() {
    assert_eq!(module_capabilities("version: 0.1.0\n"), Ok(None));
    assert_eq!(
        module_capabilities("version: 0.1.0\ncapabilities: []\n"),
        Ok(Some(vec![]))
    );
    assert!(module_capabilities("version: 0.1.0\ncapabilities: oops\n").is_err());
}

// See `.claude/decisions/2026-11-17-security-platform-run-minted-token.md` §4–§6.
#[test]
fn a_minted_run_records_its_token_and_is_revoked_once_it_ends() {
    use super::{Credential, Minted, Runs, Stage, finish_event};
    let runs = Runs::default();
    let (runs, _) = runs.propose("platform/blue", "g1", 0).expect("proposes");
    let (runs, _) = runs.grant_plan(1, "g1", "SEALED", "o", 1).expect("grants");
    let minted = Minted {
        token_id: "t1".into(),
        capabilities: vec!["d1.write".into()],
        expires: 86_401,
        revoked: false,
    };
    let (runs, run) = runs
        .credit(1, Credential::Minted, Some(minted))
        .expect("credits");
    assert_eq!(run.credential, Some(Credential::Minted));
    assert!(
        runs.to_revoke().is_empty(),
        "a planning run keeps its token"
    );
    let (runs, run) = runs.planned(1, Err("boom".into()), "t2", 2).expect("fails");
    assert_eq!(run.stage, Stage::Failed);
    assert_eq!(runs.to_revoke(), vec![(1, "t1".to_string())]);
    let runs = runs.revoked(1);
    assert!(runs.to_revoke().is_empty(), "revoked once");
    assert!(runs.credit(1, Credential::Pasted, None).is_err());
    let shown = serde_json::to_string(&runs.get(1).expect("run").shown()).expect("json");
    assert!(shown.contains("\"token_id\":\"t1\"") && !shown.contains("SEALED"));
    let (runs, _) = Runs::default()
        .propose("platform/blue", "g", 0)
        .expect("proposes");
    let (runs, _) = runs.grant_plan(1, "g", "S", "o", 1).expect("grants");
    let (runs, _) = runs.credit(1, Credential::Pasted, None).expect("credits");
    let planned = super::Planned {
        plan: "p".into(),
        sha: "2222222222222222222222222222222222222222".into(),
        changes: vec![],
    };
    let (_, run) = runs.planned(1, Ok(planned), "t", 2).expect("plans");
    let (_, said) = finish_event(&run).expect("an event");
    assert!(said.contains("with a pasted token"), "{said}");
}
