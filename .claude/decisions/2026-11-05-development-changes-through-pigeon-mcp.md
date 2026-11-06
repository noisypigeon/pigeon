## A change is planned, bounded, checked and rolled out through pigeon

* Date: 2026-11-05
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development

## Context

[`2026-11-05-development-git-hosting-cloudflare-artifacts.md`](2026-11-05-development-git-hosting-cloudflare-artifacts.md) moved pigeon's git to Cloudflare Artifacts, with GitHub as a mirror. #140 was the last change merged through GitHub. Now the owner pushes each change by hand, and the agent that writes the changes, a Claude Code cloud session, can't push at all.

The owner wants the lifecycle reimagined around this, as the beginning of pigeon's MCP:

> I prompt, you plan. I approve. Sentinel ensures it is safe and within bounds. The changes get pushed to a branch and tested with CI on Cloudflare. Builds are cheaper, faster and targeted. Then approval happens, the change is merged and rolled out against instances.

What Cloudflare offers now:

- **Artifacts push events.** `cf.artifacts.repo.pushed` can trigger a Workflow directly from the Wrangler configuration.
- **Workflows,** with durable steps and retries.
- **The Sandbox SDK:** Containers that run commands through an API, with snapshots that can be cached between runs.
- **`@cloudflare/ci`,** a CI pipeline library built on Workflows and Sandbox.
- **Workers Builds.** It builds and deploys Workers from Artifacts on push. It doesn't fit here: it builds Workers projects, not a Rust workspace's checks.

Pieces already in place:
- backplane's control plane: a Durable Object per instance, D1, plan-then-approve, Access and MCP ([`2026-11-05-runtime-control-plane-worker.md`](2026-11-05-runtime-control-plane-worker.md));
- cobot's `worker` build, which dispatches runs to Containers;
- sentinel's policies;
- the release scoping in `.config/releases.yaml`, which already knows which unit a path belongs to.

## Implementation plan.

### 1. The lifecycle

| Step | Who | Through | Recorded |
| --- | --- | --- | --- |
| 1. Prompt | owner | the agent session | — |
| 2. Plan | agent | MCP `change.propose`: the plan, and its **scope** (the paths, units and policies it may touch) | D1 `changes` row, state `proposed` |
| 3. Approve the plan | owner | MCP or CLI `change.approve-plan`, signed in with Access | `planned` |
| 4. Implement | agent | push `change/<n>-<slug>` to Artifacts | `pushed`, with the head SHA |
| 5. Bound | sentinel | `policy.check` on the change | `bounded` or `refused`, with the problems |
| 6. Check | control plane | targeted CI in Sandbox (§4) | `checked` or `failed`, with each job's result and log |
| 7. Approve the merge | owner | MCP or CLI `change.approve`, against the exact head SHA | `approved` |
| 8. Merge | control plane | squash onto `main`, as `<title> (!n)` | `merged` |
| 9. Release | control plane | `cobot release` and tags, as in the hosting ADR §5 | `released` |
| 10. Roll out | control plane | each affected instance's plan, approved by its owner, then applied (control-plane ADR) | per instance: `planned`, `applied` |

- **A new push resets the review.** It returns the change to step 5, and an approval only covers the head SHA it names.
- **The agent never writes `main`, tags or another change's branch.** Until the front door (hosting ADR §2) enforces this, it's a rule, checked by step 5's policy and audited by the mirror.

### 2. The agent's access

- **The credential.** A **repository-scoped write token** for `pigeon` only. The owner mints it, `wrangler artifacts repos issue-token` (or `pigeon cobot git token`, once that ships) for repository `pigeon`, namespace `pigeon`, scope `write`, and saves it as the session environment's **network secret** for the account's Artifacts git host (`Authorization: Bearer`).
  - The proxy injects it, so the agent never reads it.
  - It can't call the API, reach another repository, or touch another account.
- **The environment.** A personal environment named `pigeon`, not the Default, holds that secret and allows that host. Only these sessions use it.
- **Expiry.** The owner re-mints the token before it expires, and `status` warns when it's close. Once the front door exists, the agent pushes there through Access, and this token is retired.
- **Reading.** The session still starts from the GitHub mirror. It adds `artifacts` as a remote, and bases every change on `artifacts/main`.

### 3. Sentinel bounds a change

- **New policy target `change`.** It holds the change's title, body, number, base, head, branch, files and diff stats, plus its approved plan's scope. It replaces `pull-request` when the front door ships, and both run until then.
- **Rules:**
  - the branch is `change/<n>-<slug>`;
  - the title is a Conventional Commit;
  - the body cites an ADR;
  - no `CHANGELOG.md` and no `ENC[` value;
  - every touched path is inside the plan's scope, or the policy names the paths outside it;
  - no change to `.config/policies/`, `.github/` or the release configuration unless the plan names it;
  - a size limit per change.
- **Where it runs.** The agent runs it before pushing, `policy.check --change`. The control plane runs it again on the push event, and only that result counts.

### 4. CI on Cloudflare, targeted and cached

- **The trigger.** `cf.artifacts.repo.pushed` on `change/*` and `main` starts a Workflow in backplane's Worker, the `ci` Workflow.
- **Scope first** (`change.scope`, a backplane operation). The diff against the base is mapped to the jobs it needs, using the path rules `.config/releases.yaml` already defines, plus `cargo tree` for dependants.

  | Touched | Jobs |
  | --- | --- |
  | any Rust crate | `fmt`; `clippy` and `test` for that crate and its dependants (`-p …`); per-feature clippy only for `pigeon` features whose slice changed |
  | a `worker` build (`cobot`, `backplane`) | the wasm clippy, and the bundle build |
  | `catalog/`, a kind or a module | `infra.validate`; the catalog, manifest and apex tests; `tofu validate` on the modules touched |
  | `src/site-templates/` or a site's content | `sites.build` for the affected sites |
  | `.claude/decisions/`, `.config/policies/` | `policy.check` only |
  | docs only | `policy.check` only |

  `main` runs the full set nightly, so a scoping mistake can't hide for long.
- **Where jobs run.** In **Sandbox** containers, from one CI image: the cobot image plus the pinned Rust toolchain (`mise.toml`). Each job is a Workflow step with its own retries and timeout. Independent jobs run concurrently.
- **Caching.**
  - A **Sandbox snapshot** holds the warmed cargo registry and `target/`, keyed by `Cargo.lock` and the toolchain.
  - `sccache` keeps its cache in an R2 bucket, so a cold snapshot still reuses compiled crates.
  - A job whose inputs' hash already passed on that SHA is skipped, with its result reused.
- **Credentials.** None for checks. Each run fetches with a read token minted for that run, and `plan` jobs that need state run only after merge, through the control plane.
- **How it's built.** The `ci` Workflow uses `@cloudflare/ci` if backplane's Rust Worker can host it through bindings. Otherwise it's a small TypeScript Worker in `workers/ci/`, deployed by `pigeon cobot workers`, that calls backplane for scope and results. That gets decided at implementation, by whichever ships less code.
- **Results.** Each job's result is a D1 row on the change, with its log in R2. `change.status` shows them, through the MCP and the CLI.

### 5. pigeon's MCP

backplane's MCP, behind Access, gains these tools:
- `change.propose`, `change.approve-plan`, `change.status`, `change.diff`, `change.approve`, `change.merge` and `change.rollout`;
- `ci.rerun`, for one job.

Each tool is also a `pigeon backplane change …` command. The agent session calls the MCP. The owner approves through the MCP, the CLI or the dashboard. An approval is recorded with the Access identity and the SHA it covers.

### 6. Execution

Each step is pushed to Artifacts `main` and mirrored. Until step 3 lands, the owner runs `mise run ci` before fast-forwarding.

1. **Access** (no code):
   - create the `pigeon` environment;
   - mint the repository token and save it as its network secret;
   - the agent proves it can fetch and push a `change/` branch.

   Until §1 is served, the agent pushes `change/<slug>` branches, and the owner diffs, runs `mise run ci`, fast-forwards `main` and mirrors. This ADR and the previous one's root `config.yaml` are the first changes through that path.
2. **sentinel:** the `change` policy target and its rules.
3. **backplane:**
   - D1 `changes` and `jobs` tables;
   - the `change.*` operations and MCP tools;
   - `change.scope`.
4. **The `ci` Workflow:** push events, the CI image, Sandbox jobs, snapshot and `sccache` caching, and results.
5. **Merge, release and rollout** through the control plane. The front door enforces §1's branch rules, the repository token is retired, and the GitHub Actions workflows are removed (hosting ADR, step 5).

### 7. Verification

- The agent pushes `change/1-…` with the repository token, and a push to `main` with that token is refused by step 5's policy, or by the front door once it exists.
- A change touching only `catalog/` runs only the catalog jobs, and a Rust change runs only its crate and dependants. The nightly full run on `main` agrees.
- A second push with no input changes reuses the snapshot: its wall time is under a minute for a docs-only change, and a few minutes for one crate.
- A new push after approval returns the change to `bounded`, and merging needs a fresh approval for the new SHA.
- No token appears in a change row, a job log, the repository or the mirror.

## Amendments

### 2026-11-05: The change policy lands before GitHub has a sentinel that reads it, and scoping reads the crates' manifests

- `.config/policies/change.yaml` (§3) is merged on Artifacts without waiting for GitHub to release a sentinel that knows `target: change`. Checks before a merge run sentinel from source, so nothing here depends on the released image. Until the owner mirrors, nothing runs on GitHub at all. On the first mirror after it, `lifecycle.yml`'s `sentinel` job, which runs the released image, is red once. The same push releases the new sentinel, and the next mirror is green. The policy holds the branch, scope and size rules; the title, ADR and changelog rules stay in `pull-request.yaml`, which runs on every change too, because `mise run sentinel-change` passes its fields.
- `change.scope` (§4) finds a crate's dependants from the workspace's `Cargo.toml` files instead of `cargo tree`, so scoping a change needs no build and no toolchain.
- A change to `.config/policies/` also runs the tests that check this repository's own policies (the `sentinel`, `pigeon` and `pigeon-backplane` crates' tests), not `policy.check` alone. The first scoped run of the change policy found that gap: its new policy changed counts those tests pin.

### 2026-11-05: CI runs in a Durable Object per change and Container jobs, not Workflows and Sandbox

Built as phase E, in Rust: workers-rs 0.8.7 has no Workflows API, and the phase 3 spike showed every other binding works from Rust.

- **The pieces (§4, §5):**
  - `ChangeRun`, a Durable Object per change, holds `backplane::changes` in its storage, in place of the `ci` Workflow. Requests apply events and return at once. Its alarm does the slow work: minting a read token, starting and stopping job Containers, polling running jobs every 20 s, and mirroring the change to D1's `changes` and `jobs`.
  - `Checker` is one job's Container, from the Dockerfile's `ci` stage, in place of Sandbox. It holds only the signing key; each job's read token arrives with its request over the signed link. `pigeon backplane jobs` serves it.
  - A queue consumer takes the Artifacts push events, through an event subscription delivering to the queue. `change.pushed` is the backstop: it takes the SHA, and the bound job refuses one that isn't the branch's head.
  - The MCP server is on `/mcp`, behind Access, for every catalog operation. An apply-effect operation (approving a plan or a head, closing a change) needs a person: a service token proposes and pushes, and never approves.
- **The bound job is §3's check, and the only one that counts.** It runs this repository's own policies on the pushed tree: the change policy with the *approved* scope (no longer the commit trailers) and the pull-request policy, with the plan as the body. So a plan cites its ADR. It then runs `change.scope` on the changed paths. The jobs it names run concurrently, one Container each.
- **`change.approve-plan` is `change.authorize`**, since catalog names are lowercase letters only.
- **Caching, for now, is the `ci` image's pre-built dependencies only.** Sandbox snapshots and an sccache bucket are deferred: either would put a writable cache inside change code.
- **Logs.** Each finished job's log tail is in R2 under `ci/<job id>.log`. A failed job's last 8 KiB and its violations show in `change.status`.
- **Change numbers start at `CHANGE_FLOOR`.** The changes pushed by hand used 1 to 11.
- **The owner's first deploy**, from pigeon with a token holding `pigeon cobot workers permissions`:
  1. Apply `infra/pigeon` for the new `pigeon-ci` bucket (`infra.plan`, then `infra.apply`).
  2. `wrangler queues create pigeon-ci-events`, then subscribe it to the `pigeon` namespace's `cf.artifacts.repo.pushed` events (dashboard or API: wrangler can't filter by namespace yet).
  3. `pigeon cobot workers plan --root . --state <state>`, then `deploy --plan ID`. That migrates D1, builds the `ci` image (all the workspace's dependencies, so it takes a while) and deploys `pigeon-control`.
  4. `pigeon cobot workers secrets put pigeon-control KEY` for each secret:
     - `BACKPLANE_TOKEN`: 32 or more random characters;
     - `ARTIFACTS_TOKEN`: an API token with only Artifacts Read and Edit on the account;
     - `ACCESS_TEAM` and `ACCESS_AUD`: the Access team's name, and the application's audience tag.
  5. Put an Access application on the Worker's `workers.dev` hostname, allowing the owner and a service token for the agent. Add `https://<that host>/mcp` to the agent session as an MCP server, with the service token's headers.
- **The first changes through it:**
  - a docs-only change reaches `checked` with no job but the bound one;
  - a `catalog/` change runs only `infra.validate` and the catalog tests;
  - a push outside the scope ends `refused`;
  - the agent's `change.approve` is refused, and the owner's merges.

### 2026-11-06: The control plane merges an approved head, and each push to `main` publishes the sites it changed

Steps 8 and 10 of §1, for sites. Infrastructure applies stay the owner's.

- **`change.merge`** (effect apply, so a person runs it) queues a merge job on an approved change. The job runs in a `Checker` Container with a write token minted for it alone. It:
  - refuses unless `main` is an ancestor of the approved head, so `main` gets exactly the checked tree;
  - commits that tree onto `main` as `<title> (!<number>)`, with the plan, a `Change-Approved-By:` trailer and a `Change-Head:` trailer;
  - pushes without force, then deletes the branch.

  A failed merge keeps the change approved, with the reason.
- **`MainRun`, a Durable Object, is `main`'s pipeline.** The queue sends it each push to `main`.
  - It runs one deploy job at a time, always of the newest push.
  - That job publishes the instance's manifest sites whose inputs changed since the last successful deploy (`cobot::scope::deploys`): a site's unit and content, and, for the architecture template, everything `architecture.describe` reads. It publishes through `sites.publish`, with `DEPLOY_TOKEN`, an account token holding Workers R2 Storage Edit and Zone Read.
  - A failed deploy waits for the next push, which covers its changes too.
  - `deploy.status` shows the last deployed commit and the twenty most recent deploys.
- **Legacy `workloads/` sites are skipped.** They publish from their own instance's account.

### 2026-11-06: The first deploy's runbook, as it ran

`pigeon-control` was first deployed from the owner's machine, and its first change (this one) was proposed through `/mcp`. What the earlier runbook didn't say:

- **Access is enabled from the Worker.** Open Workers & Pages → `pigeon-control` → Domains, then **Enable Access** on the production `workers.dev` row.
  - Choose scope **All traffic**: "Previews only" leaves the production URL open.
  - Use a **Cloudflare account** Allow policy, or an exact **Emails** rule. Never an **Email domain** rule for a public mail provider: it would admit everyone with an address there.
  - Turn off the preview URL; nothing uses it.
  - The application this creates holds the AUD tag (`ACCESS_AUD`). The team domain `<team>.cloudflareaccess.com` gives `ACCESS_TEAM`.
- **The agent's service token** comes from Zero Trust → Access controls → Service credentials.
  - Its secret is shown only once, on the page after **Generate token**. **Refresh** only extends the expiry.
  - It gets its own policy on the application, with action **Service Auth**, never Allow.
  - The agent session reaches `/mcp` through its environment's network secrets: two custom headers on the Worker's host, `CF-Access-Client-Id` and `CF-Access-Client-Secret`, both with no prefix. The session never sees them.
  - The Worker names a service token by its client ID (`<id>.access`). It can propose and push, and is refused every apply-effect operation.
- **Images build for `linux/amd64`,** the only platform Containers run. On Apple Silicon:
  - Docker Desktop needs Rosetta for amd64, or the build runs under QEMU and takes hours;
  - it needs about 8 GB of memory. The `ci` stage builds without debug info, two jobs at a time (`WARM_JOBS`).
- **Checks before deploying:**
  - `pigeon cobot workers validate` refuses a binding that shares a Durable Object's name;
  - `plan` resolves `bucket_names[<name>]` from a `for_each` output, which is a map keyed by item.
- **The smoke tests:**
  - `cloudflared access curl https://<host>/v0/operations` lists the catalog as the owner;
  - `initialize` on `/mcp` answers as the agent.

  A plain `curl` gets Access's 302 to its login page.
