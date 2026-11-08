## A change's checks run on GitHub Actions and Buildkite, with Cloudflare as the fallback

* Date: 2026-11-08
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development

## Context

[`2026-11-05-development-changes-through-pigeon-mcp.md`](2026-11-05-development-changes-through-pigeon-mcp.md) runs every check of a change in Cloudflare Containers: `ChangeRun`, a Durable Object per change, starts one `Checker` Container per job (`standard-4`, at most 4), and polls it every 20 seconds. That works, but Containers are billed per vCPU-second, memory and disk beyond what Workers Paid includes. A Rust workspace's `clippy` and `test` jobs are the bulk of that time.

Two free alternatives exist for testing code. These facts were gathered when this ADR was written, and are checked against each provider's documentation in §9 step 3:

- **GitHub Actions.** Standard GitHub-hosted runners are free, with no minute limit, for public repositories. The Free plan runs at most 20 jobs at once and queues the rest. A job may run for at most 6 hours. Its cache holds 10 GB per repository. GitHub's terms allow Actions in a public repository for that repository's software; testing pigeon's code fits that.
- **Buildkite.** The free plan includes hosted agents: about 2,000 Linux vCPU-minutes a month, the Small shape only, up to 20 vCPU at once. Open-source projects can ask for special pricing. Builds start from a REST API (`POST /v2/organizations/<org>/pipelines/<pipeline>/builds`) and are read back the same way.

Both sit outside the critical path. The owner's words: this is "not for actual work and operations but rather the testing of code". It must respect each provider's limits, notice when GitHub is down or stalled, and route the work back to Buildkite or Cloudflare.

Constraints from earlier ADRs, all kept:

- **CI holds no credentials** ([`2026-11-05-development-pigeon-its-own-instance.md`](2026-11-05-development-pigeon-its-own-instance.md)). A runner gets no Cloudflare, Artifacts or GitHub write credential.
- **The mirror carries `main` and the tags, never `change/*`** ([`2026-11-05-development-git-hosting-cloudflare-artifacts.md`](2026-11-05-development-git-hosting-cloudflare-artifacts.md) §6).
- **Only the bound job counts** for a change's bounds (the lifecycle ADR's phase E amendment).
- **The GitHub App** from [`2026-11-08-development-github-mirror-on-merge.md`](2026-11-08-development-github-mirror-on-merge.md) is the only GitHub credential, and its tokens are minted per call.

## Implementation plan.

### 1. What moves and what stays

| Work | Where |
| --- | --- |
| The bound job: `policy.check` with the approved scope, then `change.scope` | Cloudflare, always |
| `fmt`, `clippy`, `test`, `features`, the wasm build, `infra.validate`, catalog tests, `sites.build`: every job `change.scope` names | routed (§4): GitHub, then Buildkite, then Cloudflare |
| The nightly full run on `main` | routed, GitHub first |
| Merge, release, mirror, plan, apply, rollout | Cloudflare, always |

The bound job is seconds of policy checking from a pre-built image, and it is the gate, so it stays where its result can't be forged. Everything routed is testing: a result decides nothing on its own, it is evidence for the owner's approval.

### 2. How a runner gets the code: `noisypigeon/pigeon-ci`

A change's tree can't go to the mirror, and a runner can't hold an Artifacts token. So a separate **public** repository, `noisypigeon/pigeon-ci`, carries the trees under test:

- **Its `main`** holds only `.github/workflows/run.yml` and a README naming its purpose. Both are rendered from pigeon's `tooling/ci/github/`, and pushed by the mirror job (§2 of the mirror ADR) whenever they change on pigeon's `main`. A change can't alter the workflow that tests it.
- **A relay** pushes each pushed change's head to `pigeon-ci` as the branch `ci/<n>`, force-updated per push, and `main`'s head as `ci/main`. It is a `relay` job kind in `Checker`, like `mirror`: an Artifacts read token and a GitHub installation token narrowed to `pigeon-ci` with `contents: write`, both minted per job. Its cost is one fetch and one push.
- **Cleanup.** `ci/<n>` is deleted when its change is merged or closed. The reconcile from the mirror ADR deletes any `ci/*` branch whose change is done.
- **Exposure.** An unmerged change's code becomes public before it merges. pigeon is public, every change is meant to merge into it, and §3 of the lifecycle ADR already forbids secrets in anything pushed, so this is accepted.

Buildkite checks out the same public repository anonymously, so it needs no git credential either.

### 3. The runners

**GitHub Actions: `run.yml`.**

- Triggered only by `workflow_dispatch`, with inputs `change`, `sha`, `nonce` and `jobs` (a JSON list of `{name, argv}`, as `change.scope` gives them).
- `run-name: ci ${{ inputs.change }} ${{ inputs.nonce }}`, so the control plane finds the run it started.
- `permissions: contents: read`, and no secrets; the repository has none.
- One matrix job per entry in `jobs`, on `ubuntu-latest`:
  1. check out `ci/<change>`, and fail unless `HEAD` equals `sha`;
  2. install the toolchain from `mise.toml`;
  3. restore the cargo cache, keyed by `Cargo.lock` and the toolchain;
  4. run `argv` in `src/pigeon-cli`, with `timeout-minutes` from `.config/ci.yaml`.
- Every action is pinned by SHA. `.config/policies/sentinel-actions.yaml` gains `tooling/ci/github/` as a target.

**Buildkite: the `pigeon-ci` pipeline.**

- Its steps are fixed in the pipeline's settings, from `tooling/ci/buildkite/pipeline.yml`, never uploaded from the checkout.
- A build is created with `commit: <sha>`, `branch: ci/<n>`, and `env: {PIGEON_CI_JOBS, PIGEON_CI_NONCE}`.
- One step per job, on the Small hosted shape, through `tooling/ci/buildkite/run.sh`, which makes the same `HEAD` check and runs the same commands as `run.yml`.
- The pipeline has no secrets. Its agent token can only act on its own build.

**Cloudflare.** Unchanged: `Checker` Containers, as today.

The same `argv` runs everywhere, so a job's result means the same thing on each backend.

### 4. Routing

**`.config/ci.yaml`** holds the routing, so changing it is a reviewed change:

```yaml
# Where a change's checks run; see .claude/decisions/2026-11-08-development-ci-runners-github-buildkite.md.
order: [github, buildkite, cloudflare]
backends:
  github:
    repository: noisypigeon/pigeon-ci
    workflow: run.yml
    concurrency: 16        # of the Free plan's 20, leaving room for the release workflows
    start_within: 10m      # queued longer than this is a stall
    job_timeout: 45m
  buildkite:
    organization: noisypigeon
    pipeline: pigeon-ci
    concurrency: 4         # Small agents, within 20 vCPU
    monthly_minutes: 1800  # of the free plan's 2,000 vCPU-minutes; stop at this
    start_within: 10m
    job_timeout: 45m
  cloudflare:
    concurrency: 4
    monthly_vcpu_hours: 20 # beyond this, a job waits for another backend
    job_timeout: 45m
```

- **The router** is a pure function in `backplane::ci`, from a backend's health, its budget and its in-flight count to a choice. `ChangeRun` calls it for each job. It picks the first backend in `order` that is healthy, under its concurrency and under its budget.
- **Batching.** A change's routed jobs go to one backend together, as one dispatch: one workflow run with a matrix, or one Buildkite build with several steps. That keeps API calls to a handful per push.
- **When nothing is available,** the jobs wait in `queued`, and the router retries on `ChangeRun`'s alarm. A job never goes to a backend over its budget. The owner can force one with `ci_rerun --on <backend>`.
- **What is recorded.** Each job row gains `backend`, `attempt` and the backend's run or build ID. `change_status` shows them, with a link to the run.

### 5. Results, read back by the control plane

Nothing on a runner holds a way to report back, so the control plane reads results itself:

- **GitHub.** After dispatching, it lists the workflow's runs created since then, and matches `run-name`'s nonce. Then, on `ChangeRun`'s alarm, it reads the run's jobs and their conclusions. A failed job's last 8 KiB of log is fetched once and stored in R2 under `ci/<job id>.log`, as today. The tokens are installation tokens narrowed to `pigeon-ci` with `actions: write` (to dispatch and cancel) or `actions: read`.
- **Buildkite.** The build's ID comes back from its creation. Its jobs' states are read from the builds API, and a failed job's log from the job log API. The token is `BUILDKITE_TOKEN`, an API token with only `read_builds`, `write_builds` and `read_build_logs`, scoped to the organization.
- **Matching a result to a change.** A result counts only for the `(change, sha, nonce)` it was dispatched with. A late result for an earlier SHA or attempt is recorded and ignored, as `ChangeRun` already ignores a stale job.
- **Polling respects the limits.** One request per active run per alarm, never per job. The alarm backs off from 20 seconds to 2 minutes while a run is queued. When GitHub's `x-ratelimit-remaining` falls under 10% of the hour's limit, polling slows to once every 5 minutes, and new dispatches route past GitHub until the window resets.

### 6. Stalls and outages

A backend is **healthy**, **degraded** or **down**, kept in a `Backends` Durable Object shared by every change, and shown by `pigeon backplane run ci.backends`.

**A job is stalled when:**

- it has been queued longer than its backend's `start_within`;
- it has run past its `job_timeout`;
- dispatching or reading its result fails with a 5xx, a timeout or a secondary rate limit, three times in a row;
- or no run matching its nonce appears within 2 minutes of a dispatch that was accepted.

**What a stall does:**

1. The stalled run is cancelled, best-effort: GitHub's cancel API, or Buildkite's.
2. The job's `attempt` grows, and the job is routed again, skipping the backend that stalled.
3. That backend's stall count grows. Two stalls in 15 minutes mark it **down**.

**Signals besides stalls:**

- GitHub's public status API (`githubstatus.com/api/v2/components.json`), read on the `Backends` alarm every 5 minutes. An incident on **Actions** or **API Requests** marks GitHub **degraded**, and new work routes past it. A **major outage** marks it **down**.
- A backend over its budget, or near its rate limit, is **degraded** for new work. What it is already running is left to finish.

**Recovery.** A **down** backend gets one probe after 15 minutes: the next job routed to it, alone. A probe that finishes marks it **healthy**, and one that stalls doubles the wait, up to 2 hours.

**The fallback order is the same in every case:** GitHub, then Buildkite, then Cloudflare. Cloudflare stays within its own budget. Once all three are unavailable, jobs wait, and the owner is told through `change_status`.

### 7. Budgets

- **GitHub:** no minute budget, only concurrency and the API's rate limits.
- **Buildkite:** the minutes each finished job used are summed per calendar month in D1's `ci_usage`, multiplied by the shape's vCPUs. At `monthly_minutes`, Buildkite takes no new work until the month turns.
- **Cloudflare:** a routed job's vCPU-seconds are summed the same way, against `monthly_vcpu_hours`. The bound job, merges, mirrors and relays don't count against it: they always run.
- `ci.backends` shows each budget's use this month.

### 8. Security

- **No runner holds anything worth stealing.** Neither `pigeon-ci` nor the Buildkite pipeline has a secret. GitHub's token is read-only, and Buildkite's agent token is scoped to its own job. A change's code already runs during its tests; here it runs where it can reach nothing of pigeon's.
- **A change can't pick its own checks or runner.** The workflow and pipeline come from `main`, the job list from the bound job, and the backend from the router.
- **Results are advisory.** A forged pass could at worst mislead the owner's approval, which also reads the diff. The bound job, the only gate, never leaves Cloudflare.
- **The App's new permissions.** The mirror ADR's App gains **Actions: read and write**, and is also installed on `pigeon-ci`. Each token is narrowed to one repository and only the permissions that call needs.
- **Nothing in a log.** The relay's tokens travel only in the environment, and job logs pulled from GitHub or Buildkite are scrubbed with the change's other logs.

### 9. Execution

1. **This ADR.**
2. **`feat(backplane)`: routing,** with Cloudflare as the only backend. This adds:
   - `.config/ci.yaml` and its schema;
   - `backplane::ci`'s router, the `Backends` Durable Object, budgets, and `ci_usage`;
   - `backend` and `attempt` on job rows;
   - `ci.backends`.

   This changes nothing at run time, and the router's tests cover every rule in §4, §6 and §7.
3. **The owner, once:**
   1. Create the public repository `noisypigeon/pigeon-ci`, with Actions enabled and every other feature off.
   2. Give the GitHub App **Actions: read and write**, and install it on `pigeon-ci`.
   3. Create the Buildkite organization and the `pigeon-ci` pipeline from `tooling/ci/buildkite/pipeline.yml`. Ask Buildkite for open-source pricing.
   4. Check §Context's limits against GitHub's and Buildkite's documentation, and correct `.config/ci.yaml` where they differ.
   5. `pigeon cobot workers secrets put pigeon-control BUILDKITE_TOKEN`.
4. **`feat(backplane)`: the GitHub backend,** in one change:
   - `tooling/ci/github/run.yml`, published to `pigeon-ci` by the mirror job;
   - the `relay` job kind;
   - dispatch, nonce matching, polling, cancel, and the status API signal.
5. **`feat(backplane)`: the Buildkite backend:** `tooling/ci/buildkite/`, builds, polling, cancel and logs.
6. **The order becomes `[github, buildkite, cloudflare]`** in `.config/ci.yaml`, in its own small change, so it can be reverted alone.

### 10. Verification

- The router's unit tests:
  - the order, concurrency and budgets;
  - each stall rule reroutes, skipping the backend that stalled;
  - two stalls mark a backend down, and a probe brings it back;
  - a late result for an old SHA or attempt is ignored.
- End to end, after step 6:
  - a docs-only change runs its bound job on Cloudflare and nothing else;
  - a Rust change's jobs run on GitHub, and its row links to the run;
  - a run held queued past `start_within` (the backend's concurrency set to 0 in a scratch config) is cancelled and finishes on Buildkite;
  - with GitHub's dispatch blocked and Buildkite's budget at its limit, the jobs finish on Cloudflare, and Cloudflare's monthly use rises by only those jobs.
- No token appears in `pigeon-ci`, a run's logs, a Buildkite build, a change row or R2.
- After a month, `ci.backends` shows Cloudflare's routed use well under its budget, and most jobs on GitHub.
