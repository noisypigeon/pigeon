## Git through the control plane: the session token's failure, and a front door

* Date: 2026-11-06
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: security

## Context

[`2026-11-05-development-changes-through-pigeon-mcp.md`](2026-11-05-development-changes-through-pigeon-mcp.md)
§2 gives the agent's cloud session a **repository-scoped write token** for
`pigeon`. The owner mints it and saves it as the session environment's network
secret for the account's Artifacts git host. The proxy then injects it as
`Authorization: Bearer`.

On 2026-11-06 every git request from the session failed. The Artifacts git
host answered `401` with `Www-Authenticate: Basic realm="artifacts"`, and the
proxy added `upstream auth failed` for that secret. The failure repeated after
two re-saves and two minutes of waiting for the proxy to pick them up. The
control plane's MCP endpoint, behind Access with its own injected service
token, kept working.

What is known about Artifacts' git authentication:

- **The header form is right.** Git accepts a repo token as
  `Authorization: Bearer <token>`, the form cobot uses (`cobot/src/git.rs`), or
  as the password of Basic auth. The injected header is therefore not the
  problem; its value is.
- **A token is `art_v1_<40 hex>?expires=<unix seconds>`.** The Bearer form
  takes the whole string. Only the Basic form takes the part before `?expires=`.
- **TTLs.** `wrangler artifacts repos issue-token` and `POST …/tokens` default
  to 24 hours, from 60 seconds to a year. cobot's minted tokens live 900 s
  (`artifacts_wire::TOKEN_TTL`).
- **Tokens can be listed without their values.**
  `GET …/repos/<repo>/tokens?state=all` gives each token's scope, its state
  (`active`, `expired` or `revoked`) and `expires_at`.

The hypotheses, most likely first:

1. The saved token has expired or was revoked.
2. The saved value isn't the whole token: it lacks `?expires=…`, or it carries
   `Bearer `, quotes or whitespace.
3. The token is for another repository, namespace or account, or has `read`
   scope. A read token would still fetch, so this explains a refused push but
   not a refused fetch.

Behind the failure is the design itself. A long-lived, repository-wide write
token sits in the session. The owner re-mints it by hand, it covers `main` and
the tags, and nothing but a rule stops the agent using it there.
[`2026-11-05-development-git-hosting-cloudflare-artifacts.md`](2026-11-05-development-git-hosting-cloudflare-artifacts.md)
§2 already plans the answer: a front door on backplane's Worker that proxies
smart HTTP with a token minted per request and accepts pushes only to
`change/*`. The changes ADR retires the session token once that exists. This
ADR runs the spike, then builds the front door now.

## Implementation plan.

### 1. The spike

The owner runs it locally. No token value appears in the chat, the repository
or a log.

- List the repository's tokens with their states and expiry, using an API token
  holding Artifacts Read:
  `GET /accounts/<account>/artifacts/namespaces/pigeon/repos/pigeon/tokens?state=all`.
  Note whether the session's token is `active`.
- Check the saved value's shape:
  - it starts `art_v1_`;
  - it ends in `?expires=<n>`, where `<n>` is in the future;
  - it has no `Bearer ` prefix, no quotes and no trailing newline.
- Try the token from the owner's machine:
  `git -c http.extraHeader="Authorization: Bearer $T" ls-remote <remote>`.
  If it works there and fails in the session, the difference is in the saved
  value.
- Mint a fresh token and save the whole string:
  `wrangler artifacts repos issue-token pigeon --namespace pigeon --scope write --ttl 604800 --json`.
  The agent then fetches again.

The finding is recorded as an amendment here, with the change that implements
§2. Until the front door is deployed, the owner pushes the agent's changes from
patches it writes, so the spike blocks nothing.

### 2. The front door

backplane's Worker serves git's smart HTTP on its own host, behind the same
Access application as `/mcp`:

```
/git/<namespace>/<repo>.git/info/refs?service=git-upload-pack|git-receive-pack
/git/<namespace>/<repo>.git/git-upload-pack
/git/<namespace>/<repo>.git/git-receive-pack
```

- **Only the control plane's repository.** `<namespace>` and `<repo>` must be
  the Worker's `NAMESPACE` and `REPO`; anything else is `404`.
- **A token per request, never shown.** The Worker mints a repo token with its
  `ARTIFACTS_TOKEN`, as job Containers' tokens are minted
  (`worker/platform.rs` `credentials`). Fetches and the ref advertisement get a
  read token; a push gets a write token. Each request is forwarded to the
  repository's `remote`, with its body and git's headers unchanged, and the
  answer is streamed back. No token reaches the caller, a response or a log.
- **The push rule.** The Worker parses a push's ref updates from the start of
  its `git-receive-pack` body. Artifacts speaks protocol v1 for push, and git
  doesn't compress that body. Every updated ref must be
  `refs/heads/change/<n>-<slug>`, where:
  - change `n` exists, with that slug;
  - its plan is approved, so its state is `planned` through `approved`, not
    `proposed`, `merged` or `closed`;
  - the update isn't a deletion.

  `main`, tags and other refs are refused. The merge job writes `main` with its
  own token, as now.
- **All or nothing.** If any ref is refused, nothing is forwarded. Git gets a
  report-status answer with `ng <ref> <reason>` for each ref, on side-band
  channel 1 when the client asked for side-band, so the reason prints as
  `! [remote rejected]`.
- **Who.** The rule is the same for a service token and a person. The owner's
  own `pigeon cobot git push`, with an account token, goes straight to
  Artifacts and is unchanged.
- **The approval is the plan's.** Pushing needs no new approval:
  `change_authorize`, which only a person runs, is what opens a change's branch
  to pushes. Approving a head and merging stay as they are.
- **Audit.** Each push is audited like an operation: `git.push`, the change's
  number, and `ok` or `refused`.
- **Pure parts.** A new `backplane::gitdoor` holds them, shared by both builds
  and tested natively: parsing the pkt-lines, judging each ref against its
  change, and building the report.
- **Limits.** A push's body is read whole before it is judged. Workers accept
  request bodies up to 100 MB, far beyond a change's push.

### 3. `git.tokens`

- **A read operation**, `git.tokens`, run by the control plane. It lists the
  repository's tokens: each one's ID, scope, state, `created_at` and
  `expires_at`, and never a value.
- **Value-like fields are dropped.** The Worker keeps only those five fields
  from the API's answer, so a value-like field the API adds later is dropped
  too.
- **It is spike step 1, the agent's own.** A later failure can be diagnosed
  from the session.

### 4. Retiring the session token

- **The remote moves.** Once the front door is deployed, the agent's remote is
  `https://<control plane host>/git/pigeon/pigeon.git`.
- **Access's headers suffice.** The proxy already injects the service token's
  Access headers for that host, so git needs nothing else.
- **The secret is deleted.** The owner deletes the session environment's
  Artifacts network secret, which retires the token of the changes ADR §2.
- **Later sessions** get only the control plane's host.

### Execution

1. This ADR, with a `CLAUDE.md` pointer. The owner pushes it from the agent's
   patch.
2. `backplane::gitdoor`, the Worker's `/git/` routes and `git.tokens`, with the
   spike's finding as an amendment. The owner pushes this from a patch too,
   then redeploys:
   - `pigeon cobot workers plan --root . --state "$HOME/src/pigeon-platform-state"`;
   - then `deploy --plan <ID>`.
3. The agent moves its remote to the front door, and the owner deletes the
   network secret. Every later change, starting with the approvals of
   [`2026-11-06-development-control-plane-approvals-images-state.md`](2026-11-06-development-control-plane-approvals-images-state.md)
   §1, is pushed through it.

### Verification

- **The spike** names the cause, in an amendment.
- **Through the front door, with only the injected Access headers:**
  - `git ls-remote` and `git fetch` work;
  - pushing a planned change's branch succeeds, and its push event arrives as
    before;
  - pushing each of the following gets `[remote rejected]` with the reason,
    and Artifacts is unchanged: `main`, a tag, another change's branch, a
    proposed or closed change's branch, and a deletion.
- **No leaks.** D1's audit has `git.push` rows, and no token appears in any
  answer, log or audit row.
- **`git.tokens`** over MCP lists tokens with no value.
- **Retirement.** After the network secret is deleted, the agent still fetches
  and pushes its change branches.
