//! The cobot image's tool versions equal mise.toml's; see
//! `.claude/decisions/2026-11-03-release-cobot-image-in-workflows.md`.

use std::fs;
use std::path::Path;

fn read(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every `ARG <name>=<value>` default in the Dockerfile.
fn defaults(dockerfile: &str, name: &str) -> Vec<String> {
    let prefix = format!("ARG {name}=");
    dockerfile
        .lines()
        .filter_map(|l| l.trim().strip_prefix(&prefix))
        .map(|v| v.trim().to_string())
        .collect()
}

/// The quoted version of `<tool> = "<version>"` in mise.toml.
fn mise(manifest: &str, tool: &str) -> String {
    manifest
        .lines()
        .find_map(|l| {
            let (key, value) = l.split_once('=')?;
            (key.trim() == tool).then(|| value.trim().trim_matches('"').to_string())
        })
        .unwrap_or_else(|| panic!("mise.toml has no {tool}"))
}

#[test]
fn image_tools_match_mise() {
    let dockerfile = read("../../Dockerfile");
    let toml = read("../../../../mise.toml");
    for (key, tool) in [
        ("TOFU_VERSION", "opentofu"),
        ("ZOLA_VERSION", "zola"),
        ("NODE_VERSION", "node"),
        ("WRANGLER_VERSION", "\"npm:wrangler\""),
        ("WORKER_BUILD_VERSION", "\"cargo:worker-build\""),
    ] {
        let want = mise(&toml, tool);
        let got = defaults(&dockerfile, key);
        assert!(!got.is_empty(), "Dockerfile has no ARG {key}");
        for v in got {
            assert_eq!(
                v, want,
                "Dockerfile ARG {key} differs from mise.toml's {tool}"
            );
        }
    }
}
