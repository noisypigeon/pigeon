//! The backplane image's tool versions equal mise.toml's; see
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

/// Every `.config/<file>` a crate's source `include_str!`s.
fn included_config(dir: &std::path::Path, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("read_dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            included_config(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            let text = std::fs::read_to_string(&path).expect("read");
            for part in text.split("include_str!(\"").skip(1) {
                let lit = part.split('"').next().unwrap_or_default();
                if let Some((_, name)) = lit.split_once(".config/") {
                    out.push(name.to_string());
                }
            }
        }
    }
}

#[test]
fn the_image_build_sees_every_config_file_a_crate_includes() {
    let dockerfile = read("../../Dockerfile");
    assert!(
        dockerfile
            .lines()
            .any(|l| l.trim() == "COPY --from=config . /.config/"),
        "the build stage copies the whole `config` context, so no include_str! of .config/ misses"
    );
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut names = Vec::new();
    included_config(&crates, &mut names);
    assert!(names.iter().any(|n| n == "pool.yaml"), "{names:?}");
    let root = crates.join("../../../.config");
    for n in &names {
        assert!(
            root.join(n).is_file(),
            ".config/{n} is included but missing"
        );
    }
}

/// The recipe stage's `RUN`, as one shell command.
fn recipe_run(dockerfile: &str) -> String {
    let start = dockerfile
        .find("AS recipe")
        .and_then(|i| dockerfile[i..].find("RUN ").map(|j| i + j + 4))
        .expect("the recipe stage has a RUN");
    let block = &dockerfile[start..];
    let end = block.find("\n\n").unwrap_or(block.len());
    block[..end].replace("\\\n", " ")
}

/// Every target path a crate's manifest declares, and its default `src/lib.rs` and
/// `src/main.rs` when they exist; relative to the workspace.
fn declared_targets(workspace: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(workspace.join("crates"))
        .expect("crates")
        .flatten()
    {
        let dir = entry.path();
        let Ok(manifest) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
            continue;
        };
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        for default in ["src/lib.rs", "src/main.rs"] {
            if dir.join(default).is_file() {
                out.push(format!("crates/{name}/{default}"));
            }
        }
        for line in manifest.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("path") {
                let value = rest
                    .trim_start()
                    .trim_start_matches('=')
                    .trim()
                    .trim_matches('"');
                if std::path::Path::new(value)
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case("rs"))
                {
                    out.push(format!("crates/{name}/{value}"));
                }
            }
        }
    }
    out
}

#[test]
fn the_recipe_stubs_every_declared_target() {
    let dockerfile = read("../../Dockerfile");
    let workspace = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = tmp.path().join("recipe");
    let script = recipe_run(&dockerfile).replace("/recipe", out.to_str().expect("utf-8"));
    let ran = std::process::Command::new("sh")
        .arg("-c")
        .arg(&script)
        .current_dir(&workspace)
        .output()
        .expect("sh runs");
    assert!(
        ran.status.success(),
        "{}",
        String::from_utf8_lossy(&ran.stderr)
    );
    let targets = declared_targets(&workspace);
    assert!(
        targets.iter().any(|t| t.ends_with("tests/suite/main.rs")),
        "{targets:?}"
    );
    for t in &targets {
        assert!(out.join(t).is_file(), "the recipe doesn't stub {t}");
    }
}
