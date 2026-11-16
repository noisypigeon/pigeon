//! The old spellings of §3's commands parse, run the same code and print the new one on
//! stderr; see .claude/decisions/2026-11-16-development-src-crates-neutral-names.md §3.
#![cfg(all(
    feature = "eviction",
    feature = "infra",
    feature = "agent",
    feature = "signing"
))]

use assert_cmd::Command;
use clap::Parser as _;
use pigeon::cli::Cli;
use pigeon::commands::renamed;

/// Each old spelling, its new one, and the arguments both take.
const SPELLINGS: &[(&str, &str, &[&str])] = &[
    ("sentinel check", "eviction check", &[]),
    ("cobot validate", "infra validate", &[]),
    ("cobot render", "infra render", &["--state", "s"]),
    ("cobot plan", "infra plan", &["--state", "s", "--store"]),
    (
        "cobot apply",
        "infra apply",
        &["--state", "s", "--plan", "p"],
    ),
    ("cobot show", "infra show", &["--state", "s", "--plan", "p"]),
    ("cobot lock", "infra lock", &[]),
    ("cobot capabilities", "infra capabilities", &[]),
    (
        "cobot secrets set",
        "infra secrets set",
        &["infra/x", "KEY"],
    ),
    ("cobot secrets check", "infra secrets check", &["infra/x"]),
    ("cobot sites build", "sites build", &["--data", "a=b.json"]),
    ("cobot sites publish", "sites publish", &["--state", "s"]),
    ("cobot git apply", "git apply", &[]),
    ("cobot git status", "git status", &[]),
    ("cobot git push", "git push", &["pigeon", "HEAD:main"]),
    ("cobot git fetch", "git fetch", &["pigeon"]),
    ("cobot git clone", "git clone", &["pigeon", "dir"]),
    ("cobot git mirror", "git mirror", &[]),
    ("cobot git sync", "git sync", &["pigeon"]),
    ("cobot workers validate", "workers validate", &[]),
    ("cobot workers permissions", "workers permissions", &[]),
    ("cobot workers render", "workers render", &["--state", "s"]),
    ("cobot workers plan", "workers plan", &["--state", "s"]),
    (
        "cobot workers deploy",
        "workers deploy",
        &["--state", "s", "--plan", "p"],
    ),
    ("cobot workers status", "workers status", &[]),
    (
        "cobot workers secrets put",
        "workers secrets put",
        &["control", "KEY"],
    ),
    ("cobot workers teardown", "workers teardown", &["control"]),
    (
        "cobot workers outputs",
        "workers outputs",
        &["--state", "s"],
    ),
    ("cobot workers images push", "workers images push", &[]),
    ("cobot scope", "dev scope", &["--file", "a", "--json"]),
    ("cobot release", "release", &["--dry-run"]),
    ("cobot tags", "release tags", &[]),
    ("backplane instance", "instance", &["--json"]),
    ("backplane engines", "engines", &["--json"]),
    ("backplane operations", "ops", &[]),
    ("backplane run", "run", &["policy.check", "--text", "a=b"]),
    ("backplane decisions", "decisions", &["src/x.rs"]),
    (
        "backplane serve",
        "agent serve",
        &["--listen", "a", "--state", "s"],
    ),
    (
        "backplane jobs",
        "agent listen",
        &["--listen", "a", "--bench"],
    ),
    ("backplane totp-secret", "keys totp", &[]),
    ("backplane enroll-secret", "keys enroll", &[]),
    ("backplane vapid-key", "keys vapid", &[]),
    ("backplane signing load", "signing load", &[]),
    (
        "backplane signing upload",
        "signing upload",
        &["--ipa", "a.ipa", "--version", "1.0", "--build", "1"],
    ),
    ("backplane signing clean", "signing clean", &[]),
];

fn argv(words: &str, args: &[&str]) -> Vec<String> {
    std::iter::once("pigeon")
        .chain(words.split(' '))
        .chain(args.iter().copied())
        .map(String::from)
        .collect()
}

#[test]
fn every_old_spelling_parses_and_names_its_new_one() {
    for (old, new, args) in SPELLINGS {
        let parsed =
            Cli::try_parse_from(argv(old, args)).unwrap_or_else(|e| panic!("`pigeon {old}`: {e}"));
        assert_eq!(renamed(&parsed.command), Some((*old, *new)), "{old}");
    }
}

#[test]
fn every_new_spelling_parses_and_is_not_renamed() {
    for (_, new, args) in SPELLINGS {
        let parsed =
            Cli::try_parse_from(argv(new, args)).unwrap_or_else(|e| panic!("`pigeon {new}`: {e}"));
        assert_eq!(renamed(&parsed.command), None, "{new}");
    }
}

#[test]
fn every_old_command_has_a_spelling() {
    let help = |words: &str| {
        let out = Command::cargo_bin("pigeon")
            .expect("binary")
            .args(words.split(' '))
            .arg("--help")
            .output()
            .expect("run");
        assert!(out.status.success(), "{words} --help");
        String::from_utf8(out.stdout).expect("utf-8")
    };
    // Each subcommand an old group's help lists, and those of its nested groups.
    let commands = |text: &str| -> Vec<String> {
        text.split("Commands:")
            .nth(1)
            .and_then(|s| s.split("\n\n").next())
            .unwrap_or("")
            .lines()
            .filter_map(|l| l.split_whitespace().next())
            .filter(|c| *c != "help")
            .map(String::from)
            .collect()
    };
    let mut seen = Vec::new();
    let mut groups = vec!["sentinel".to_string(), "cobot".into(), "backplane".into()];
    while let Some(group) = groups.pop() {
        for c in commands(&help(&group)) {
            let words = format!("{group} {c}");
            if help(&words).contains("Commands:") {
                groups.push(words);
            } else {
                seen.push(words);
            }
        }
    }
    for words in seen {
        assert!(
            SPELLINGS.iter().any(|(old, _, _)| *old == words),
            "`pigeon {words}` has no new spelling"
        );
    }
}

fn run(words: &str, args: &[&str], dir: &std::path::Path) -> std::process::Output {
    Command::cargo_bin("pigeon")
        .expect("binary")
        .current_dir(dir)
        .args(words.split(' '))
        .args(args)
        .env_remove("BACKPLANE_TOKEN")
        .output()
        .expect("run")
}

#[test]
fn an_old_spelling_runs_the_same_command_after_one_line_on_stderr() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cases: &[(&str, &str, &[&str])] = &[
        ("backplane operations", "ops", &[]),
        ("backplane engines", "engines", &["--json"]),
        ("backplane instance", "instance", &[]),
        ("backplane run", "run", &["nope.nope"]),
        ("backplane decisions", "decisions", &["src/x.rs"]),
        (
            "backplane serve",
            "agent serve",
            &["--listen", "127.0.0.1:0", "--state", "s"],
        ),
        ("cobot validate", "infra validate", &[]),
        (
            "cobot scope",
            "dev scope",
            &["--file", "mise.toml", "--json"],
        ),
        ("cobot tags", "release tags", &[]),
        ("cobot workers validate", "workers validate", &[]),
    ];
    for (old, new, args) in cases {
        let a = run(old, args, dir.path());
        let b = run(new, args, dir.path());
        assert_eq!(a.status.code(), b.status.code(), "{old}");
        assert_eq!(a.stdout, b.stdout, "{old}");
        let a_err = String::from_utf8(a.stderr).expect("utf-8");
        let b_err = String::from_utf8(b.stderr).expect("utf-8");
        let (first, rest) = a_err.split_once('\n').expect("a notice");
        assert_eq!(
            first,
            format!("pigeon: `pigeon {old}` is now `pigeon {new}`")
        );
        assert_eq!(rest, b_err, "{old}");
        assert!(!b_err.contains("is now"), "{new}");
    }
}

#[test]
fn release_and_release_tags_are_one_group() {
    let parsed = Cli::try_parse_from(["pigeon", "release", "tags", "--root", "r"]).expect("tags");
    let pigeon::cli::Commands::Release(g) = parsed.command else {
        panic!("not release");
    };
    assert!(matches!(
        g.command,
        Some(pigeon::cli::ReleaseCommand::Tags(_))
    ));
    let parsed = Cli::try_parse_from(["pigeon", "release", "--to", "x"]).expect("release");
    let pigeon::cli::Commands::Release(g) = parsed.command else {
        panic!("not release");
    };
    assert!(g.command.is_none());
    assert_eq!(g.args.to, "x");
    assert!(Cli::try_parse_from(["pigeon", "release", "--dry-run", "tags"]).is_err());
}

#[test]
fn agent_listen_takes_its_address_either_way() {
    for args in [
        &["pigeon", "agent", "listen", "0.0.0.0:8080"][..],
        &["pigeon", "agent", "listen", "--listen", "0.0.0.0:8080"][..],
    ] {
        let parsed = Cli::try_parse_from(args).expect("parses");
        let pigeon::cli::Commands::Agent(pigeon::cli::AgentCommand::Listen(l)) = parsed.command
        else {
            panic!("not listen");
        };
        assert_eq!(l.into_jobs().listen, "0.0.0.0:8080");
    }
    assert!(Cli::try_parse_from(["pigeon", "agent", "listen"]).is_err());
    assert!(Cli::try_parse_from(["pigeon", "agent", "listen", "a", "--listen", "b"]).is_err());
}

#[test]
fn keys_print_a_secret_on_stdout_under_each_spelling() {
    let dir = tempfile::tempdir().expect("tempdir");
    for (old, new) in [
        ("backplane totp-secret", "keys totp"),
        ("backplane vapid-key", "keys vapid"),
        ("backplane enroll-secret", "keys enroll"),
    ] {
        let a = run(old, &[], dir.path());
        let b = run(new, &[], dir.path());
        assert!(a.status.success() && b.status.success(), "{old}");
        assert!(!a.stdout.is_empty() && !b.stdout.is_empty(), "{old}");
        assert_ne!(a.stdout, b.stdout, "{old}: each is new");
        assert!(String::from_utf8_lossy(&a.stderr).starts_with("pigeon: `pigeon backplane"));
    }
}
