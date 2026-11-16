//! §3's commands parse under their new spellings only; see
//! .claude/decisions/2026-11-16-development-src-crates-neutral-names.md §3.
#![cfg(all(
    feature = "eviction",
    feature = "infra",
    feature = "agent",
    feature = "signing"
))]

use clap::Parser as _;
use pigeon::cli::Cli;

/// Each old spelling, its new one, and the arguments the new one takes.
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
fn an_old_spelling_is_refused_and_its_new_one_parses() {
    for (old, new, args) in SPELLINGS {
        assert!(
            Cli::try_parse_from(argv(old, args)).is_err(),
            "`pigeon {old}` still parses"
        );
        Cli::try_parse_from(argv(new, args)).unwrap_or_else(|e| panic!("`pigeon {new}`: {e}"));
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
