## The control plane mirrors to GitHub on every push to `main`

* Date: 2026-11-08
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development

## Context

[`2026-11-05-development-git-hosting-cloudflare-artifacts.md`](2026-11-05-development-git-hosting-cloudflare-artifacts.md) made Cloudflare Artifacts the source of truth and GitHub a mirror. Its §6 left the mirror to the owner, who runs `pigeon cobot git mirror` by hand "until phase 5". Its 2026-11-06 amendment made GitHub a source backup only: no workflows run there, so GitHub never moves ahead and `mirror` needs no `sync`.

Nobody runs the mirror by hand any more. When this ADR was written, GitHub's `main` was 71 commits behind Artifacts' (`196e593` against `af7dddd`). A Claude Code session cloned from it built on that stale tree, and pushed to GitHub, which [`2026-11-08-development-concurrent-agent-sessions.md`](2026-11-08-development-concurrent-agent-sessions.md)'s amendment of that day now forbids. A backup that far behind isn't a backup.

The owner's direction: on merge, pigeon pushes to the GitHub mirror.

What is already in place:

- **`MainRun`'s pipeline.** On each push to `main`, it runs one deploy at a time, of the newest push, per [`2026-11-07-development-control-plane-deploys-itself.md`](2026-11-07-development-control-plane-deploys-itself.md). It is where post-merge work starts.
- **The `Runner` class.** `standard-1` Containers for small jobs, per [`2026-11-06-development-control-plane-approvals-images-state.md`](2026-11-06-development-control-plane-approvals-images-state.md) §4. Its image holds git and `pigeon`, and it takes per-job tokens over the signed link.
- **The mirror code.** `cobot::git::mirror_from` takes a remote, a token and a staging directory. It only fast-forwards the default branch, only adds tags, and never pushes `change/*`.
- **Read tokens.** The control plane already mints a repository-scoped Artifacts read token for each job.

What is missing is a GitHub credential the control plane can hold, and the step that uses it.

## Implementation plan.

### 1. What starts a mirror

- **Each push to `main`.** `MainRun`'s pipeline gains a `mirror` job beside its deploy. They are independent: a failed deploy still mirrors, and a failed mirror still deploys. Like the deploy, it acts only on the newest push, so a burst of merges makes one mirror.
- **Tags.** The release job, once it runs on Cloudflare (hosting ADR §7 step 4), starts the same job after pushing its tags.
- **A reconcile, hourly,** on `MainRun`'s alarm. It reads both sides' refs with a plain smart-HTTP `GET <remote>/info/refs?service=git-upload-pack`: Artifacts through the front door, GitHub anonymously, since the mirror is public. That happens in the Worker, so no Container starts unless the two sides differ.
  - **Artifacts is ahead:** it mirrors.
  - **GitHub has commits or tags Artifacts lacks:** nothing is written. It is reported (§4), and the owner decides, with `pigeon cobot git sync` or a failback.
- **`deploy_retry`** also retries a failed mirror.

### 2. The job

- **Where.** On `Runner`, as the job kind `mirror`.
- **What it is sent,** over the signed link:
  - the Artifacts remote, with a **read** token minted for that job;
  - the GitHub remote, the `cloudflare/git-repo` unit's `mirror`;
  - a GitHub installation token for that one repository (§3).
- **What it runs.** `pigeon backplane jobs` runs `cobot::git::mirror_from` in a fresh bare directory. The rules are unchanged: the default branch only fast-forwards, tags are only added, and a divergence stops the job with both SHAs.
- **Its result.** The job returns both sides' refs. `MainRun` keeps its last success and last failure, and writes them to D1 with the deploy's row.
- **Retries.** A failed mirror is retried with backoff (1, 5 and 30 minutes), then on each reconcile. A mirror never blocks a merge, a deploy or a rollout.

### 3. The GitHub credential

- **A GitHub App,** `pigeon-mirror`, owned by the owner's account and installed only on the mirror repositories.
  - Its permissions are **Contents: read and write**, and **Workflows: read and write**: GitHub refuses a push that touches `.github/workflows/` without the second one, even one that deletes a file.
  - It has no webhook.
- **No token is stored.** The control plane signs a JWT with the App's key, and mints an **installation token per job**, narrowed with `repositories: [<repo>]` and the two permissions. It expires within the hour.
- **The secrets.** The Worker gains `GITHUB_APP_ID` and `GITHUB_APP_KEY` (the key in PKCS#8, since WebCrypto's RS256 signing reads only that). The owner sets them with `pigeon cobot workers secrets put`. The installation's ID is looked up, never written down.
- **Only the mirror job receives a token,** as only the deploy job receives `WORKERS_DEPLOY_TOKEN`. Checks never do.
- **How the token reaches git.** As the hosting ADR §2 does for Artifacts: `http.https://github.com/.extraHeader`, through `GIT_CONFIG_COUNT`/`KEY`/`VALUE`, never through argv, a file or a log. The job's output scrubbing cuts it as well.
- **The owner's own credentials still work.** `pigeon cobot git mirror` from a laptop is unchanged, and it is the fallback while the control plane is down.

### 4. Reporting

- **`deploy_status`** shows, for the newest push, the mirror's outcome beside the deploy's: both sides' default-branch SHAs and the number of tags missing on GitHub.
- **The owner's digest** (`digest_show`) gains "mirror behind": GitHub has been behind for more than 24 hours, or has commits or tags Artifacts lacks. That is enough to push to the owner.

### 5. Supersedes

The hosting ADR §6's "after that" is brought forward, and holds from this ADR's execution:

- the control plane mirrors, not the owner;
- its credential is an App installation token minted per job, not a fine-grained token in the Secrets Store;
- the reconcile is hourly, not daily.

### 6. Cost

A mirror job is one fetch and one push: seconds of `standard-1`, once per merge. The reconcile is two HTTP requests an hour, and it starts nothing while the two sides agree.

### 7. Execution

1. **This ADR.**
2. **`feat(backplane)`: the mirror,** in one change:
   - the `mirror` job kind in `pigeon backplane jobs`, calling `mirror_from` with the tokens it is sent;
   - a GitHub token through `cobot::git::command`'s environment, for `https://github.com/`;
   - App JWT signing and installation-token minting;
   - `MainRun`'s `mirror` step, the hourly `info/refs` reconcile, retries, and the D1 columns;
   - `deploy_status` and the digest;
   - the spec's two secrets.

   The merge deploys it.
3. **The owner, once:**
   1. Create the GitHub App with the two permissions and no webhook, and install it on `noisypigeon/pigeon` only.
   2. Convert its key: `openssl pkcs8 -topk8 -nocrypt -in key.pem`.
   3. `pigeon cobot workers secrets put pigeon-control GITHUB_APP_ID`, then `GITHUB_APP_KEY`.
   4. Run `deploy_retry`. The first mirror fast-forwards GitHub's `main` over everything merged since `196e593`.
4. **noisypigeon** installs the same App on `noisypigeon/noisypigeon` once its repository is declared (hosting ADR §7 step 6). Until the control plane mints per-instance credentials, its mirror stays on the owner's CLI.

### 8. Verification

- `tests/git.rs`, against local remotes:
  - the `mirror` job fast-forwards, adds tags, and stops on a divergence without writing;
  - the GitHub token appears in no argv, file or output.
- backplane's tests:
  - a burst of pushes to `main` makes one mirror, of the newest;
  - a reconcile that finds both sides equal starts no job;
  - a failed mirror leaves the deploy's outcome alone, and the other way round.
- After the deploy:
  - GitHub's `main` equals Artifacts';
  - a change merged through the control plane reaches GitHub within a minute, with no command run;
  - with GitHub unreachable, merges and deploys keep succeeding, `deploy_status` shows the mirror behind, and the next reconcile after GitHub returns catches up.
