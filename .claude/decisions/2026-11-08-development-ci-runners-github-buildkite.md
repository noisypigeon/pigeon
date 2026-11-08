## A change's tests run on GitHub Actions and Buildkite, with Cloudflare as the fallback

* Date: 2026-11-08
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development

## Context

A change's checks run in Cloudflare Containers. `ChangeRun` starts one job per check named by `change.scope`, per [`2026-11-05-development-changes-through-pigeon-mcp.md`](2026-11-05-development-changes-through-pigeon-mcp.md). Since [`2026-11-06-development-control-plane-approvals-images-state.md`](2026-11-06-development-control-plane-approvals-images-state.md) §4, there are two classes:

- `Runner` (`standard-1`): the bound job, merges and site deploys;
- `Checker` (`standard-4`): Rust checks, builds and image builds.

Containers bill for active CPU, memory and disk beyond what Workers Paid includes. A Rust workspace's `clippy` and `test` jobs on `Checker` are most of that.

The owner's direction: run the *testing* of code on free CI, GitHub Actions and Buildkite, mindful of their limits. If GitHub is down or stalls, recognise it and route back to Buildkite or Cloudflare. This is "not for actual work and operations".

These facts were gathered when this ADR was written, and are checked in §9 step 3:

- **GitHub Actions.**
  - Standard GitHub-hosted runners are free, with no minute limit, for public repositories.
  - The Free plan runs 20 jobs at once and queues the rest.
  - A job may run for at most 6 hours.
  - The cache holds 10 GB per repository.
  - GitHub's terms allow Actions in a public repository for that repository's software, and testing pigeon's code fits that.
  - The workflow-dispatch API has historically returned no run ID.
- **Buildkite.**
  - The free plan's hosted agents give about 2,000 Linux vCPU-minutes a month, on the Small shape only, up to 20 vCPU at once. Older listings said 500 minutes and 3 jobs.
  - Open-source projects can ask for special pricing.
  - Builds are created and read through the REST API (`/v2/organizations/<org>/pipelines/<pipeline>/builds`).

What earlier ADRs fix, and this one keeps:

- **CI holds no credentials** ([`2026-11-05-development-pigeon-its-own-instance.md`](2026-11-05-development-pigeon-its-own-instance.md)).
- **Nothing runs on the mirror.** `.config/policies/mirror.yaml` forbids `.github/workflows/` in pigeon's tree, per the hosting ADR's 2026-11-06 amendment. That amendment also said "CI and delivery are Cloudflare's"; this ADR narrows it (§1).
- **The bound job is the only gate.**
- **The GitHub App** from [`2026-11-08-development-github-mirror-on-merge.md`](2026-11-08-development-github-mirror-on-merge.md) is the only GitHub credential, and its tokens are minted per job.

## Implementation plan.

### 1. What moves and what stays

| Work | Where |
| --- | --- |
| The bound job: policies with the approved scope, then `change.scope` | `Runner`, always |
| Every check `change.scope` names that only tests: `fmt`, `clippy`, `test`, `features`, the wasm build, `infra.validate`, catalog tests, `sites.build` | routed (§4): GitHub, then Buildkite, then `Checker` |
| The nightly full run on `main` | routed, GitHub first |
| Merges, restacks, the deploy, image builds, the mirror, plans and applies | Cloudflare, always |

- **Delivery stays Cloudflare's.** The hosting amendment's "CI is Cloudflare's" now holds only for the bound job and for everything that writes, deploys or holds a credential.
- **The bound job stays put** for two reasons:
  - it is seconds of policy checking on `standard-1`;
  - it is the gate, so its result is never read back from a runner.
- **A routed result is evidence for the owner's approval,** not a gate of its own.

### 2. How a runner gets the code: `noisypigeon/pigeon-ci`

A change's tree can't go to the mirror, and a runner can't hold an Artifacts token. So a separate **public** repository, `noisypigeon/pigeon-ci`, carries the trees under test.

- **Its `main` holds two files:** `.github/workflows/run.yml` and a README stating the repository's purpose.
  - Both come from pigeon's `tooling/ci/github/`. pigeon's tree holds no `.github/workflows/`, so `mirror.yaml` still holds.
  - The mirror job publishes them whenever they change on pigeon's `main`, so a change can't alter the workflow that tests it.
- **A relay job** pushes each change's head to `pigeon-ci` as the branch `ci/<n>`, force-updated per push, and `main`'s head as `ci/main`.
  - It runs on `Runner`, with an Artifacts read token and a GitHub installation token narrowed to `pigeon-ci` with `contents: write`. Both are minted per job.
  - It runs only after the bound job passes, so a refused change never leaves Cloudflare.
- **Cleanup.** `ci/<n>` is deleted when its change merges or closes. The mirror's hourly reconcile also deletes any `ci/*` branch whose change is done.
- **Exposure.** A change's code is public before it merges. pigeon is public, every change is meant to merge into it, and nothing pushed may hold a secret, so this is accepted.

Buildkite checks out the same public repository anonymously.

### 3. The runners

**GitHub Actions: `run.yml`.**

- Triggered only by `workflow_dispatch`, with these inputs:
  - `change` and `sha`;
  - `nonce`;
  - `jobs`, a JSON list of `{name, argv}` as `change.scope` gives them.
- `run-name: ci ${{ inputs.change }} ${{ inputs.nonce }}`, so the control plane can find the run it started.
- `permissions: contents: read`. The repository holds no secrets.
- One matrix job per entry in `jobs`, on `ubuntu-latest`:
  1. check out `ci/<change>`, and fail unless `HEAD` equals `sha`;
  2. install the toolchain from `mise.toml`;
  3. restore the cargo cache, keyed by `Cargo.lock` and the toolchain;
  4. run `argv` in `src/pigeon-cli`, under `timeout-minutes` from `.config/ci.yaml`.
- Every action is pinned by SHA, and a sentinel rule on `tooling/ci/github/` checks that.

**Buildkite: the `pigeon-ci` pipeline.**

- Its steps are fixed in the pipeline's settings, from `tooling/ci/buildkite/pipeline.yml`. They are never uploaded from the checkout.
- A build is created with:
  - `commit: <sha>` and `branch: ci/<n>`;
  - the environment variables `PIGEON_CI_JOBS` and `PIGEON_CI_NONCE`.
- It runs one step per job, on the Small hosted shape, through `tooling/ci/buildkite/run.sh`. That script makes the same `HEAD` check and runs the same commands.
- The pipeline holds no secrets.

**Cloudflare:** `Checker`, unchanged.

The same `argv` runs everywhere, so a job's result means the same on each backend.

### 4. Routing

**`.config/ci.yaml`** holds the routing, so changing it is a reviewed change:

```yaml
# Where a change's tests run; see .claude/decisions/2026-11-08-development-ci-runners-github-buildkite.md.
order: [github, buildkite, cloudflare]
backends:
  github:
    repository: noisypigeon/pigeon-ci
    workflow: run.yml
    concurrency: 16        # of the Free plan's 20
    start_within: 10m      # queued longer is a stall
    job_timeout: 45m
  buildkite:
    organization: noisypigeon
    pipeline: pigeon-ci
    concurrency: 4         # Small agents, within 20 vCPU
    monthly_minutes: 1800  # of about 2,000 vCPU-minutes; no new work past this
    start_within: 10m
    job_timeout: 45m
  cloudflare:
    concurrency: 6         # a change's limit on Checker, as now
    monthly_vcpu_hours: 40 # routed jobs only; past this a job waits
    job_timeout: 45m
```

- **The router** is a pure function in `backplane::ci`. It takes each backend's health, budget and in-flight count, and picks the first backend in `order` that is healthy and under both its concurrency and its budget. `ChangeRun` and the nightly run call it.
- **Batching.** A change's routed jobs go to one backend together, as one dispatch: one workflow run with a matrix, or one Buildkite build. That keeps it to a few API calls per push.
- **When nothing is available,** jobs wait as `queued`, as they wait for capacity today. No job goes to a backend past its budget. The owner can force one with `ci_rerun` and a new `on` argument naming the backend.
- **The record.** Each job gains `backend`, `attempt` and the backend's run or build ID. `change_status` shows them, with a link.
- **Events.** A reroute is an event on the change, so a session's `events_wait` sees it.

### 5. Results, read back by the control plane

A runner has no way to report back, so the control plane reads the results itself.

- **GitHub.**
  - After dispatching, the control plane lists the workflow's runs created since the dispatch, and matches the nonce in `run-name`.
  - On `ChangeRun`'s alarm, it reads that run's jobs and their conclusions.
  - It fetches a failed job's last 8 KiB of log once, into R2 at `ci/<job id>.log`, as today.
  - Its tokens are App installation tokens narrowed to `pigeon-ci`: `actions: write` to dispatch and cancel, `actions: read` to read.
- **Buildkite.**
  - The build's ID comes back from its creation.
  - Its jobs, and a failed job's log, are read from the REST API.
  - The token is the Worker secret `BUILDKITE_TOKEN`, with only `read_builds`, `write_builds` and `read_build_logs`.
- **Matching.** A result counts only for the `(change, sha, nonce)` it was dispatched with. A late result for an older head or attempt is recorded and ignored, as `ChangeRun` already ignores stale jobs.
- **Limits:**
  - one request per active run per alarm, never one per job;
  - the alarm backs off from 20 seconds to 2 minutes while a run is queued;
  - when GitHub's `x-ratelimit-remaining` falls below 10% of the hour's limit, polling drops to every 5 minutes, and new work routes past GitHub until the window resets.

### 6. Stalls and outages

A `Backends` Durable Object, shared by every change, marks each backend **healthy**, **degraded** or **down**. `pigeon backplane run ci.backends` shows it.

**A job is stalled when any of these holds:**

- it has been queued longer than its backend's `start_within`;
- it has run past `job_timeout`;
- dispatching it, or reading it, fails three times in a row with a 5xx, a timeout or a secondary rate limit;
- no run matching its nonce appears within 2 minutes of an accepted dispatch.

**A stall:**

1. cancels the stalled run, best-effort, through GitHub's or Buildkite's API;
2. increments the job's `attempt` and routes it again, skipping the backend that stalled;
3. counts against that backend. Two stalls in 15 minutes mark it **down**.

**Other signals:**

- **GitHub's status API** (`githubstatus.com/api/v2/components.json`), read every 5 minutes on the alarm of `Backends`:
  - an incident on **Actions** or **API Requests** marks GitHub **degraded**, and new work routes past it;
  - a major outage marks it **down**.
- **Budgets and rate limits.** A backend past its budget, or near its rate limit, is **degraded** for new work. Its running work finishes.

**Recovery.** A **down** backend gets one probe after 15 minutes: the next job routed to it, alone. If the probe finishes, the backend is **healthy** again. If it stalls, the wait doubles, up to 2 hours.

**When all three are unavailable,** jobs wait, and `change_status` says why. The digest names a backend that has been down for more than an hour.

### 7. Budgets

- **GitHub:** concurrency and API rate limits only.
- **Buildkite:** each finished job's minutes, times the shape's vCPUs, are summed per calendar month in D1's `ci_usage`. At `monthly_minutes`, Buildkite takes nothing new until the month turns.
- **Cloudflare:** routed jobs' vCPU-seconds on `Checker` are summed the same way, against `monthly_vcpu_hours`. The bound job, merges, restacks, deploys, image builds, mirrors and relays don't count, and always run.
- **`ci.backends`** shows each budget's use this month.

### 8. Security

- **No runner holds anything worth stealing.**
  - `pigeon-ci` and the Buildkite pipeline hold no secrets.
  - GitHub's job token is read-only, and Buildkite's agent token reaches only its own job.
  - A change's code already runs in its tests; here it runs where it can reach nothing of pigeon's.
- **A change can't pick its own checks or runner.** The workflow and pipeline come from `main`, the job list from the bound job, and the backend from the router.
- **Results are advisory.** A forged pass could at worst mislead the owner's approval, which also reads the diff. The gate, the bound job, never leaves Cloudflare.
- **The App's new permission.** The mirror ADR's App gains **Actions: read and write**, and is also installed on `pigeon-ci`. Each token is narrowed to one repository and the permissions its call needs.
- **No token in a log.** Tokens travel only in the environment, and logs pulled from GitHub or Buildkite are scrubbed like any job's.
- **Approval tiers.** `.config/ci.yaml` and `tooling/ci/**` route only tests, so `.config/approvals.yaml` doesn't need to guard them.

### 9. Execution

1. **This ADR.**
2. **`feat(backplane)`: routing,** with `cloudflare` the only backend in `order`:
   - `.config/ci.yaml` and its schema;
   - the router;
   - the `Backends` Durable Object, budgets and `ci_usage`;
   - `backend` and `attempt` on jobs;
   - `ci.backends`, and `ci_rerun`'s `on` argument.

   This changes nothing at run time, and the router's tests cover §4, §6 and §7.
3. **The owner, once:**
   1. Create the public repository `noisypigeon/pigeon-ci`, with Actions on and every other feature off.
   2. Give the App **Actions: read and write**, and install it on `pigeon-ci`.
   3. Create the Buildkite organization and the `pigeon-ci` pipeline from `tooling/ci/buildkite/pipeline.yml`, and ask Buildkite for open-source pricing.
   4. Check §Context's limits against GitHub's and Buildkite's documentation, and correct `.config/ci.yaml` where they differ.
   5. `pigeon cobot workers secrets put pigeon-control BUILDKITE_TOKEN`.
4. **`feat(backplane)`: the GitHub backend:**
   - `tooling/ci/github/run.yml` and its sentinel rule;
   - the publish of `run.yml` by the mirror job;
   - the `relay` job kind;
   - dispatch, nonce matching, polling and cancel;
   - the status API signal.
5. **`feat(backplane)`: the Buildkite backend:** `tooling/ci/buildkite/`, and its builds, polling, cancel and logs.
6. **`order: [github, buildkite, cloudflare]`** in its own small change, so it can be reverted alone.

### 10. Verification

- The router's tests:
  - order, concurrency and budgets;
  - each stall rule reroutes, skipping the backend that stalled;
  - two stalls mark a backend down, and a probe brings it back;
  - a late result for an old head or attempt is ignored.
- End to end, after step 6:
  - a docs-only change runs only its bound job, on `Runner`;
  - a Rust change's tests run on GitHub, and its jobs link to the run;
  - a run held in the queue past `start_within` (a scratch config with GitHub's concurrency at 0) is cancelled and finishes on Buildkite;
  - with GitHub's dispatch blocked and Buildkite at its budget, the jobs finish on `Checker`, and Cloudflare's routed use rises by only those jobs.
- No token appears in `pigeon-ci`, a run's log, a Buildkite build, a change row or R2.
- After a month, `ci.backends` shows most tests on GitHub, and Cloudflare's routed use well under its budget.
