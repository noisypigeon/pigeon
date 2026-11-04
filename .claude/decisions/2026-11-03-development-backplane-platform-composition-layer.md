## Backplane, the composition layer of the pigeon platform

* Date: 2026-11-03
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development

## Context

pigeon is becoming a product: a platform that others could run. This repository holds two things that are no longer the same thing:

- **The product.** `src/pigeon-cli`, its crates and the images built from them.
- **One instance of the product.** This repository's `.config/` (policies, providers, releases), `infra/` and `workloads/`.

Today nothing names that split. Each engine is called on its own, by a subcommand or a workflow, and the paths that belong to the instance are fixed in the CLI and the workflows.

The engines already have roles, each decided by its own ADR:

- **`sentinel`** enforces policy on what actions produce, and reports deviations from it. See [`2026-11-02-development-sentinel-policy-cobot-actions.md`](2026-11-02-development-sentinel-policy-cobot-actions.md).
- **`cobot`** performs the actions: infrastructure changes, releases, changelogs and tags. See [`2026-11-02-runtime-cobot-infrastructure-as-code.md`](2026-11-02-runtime-cobot-infrastructure-as-code.md).
- **`gantry`** is a placeholder. Its purpose is left to a later amendment by [`2026-11-02-release-crate-units-and-images.md`](2026-11-02-release-crate-units-and-images.md).
  - The intent is that it will schedule work and run data operations.
  - This ADR notes that intent only. Gantry's own ADR decides it.

This ADR adds a fourth crate, **backplane**. It composes the engines into one platform, and it reads the instance it serves from a manifest. Backplane owns no policy and no action:

- it judges through sentinel;
- it acts through cobot;
- it will schedule through gantry.

It writes no file and starts no process itself. That is a third role beside sentinel's and cobot's. An amendment to the sentinel and cobot ADR records it, as that ADR's §1 asks of every new crate.

Backplane stays CLI-only, through `pigeon backplane`. Its operation catalog and JSON wire form are the seam that a web console, an MCP server or another surface can later call without reaching into the engines.

**Naming.**

- The crate lives in `src/pigeon-cli/crates/backplane/`.
- Its package is `pigeon-backplane`, and its library target is `backplane`.
- The package name carries the product's name because backplane is the product's public face.
- The other crates keep their names.

Out of scope, each for a later amendment or ADR:

- a web console, an MCP server, any HTTP API, and authentication for any of them;
- drift detection, and the channels that would deliver deviation alerts;
- `plan` and `apply` operations through backplane;
- gantry's purpose and work;
- more than one instance per repository, and instances outside this repository;
- moving the existing workflows onto backplane;
- publishing to crates.io;
- renaming the other packages.

## Implementation plan.

### 1. The crate

- `src/pigeon-cli/crates/backplane/Cargo.toml`:
  - `name = "pigeon-backplane"`, with its own `version = "0.1.0"` like the other release units;
  - `[lib] name = "backplane"`;
  - the workspace's `edition`, `rust-version`, `license`, `repository`, `publish` and `[lints]`;
  - dependencies: `sentinel`, `cobot`, `gantry`, `serde`, `serde-saphyr` and `serde_json`, all from the workspace.
- The workspace's `[workspace.dependencies]` gains `backplane = { path = "crates/backplane", package = "pigeon-backplane" }`.
- The library exposes:
  - `pub const VERSION: &str = env!("CARGO_PKG_VERSION")`;
  - an `Error` enum with `Io { path, message }`, `Invalid(Vec<String>)` and `Engine { engine, message }`, which carries a sentinel or cobot error as text.
- Its crate doc is one line, naming this ADR.

### 2. The instance manifest: `.config/instance.yaml`

```yaml
description: This repository as an instance of the pigeon platform.
schema: v0
name: noisypigeon
repository: https://github.com/noisypigeon/pigeon
paths:
  policies: .config/policies
  providers: .config/providers.yaml
  releases: .config/releases.yaml
```

- The manifest's location, `.config/instance.yaml` under the root, is fixed.
- `backplane::Instance::load(root)` reads and validates it:
  - unknown keys fail;
  - `schema` must be `v0`;
  - `name` is lowercase `[a-z0-9-]+`;
  - each path is relative, contains no `..`, and must exist under the root;
  - `paths.providers` is named `providers.yaml`, because cobot reads its allowlist and lock file from that file's directory.
- All problems are reported together, as `Error::Invalid`.
- `infra/` and `workloads/` are fixed by cobot's ADRs, so v0 does not list them. A later schema version may.

### 3. Engines

`backplane::engines()` returns one row per engine:

| Engine | Version | Role |
| --- | --- | --- |
| `sentinel` | `sentinel::VERSION` | `policy` |
| `cobot` | `cobot::VERSION` | `action` |
| `gantry` | `gantry::VERSION` | `planned` |

### 4. The operation catalog

- `backplane::Operation` is `{ name, engine, effect, summary }`:
  - `name` is dotted lowercase, `<area>.<verb>`;
  - `effect` is `read`, `plan` or `apply`. A surface can gate on it; for example, a future console may expose only `read`.
- `backplane::catalog()` returns the operations. In v0 they are all `read`:

  | Name | Engine | Does |
  | --- | --- | --- |
  | `instance.show` | backplane | the loaded manifest |
  | `policy.check` | sentinel | `sentinel::load_dir` on `paths.policies`, then `sentinel::check` on the root |
  | `infra.validate` | cobot | `cobot::validate(root, <directory of paths.providers>)`, returning each unit's path |

- `backplane::run(&Instance, &Request) -> Result<Response, Error>`:
  - `Request` is `{ operation, args }`. `args` is a JSON object, empty for every v0 operation;
  - `Response` is `{ operation, ok, result, violations }`, and it is serde-serializable. It is the wire form that every later surface reuses unchanged;
  - an unknown operation is `Error::Invalid`;
  - an operation whose `effect` is not `read` is refused until a later amendment adds one.

### 5. `pigeon backplane`

- In `crates/pigeon/Cargo.toml`:
  - a new feature, `backplane = ["dep:backplane", "dep:serde_json"]`;
  - `default` gains `backplane`.
- Subcommands:

  ```
  pigeon backplane instance   [--root DIR] [--json]
  pigeon backplane engines    [--json]
  pigeon backplane operations [--json]
  pigeon backplane run <OPERATION> [--root DIR] [--json]
  ```

  - `--root` defaults to `.`.
  - Text output is the default. `--json` prints the `Response`, or a JSON array for `engines` and `operations`.
  - Exit codes are those of `commands/mod.rs`:
    - `SUCCESS`;
    - `VIOLATION` when an operation reports violations;
    - `INVALID` for a bad manifest, an unknown operation or a usage error.
  - `pigeon backplane --version` prints backplane's version.
- Files:
  - `cli/backplane.rs` and `commands/backplane.rs`, each gated once on its `mod` line;
  - the `cfg` lists in `main.rs` and `commands/mod.rs` gain `backplane`.
- Tests:
  - `tests/backplane.rs` exercises each subcommand end to end;
  - `tests/cli.rs` asserts that `backplane` appears in the help only when the feature is built.

### 6. Release unit and image

- `.config/releases.yaml` gains a `backplane` unit, shaped like the others:
  - `manifest: src/pigeon-cli/crates/backplane/Cargo.toml`;
  - `features: backplane`;
  - `image: ghcr.io/noisypigeon/pigeon-backplane`;
  - `paths`:
    - the crate;
    - `commands/backplane.rs` and `cli/backplane.rs`;
    - the shared `pigeon` files every unit lists;
    - `Dockerfile` and `.dockerignore`;
  - `exclude: [tests/, benches/, '*.md']`;
  - `cargo: { workspace: src/pigeon-cli, package: pigeon }`.
- `tiers` gains `{ file: src/pigeon-cli/crates/backplane/CHANGELOG.md, unit: backplane }`. The bot creates that file; no PR writes it.
- Backplane depends on sentinel, cobot and gantry. Through the `cargo tree` rule, a change to any of them releases backplane too.
- `src/pigeon-cli/Dockerfile` gains `FROM cobot AS backplane`. Backplane calls cobot, which needs `tofu`, `zola` and `git`. The binary comes from the same `build` stage, built with `FEATURES=backplane`.
- In `.github/workflows/rust.yml`, the `image` job's matrix gains `backplane`.
- In `mise.toml`, the `features` task gains a backplane-only clippy line.

### 7. Enforcement

- A new tree policy, `.config/policies/backplane.yaml`, uses only the existing `find` rule kind, so it lands with the code:

  ```yaml
  description: Backplane composes; it writes no file and starts no process, per 2026-11-03-development-backplane-platform-composition-layer.md.
  target: tree
  rules:
    - find:
        match: '^src/pigeon-cli/crates/backplane/src/'
        forbid_content: 'std::process::Command|Command::new|fs::write'
  ```

  Tests that need fixture files write them from `crates/backplane/tests/`, which the rule does not cover.
- A new Mise task, `backplane`, runs `cargo run -q --locked -p pigeon -- backplane instance --root ../..`, and `ci` depends on it.
- Running the released `pigeon-backplane` image in `lifecycle.yml` waits for its first release, per [`2026-11-03-release-cobot-image-in-workflows.md`](2026-11-03-release-cobot-image-in-workflows.md).

### 8. Docs

`CLAUDE.md` gains:

- a pointer bullet: backplane composes the engines, reads `.config/instance.yaml`, and writes and starts nothing itself, citing this ADR;
- `backplane` in the list of release units;
- `pigeon backplane instance|engines|operations|run` under Commands.

### 9. Execution

1. **PR A**, `docs(adr): backplane, the platform's composition layer`. It holds this ADR, and the amendment to `2026-11-02-development-sentinel-policy-cobot-actions.md`.
2. **PR B**, `feat(backplane): pigeon-backplane crate, instance manifest and pigeon backplane`. It holds §1–§8 and is restarted from the new `main`. It edits no `CHANGELOG.md`.
   - Merging it cuts `backplane/v0.1.0` and publishes `pigeon-backplane`.
   - Its edits to shared `pigeon` files also bump `sentinel`, `gantry` and `cobot` once.
3. **PR C**, `ci: check the instance manifest with the pigeon-backplane image`. After that release, it adds a `lifecycle.yml` job that runs `backplane instance --root /repo` from `pigeon-backplane:latest`.

### 10. Verification

- `mise run ci` passes, including `features` and the new `backplane` task.
- A build with only `backplane` has the `backplane` subcommand and no `sentinel`, `gantry` or `cobot` subcommand.
- Tests cover:
  - the manifest:
    - a valid manifest, which loads;
    - an unknown key, a wrong `schema`, an absolute path, a `..` path and a missing path, which fail, all reported together;
  - the catalog:
    - its names are unique and well-formed;
    - a non-`read` operation is refused;
  - `run`:
    - `policy.check` against a fixture with a violated policy exits `VIOLATION`;
    - `infra.validate` against the real repository exits `SUCCESS`;
    - the `--json` `Response` has `operation`, `ok`, `result` and `violations`.
- Temporarily adding `Command::new` to a backplane source file makes `mise run sentinel` fail.
- After PR B merges, the release run tags `backplane/v0.1.0`, creates its GitHub Release, and pushes `ghcr.io/noisypigeon/pigeon-backplane`.

## Amendments

### 2026-11-04: Plan and apply, served from a Container

[`2026-11-04-development-platform-instance-separation.md`](2026-11-04-development-platform-instance-separation.md) extends this ADR:
- `plan` and `apply` operations join the catalog, and `apply` takes only a stored, unexpired plan ID;
- `pigeon backplane serve` answers the control plane inside a Cloudflare Container, in this ADR's JSON wire form;
- in phase 7, the platform's D1 instance records replace `.config/instance.yaml`.

The text above stays as written.
