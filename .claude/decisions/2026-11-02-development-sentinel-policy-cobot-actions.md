## Sentinel is policy; cobot performs the actions policies govern

* Date: 2026-11-02
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `.config/policies/sentinel-actions.yaml`, `src/pigeon-cli/crates/sentinel/src/**`, `src/pigeon-cli/crates/backplane/src/cobot/release/**`, `src/pigeon-cli/crates/backplane/tests/cobot_release*.rs`, `src/pigeon-cli/crates/pigeon/src/commands/cobot.rs`

## Context

`sentinel` began as a policy engine: it loads `.config/policies/` and judges a repository against them. [`2026-11-02-release-crate-units-and-images.md`](2026-11-02-release-crate-units-and-images.md) then gave it the release machinery too. `pigeon sentinel release` runs `git` and `cargo tree`, bumps manifests, writes changelogs and notes, and `pigeon sentinel tags` lists tags for publication. So the engine that judges hand edits to changelogs is also the tool that writes them.

`cobot` already performs the repository's actions: it renders units, runs OpenTofu and writes encrypted state, per [`2026-11-02-runtime-cobot-infrastructure-as-code.md`](2026-11-02-runtime-cobot-infrastructure-as-code.md). Releasing is an action of the same kind: a merge happens, and machinery changes the repository in response.

Keeping the two apart has three benefits:

- **Sentinel stays small and read-only.** The image every workflow runs for `sentinel check` carries no code that can write to the tree or start a process.
- **A policy can be trusted.** A policy judges what an action produced; the action is never the judge of its own output.
- **One rule decides where new code goes**, instead of a judgement each time.

Today the split is clean. In `crates/sentinel/src`, only `release.rs` imports `changelog.rs` and `git.rs`. The three files share only `crate::Error`, and its `Git` and `Tool` variants exist only for them. `cobot` does not depend on `sentinel`. `.github/workflows/release.yml` is the only caller of `sentinel release` and `sentinel tags`. `lifecycle.yml` runs only `sentinel check`.

## Implementation plan.

### 1. The rule

For every crate and subcommand, now and later:

- **`sentinel` is policy and policy enforcement.** It reads the repository, a pull request's fields and its policies, and reports violations. It writes no file and starts no process.
- **`cobot` performs the actions and behaviours those policies govern.** Releases, changelogs, tags and infrastructure all belong here. It is anything that changes the repository, its history or the world outside it.
- A new capability that does both is split: the check goes in sentinel as a policy or rule kind, and the action goes in cobot.
- A new crate states which side it is on in the ADR that creates it. `gantry` is unaffected.

### 2. The move

- `release.rs`, `changelog.rs` and `git.rs` move from `crates/sentinel/src/` to `crates/cobot/src/release/`, as `release/mod.rs`, `release/changelog.rs` and `release/git.rs`.
  - They are exposed as `pub mod release` in cobot's `lib.rs`, which is the only change there.
  - The public API is unchanged apart from its path: `cobot::release::{Config, run, tags, Released, Outcome, TagStatus}`.
- `Error::Git` and `Error::Tool` move to `cobot::Error`, unchanged. Release code maps its I/O errors onto cobot's existing `Error::Io { path, message }`.
- `crates/sentinel/tests/release.rs` and `release_scope.rs` move to `crates/cobot/tests/`, importing `cobot::release`.
- `sentinel`'s `Cargo.toml` and `lib.rs` lose what only the moved code used. `cobot` gains any dependency the moved code needs that it lacks.
- In `pigeon`, `serde` and `serde_json` move from the `sentinel` feature to `cobot`, if only the release JSON used them.
- The move is pure: commits, versions, changelog text, notes and JSON are identical.

### 3. The subcommands

- `pigeon sentinel release` and `pigeon sentinel tags` become `pigeon cobot release` and `pigeon cobot tags`. Their flags, defaults, JSON and exit codes stay the same.
- The `Release`, `Tags`, `ReleaseArgs` and `TagsArgs` items move from `cli/sentinel.rs` and `commands/sentinel.rs` to `cli/cobot.rs` and `commands/cobot.rs`.
- The sentinel subcommands are removed, not aliased. `release.yml` is their only caller, and it changes in the same PR.
- `pigeon sentinel` keeps only `check`.

### 4. `.github/workflows/release.yml`

Both `cargo run … -- sentinel release` and `-- sentinel tags` become `-- cobot release` and `-- cobot tags`. Nothing else in the workflow changes. It still builds from source with default features, so it runs the code of the commit being released.

### 5. Release scopes

- `.config/releases.yaml` needs no path change. The cobot unit already owns `src/pigeon-cli/crates/cobot/`, `commands/cobot.rs` and `cli/cobot.rs`, and its `exclude` covers the moved tests.
- The sentinel unit keeps its paths. Deleting the code from `crates/sentinel/` changes its build, so the implementation's merge releases `sentinel` once, then never for release work again.
- The implementation's merge also releases `cobot`. Its bot commit is the first written by `pigeon cobot release`, because the workflow and the binary both come from the merged commit.

### 6. Enforcement

A new tree policy, `.config/policies/sentinel-actions.yaml`, uses only the existing `find` rule kind, so no engine change has to land and release first:

```yaml
description: Sentinel only judges; actions are cobot's, per the sentinel-policy-cobot-actions ADR.
target: tree
rules:
  - find:
      match: '^src/pigeon-cli/crates/sentinel/src/'
      forbid_content: 'std::process::Command|Command::new|fs::write'
  - find:
      match: '^\.github/workflows/.*\.ya?ml$'
      forbid_content: '(\bpigeon|--|IMAGE"?)\s+sentinel\s+([abd-z]|c[a-gi-z]|ch[a-df-z]|che[a-bd-z]|chec[a-jl-z])'
```

- **The first rule** fails on any line in sentinel's library that starts a process or writes a file.
  - `rules.rs`'s `entries` and `find` unit tests write fixtures with `fs::write`, so they move to `crates/sentinel/tests/` and run through the public `load_file` and `check`. Tests are not shipped, and the rule does not cover them.
- **The second rule** fails when a workflow invokes a `sentinel` subcommand other than `check`. It catches the `pigeon sentinel …`, `cargo run … -- sentinel …` and `"$IMAGE" sentinel …` forms on a single line.
  - The regex crate has no lookahead, so the pattern spells out "a word that does not start with `check`".
  - The invoker prefix keeps prose, such as "the pigeon-sentinel image", from matching.
- The policy lands in the implementation PR, after `release.yml` stops calling the old subcommands.
- If either pattern can't be expressed with `find` as it stands, that rule is dropped from the PR. The engine change then lands and releases first, and the rule follows in a later PR, per `CLAUDE.md`.

### 7. Docs

`CLAUDE.md`'s release bullets say:

- that the bot runs `pigeon cobot release` and `pigeon cobot tags`;
- that sentinel is policy and cobot performs actions, citing this ADR.

`2026-11-02-release-crate-units-and-images.md` gains a dated amendment pointing here. Its merged text stays as written.

### 8. Execution

1. **This PR**, `docs(adr): sentinel is policy, cobot performs actions`, holds this ADR, the amendment, and nothing else.
2. **The implementation PR**, `refactor(cobot): move release machinery from sentinel to cobot`, holds §2–§7.
   - It is restarted from the new `main`.
   - It touches only `crates/sentinel`, `crates/cobot/src/release/**` and `crates/cobot/tests/release*.rs`, the `pub mod` line in cobot's `lib.rs`, `pigeon`'s cli and commands files and `Cargo.toml`, `release.yml`, the new policy and `CLAUDE.md`.
   - It never edits a `CHANGELOG.md`.

### 9. Verification

- `cargo fmt --check`, clippy with `-D warnings` for all features and for each feature alone, `cargo test`, and `pigeon sentinel check` all pass.
- A build with only `sentinel` has no `release` or `tags` subcommand. A build with only `cobot` has both.
- In a throwaway clone with tags fetched from `origin`, `pigeon cobot release --root . --dry-run` prints the same JSON as `pigeon sentinel release --root . --dry-run` did at the parent commit.
- Temporarily adding `Command::new` to a sentinel source file, or `-- sentinel tags` to a workflow, makes `pigeon sentinel check` fail.
- The implementation merge's `chore(release):` commit comes from a `release` run whose log shows `cobot release`.

## Amendments

### 2026-11-03: A third role, composition

§1 asks every new crate to state its side. [`2026-11-03-development-backplane-platform-composition-layer.md`](2026-11-03-development-backplane-platform-composition-layer.md) adds `backplane`, which is on neither side:

- **`backplane` composes.**
  - It judges through sentinel.
  - It acts through cobot.
  - It writes no file and starts no process itself.
- A capability backplane offers is still split by §1: the check goes in sentinel and the action in cobot. Backplane only sequences and exposes them.
- `.config/policies/backplane.yaml` enforces this, as `sentinel-actions.yaml` does for sentinel.
