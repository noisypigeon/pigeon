//! Stored state that the version before and the version after both read; see
//! `.claude/decisions/2026-11-08-runtime-blue-green-self-standup.md` §1.
//!
//! `tests/state/<type>/<date>.json` holds each stored type as a release wrote it: every one
//! decodes, the newest round-trips, and each decodes with fields from a later release.

use std::path::{Path, PathBuf};

use backplane::approve::Waiting;
use backplane::changes::{Change, Event};
use backplane::control::{JobSpec, Pipeline, Report};
use backplane::grants::Grant;
use backplane::mandates::Mandates;
use backplane::mirroring::Mirroring;
use backplane::runs::Queue;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// A field no release has written.
const LATER: &str = "from_a_later_release";

fn fixtures(name: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/state")
        .join(name);
    let mut all: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    all.sort();
    assert!(!all.is_empty(), "{name} has no fixture");
    all
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).expect("reads")).expect("is JSON")
}

/// Maps by key, not structs: a later release adds entries to them, never fields.
fn is_map(key: &str, m: &serde_json::Map<String, Value>) -> bool {
    matches!(key, "grants" | "env") || (key == "images" && !m.contains_key("source"))
}

/// Adds [`LATER`] to every struct in `v`, found under `key`.
fn later(key: &str, v: &mut Value) {
    match v {
        Value::Object(m) => {
            let map = is_map(key, m);
            for (k, child) in m.iter_mut() {
                later(if map { "" } else { k }, child);
            }
            if !map {
                m.insert(LATER.into(), Value::Bool(true));
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|c| later(key, c)),
        _ => {}
    }
}

fn check<T: Serialize + DeserializeOwned>(name: &str) {
    let all = fixtures(name);
    for path in &all {
        serde_json::from_value::<T>(read(path))
            .unwrap_or_else(|e| panic!("{} no longer decodes: {e}", path.display()));
    }
    let newest = all.last().expect("one");
    let value = read(newest);
    let again = serde_json::to_value(serde_json::from_value::<T>(value.clone()).expect("decodes"))
        .expect("encodes");
    assert_eq!(
        again,
        value,
        "{} doesn't round-trip: a field was added, renamed or dropped. Add a new fixture beside it, \
         as this release writes it, and keep the old one",
        newest.display()
    );
    let mut ahead = value.clone();
    later("", &mut ahead);
    let read_back = serde_json::from_value::<T>(ahead).unwrap_or_else(|e| {
        panic!(
            "{} with a later release's fields doesn't decode: {e}",
            newest.display()
        )
    });
    assert_eq!(
        serde_json::to_value(read_back).expect("encodes"),
        value,
        "{}",
        newest.display()
    );
}

#[test]
fn a_change_and_its_events() {
    check::<Change>("change");
    check::<Event>("event");
}

#[test]
fn grants_the_inbox_and_mandates() {
    check::<Grant>("grant");
    check::<Vec<Waiting>>("waiting");
    check::<Mandates>("mandates");
}

#[test]
fn the_pipeline_and_what_jobs_exchange() {
    check::<Pipeline>("pipeline");
    check::<Report>("report");
    check::<JobSpec>("job_spec");
}

#[test]
fn the_mirror() {
    check::<Mirroring>("mirroring");
}

#[test]
fn the_run_queue() {
    check::<Queue>("queue");
}

#[test]
fn every_fixture_directory_is_checked() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/state");
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("reads")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "change",
            "event",
            "grant",
            "job_spec",
            "mandates",
            "mirroring",
            "pipeline",
            "queue",
            "report",
            "waiting"
        ]
    );
}
