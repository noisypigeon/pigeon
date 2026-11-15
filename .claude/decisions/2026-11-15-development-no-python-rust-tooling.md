## No Python: the repository's tooling is Rust (`pigeon …`) or POSIX sh

* Date: 2026-11-15
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `tooling/session/session.sh`, `tooling/session/models.json`, `tooling/session/*.py`, `src/pigeon-cli/crates/backplane/src/core/session/*.rs`, `src/pigeon-cli/crates/pigeon/src/cli/session.rs`, `src/pigeon-cli/crates/pigeon/src/commands/session.rs`

## Context

The owner asked for every line of Python in the repository to be rewritten in
Rust, and for a rule that no Python comes back.

### What is Python today (`artifacts/main`, 2026-11-15)

- **Files:** `tooling/session/session.py` and its tests,
  `tooling/session/test_session.py` (7 stdlib `unittest` tests, run by hand,
  in no CI).
- **Invocations:** only `tooling/session/session.sh`, which runs
  `session.py install` before `brief`, and `session.py <command>` when no
  `pigeon` binary works.
- **Text that names it:** `CLAUDE.md` (a planner runs
  `python3 -I tooling/session/session.py executor b<n>`), and merged ADRs'
  history, which stays as written.
- **Nothing else:** no `.py` or Python shebang elsewhere, and none in
  `mise.toml`, `.github/workflows/`, the Dockerfiles or `pigeon-start`.
  `.claude/settings.json`'s hooks and status line run `session.sh`.

### What `pigeon session` already has

[`2026-11-13-development-rust-session-event-follower.md`](2026-11-13-development-rust-session-event-follower.md)
moved `follow`, `brief`, `statusline` and `stop` (with `label`, `get`, `run`,
`status`, `cached` and `line` beneath them) to `pigeon session`. What is left
in Python:

| `session.py` | Where it goes |
|---|---|
| `executor b<n> [--tag T]…` and its `EXECUTOR` prompt | `pigeon session executor` (§1) |
| `repository` | `core::session::repository`, read by `executor` (§1) |
| `install` | `pigeon session install`, with a sh bootstrap (§2) |
| `follow`, `brief`, `statusline`, `stop`, `START` | already `pigeon session`'s |

### Why the bootstrap is the hard part

`session.py` is stdlib-only, so it runs in a fresh cloud container before any
Rust build exists, and it fetches the first `pigeon`. A Rust-only path needs
a first binary from somewhere, inside the `SessionStart` hook's 20 s, with no
build there: a cold `cargo build` of `pigeon` takes minutes.

## Implementation plan.

### 1. `pigeon session executor b<n> [--tag TAG]…`

- **The same arguments as `session.py executor`:** it reads the brief with
  `brief.show` and prints `create_session`'s arguments as JSON, indented by
  2, keys in this order: `model`, `title`, `prompt`, `source_url`, `tags`.
  A planner parses it, so its values match `session.py`'s exactly.
- **The prompt changes in one place:** step 2 arms `Monitor` with
  `sh tooling/session/session.sh follow`, not `session.py follow`.
- **Same refusals and exit codes:** `2` for usage (clap's own), `1` for no
  `repository`, an unknown brief, or a class no executor carries out; it
  prints nothing on stdout then and asks nothing when `repository` is
  missing.
- **The core decides, the shell reads.** `core::session::executor` takes the
  brief's JSON, `models.json`'s map, `config.yaml`'s `repository` and the
  extra tags, and answers the arguments or a refusal. The shell finds the
  repository root as the nearest ancestor of the working directory holding
  `tooling/session/models.json`, reads both files, and calls `brief.show`.
- **`tooling/session/models.json` stays** the one place model IDs live,
  read at run time, so changing an ID needs no build.

### 2. `pigeon session install`, and a sh bootstrap

- **`pigeon session install`** does what `session.py install` does: reads
  `deploy.status`'s `latest_build`, does nothing when
  `~/.cache/pigeon/bin/pigeon.sha` already names its `sha` and the binary is
  executable, else fetches `/builds/<sha>/pigeon`, checks its SHA-256, and
  renames it into `~/.cache/pigeon/bin/pigeon` with `pigeon.sha` beside it.
  The check is `core::session::verified`, pure.
- **The bootstrap is POSIX sh in `session.sh`,** for a container with no
  working binary yet: `curl` posts `deploy.status`, `sed` takes
  `latest_build`'s `sha` (40 hex) and `sha256` (64 hex) from its one flat
  object, `curl -f` fetches the build, and `sha256sum`, `shasum -a 256` or
  `openssl dgst -sha256` checks it before `mv`. Anything else, including a
  value that isn't hex of the right length, fetches nothing.
- **`session.sh brief` updates first, bounded to 10 s:** with a working
  binary, `pigeon session install`; if that fails (a build from before this
  step has no `install`), the sh bootstrap. So each start takes `main`'s
  newest build, as now.
- **Rejected:** a `cargo build` in the hook (minutes, not 20 s); a release
  asset on GitHub (a mirror, often behind, and not where builds land); a
  committed binary (large, and a build product in source).
- **With no binary at all** (the control plane unreachable on a first
  start):
  - the hooks degrade and exit 0: `brief` prints that there is no `pigeon`
    yet, and how to get one; `statusline` prints
    `pigeon: no pigeon binary yet`; `stop` prints nothing;
  - `follow`, `executor` and `install`, which aren't hooks, build and run
    `pigeon` from the checkout with `cargo run -q --release -p pigeon`, when
    `cargo` is there, else print why and exit 1.
- **Trust is unchanged:** the build is the one job Containers run, served
  behind Access, and checked against the control plane's SHA-256, per the
  Rust-follower ADR §4.

### 3. Tests move to Rust

- `test_session.py`'s seven tests become `core::session` unit tests, run by
  `cargo test --workspace`, so by `mise run test`, `mise run ci` and the code
  checks `change.scope` selects for `src/pigeon-cli/`:
  1. every argument `create_session` needs, and only `brief.show` asked;
  2. the prompt's check-first step comes before step 1, names the planner's
     session ID, forbids `add_repo`, and names the revision and planner;
  3. `--tag` adds each tag once, after the brief's;
  4. a brief without an ADR still has its brief tag;
  5. no `repository`: nothing printed, nothing asked;
  6. the refusals (here the core's; usage is clap's);
  7. `repository` reads the real `config.yaml` (an integration test in
     `backplane/tests/`, from the repository root).
- `verified` and the sh bootstrap's parsing get a test each: a known
  digest, and `latest_build` read from a recorded `deploy.status` answer.

### 4. Enforcement

- **`.config/policies/no-python.yaml`,** a `tree` policy of `find` rules,
  ignoring `.git`, `target` and `.cobot`, failing with its description,
  "No Python: tooling is Rust (`pigeon …`) or POSIX sh; see this ADR":
  - any `*.py`, `*.pyi`, `*.pyw`, `*.ipynb`, `pyproject.toml`, `setup.cfg`,
    `requirements*.txt`, `Pipfile`, `uv.lock` or `poetry.lock`;
  - any file with a `#!…python` line;
  - `python`, `python3` or `python3.N` as a word in a script or harness
    file: `*.sh`, `*.toml` (so `mise.toml`), `*.yml` and `*.yaml` (so
    `.github/workflows/`), `*.json` (so `.claude/settings.json`), a
    `Dockerfile`, or `pigeon-start`, except the policy itself;
  - `Command::new("python…` in Rust.
- **`CLAUDE.md`** gains, under Rules: "No Python: tooling is Rust
  (`pigeon …`) or POSIX sh; see this ADR." Its planner line becomes
  `sh tooling/session/session.sh executor b<n>`.
- Both are guarded, so that step is the owner's, with the code.
- **What stays allowed:** POSIX sh glue, and Python named in Markdown (ADRs'
  history, docs).

### Execution

Each step is one change; the planner who wrote this ADR does steps 2 and 3
itself. They decide the port's shape across core, shell and sh, which a
brief would have to spell out line by line.

1. **`docs(adr)`:** this ADR.
2. **`feat(pigeon)`: `pigeon session executor` and `install`, and
   `session.sh` without Python.** §1–§3. `session.py` stays for one step,
   since `CLAUDE.md` still names it. Amends the model-tiered briefs ADR
   (`executor` is `pigeon session executor`) and the Rust-follower ADR
   (`install` and the bootstrap).
3. **`chore(policies)`: no Python.** §4's policy and `CLAUDE.md` lines, and
   `session.py` and `test_session.py` deleted. The owner's, with the code.
   `Governs` gains `.config/policies/no-python.yaml`.
4. **Verification,** in a fresh cloud session, then a report to the owner.

### Verification

- `mise run ci`, and `sentinel-change` on each step's branch; after step 3,
  `mise run sentinel` fails a branch that adds a `.py` file, a Python
  shebang, or `python3` in `mise.toml`, each with the policy's message.
- In a cloud session, with `python` and `python3` shadowed on `PATH` by
  stubs that log and fail: the `SessionStart` hook's `brief`, the status
  line, `stop`, `follow` under `Monitor`, and
  `sh tooling/session/session.sh executor b<n>` (with `source_url` and tags)
  work, and the stubs' log is empty.
- With `~/.cache/pigeon` removed, `session.sh brief` bootstraps a binary
  within 10 s; with it removed and the control plane unreachable, the hooks
  print their degraded lines and exit 0.
