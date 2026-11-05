//! Platform definitions invoked by an instance, then checked against a tree; see the
//! control-plane ADR's "A run's instance root, built from D1" amendment §2.

use std::fs;
use std::path::Path;

use sentinel::Invocation;
use tempfile::TempDir;

const ZONES: &str = "description: Workloads sit only under the instance's zones.\nparams:\n  zones: { type: list, description: Workload directories the instance owns. }\ntarget: tree\nrules:\n  - find:\n      match: '^workloads/[^/]+/config\\.yaml$'\n      allow_under: ${{ zones }}\n";

const README: &str = "description: The root has a README.\ntarget: { file: README.md }\nrules:\n  - contains: { file: README.md, text: '# ' }\n";

fn write(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
    fs::write(path, content).expect("write");
}

fn definitions() -> TempDir {
    let dir = TempDir::new().expect("definitions");
    write(dir.path(), "zones.yaml", ZONES);
    write(dir.path(), "readme.yaml", README);
    write(dir.path(), "notes.txt", "not a definition");
    dir
}

fn invocations(yaml: &str) -> Vec<Invocation> {
    serde_saphyr::from_str(yaml).expect("invocations")
}

#[test]
fn an_instance_invokes_definitions_by_name() {
    let dir = definitions();
    let loaded = sentinel::load_definitions(dir.path()).expect("loads");
    assert_eq!(
        loaded.keys().map(String::as_str).collect::<Vec<_>>(),
        ["readme", "zones"]
    );
    let policies = sentinel::invoke(
        &loaded,
        &invocations("- use: readme\n- use: zones\n  with: { zones: [workloads/a.com/] }\n"),
        Path::new(".config/instance.yaml"),
    )
    .expect("invokes");
    assert_eq!(
        policies.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        ["readme", "zones"]
    );

    let root = TempDir::new().expect("root");
    write(root.path(), "README.md", "# An instance\n");
    write(root.path(), "workloads/a.com/config.yaml", "zone: a.com\n");
    let report = sentinel::check(&policies, root.path(), None).expect("checks");
    assert!(report.violations.is_empty(), "{:?}", report.violations);

    write(root.path(), "workloads/b.org/config.yaml", "zone: b.org\n");
    let report = sentinel::check(&policies, root.path(), None).expect("checks");
    assert_eq!(report.violations.len(), 1, "{:?}", report.violations);
    assert_eq!(report.violations[0].policy, "zones");
    assert!(report.violations[0].path.contains("b.org"));
}

#[test]
fn an_unknown_name_or_a_refused_parameter_names_the_invocation() {
    let dir = definitions();
    let loaded = sentinel::load_definitions(dir.path()).expect("loads");
    for (yaml, want) in [
        ("- use: nope\n", "policy `nope`: no such definition"),
        (
            "- use: zones\n",
            "policy `zones`: parameter `zones` is required",
        ),
        (
            "- use: readme\n  with: { zones: [a] }\n",
            "policy `readme`: unknown parameter `zones`",
        ),
    ] {
        let err = sentinel::invoke(
            &loaded,
            &invocations(yaml),
            Path::new(".config/instance.yaml"),
        )
        .expect_err(yaml)
        .to_string();
        assert_eq!(err, format!(".config/instance.yaml: {want}"));
    }
}

#[test]
fn a_bad_definition_fails_the_load_at_its_file() {
    let dir = definitions();
    write(
        dir.path(),
        "broken.yaml",
        "description: x\ntarget: { file: f }\nrules:\n  - contains: { file: f, text: '${{ missing }}' }\n",
    );
    let err = sentinel::load_definitions(dir.path())
        .expect_err("broken")
        .to_string();
    assert!(
        err.contains("broken.yaml") && err.contains("names no parameter"),
        "{err}"
    );
}
