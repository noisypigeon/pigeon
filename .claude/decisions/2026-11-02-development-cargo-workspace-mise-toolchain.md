## Cargo workspace with a Mise-managed toolchain

* Date: 2026-11-02
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development

## Context

pigeon is going to be several Rust crates in one repository. The idiomatic shape for that is a single Cargo workspace:

- one virtual root manifest;
- members under `crates/`;
- one shared `Cargo.lock` and `target/`;
- metadata, dependencies and lints declared once at the root, and inherited by each member.

The toolchain needs a single pin that works the same on a laptop and in CI. Mise provides it. Mise installs Rust through rustup, and it also runs the repository's tasks, so `mise run ci` becomes the local gate before every PR. A `rust-toolchain.toml` would be a second pin that could drift from Mise's, so there isn't one.

The workspace starts with two crates:

- **`pigeon`**, the binary;
- **`sentinel`**, a library that `pigeon` depends on. It is a placeholder, and a later ADR decides its purpose.

Two crates with a dependency between them are enough to prove that inheritance, inter-crate dependencies, tests and lints all work end to end.

This ADR follows `.claude/decisions/2026-11-02-development-adr-driven-lifecycle.md`, including its 2026-11-02 amendment: terse comments, scripts under `.github/scripts/`, and the changelog. It lands as an ADR PR and then an implementation PR.

Out of scope, each for a later ADR:

- what `sentinel` is for;
- releases, versioning beyond `0.1.0`, and publishing to crates.io (every crate is `publish = false`);
- caching Cargo builds in CI;
- `cargo-deny`, `cargo-audit` and supply-chain policy, which belong in a `security` ADR;
- containers;
- a `rustfmt.toml` style beyond edition 2024's defaults.

## Implementation plan.

### 1. Toolchain: `mise.toml`

`mise.toml` at the repository root pins the toolchain:

```toml
min_version = "2026.10.12"

[tools]
rust = { version = "1.99.1", profile = "minimal", components = "clippy,rustfmt" }
```

- The Rust version is always exact, never `stable`. 1.99.1 is the newest stable release at least two weeks old on the date of this ADR.
- Changing the version is a PR that changes both `mise.toml` and `rust-version` in `Cargo.toml`.
- `min_version` is the Mise release this ADR was verified with: 2026.10.12, the newest at least two weeks old.

### 2. Tasks: `mise.toml`

| Task | Runs |
| --- | --- |
| `build` | `cargo build --workspace --locked` |
| `test` | `cargo test --workspace --locked` |
| `fmt` | `cargo fmt --all` |
| `fmt-check` | `cargo fmt --all -- --check` |
| `lint` | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| `rust` | depends on `fmt-check`, `lint` and `test` |
| `lifecycle` | `.github/scripts/check-decisions.sh`, `check-changelog.sh` and `test-checks.sh` |
| `ci` | depends on `rust` and `lifecycle`; the local gate before every PR |

Each task is a single line. The file's only comment names this ADR.

### 3. Workspace: root `Cargo.toml`

The root is a virtual manifest, with no root package:

```toml
[workspace]
resolver = "3"
members = ["crates/*"]

[workspace.package]
version = "0.1.0"
edition = "2024"
rust-version = "1.99.1"
license = "GPL-3.0-or-later"
repository = "https://github.com/noisypigeon/pigeon"
publish = false

[workspace.dependencies]
sentinel = { path = "crates/sentinel" }

[workspace.lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"

[workspace.lints.clippy]
pedantic = { level = "warn", priority = -1 }
```

- Every dependency used by more than one member, and every internal crate, is declared once in `[workspace.dependencies]`. Members name it with `x.workspace = true`.
- Warnings are errors in CI through `-D warnings`. Locally they stay warnings while iterating.
- Code opts out of a lint at the site, with `#[expect(lint, reason = "...")]`. Never in a manifest, and never with `#[allow]`.
- `Cargo.lock` is committed, because the workspace ships a binary.
- `.gitignore` holds `/target`.

### 4. Members

- Each crate is `crates/<name>/`, and its directory name is its package name.
- Its `Cargo.toml` does three things:
  - takes every `[workspace.package]` field with `.workspace = true`;
  - sets its own `description`;
  - opts into `[lints] workspace = true`.
- **`crates/sentinel`** is a library.
  - `src/lib.rs` has a `//!` crate doc and `pub fn version() -> &'static str`, which returns `env!("CARGO_PKG_VERSION")`.
  - A unit test asserts that `version()` is non-empty and equals the package version.
- **`crates/pigeon`** is a binary that depends on `sentinel.workspace = true`.
  - `src/main.rs` prints `pigeon <version> (sentinel <version>)`.
  - `tests/version.rs` runs the built binary through `env!("CARGO_BIN_EXE_pigeon")` and asserts that exact line.

Adding a crate takes three steps:

1. create `crates/<name>/` in this shape;
2. add it to `[workspace.dependencies]` if another member uses it;
3. land the ADR that decides its purpose.

### 5. CI: `.github/workflows/rust.yml`

- Triggers: `pull_request`, and `push` to `main`.
- `permissions: contents: read`.
- One job, `rust`, on `ubuntu-latest`:
  1. `actions/checkout@de0fac2e4500dabe0009e67214ff5f5447ce83dd` (v6.0.2), with `persist-credentials: false`;
  2. `jdx/mise-action@9b200bcecbfd6303e69a4a261b2e09e3757440a7` (v5.2.0), with `version: 2026.10.12`;
  3. `mise run rust`.
- `lifecycle.yml` is unchanged and stays dependency-free.

### 6. Docs

- `CLAUDE.md` gains:
  - a pointer to this ADR;
  - `mise run ci` as the gate before every PR;
  - the `mise run` tasks, in place of the raw script commands.
- Each PR adds its `CHANGELOG.md` bullet.

### 7. Execution

1. **PR A** holds this ADR and its changelog bullet. It is squash-merged once green.
2. **PR B**, titled `build: set up the Cargo workspace and Mise toolchain`, is on a branch restarted from the new `main`.
   1. Add steps 1–6.
   2. Run `cargo generate-lockfile`, then run `mise run ci` locally until clean.
   3. Push, add the changelog bullet, wait for `rust`, `decisions` and `pr` to go green, and squash-merge.
   4. Confirm the `push` runs on `main` are green.

### 8. Verification

- `mise exec -- rustc --version` reports 1.99.1.
- `mise run ci` passes locally, which includes:
  - rustfmt;
  - clippy with pedantic and `-D warnings`;
  - `sentinel`'s unit test and `pigeon`'s binary test;
  - the lifecycle checks.
- `cargo run -p pigeon` prints `pigeon 0.1.0 (sentinel 0.1.0)`.
- On GitHub, PR B's `rust`, `decisions` and `pr` checks are green, and so are the post-merge `push` runs.

## Amendments

### 2026-11-02: Workspace under pigeon/, pigeon sentinel and YAML policies

Three things change:

- **The Rust workspace moves out of the repository root.** The root then holds only repository-level concerns: decisions, CI, Mise and policies.
- **The lifecycle checks become `pigeon sentinel`.** The three bash scripts in `.github/scripts/` grew one bespoke rule at a time. They are replaced by a policy engine in the `sentinel` crate, run as `pigeon sentinel`.
- **The engine is generic.** It knows rule kinds, not this repository. What this repository requires lives in `.config/policies/`, one brief YAML file per subject, and CI runs `pigeon sentinel` to uphold them.

This amendment supersedes:

- the root-level paths in §3, §4 and §7 of this ADR;
- step 2 of the 2026-11-02 amendment to `.claude/decisions/2026-11-02-development-adr-driven-lifecycle.md`, which placed the checks in `.github/scripts/`.

Both texts stay as written.

#### 1. Layout

- `crates/`, `Cargo.toml` and `Cargo.lock` move to `pigeon/crates/`, `pigeon/Cargo.toml` and `pigeon/Cargo.lock`, and the build output to `pigeon/target/`.
- `.gitignore` holds `/pigeon/target`.
- `mise.toml` stays at the root, so there is still one toolchain for the repository. Every Cargo task sets `dir = "pigeon"`.

#### 2. Stack

`pigeon sentinel` follows pigeon-cli's CLI stack, which is a library/binary split:

- `main.rs` parses arguments and calls `dispatch`;
- `cli.rs` holds the `clap` derive types;
- `commands/<group>.rs` holds each group's logic.

Workspace dependencies, each at the newest release at least two weeks old when this is implemented:

| Crate | Use |
| --- | --- |
| `clap` (derive) | Arguments |
| `serde` (derive) | Policy types |
| `serde-saphyr` | YAML. `serde_yaml` is archived, and this is a pure-Rust replacement, in keeping with `unsafe_code = "forbid"` |
| `regex` | Patterns |
| `assert_cmd`, `predicates`, `tempfile` | Dev-dependencies, as in pigeon-cli's tests |

- Sentinel is synchronous and local, so pigeon-cli's `tokio`, `tracing` and metrics stack are not adopted.
- Errors are a `sentinel::Error` enum implementing `Display`. pigeon-cli uses neither `anyhow` nor `thiserror`, and neither is used here.

#### 3. The command

```
pigeon sentinel check [--root DIR] [--policies DIR] [--pull-request]
```

- `--root` defaults to the current directory, and `--policies` to `<root>/.config/policies`.
- Policies whose target is `pull-request` run only with `--pull-request`.
  - They read `PR_TITLE`, `PR_BODY` and `PR_NUMBER` from the environment, which CI sets through `env:`, never by interpolation.
  - A missing variable is a violation.
- Each violation prints as `path:line: <policy>: <message>`, where the policy is the file's stem. A one-line summary follows.
- Exit codes:
  - `0` when every policy holds;
  - `1` on any violation;
  - `2` on an invalid policy file or a usage error. Every policy is parsed and validated before any of them runs.
- Code:
  - `pigeon/crates/pigeon/src/{lib.rs, main.rs, cli.rs, commands/sentinel.rs}`;
  - `pigeon/crates/sentinel/src/{lib.rs, policy.rs, target.rs, markdown.rs, rules.rs, report.rs}`.
- `markdown.rs` gives the rules each line's number, a heading level and text where the line is a heading, and whether it is inside a fenced code block. Lines inside fences are never headings or items.

#### 4. The policy format

A policy is `.config/policies/<subject>.yaml`:

```yaml
description: <one line: what this policy guards>
target: <a target>
rules:
  - <kind>: { ...options }
```

**Targets:**

| Target | Checks |
| --- | --- |
| `{ file: PATH }` | The file at `PATH`, relative to `--root`. A missing file is a violation. |
| `{ dir: PATH, exclude: [NAME...], min: N }` | Every regular file directly in `PATH`, except `exclude`. Fewer than `N` files is a violation. |
| `pull-request` | The PR's `title`, `body` and `number`. |

**Variables.** Named regex captures (`(?<name>...)`) become variables. Each rule's string options can interpolate them as `{name}`, and a pull request's `title`, `body` and `number` are variables from the start. Variables are scoped to one file, or to the pull request.

**Rule kinds.** Each rule is a single-key map. An unknown kind or option is an invalid policy.

| Kind | Options | Checks |
| --- | --- | --- |
| `filename` | `pattern` | The file's basename matches. Captures become variables. |
| `line` | `at` (1-based), `matches`, `not` | That line matches `matches`, and does not match `not`. Captures become variables. |
| `fields` | `after`, `pattern`, `expect: [{ key, equals \| nonempty \| matches }]` | The non-blank lines after line `after` are `key`/`value` pairs, one per `expect` entry and in that order, each meeting its condition. |
| `headings` | `level`, `from`, `sequence: [{ text, optional }]` or `each`, `min`, `forbid_levels`, `order` | Outside fences, from line `from`: the level-`level` headings are exactly `sequence` (skipping missing optionals), or each one matches `each`. There are at least `min` of them, and no heading uses a level in `forbid_levels`. |
| `items` | `under: { level, text? }`, `heading: N` or `lines: true`, `matches`, `min`, `elsewhere: forbid`, `order`, `unique`, `equal`, `prefix` | In each section headed `under` (or every section at that level), each level-`N` heading, or each non-blank line, matches. Each section holds at least `min`. With `elsewhere: forbid`, a non-blank line outside every such section, after line 1, is a violation. |
| `field` | `name` (`title`, `body` or `number`), `matches` or `references`, `exclude`, `min` | The named field matches. With `references`, every match of the regex in it is a path that exists under `--root`, except `exclude`, and there are at least `min`. |
| `contains` | `file`, `text` | The file contains `text` literally, after interpolation. |

Constraints that `headings` (with `each`) and `items` share, applied to the captures of each match:

| Constraint | Meaning |
| --- | --- |
| `order: { by, direction: ascending \| descending, strict, from }` | The `by` values run in that direction, strictly if `strict`. If `from` is set, none is before it. |
| `unique: NAME` | No value of `NAME` repeats in the file. |
| `equal: [A, B]` | Where both are captured, `A` equals `B`. |
| `prefix: [A, B]` | Where both are captured, `A` is a prefix of `B`. |

Values compare as strings, so ISO dates order correctly.

#### 5. This repository's policies

They replace `.github/scripts/` rule for rule.

**`.config/policies/decisions.yaml`**

- `description`: "ADRs are named, shaped and dated as the lifecycle ADR decides."
- Target: `{ dir: .claude/decisions, exclude: [README.md], min: 1 }`.
- Rules:
  - `filename`, the lifecycle ADR's regex, capturing `date` and `category`;
  - `line` 1, matching `^## (?<title>.*\S.*)$` and not `^## Title$`;
  - `fields` after line 1, with the pattern `^\* (?<key>[A-Za-z]+):[ \t]*(?<value>.*?)\s*$`:
    - `Date` equals `{date}`;
    - `Model` is non-empty;
    - `Author` equals `Willow Graysen (@noisypigeon).`;
    - `Category` equals `{category}`;
  - `headings` at level 2, from line 2: the sequence `Context`, `Implementation plan.`, then an optional `Amendments`, with level 1 forbidden;
  - `items` under the level-2 `Amendments` section: level-3 headings match `^(?<when>\d{4}-\d{2}-\d{2}): \S`, with `min: 1` and `order: { by: when, direction: ascending, from: "{date}" }`.

**`.config/policies/changelog.yaml`**

- `description`: "CHANGELOG.md has dated sections, newest first, one linked line per PR."
- Target: `{ file: CHANGELOG.md }`.
- Rules:
  - `line` 1 matches `^# Changelog$`;
  - `headings` at level 2, each matching `^(?<day>\d{4}-\d{2}-\d{2})$`, with `min: 1` and `order: { by: day, direction: descending, strict: true }`;
  - `items` under every level-2 section, `lines: true`, with `min: 1`, `elsewhere: forbid`, `equal: [pr, pr_url]`, `prefix: [sha, sha_url]` and `unique: pr`. Each line matches one of two link shapes:
    - a PR link, `[#<pr>](https://github.com/noisypigeon/pigeon/pull/<pr_url>)`;
    - a commit link, `[<sha>](https://github.com/noisypigeon/pigeon/commit/<40-hex sha_url>)`.

**`.config/policies/pull-request.yaml`**

- `description`: "PRs have a Conventional Commit title, cite an ADR, and add their changelog line."
- Target: `pull-request`.
- Rules:
  - `field` `title` matches the lifecycle ADR's Conventional Commit regex;
  - `field` `number` matches `^[1-9][0-9]*$`;
  - `field` `body` has `references` to `\.claude/decisions/[A-Za-z0-9._-]+\.md`, with `exclude: [.claude/decisions/README.md]` and `min: 1`;
  - `contains`: `CHANGELOG.md` holds `([#{number}](https://github.com/noisypigeon/pigeon/pull/{number}))`.

#### 6. Tests

- **`sentinel` unit tests** cover:
  - each rule kind and constraint, passing and failing;
  - fence handling;
  - interpolation;
  - rejection of an invalid policy (an unknown kind, an unknown option, a bad regex).
- **`pigeon/crates/sentinel/tests/policies.rs`** loads the real `.config/policies/*.yaml` and ports every fixture in `.github/scripts/test-checks.sh` against `tempfile` trees. A policy that passes everything is a failing test.
- **`pigeon/crates/pigeon/tests/sentinel.rs`** uses `assert_cmd` to cover:
  - exit codes `0`, `1` and `2`;
  - the `path:line: policy: message` format;
  - `--pull-request` with and without its variables;
  - that `pigeon sentinel check` passes on this repository's own root.

#### 7. Tasks and CI

- Mise tasks:
  - `sentinel`: `cargo run -q --locked -p pigeon -- sentinel check --root ..`, with `dir = "pigeon"`;
  - `sentinel-pr`: the same with `--pull-request`;
  - `ci`: depends on `rust` and `sentinel`.
- `lifecycle` is removed.
- `.github/workflows/lifecycle.yml` keeps its triggers, `contents: read` and the pinned checkout. It adds `jdx/mise-action`, pinned as in `rust.yml`. Its jobs:
  - `sentinel`, on every trigger: `mise run sentinel`;
  - `sentinel-pr`, on `pull_request` only, with `PR_TITLE`, `PR_BODY` and `PR_NUMBER` set through `env:`: `mise run sentinel-pr`.
- The required checks become `rust`, `sentinel` and `sentinel-pr`, in place of `decisions` and `pr`.
- `.github/scripts/` is deleted.

#### 8. Docs

- `CLAUDE.md`'s commands name `mise run sentinel` and `pigeon sentinel check`.
- `.claude/decisions/README.md` says its rules are enforced by `.config/policies/decisions.yaml`.

#### 9. Execution

1. **PR A** holds this amendment and its changelog bullet. It is squash-merged once green.
2. **PR B**, titled `feat(sentinel): replace the lint scripts with pigeon sentinel and YAML policies`, is on a branch restarted from the new `main`.
   1. Make the steps 1–8 changes.
   2. Run `mise run ci` until clean.
   3. Push, add the changelog bullet, wait for `rust`, `sentinel` and `sentinel-pr` to go green, and squash-merge.
   4. Confirm the `push` runs on `main` are green.

#### 10. Verification

- `mise run ci` passes locally: fmt, clippy pedantic with `-D warnings`, every test (including the ported fixtures), and `pigeon sentinel check` on this repository.
- Breaking a decision, the changelog or a PR title by hand makes `pigeon sentinel check` exit `1`, naming `path:line`.
- A malformed policy exits `2`, naming the file.
- On GitHub, PR B's `rust`, `sentinel` and `sentinel-pr` checks are green, and so are the post-merge `push` runs.

### 2026-11-02: `rust.yml` runs only when Rust changed

§5 runs `rust.yml` on every pull request and every push to `main`, so a docs-only or `infra/` change pays for fmt, four clippy passes, the tests and two image builds. Its `rust` job is one of `main`'s required checks, which rules out the obvious fix: a workflow skipped by an `on: paths` filter never reports, and a required check that never reports blocks the merge forever. A job skipped by its own `if:` reports Skipped, which a required check accepts.

#### 1. A `changes` job decides

- It outputs `rust=true` when any changed path is a Rust input:
  - anything under `src/pigeon-cli/` except `*.md`;
  - `mise.toml`, which pins the toolchain and defines `mise run rust`;
  - `.github/workflows/rust.yml` itself.
- On `pull_request`, the changed paths are the PR's files from `gh api repos/<repo>/pulls/<n>/files --paginate`, written to a file first so a failed call fails the job rather than reading as "nothing changed".
- On `push`, they are `git diff --name-only <before> <after>`, from a full-depth checkout. A `before` of zeroes means `rust=true`.
- It needs `pull-requests: read` and nothing else.

#### 2. `rust` and `image` follow it

- Both take `needs: changes` and `if: needs.changes.outputs.rust == 'true'`.
- Job names are unchanged, so the required `rust` check keeps its name and reports Skipped when nothing Rust changed.
- Their steps are unchanged.

#### 3. Execution

1. This amendment lands first, with the release amendment of the same date.
2. Then `ci(rust): run only when Rust changed`, holding steps 1–2, in the same PR as the release implementation.

#### 4. Verification

- A docs-only PR shows `rust` and `image` as Skipped and is still mergeable.
- A PR touching `src/pigeon-cli/` runs both.
