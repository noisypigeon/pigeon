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
