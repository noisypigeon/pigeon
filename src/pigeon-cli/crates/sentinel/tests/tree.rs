//! `entries` and `find` rules against throwaway trees.

use std::path::Path;

use sentinel::{LoadedPolicy, Policy};

fn run(rule: &str, root: &Path) -> Vec<String> {
    let yaml = format!("description: test.\ntarget: tree\nrules:\n  - {rule}\n");
    let loaded = LoadedPolicy {
        name: "t".into(),
        policy: Policy::parse(&yaml).expect("valid"),
    };
    sentinel::check(&[loaded], root, None)
        .expect("check runs")
        .violations
        .iter()
        .map(ToString::to_string)
        .collect()
}

mod tree_tests {
    use std::fs;

    use tempfile::TempDir;

    use super::run;

    fn run_entries(rule: &str, root: &TempDir) -> Vec<String> {
        run(&format!("entries: {rule}"), root.path())
    }

    fn tree() -> TempDir {
        let root = TempDir::new().expect("tempdir");
        fs::create_dir(root.path().join("a")).expect("mkdir");
        fs::create_dir(root.path().join(".git")).expect("mkdir");
        fs::write(root.path().join("f.md"), "").expect("write");
        root
    }

    #[test]
    fn allows_by_kind() {
        let root = tree();
        let rule = "{ path: ., dirs: ['^a$'], files: ['^f\\.md$'], ignore: ['^\\.git$'] }";
        assert_eq!(run_entries(rule, &root), Vec::<String>::new());
        let swapped = "{ path: ., dirs: ['^f\\.md$'], files: ['^a$'], ignore: ['^\\.git$'] }";
        assert_eq!(
            run_entries(swapped, &root),
            vec![
                ".: t: unexpected directory 'a'",
                ".: t: unexpected file 'f.md'"
            ]
        );
    }

    #[test]
    fn absent_lists_allow_nothing_and_ignore_skips() {
        let root = tree();
        assert_eq!(run_entries("{ path: . }", &root).len(), 3);
        assert_eq!(run_entries("{ path: ., ignore: ['.'] }", &root).len(), 0);
    }

    #[test]
    fn require_and_unlistable_paths() {
        let root = tree();
        assert_eq!(
            run_entries(
                "{ path: ., dirs: ['.'], files: ['.'], require: [a, b] }",
                &root
            ),
            vec![".: t: missing 'b'"]
        );
        assert_eq!(run_entries("{ path: nope }", &root).len(), 1);
    }
}

mod find_tests {
    use std::fs;

    use sentinel::Policy;
    use tempfile::TempDir;

    use super::run;

    fn run_find(rule: &str, root: &TempDir) -> Vec<String> {
        run(&format!("find: {rule}"), root.path())
    }

    fn tree(files: &[(&str, &str)]) -> TempDir {
        let root = TempDir::new().expect("tempdir");
        for (path, content) in files {
            let p = root.path().join(path);
            fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
            fs::write(p, content).expect("write");
        }
        root
    }

    #[test]
    fn allows_only_under_prefixes_and_skips_ignored() {
        let root = tree(&[
            ("infra/a/main.tf", ""),
            ("src/x/main.tf", ""),
            ("target/y.tf", ""),
            ("README.md", ""),
        ]);
        let rule = "{ match: '\\.tf$', allow_under: [infra/], ignore: ['^target$'] }";
        assert_eq!(
            run_find(rule, &root),
            ["src/x/main.tf: t: allowed only under infra/"]
        );
        let nowhere = "{ match: '\\.tf$', ignore: ['^target$'] }";
        assert_eq!(run_find(nowhere, &root).len(), 2);
    }

    #[test]
    fn forbid_content_fails_only_on_matching_lines() {
        let root = tree(&[
            (".github/workflows/a.yml", "name: a\nrun: tofu apply\n"),
            (".github/workflows/b.yml", "name: b\n"),
            (".github/workflows/ok.yml", "run: tofu plan\n"),
        ]);
        let rule = "{ match: '^\\.github/workflows/', allow_under: [.github/workflows/ok.yml], forbid_content: '\\btofu\\b' }";
        assert_eq!(
            run_find(rule, &root),
            [".github/workflows/a.yml:2: t: mentions `tofu`, which is not allowed here"]
        );
    }

    #[test]
    fn missing_root_and_unreadable_content_are_violations() {
        let missing = TempDir::new().expect("tempdir");
        let gone = missing.path().join("gone");
        let out = run("find: { match: x }", &gone);
        assert_eq!(out.len(), 1);
        assert!(out[0].contains("cannot be inspected"));

        let root = tree(&[(".github/workflows/bin.yml", "")]);
        fs::write(
            root.path().join(".github/workflows/bin.yml"),
            b"name: \xff\nrun: tofu apply\n",
        )
        .expect("write");
        let rule = "{ match: '^\\.github/', forbid_content: '\\btofu\\b' }";
        assert_eq!(
            run_find(rule, &root),
            [".github/workflows/bin.yml:2: t: mentions `tofu`, which is not allowed here"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_reported_not_followed() {
        let root = tree(&[("infra/main.tf", "")]);
        fs::create_dir_all(root.path().join("src")).expect("mkdir");
        std::os::unix::fs::symlink("../infra/main.tf", root.path().join("src/main.tf"))
            .expect("symlink");
        std::os::unix::fs::symlink("../infra", root.path().join("src/linked")).expect("symlink");
        let rule = "{ match: '\\.tf$', allow_under: [infra/] }";
        assert_eq!(
            run_find(rule, &root),
            ["src/main.tf: t: allowed only under infra/"]
        );
    }

    #[test]
    fn find_needs_the_tree_target() {
        let yaml = "description: x\ntarget: { file: f }\nrules:\n  - find: { match: x }\n";
        assert!(Policy::parse(yaml).expect_err("invalid").contains("`tree`"));
    }
}
