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
