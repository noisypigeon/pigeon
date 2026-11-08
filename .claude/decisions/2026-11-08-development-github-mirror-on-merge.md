## The control plane mirrors to GitHub on every merge

* Date: 2026-11-08
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development

## Context

[`2026-11-05-development-git-hosting-cloudflare-artifacts.md`](2026-11-05-development-git-hosting-cloudflare-artifacts.md) made Cloudflare Artifacts the source of truth and GitHub a mirror. Its §6 left the mirror to the owner, who runs `pigeon cobot git mirror` by hand "until phase 5". Its amendment added `pigeon cobot git sync`, because GitHub's release bot still commits and tags there. Every merge now ends with the same loop, all by hand:

1. merge on Artifacts;
2. `pigeon cobot git mirror`;
3. wait for GitHub's release bot;
4. `pigeon cobot git sync`.

A forgotten step leaves GitHub behind, or Artifacts without its release. The owner wants this to happen on its own: on merge, pigeon pushes to the mirror.

What is already in place:

- **The push events.** The control plane's queue consumer already receives the `pigeon` namespace's `cf.artifacts.repo.pushed` events, per [`2026-11-05-development-changes-through-pigeon-mcp.md`](2026-11-05-development-changes-through-pigeon-mcp.md)'s phase E amendment. Today it only acts on `change/*`.
- **The job Container.** `Checker`, from the Dockerfile's `ci` stage, already holds git and `pigeon`, and takes per-job tokens over the signed link.
- **The mirror code.** `cobot::git::mirror_from` and `sync_from` take a remote, a token and a staging directory. They already refuse a force-push, never move or delete a tag, and never push `change/*`.
- **The minting.** `ARTIFACTS_TOKEN` already mints repository-scoped Artifacts tokens per run.

What is missing is a GitHub credential the control plane can hold, and something that starts the job.

## Implementation plan.

### 1. What starts a mirror

- **A push to the default branch or a tag.** The queue consumer forwards an Artifacts push event whose ref is `refs/heads/<default branch>` or `refs/tags/*` to `Mirror`. It doesn't matter who pushed: the control plane's merge, the release job, or the owner by hand.
- **A reconcile, every 15 minutes,** from a Cron Trigger on the control plane's Worker. It reads both sides' refs with a plain smart-HTTP `GET <remote>/info/refs?service=git-upload-pack`: Artifacts with a read token, GitHub anonymously, since the mirror is public. This happens in the Worker, so no Container starts unless the refs differ.
  - **Artifacts ahead:** mirror.
  - **GitHub ahead** (the release bot committed or tagged): sync, then mirror.
  - **Diverged:** no write. The repository is reported as drift (§5).
- **`pigeon backplane run git.mirror [repo]`,** an operation with an MCP tool `git_mirror`, so the owner can start one by hand.

### 2. `Mirror`, a Durable Object per repository

- **One at a time.** It holds at most one running job and one pending flag. An event that arrives while a job runs sets the flag, so a burst of merges and tags makes one more run, not many.
- **The job.** It starts `Checker` with the job kind `mirror`, and sends over the signed link:
  - the Artifacts remote and a **write** token minted for that run (sync writes the default branch and tags);
  - the GitHub remote, from the `cloudflare/git-repo` unit's `mirror`;
  - a GitHub installation token for that one repository (§3).
- **What the job runs.** `pigeon backplane jobs` runs `cobot::git::sync_from`, then `mirror_from`, in a fresh bare directory. The steps are unchanged, so the hosting ADR's amendment still holds: the default branch only fast-forwards, tags are only added, and a divergence stops the job with both SHAs.
- **Its result.** The job returns both sides' refs. `Mirror` keeps the last success, the last failure and their SHAs in its storage, and writes a row to D1's `mirrors`.
- **Retries.** A failed job is retried with backoff (1, 5 and 30 minutes, then each reconcile). A mirror never blocks a merge, a release or a rollout.

### 3. The GitHub credential

- **A GitHub App,** `pigeon-mirror`, owned by the owner's account and installed only on the mirror repositories. Its permissions are **Contents: read and write**, and **Workflows: read and write**: a mirrored commit that changes `.github/workflows/` is refused without the second one.
- **No token is stored.** The control plane signs a JWT with the App's key and mints an **installation token per job**, narrowed with `repositories: [<repo>]` and `permissions: {contents: write, workflows: write}`. It expires within the hour.
- **The secrets.** The Worker binds `GITHUB_APP_ID` and `GITHUB_APP_KEY` (the key in PKCS#8, because WebCrypto's RS256 signing reads only that). The owner sets them with `pigeon cobot workers secrets put`. The installation's ID is looked up, never written down.
- **How the token reaches git.** As the hosting ADR §2 does for Artifacts: `http.https://github.com/.extraHeader` through `GIT_CONFIG_COUNT`/`KEY`/`VALUE`, never through argv, a file or a log. `cobot::ci`'s output scrubbing also cuts it.
- **The owner's own credentials stay valid.** `pigeon cobot git mirror` and `sync` from a laptop are unchanged, and they're the fallback when the control plane is down.

### 4. Supersedes

This is the "after that" in the hosting ADR §6, brought forward. It holds from this ADR's execution, not from phase 5:

- the mirror is pushed by the control plane, not by the owner;
- its credential is an App installation token minted per job, not a fine-grained token in the Secrets Store;
- the reconcile runs every 15 minutes rather than daily, because it also brings the release bot's work back.

The amendment's loop (merge, `mirror`, release, `sync`) is unchanged; only the person running it goes away. When the release job moves to the control plane (hosting ADR §7 step 4), the reconcile only ever finds Artifacts ahead.

### 5. Reporting

- **`pigeon backplane run git.status`** (and MCP `git_status`) shows, per repository: both sides' default-branch SHAs, the number of tags missing on either side, the last mirror and its outcome.
- **Drift.** A repository that has diverged, or whose mirror has failed for 24 hours, is reported as drift on the instance, alongside the infrastructure drift in [`2026-11-05-runtime-control-plane-worker.md`](2026-11-05-runtime-control-plane-worker.md) §11.

### 6. Cost

A mirror job is a fetch and a push: seconds of the smallest Container instance type the spec allows, a few times a day. The reconcile is two HTTP requests per repository every 15 minutes, and starts nothing when the two sides agree.

### 7. Execution

1. **This ADR.**
2. **`feat(backplane)`: the mirror,** in one change:
   - the `mirror` job kind in `pigeon backplane jobs`, calling `sync_from` and `mirror_from` with the tokens it is sent;
   - a GitHub token through `cobot::git::command`'s environment, for `https://github.com/`;
   - the `Mirror` Durable Object, the queue consumer's forwarding, the Cron Trigger and the `info/refs` reconcile;
   - GitHub App JWT signing and installation-token minting;
   - D1's `mirrors` table, `git.mirror` and `git.status` with their MCP tools;
   - `infra/pigeon/workers/control/worker.yaml`: the `Mirror` class, the cron and the two secrets.
3. **The owner, once:**
   1. Create the GitHub App with the two permissions, and no webhook. Install it on `noisypigeon/pigeon` only.
   2. Convert its key: `openssl pkcs8 -topk8 -nocrypt -in key.pem`.
   3. `pigeon cobot workers secrets put pigeon-control GITHUB_APP_ID`, then `GITHUB_APP_KEY`.
   4. Deploy the Worker (`workers plan`, then `deploy --plan ID`).
   5. Run `git.mirror pigeon`.
4. **noisypigeon** installs the same App on `noisypigeon/noisypigeon` once its repository is declared (hosting ADR §7 step 6). Until the control plane mints per-instance credentials, its mirror stays on the owner's CLI.

### 8. Verification

- `tests/git.rs`, against local remotes:
  - the `mirror` job syncs a mirror that is ahead and then mirrors, and stops on a divergence without writing either side;
  - the GitHub token appears in no argv, file or output.
- backplane's tests:
  - a burst of push events produces one running mirror and one pending one;
  - a reconcile that finds both sides equal starts no job;
  - a failed mirror leaves the change `merged`.
- After the deploy:
  - a change merged through the control plane reaches GitHub's `main` within a minute, with no command run;
  - a release made by GitHub's bot is on Artifacts within 15 minutes;
  - with GitHub unreachable, merges keep succeeding, `git.status` shows the mirror behind, and the next reconcile after GitHub returns catches up.
