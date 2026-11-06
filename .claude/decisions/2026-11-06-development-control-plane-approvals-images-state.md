## The control plane: approvals in Claude, images and state on Cloudflare, and more Containers

* Date: 2026-11-06
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development

## Context

The first changes went through the control plane of
[`2026-11-05-runtime-control-plane-worker.md`](2026-11-05-runtime-control-plane-worker.md)
by the lifecycle of
[`2026-11-05-development-changes-through-pigeon-mcp.md`](2026-11-05-development-changes-through-pigeon-mcp.md):
change 12 was proposed, checked in job Containers, approved, and merged as
`!12`. Four frictions showed on the way, and this ADR decides each.

1. **Approvals are hand-typed.** The owner approves by writing a JSON-RPC
   `tools/call` body for `cloudflared access curl`, with a full head SHA in it.
   The agent drafts those bodies, and once drafted one with a SHA it had made
   up; the server refused it, but the path invites the mistake.
2. **Images build on the owner's machine.** `pigeon cobot workers deploy`
   builds the Dockerfile's `ci` stage for `linux/amd64`, under emulation on
   Apple Silicon, then rolls it out with the Worker. Every fix to the Worker
   or the job runner waits for that local build.
3. **Platform state is a local directory.** `infra/pigeon` and the workers are
   planned and applied with `--state "$HOME/src/pigeon-platform-state"`. The
   platform's state bucket exists and is empty, and cobot already reads and
   writes `r2://<bucket>/<prefix>` state (`cobot/src/state.rs`,
   `session::open`): a mirror is pulled, writes are conditional, and each unit
   is pushed after its apply.
4. **Four Containers are too few.** The `Checker` class runs at most four
   instances of `standard-4`. Change 13's five jobs, with finished jobs still
   holding instances, hit the limit and lost attempts to it.

The parts fixed already stay: a finished job's report frees its Container,
and a start refused for capacity waits for the next poll instead of counting
as a lost attempt.

## Implementation plan.

### 1. Approvals as permission prompts in Claude

- **MCP elicitation** (protocol revision `2025-06-18`, `elicitation/create`).
  When `change.authorize`, `change.approve`, `change.merge` or `change.close`
  is called, the server asks the client to show a form instead of refusing:
  the change's number and title, its plan or diff stat, the head SHA, and
  Accept or Decline. The owner answers on a card in Claude.
- **The SHA is the server's.** An accepted prompt records the SHA the server
  put in it, which is the change's current head; the arguments carry no SHA
  the model typed. A head that moves while the prompt is open voids it.
- **Who approves.** A service token never approves, with or without a prompt.
  An approval needs one of:
  - a person's Access identity on the request, as today with `cloudflared`,
    which stays as the fallback;
  - an accepted prompt in an MCP session signed in as a person: a connector
    the owner adds in Claude, authenticated by Access as the OAuth provider for
    MCP (Access's MCP server portals). The prompt is a second factor on top of
    that sign-in, and names the exact SHA.

  The agent's service-token session proposes, pushes and reads. Its calls to
  the four operations are refused, as now.
- **Transport.** Elicitation needs a server request in the middle of a call,
  so `/mcp` answers those four calls with Streamable HTTP's SSE response mode;
  every other call keeps its single JSON answer. `backplane::mcp` gains the
  `elicitation/create` round trip, and the pending call waits in the change's
  `ChangeRun` Durable Object, keyed by its request ID, for at most ten minutes.
- **Checked first, at implementation:** that Claude Code (desktop and web) and
  claude.ai show elicitation forms for a remote MCP server, and that Access's
  MCP sign-in works with them. **Where a client doesn't,** the tool result
  returns a one-click link instead: an Access-protected page on the Worker,
  `/approve/<number>/<sha>`, which shows the same facts and records the same
  approval for the person Access names.

### 2. Images built on Cloudflare

- **What changes often is split from what changes rarely.** The `ci` image's
  toolchain and pre-built dependencies change only with `Cargo.lock`,
  `mise.toml`, the toolchain or the Dockerfile. The `pigeon` binary and the
  Worker's bundle change with every merge.
- **On every merge to `main`, a build job.** `MainRun` runs it in a job
  Container, with no Docker:
  - `cargo build --release -p pigeon` and `worker-build` for backplane's
    `worker` build;
  - both are uploaded to the CI bucket under `builds/<sha>/`;
  - `wrangler deploy` rolls the Worker bundle out, with a deploy token in the
    job's environment that only that job receives.

  Job Containers fetch `pigeon` for the head they check from `builds/`, falling
  back to the binary baked into the image. A fix to the Worker or the job
  runner then lands with its merge, with no image build and no local step.
- **The image itself, only when its inputs change.** An image job runs rootless
  BuildKit (`buildkitd --rootless` with `buildctl`) in a job Container. It
  builds the Dockerfile's targets for `linux/amd64` natively, pushes them to
  Cloudflare's registry with a registry token, and rolls the new image out to
  the Container classes. `cobot::scope` triggers it on `Cargo.lock`,
  `mise.toml`, `rust-toolchain.toml` or the Dockerfile.
- **A spike first:** rootless BuildKit in a Cloudflare Container needs user
  namespaces and a large writable disk. If it can't run there, image builds
  stay the owner's, but only for those rare input changes.
- `pigeon cobot workers deploy` stays, for the bootstrap and as the fallback.

### 3. Platform state in R2

- **`pigeon cobot state migrate --from <dir> --to r2://<bucket>/<prefix>`.**
  It copies each unit's state, stored plans and generations. It refuses a
  destination that isn't empty, then plans every unit against the new store,
  and succeeds only if none has changes.
- **Credentials from the run's token.** The `COBOT_STATE_R2_*` variables become
  optional: without them, the S3 keys and endpoint are derived from
  `CLOUDFLARE_API_TOKEN` by `run_credentials` (`cobot/src/engines.rs`), as
  `sites publish` already does.
- **The passphrase stays the owner's.** State stays encrypted with it, and it
  is never a Worker secret.
- **The suffixed bucket name stays out of the repository.** The runbooks and
  `infra/pigeon/README.md` read it from the unit's outputs:
  `--state "r2://$(pigeon cobot outputs infra/pigeon bucket_names pigeon-state)/pigeon"`.
- **Order.** `infra/pigeon`'s drift is fixed first, so the migration's
  no-change check means something. Then the owner migrates, and the local
  directory is archived.
- Plans and applies of platform units by the control plane are a later ADR's;
  the state's place in R2 is its precondition.

### 4. More Containers

Two Container classes, sized by job:

| Class | Instance type | `max_instances` | Jobs |
| --- | --- | --- | --- |
| `Checker` | `standard-4` | 12 | Rust checks, builds, image builds |
| `Runner` | `standard-1` | 8 | the bound job, merges, site deploys, state jobs |

- `ChangeRun` and `MainRun` choose the class by job kind, and each class is a
  Durable Object class with its own Container in the spec.
- **At most six jobs of one change run at once,** so one large change doesn't
  take the fleet; the rest wait, as for capacity.
- **Cost.** Containers bill active CPU, memory and disk while running, and a
  finished job's Container is destroyed, so the higher limits cost nothing
  while idle. They stay well under the account's limits.

### Execution

Each step is a change through the pipeline, in this order:

1. This ADR, with a `CLAUDE.md` pointer.
2. The capacity split: the `Runner` class and the limits in
   `infra/pigeon/workers/control/worker.yaml`, and the class chosen per job.
3. `cobot state migrate`, after the drift fix; then the owner migrates.
4. Approvals by elicitation, after the client check.
5. `pigeon` from `builds/`, and the build-and-deploy job on merge.
6. The BuildKit spike, then image builds on Cloudflare.

### Verification

- **Approvals:** the owner approves a change by accepting a card in Claude,
  and the recorded approval names the server's SHA. The agent's session is
  still refused.
- **Images:** a merge that changes only Worker code redeploys the control
  plane with no local step; an image build runs only when its inputs change.
- **State:** after the migration every unit plans with no changes from
  `r2://`.
- **Capacity:** a change with eight jobs runs them at once, with no capacity
  wait.
