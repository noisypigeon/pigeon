//! `core::session::launch` against the repository's own files.

use std::path::PathBuf;

use agent::core::session::launch;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn the_repository_is_config_yamls() {
    let config = std::fs::read_to_string(repo_root().join("config.yaml")).expect("config.yaml");
    let want = config
        .lines()
        .find_map(|r| r.strip_prefix("repository:"))
        .map(str::trim)
        .expect("a repository line");
    assert_eq!(launch::repository(&config).as_deref(), Some(want));
}

#[test]
fn models_json_names_each_executor_class() {
    let text = std::fs::read_to_string(repo_root().join("tooling/session/models.json"))
        .expect("models.json");
    let models: serde_json::Value = serde_json::from_str(&text).expect("json");
    for class in ["haiku", "sonnet"] {
        assert!(
            models[class]
                .as_str()
                .is_some_and(|m| m.starts_with("claude-")),
            "{class}"
        );
    }
}
