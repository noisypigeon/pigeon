## Repository areas

* Date: 2026-11-02
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `.config/policies/layout.yaml`, `infra/README.md`, `tooling/README.md`, `workloads/README.md`, `src/pigeon-cli/crates/sentinel/src/target.rs`, `src/pigeon-cli/crates/sentinel/tests/tree.rs`

## Context

pigeon's repository is going to hold more than the CLI. runway has shown the shape the rest takes:

- reusable tooling (garden and the library under runway's `tools/`);
- the infrastructure that bootstraps zones and providers;
- the workloads that run on that infrastructure.

Each of those deserves a top-level area of its own, decided before anything lands in it, so that nothing is placed by accident.

This ADR establishes four areas and moves the Rust workspace into the first one:

- **`src/`** holds self-contained projects, one directory each. `pigeon/` becomes `src/pigeon-cli/`. Its crates, the `pigeon` binary and `pigeon sentinel` keep their names.
- **`tooling/`** becomes the future home of what garden and the library were in runway.
- **`infra/`** holds break-glass access and the basic bootstrapping of zones and providers.
- **`workloads/`** holds workload definitions and the invoking of nodes.

This ADR sets the initial structure only. Later amendments fill each area, and each amendment decides its own content. Until then, an area holds a README saying what it is for.

The layout is enforced rather than merely documented. A new `.config/policies/layout.yaml` lists what the root and `src/` may hold, and `pigeon sentinel` checks it, following the generic policy engine decided in `.claude/decisions/2026-11-02-development-cargo-workspace-mise-toolchain.md` and its amendment. Checking a directory's entries is a new kind of target for that engine, so this ADR adds one generic target and one rule kind, and nothing specific to this repository.

This ADR supersedes the `pigeon/` paths in that ADR's 2026-11-02 amendment. That text stays as written.

Out of scope, each for a later amendment or ADR:

- what goes in `tooling/`, `infra/` and `workloads/`;
- a second project under `src/`;
- moving `.config/` or `.github/` into an area.

## Implementation plan.

### 1. Areas

| Path | Holds after this ADR | Filled by later amendments with |
| --- | --- | --- |
| `src/<project>/` | `src/pigeon-cli/`, moved from `pigeon/` | further projects |
| `tooling/` | `README.md` | what garden and the library were |
| `infra/` | `README.md` | break-glass access, and zone and provider bootstrapping |
| `workloads/` | `README.md` | workload definitions and node invocation |

- A project's directory name under `src/` is lowercase words joined by dashes.
- A project is self-contained: its build manifest, lockfile and build output stay inside it.
- Each area README is a few lines: what the area holds, and a link to this ADR. It holds nothing else until an amendment adds it.

### 2. Moving the workspace

- `git mv pigeon src/pigeon-cli`.
- `mise.toml`:
  - every Cargo task sets `dir = "src/pigeon-cli"`;
  - `sentinel` and `sentinel-pr` pass `--root ../..`.
- `.gitignore` holds `/src/pigeon-cli/target`.
- The tests that find the repository root from `CARGO_MANIFEST_DIR` walk up four levels instead of three:
  - `src/pigeon-cli/crates/sentinel/tests/policies.rs`;
  - `src/pigeon-cli/crates/pigeon/tests/sentinel.rs`.
- `CLAUDE.md` names `src/pigeon-cli/crates/<name>/`, and the four areas with a link to this ADR.

### 3. Sentinel: the `tree` target and the `entries` rule

- **Target `tree`.** A literal, like `pull-request`. It is the repository root seen as directories rather than documents, and it takes only `entries` rules.
- **Rule `entries`.** Its options:
  - `path`: the directory to list, relative to `--root`;
  - `dirs`: regexes for the subdirectories allowed;
  - `files`: regexes for the files allowed;
  - `require`: names that must exist;
  - `ignore`: regexes for entries that are skipped.

  The checks:
  - Each entry directly in `path` must match a pattern for its kind. An absent `dirs` or `files` allows none of that kind.
  - Each `require` name must exist.
  - Violations read `<path>: <policy>: unexpected directory '<name>'`, `unexpected file '<name>'`, or `missing '<name>'`. A `path` that cannot be listed is a violation too.
- **Code:**
  - `policy.rs`: the literal-target enum gains `tree`, and there is a new `EntriesRule`. Validation rejects `entries` on any other target, and any other rule on `tree`.
  - `target.rs`: dispatches `tree`.
  - `rules.rs`: adds `run_tree` and `entries`.
- **Tests:**
  - unit tests cover allow, require and ignore, with directories and files told apart;
  - `src/pigeon-cli/crates/sentinel/tests/policies.rs` runs the real `layout.yaml` against `tempfile` trees.

### 4. `.config/policies/layout.yaml`

```yaml
description: The repository root holds only the agreed areas, and src/ only projects.
target: tree
rules:
  - entries:
      path: .
      dirs: ['^\.claude$', '^\.config$', '^\.github$', '^src$', '^tooling$', '^infra$', '^workloads$']
      files: ['^\.gitignore$', '^CHANGELOG\.md$', '^CLAUDE\.md$', '^mise\.toml$']
      require: [src, tooling, infra, workloads]
      ignore: ['^\.git$']
  - entries:
      path: src
      dirs: ['^[a-z0-9]+(-[a-z0-9]+)*$']
      require: [pigeon-cli]
  - entries: { path: tooling, dirs: ['.'], files: ['.'], require: [README.md] }
  - entries: { path: infra, dirs: ['.'], files: ['.'], require: [README.md] }
  - entries: { path: workloads, dirs: ['.'], files: ['.'], require: [README.md] }
```

- A new top-level entry, or a file directly in `src/`, needs an amendment that changes this policy.
- The areas allow anything for now. Each later amendment tightens its own area's rule.

### 5. Execution

1. **PR A** holds this ADR and its changelog line. It is squash-merged once green.
2. **PR B**, titled `build: move pigeon/ to src/pigeon-cli and add tooling, infra and workloads`, is on a branch restarted from the new `main`.
   1. Make the steps 1–4 changes.
   2. Run `mise run ci` until clean.
   3. Push, add the changelog line, wait for `rust`, `sentinel` and `sentinel-pr` to go green, and squash-merge.
   4. Confirm the post-merge `push` runs are green.

### 6. Verification

- `mise run ci` passes, and `pigeon sentinel check` reports `3 policies hold, 1 skipped`.
- Each of these makes the `layout` policy fail with a named violation:
  - a stray root directory;
  - a stray root file;
  - a missing area;
  - a file in `src/`;
  - a project named outside the pattern;
  - a missing area README.
- The required checks stay `rust`, `sentinel` and `sentinel-pr`.

## Amendments

### 2026-11-04: catalog/, and infra/ for the platform alone

[`2026-11-04-development-platform-instance-separation.md`](2026-11-04-development-platform-instance-separation.md) adds a `catalog/` area for provider catalogs, which are product.
- `infra/` holds only the platform's own bootstrap in the pigeon account.
- `workloads/` leaves this repository for the instance's own repository in phase 7.
- `.config/policies/layout.yaml` follows each move in the PR that makes it.

The text above stays as written.

### 2026-11-04: tooling/spikes/

- `tooling/spikes/<name>/` holds a throwaway spike kit: code and a runbook that an owner deploys by hand to confirm what a design relies on.
- A spike is never a release unit, is outside the Cargo workspace, and holds no credential or account ID.
- Its results are recorded in the ADR that ordered it, and a later PR removes the kit.
- The first is `containers`, per the phase 3 amendment of [`2026-11-04-development-platform-instance-separation.md`](2026-11-04-development-platform-instance-separation.md).
