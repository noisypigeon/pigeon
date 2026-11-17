## Auto covers a session's own closes and fronts, secrets code without secret material, and comment-only gate edits

* Date: 2026-11-17
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: security
* Governs: `.config/approvals.yaml`, `.claude/settings.json`, `src/**/approval/tiers.rs`, `src/**/approval/windows.rs`, `src/**/approval/tests.rs`, `src/**/change/queue.rs`, `src/pigeon-agent/src/jobs.rs`, `src/**/checks/ci.rs`

## Context

On 2026-11-17, from about 07:30 to 14:30 UTC, the control plane asked for the
owner's passkey on about 30 operations. The owner approved every one without
needing to review it. Their standing policy: no manual approval "except for
destructive actions, or secrets changes". The asks fell into four kinds:

1. **`change.close` of a session's own pushed, unmerged change:** !906, !955,
   !958, !961, !973 and !975, each refused, superseded or moot.
   `tiers::tier_of` makes closing any pushed change `owner`, because it
   "throws its work away" (the approvals-once-passkeys ADR §2). It doesn't:
   closing deletes only the GitHub mirror's branch (`control::UNMIRROR`). The
   Artifacts branch stays: `change/906-…`, `955-…`, `958-…` and `961-…` were
   all still there after their close. `change_propose` with `resumes` finishes
   a closed head again. So a close by the session that proposed the change is
   reversible, and not destructive.
2. **`change.front`:** about 20 overnight (!881, !888, !904, !905, !907,
   !908, !911, !912, !922, !923, !926, !939, !940, !967 and others). Fronting
   is `owner` for every change (the standing Auto ADR's 2026-11-16
   amendment). It moves an already approved change ahead in the merge queue.
   It loses nothing, and it is neither a secret nor destructive. Its only cost
   is fairness to the other changes waiting.
3. **The `secrets:` list on code that holds no secret:** !966 (the broker's
   `worker/provider.rs`). Any file on `secrets:` makes the merge `owner`,
   whatever its diff. `secrets:` mixes two kinds of file. Some hold secret
   material or say who gets a credential: `secrets.config.yaml`, `.mcp.json`,
   the token modules, `roles.yaml` and `capabilities.yaml`. The others are
   code that reads a secret, such as the broker, the Apple connection and the
   signing code.
4. **The gate on comment-only edits:** !948 (a doc comment in
   `approval/tiers.rs`), and !905 and !907 (a page line in `approve.rs`).
   The gate keeps the passkey because a path can't tell a harmless edit from
   one that drops secrets out of the owner tier. A diff that changes only
   comments can't change behaviour, and that can be checked mechanically.

There was also a client-side stall. `change_close` is under
`.claude/settings.json`'s `ask`, so in auto mode a session waited on a Claude
Code permission prompt for about 6 hours. The control plane then evicted its
changes as "session gone". This happened to at least three sessions. The
sessions-stop-only-for-oversight ADR's 2026-11-16 amendment left whether
`change_close` leaves `ask` to the owner. The owner's request above settles
it.

Related: `2026-11-08-security-tiered-approvals-task-mandates.md`,
`2026-11-09-security-approvals-once-passkeys.md`,
`2026-11-15-security-standing-auto-owner-flags.md` and
`2026-11-15-development-sessions-stop-only-for-oversight.md`. This ADR amends
their tiers where it says so, and leaves the rest as it is.

## Implementation plan.

### 1. A session's own close is Auto's

- `tiers::own_close(change, session) -> bool` is true when every one of these
  holds:
  - `session` is the label on the call's verified ticket, and it equals
    `change.session`;
  - `change.state` isn't `Approved`, `Merged` or `Closed`, and
    `change.approval` is none.
- `windows::decide` takes the caller's session. For `change.close` with
  `own_close`, it allows the close under rule `own close`, at the `code`
  tier. A hold defers it, and a paused window asks, as for any other
  operation.
- `tier_of` doesn't change. Each of these stays `owner` for a pushed change:
  closing another session's change, closing an approved change, and a close
  when no window is running.
- **Invariant:** closing never deletes a change's Artifacts branch. A test
  pins it: a closed change produces no job that deletes its branch from
  Artifacts.
- `change.close`'s description says that a session may close its own
  superseded or refused change under Auto. It still never closes a generated
  change (the 2026-11-16 amendment).

### 2. A session's own front is Auto's, bounded

- `windows::decide` allows `change.front` of the caller's own change (the
  ticket's label equals `change.session`) under rule `own front`, when both
  of these hold:
  - the window is active;
  - fewer than `FRONT_RATE` = 4 fronts were allowed under the window in the
    last hour (`Window::fronts`, kept like `recent`).
- Past the rate, the front asks the owner, as today.
- **Fairness:** `queue::front` puts the change behind the kept changes and
  behind any change already marked `fronted`. A later front can no longer
  jump an earlier one. A front the owner asks for keeps the owner's place,
  ahead of the fronts that sessions made. Pausing Auto stops session fronts.
- Policy-driven fronting is left out for now, such as automatically fronting
  a change that blocks a deploy. Rate-limited own fronts cover the cases from
  that night.

### 3. Secrets code is the owner's only when its diff touches secret material

- `approvals.yaml` gains two keys:
  - `secret_code:` lists files that read a secret but don't hold one.
  - `secret_markers:` is a list of case-insensitive substrings: `secret`,
    `token`, `passphrase`, `password`, `private_key`, `credential`, `mint`,
    `grant`, `authorization`, `bearer`, `signing`, `p12`, `keychain`.
- The bound job sets `Facts::secret_material` when an added or removed line
  of a `secret_code:` file contains a marker. A renamed or deleted
  `secret_code:` file also counts.
- `classify` handles it this way:
  - a `secret_code:` file with `secret_material` gives `owner`
    (`secrets: <file>`);
  - without it, the file is guarded as code, which Auto covers unless it is
    on the gate.
- A false positive costs one passkey, as before. A false negative needs a
  line that changes secret handling with none of the markers on it, which
  this ADR accepts for code that holds no value.
- **Move to `secret_code:`:**
  - `src/pigeon-agent/src/worker/provider.rs`
  - `src/pigeon-agent/src/worker/platform.rs`
  - `src/pigeon-agent/src/platform_job.rs`
  - `src/pigeon-core/src/apple.rs`
  - `src/pigeon-agent/src/worker/apple.rs`
  - `src/pigeon-core/src/signing.rs`
  - `src/pigeon-harness/src/commands/signing.rs`
  - `tooling/ci/buildkite/ios-sign.sh`
- **Stay on `secrets:`, owner each time:**
  - `approvals.yaml`, `**/secrets.config.yaml`, `.mcp.json` and
    `.claude/settings*.json`;
  - the api-token and access-service-token modules;
  - `roles.yaml`;
  - `catalog/cloudflare/capabilities.yaml`. Every capability there is minted
    per run, so naming one widens what a token may carry, which is a
    credential grant (!956 included). Whether a capability that is only
    renamed may leave `secrets:` is an open question for the owner
    (`owner_ask`).
- `secret_lists`, `adds a secret`, `grants.yaml`, minting and
  `platform.apply` don't change.

### 4. A gate edit that changes only comments is Auto's

- The bound job sets `Facts::comments_only` when every gate file the head
  touches differs from the base only in comments:
  - `.rs`: the token streams after a lexer drops `//`, `/* */`, `///` and
    `//!` comments are equal. The job uses `proc_macro2`'s lexer. A file that
    doesn't lex counts as changed.
  - `.yaml`: equal after dropping whole-line `#` comments and trailing ` #…`
    outside quotes. `approvals.yaml` is excluded, because it is on `secrets:`
    and stays `owner`.
  - `.js` and every other kind of file count as changed.
- `classify`: a gate file with `comments_only` gives
  `guarded: <file> (comments only)` + `AUTO_COVERS`. A gate file without it
  stays `guarded: <file>`.
- Main's deployed checker computes the fact from the base and the head,
  never the head's own code, just as `appends_only` is computed.

### 5. The client prompt

- `.claude/settings.json` moves `mcp__pigeon__change_close` and
  `mcp__pigeon__deploy_retry` from `ask` to `allow`:
  - The control plane still tiers every close (§1).
  - `deploy_retry` redeploys `main`'s merged code, which is neither
    destructive nor a secret.
- These stay in `ask`: `change_pin`, `grant_revoke`, `grant_rotate`,
  `ci_cloudflare`, `platform_apply`, `infra_apply`, `sites_publish` and
  `ios_release`.

### What stays on the passkey

- Destructive acts: closing another session's change or an approved one,
  eviction, deletes in `infra.apply`, `platform.apply` or `sites.publish`,
  and teardowns.
- Secret values and keys, `adds a secret`, credential grants (minting,
  `grants.yaml`, `platform.apply`, `capabilities.yaml`) and `roles.yaml`.
- A gate diff that changes any code, `approvals.yaml`, `.github/**`,
  break-glass and `change.pin`.

### Execution

Every step but this ADR loosens an approval rule, so the owner approves each
one with a passkey. They touch the gate, `secrets:` or both. The planner
carries them out itself, because executors don't touch guarded paths.

1. This ADR, as one `docs(adr)` change. It loosens nothing by itself.
2. `feat(core)` (§1, §2): `tiers.rs`, `windows.rs`, `queue.rs` and
   `tests.rs`, and the worker passing the ticket's label into the window's
   verdict. Tests: an own close and an own front allowed; another session's
   close or front asks; an approved change's close asks; the rate is
   enforced; a later front stays behind an earlier one.
3. `feat(core)` (§3, §4 code): `Rules::secret_code`, `Rules::secret_markers`,
   `Facts::secret_material`, `Facts::comments_only`, and computing them in
   `jobs.rs` and `checks/ci.rs`. Tests replay !966 (code with no marker is
   covered), a marker line (`owner`), !948 (comments only, covered) and a
   one-token code change (asks).
4. `chore(config)` (§3 and §5): `approvals.yaml` gains `secret_code:` and
   `secret_markers:` and moves the files listed in §3, after step 3 deploys,
   and `.claude/settings.json` gets §5's edit.
5. `owner_ask`: may a capability rename leave `secrets:`?
