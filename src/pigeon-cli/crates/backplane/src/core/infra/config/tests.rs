//! A config unit validated and rendered from a `Disk` snapshot, with no files.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::json;

use super::{Disk, Module, ModuleInfo, from_kind, render, validate};
use crate::cobot::generations::Day;
use crate::cobot::providers::Allowlist;

fn unit() -> super::ConfigUnit {
    let schema = json!({
        "type": "object",
        "x-cobot": { "singletons": { "zone": { "module": "zone", "inputs": {} } } }
    });
    from_kind(json!({}), schema, Path::new("/unit"), None)
}

fn module(inside: bool, manifest: &str) -> Module {
    Module {
        info: ModuleInfo {
            resources: vec!["cloudflare_zone".into()],
            ..ModuleInfo::default()
        },
        manifest: manifest.into(),
        inside,
        source: "/unit/modules/zone".into(),
        display: "modules/zone".into(),
    }
}

fn problems(disk: &Disk) -> Vec<String> {
    validate(
        &unit(),
        disk,
        "infra",
        &Allowlist::default(),
        &BTreeSet::new(),
        &[],
    )
    .into_iter()
    .map(|(_, m)| m)
    .collect()
}

#[test]
fn a_module_is_judged_by_what_the_shell_found() {
    let disk = |m: Module| Disk {
        modules: BTreeMap::from([("zone".to_string(), m)]),
        ..Disk::default()
    };
    let found = problems(&disk(module(true, "version: 1.0.0\n")));
    assert!(!found.iter().any(|m| m.contains("module")), "{found:?}");
    let missing = problems(&disk(module(false, "")));
    assert!(
        missing
            .iter()
            .any(|m| m.contains("names missing module `zone`")),
        "{missing:?}"
    );
    let unversioned = problems(&disk(module(true, "")));
    assert!(
        unversioned.iter().any(|m| m.contains("version: X.Y.Z")),
        "{unversioned:?}"
    );
}

#[test]
fn a_secrets_file_is_judged_by_what_the_shell_read() {
    let disk = |secrets| Disk {
        secrets,
        ..Disk::default()
    };
    let unreadable = problems(&disk(Some(Err("secrets.config.yaml: gone".into()))));
    assert!(
        unreadable.iter().any(|m| m.contains("gone")),
        "{unreadable:?}"
    );
    let stray = problems(&disk(Some(Ok(BTreeMap::from([(
        "token".to_string(),
        "plain".to_string(),
    )])))));
    assert!(
        stray
            .iter()
            .any(|m| m.contains("differ from x-cobot.secrets")),
        "{stray:?}"
    );
    assert!(stray.iter().any(|m| m.contains("is not ENC")), "{stray:?}");
}

#[test]
fn a_render_takes_its_module_source_from_the_snapshot() {
    let disk = Disk {
        modules: BTreeMap::from([("zone".to_string(), module(true, "version: 1.0.0\n"))]),
        ..Disk::default()
    };
    let tf = render(
        &unit(),
        &disk,
        "/state/terraform.tfstate",
        &Allowlist::default(),
        Day::default(),
        &BTreeMap::new(),
    );
    assert_eq!(tf["module"]["zone"]["source"], "/unit/modules/zone");
    assert_eq!(
        tf["terraform"]["backend"]["local"]["path"],
        "/state/terraform.tfstate"
    );
}
