## Auto is standing: no expiry or merge budget, guarded paths leave it, `Accept edits` goes, and the owner is flagged only for what only they can do

* Date: 2026-11-15
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: security
* Governs: `src/pigeon-cli/crates/backplane/src/core/approval/windows.rs`, `src/pigeon-cli/crates/backplane/src/worker/app.rs`, `src/pigeon-cli/crates/backplane/src/tasks.rs`, `src/pigeon-cli/crates/backplane/src/worker/push.rs`, `src/pigeon-ios/Pigeon/HomeView.swift`, `src/pigeon-ios/Pigeon/Sheets.swift`, `src/pigeon-ios/Pigeon/Store.swift`, `src/pigeon-ios/PigeonCore/Models.swift`

## Context

The owner's goal: Auto is the default experience in the app and the
platform, not a window they open and renew. The platform builds without
interruption, and pauses and flags the owner only for:

1. destructive actions;
2. a real need for clarification: a decision only the owner can make;
3. owner action items: what only the owner can do, such as provider setup,
   credentials, account recovery, or an edit to `.claude/settings.json`.

Owner-tier changes keep the owner's passkey: secrets, guarded paths,
loosening an approval rule, and grants. They are oversight, not friction.

### What Auto is today

- **An expiring window,** `core/approval/windows.rs`, started in the app with
  a passkey for 1, 4, 8 or 24 hours and a 20-merge budget
  ([`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md)
  §5 and its step 6). When it expires or its budget runs out it ends, and
  every approval falls back to the owner. On 15 November the owner restarted
  it several times; window:6 to window:9 ran that night.
- **It covers every tier below `owner`,** guarded paths included: a guarded
  path is `code` tier, and `windows::judge` asks only for `Tier::Owner`. A
  task mandate cuts guarded and sentinel paths out
  (`mandates::decide`); a window doesn't. §5 named this as the residual risk,
  bounded by the window's length and budget.
- **A failed deploy holds it,** and a passed one resumes it (the same ADR's
  2026-11-15 amendment; !484, !486, !491, !494, !521). A hold ends the window
  at 6 hours or 3 failed deploys.
- **The budget undercounts.** `Windows::decide` counts a merge only for
  `change.merge`. Since the stop-only-for-oversight ADR §2, a `change_merge`
  on a checked head becomes a `change.approve`, and the window's approval
  queues the merge as its approver without asking about `change.merge`.
  Most merges under a window are made that way, and none of them counts.
- **The breaker** (the session-stages ADR §4) takes one stuck change out of
  Auto (`attended`); it runs in shadow (`breaker.would_trip`).
- **Mandates** ([`2026-11-08-security-tiered-approvals-task-mandates.md`](2026-11-08-security-tiered-approvals-task-mandates.md))
  run beside windows, scoped to an ADR's globs, guarded paths cut out. A
  change's mandate is asked before the window.

### Where `Accept edits` lives

**Only in the iOS app.** It is the left segment of the Approvals tab's switch
(`HomeView.swift`, `modeSwitch`): tapping it ends the running window
(`store.endAuto()`, `DELETE /v0/app/auto`). The iOS ADR §5 defines it as
"today's behaviour: each approval asks".

- **The control plane has no such mode.** Its non-Auto state is "no window
  running", where each approval is listed for the owner.
- **It isn't a Claude Code permission mode.** Pigeon sets none:
  `.claude/settings.json` has no `defaultMode`, and neither
  `tooling/session/session.sh` nor `core/session/launch.rs` passes a
  permission mode. A session runs in the mode its launcher chose. Claude
  Code's own `acceptEdits` is unrelated to pigeon's approvals and is
  unchanged here.

### What this builds on

- [`2026-11-15-development-sessions-stop-only-for-oversight.md`](2026-11-15-development-sessions-stop-only-for-oversight.md)
  §1 already says when a session stops for the owner. This ADR doesn't
  restate it. It gives §1's "owner-only decision" a way to reach the owner
  (§4 below), and makes Auto standing so that "a change at `needs: owner`" is
  only ever an owner-tier change.
- [`2026-11-15-security-owner-approvals-always-listed.md`](2026-11-15-security-owner-approvals-always-listed.md):
  every approval the owner owes is listed, pushed, and re-issued by the
  digest. Owner items reuse that path.

## Implementation plan.

### 1. Standing Auto

- **A window no longer expires and has no merge budget.** It runs, approving
  every operation it covers, until the owner pauses it, a hold or the rate
  breaker (§3) stops it. `hours`, `expires` and `used` stay in the record for
  the history. `hours` is `0` for a standing window, and `expires` equals
  `started`.
- **Starting one is the owner's, with a passkey,** as today (`window:start`,
  `POST /v0/app/auto`). `hours` is accepted and ignored, so the current app
  build keeps working until it updates.
- **Pausing is the owner's, with a passkey,** as ending is today
  (`window:end`, `DELETE /v0/app/auto`). It ends the window. Resuming starts
  the next one. The passkey on both sides stops a stolen device session from
  turning Auto on, or quietly off.
- **Approvals keep their name, `window:<id>`.** Each window records `by`, the
  owner who proved `window:start`, and `merged`, every change it merged. So
  every Auto approval in the event log, the digest's
  `merged_without_owner`, insights' `approver_kind` and the lifecycle's
  `AUTOMATIC` prefixes still names what approved it and, through the window,
  who granted that. A new prefix would touch each of those for no gain.
- **The migration.** The window running when this deploys becomes standing
  as it is: its expiry and budget stop applying at the next `settle`. The
  owner started it with a passkey, and this ADR's own approval is the
  owner's passkey on that change (Execution). A paused or ended window stays
  so, and the owner starts Auto once in the app. From then on nothing expires.

### 2. What Auto covers: guarded and sentinel paths leave it

A window that never expires can't rely on its length to bound the risk of
guarded code-tier changes. It now cuts out what a mandate cuts out.

- **`windows::judge` asks the owner** for `change.approve` and `change.merge`
  when the head's merge rule starts with `guarded` or `sentinel`, as
  `mandates::decide` does. Authorizing such a plan is still covered: the
  merge is the gate.
- **The owner tier is unchanged:** secrets, destruction, `secret_lists`
  growth, and anything `tier_of` calls `owner`, each time.
- **Loosening an approval rule** is a guarded path (`.config/approvals.yaml`,
  `.config/policies/**`, `.claude/settings*.json`, `core/approval/**`, the
  approval and mandate Workers), so it now always takes the owner's passkey,
  where a window used to cover it.
- **Grants** (`grant_request`) are minted by the owner on a page with a
  passkey, as before. No window covers them.
- **Deploying paths** (`infra/**`, migrations) stay covered, as a window
  covers them today. A failed deploy holds Auto (§3).

### 3. What still stops Auto

| | What happens | How it resumes |
| --- | --- | --- |
| A deploy of `main` fails or rolls back | **held**, as today: only `fix` and `revert` merges go through | by itself, on a passed deploy |
| The hold reaches 6 hours or 3 failed deploys | **paused** (no longer ended), pushed "Auto paused" | the owner resumes it with a passkey |
| The rate breaker: Auto has merged 48 changes in the last 6 hours | **paused**, pushed "Auto paused: 48 merges in 6 hours" | the owner resumes it |
| A change trips the breaker | that change leaves Auto (`attended`); Auto goes on | the owner answers `/attend/<n>` |
| A destructive act or an owner-tier change | that one operation asks the owner; Auto goes on | the owner's passkey on it |
| The owner pauses it | **ended** | the owner resumes it |

- **The budget becomes a rate breaker.** A cap that ends Auto is what made
  the owner renew it. A standing Auto needs a bound on how fast it changes
  `main` unseen. 48 merges in 6 hours is about double tonight's busiest
  stretch; the constants (`RATE`, `SPAN`) are in `windows.rs` and tested.
- **Every merge counts, once per change.** `Windows::decide` counts a change
  when the window allows its `change.merge`, or allows its `change.approve`
  and no hold defers its merge, since that approval queues the merge. A change
  already in `merged` isn't counted again, so a restack's re-approval isn't
  counted twice. Each count records its time, for the breaker.
- **Destructive acts stay as they are:** the `owner` tier for changes, and
  `.claude/settings.json`'s `ask` list for tools (`change_close`,
  `grant_revoke`, `platform_apply`, `infra_apply`, `deploy_retry`,
  `sites_publish`, `ios_release`, …). Neither pauses Auto. Each asks for
  itself.

### 4. Owner items: decisions and action items

The oversight ADR §1 lets a session stop for "an owner-only decision", but
gives it no way to reach the owner except its last message. A new item kind
does, beside approvals.

- **`owner_ask`** (`owner.ask`), an MCP tool at the `auto` tier:
  `{kind, title, detail, change?, options?, ticket}`.
  - `kind: decision`: a choice only the owner can make, with up to 4
    `options` and free text. The oversight ADR's table still applies: a
    choice the rules settle, or one a planner can make, is never filed.
  - `kind: action`: something only the owner can do, outside pigeon's reach:
    provider setup, a credential, account recovery, an edit to a guarded file
    an agent may not write (the oversight ADR's step 4).
  - Destructive acts aren't items: they are asked for by the tier or the
    harness prompt, each where it happens.
- **Kept on the `Instance`** under `/owner-items/`, each with `id`, `kind`,
  `session`, `change`, `title`, `detail`, `options`, `opened`, and once
  answered `answer` and `by`.
- **Flagged three ways,** as approvals are:
  - **a push** when it opens: "Decision needed" or "Action for you", with the
    title, opening the app's Approvals tab (`/approvals`);
  - **the digest:** an `items` field, one line each with its age, and
    `needs_owner` is true while one is open, so the hourly digest pushes again;
  - **the app:** the Approvals tab lists open items under "For you", above
    approvals. `GET /v0/app/home` gains `items`.
- **Answering.** The owner answers in the app, `POST /v0/app/items/<id>`
  with `{answer}`, signed by their device session. No passkey: an answer
  grants nothing. Only the owner's device answers: sessions sign in through
  Access too, so no MCP tool answers an item. The web page `/approvals` is
  guarded and unchanged. Whatever an answer leads to still goes through the
  tiers. A session may withdraw its own item (`owner_withdraw`), as when it
  sees an action done, such as a credential saved.
- **The session follows events.** `owner.answered` wakes it (`wake=agent`),
  with the answer. Until then it stops only the work that depends on the
  item (the oversight ADR §1) and says nothing more to the owner.
- **The digest's `stops`** line gains `decision d, action a`.

### 5. The app: `Accept edits` goes

- **The switch goes.** In its place, the Approvals tab shows Auto's state:
  - **On:** "Auto", since when and by whom, with Pause;
  - **Held:** why, and that it resumes when a deploy passes;
  - **Paused:** why, with Resume.

  Pause proves `window:end` and Resume proves `window:start`, each with a
  passkey. The hours sheet (`AutoSheet`) goes.
- **The banner** says "Auto" and the merges in the last 6 hours, not an
  expiry.
- **`AutoWindow`** keeps its fields, so `GET /v0/app/home` keeps its shape.
  The app stops reading `hours` and `expires`.
- **"For you"** lists owner items (§4) with their answer controls.
- **What relies on `Accept edits`:** only the switch's left segment, which
  ends the window. Pause replaces it. No control-plane route, stored state or
  session setting reads it, so nothing else migrates.

### 6. The audit trail

- Every Auto approval is run as `window:<id>`, and window `<id>` records
  `by`, `started`, `merged` and `why` it ended. The last 20 ended windows are
  kept, as today.
- Events: `auto.started` (the owner, by passkey), `auto.ended` (the owner's
  pause, or the cap), `auto.paused` (the rate breaker), `auto.held` and
  `auto.resumed`. Each names the window.
- The digest's `merged_without_owner` names each change and its window.
- `owner.asked` and `owner.answered` name the session, the item and who
  answered.

### Execution

1. **`feat(backplane)`, by this planner, the owner's passkey (!543):** this
   ADR; the amendments to the iOS ADR and the oversight ADR; `CLAUDE.md`'s
   rule line; and the code that loosens approvals, §1 to §3 in
   `core/approval/windows.rs` with its tests, `worker/mandates.rs` and
   `worker/app.rs`. It changes the owner's approval model, so the owner
   approves it with their passkey, never a window, and it is asked for once,
   as one listed approval. Every path it loosens is in this change. The steps
   below tighten nothing and loosen nothing.
2. **Brief, sonnet, `feat(backplane)`:** owner items (§4), with the control
   plane's side: `owner_ask` and `owner_withdraw` in `wire.rs` and
   `worker/mod.rs`, a new `core/owner/items.rs` and `worker/owner_items.rs`
   with its `Instance` routes, `owner.asked` and `owner.answered` in
   `events.rs` (`owner.answered` waking), `tasks.rs`' `items` and `stops`,
   `worker/push.rs`, and `worker/devices.rs`' `GET /v0/app/home` `items` and
   `POST /v0/app/items/<id>`, with tests. Two briefs if it doesn't fit one.
   Its change adds the new paths to `Governs`.
3. **Brief, sonnet, `feat(ios)`:** §5, the app. It waits for step 2's
   `items`.
4. **The owner:** add `mcp__pigeon__owner_ask` and `mcp__pigeon__owner_withdraw` to `.claude/settings.json`'s
   `allow`. The file is guarded, and an agent doesn't widen its own
   permissions (the oversight ADR's step 4). It is filed as the first owner
   item.

### Verification

- With Auto on, a night of sessions merges code-tier changes with no window
  renewed, and `merged` lists every merge, however it was asked for.
- A change to a guarded path waits, listed, for the owner, with Auto on.
- A failed deploy holds Auto, and a passed one resumes it. A 7-hour hold
  pauses it, and the owner resumes it from the app.
- A planner's `owner_ask` reaches the owner as a push, a digest line and a
  "For you" card, and the answer wakes the planner.
- The app has no `Accept edits`.

## Amendments

### 2026-11-15: Reviewed against !548: they agree

[`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md)'s
amendment "Glanceable tracker cards, and silent sessions hidden" (!548)
redesigns the Tracker's cards. This ADR's §5 changes the Approvals tab and
Auto's banner.

- **Different views.** !548 changes `TrackerView`'s rows, its change sheet
  and its filter chip. §5 changes `HomeView`'s Approvals tab, `AutoBanner`,
  `Store` and the hours sheet. The Tracker keeps showing `AutoBanner` above
  its cards, as it does today, now with Pause or Resume and no expiry.
- **`Needs you`** is still a change at `needs: owner`. Under standing Auto
  that is an owner-tier or guarded change, or a change no window covers.
  Owner items (§4) aren't changes, so they don't appear on the Tracker's
  rail. They are listed under "For you" in Approvals.
- **`Held`** is the hold's stage, as before. A paused Auto holds no change:
  each approval is listed for the owner.
- **Silent sessions:** a session waiting only on an owner item has no
  change at `needs: owner`, so the Tracker hides it as silent. The item is
  pushed and listed under "For you", so the owner still sees it.

!543 needs no change. !547, the app's step, touches none of !548's views.
### 2026-11-15: Review of !556 against this ADR: they agree

- **!556 carries out the learner ADR's 2026-11-15 countdown amendment**
  ([`2026-11-15-runtime-learner-practice-standalone-app.md`](2026-11-15-runtime-learner-practice-standalone-app.md)):
  the scan counts down 3-2-1 on a steady page, with a manual shutter.
- **What it touches:** `PigeonCore/LearnerCountdown.swift` (`ScanCountdown`),
  an overload of `readPage` in `PigeonCore/LearnerReader.swift`,
  `Pigeon/Learner/LearnerScanView.swift` and its test file.
- **It touches nothing this ADR decides:** not `HomeView.swift`,
  `Sheets.swift`, `Store.swift` or `Models.swift`; no Auto state, approval,
  owner item or route. The learner's two pre-approved writes (that ADR's §5)
  are unchanged, and the scan sends nothing.
- **The two agree, and neither changes.**

### 2026-11-15: Reviewed against !561: they agree

[`2026-11-15-runtime-learner-practice-standalone-app.md`](2026-11-15-runtime-learner-practice-standalone-app.md)
(!561) decides how the learner's standalone joins, stands up and carries
over a phone's practice, outside `infra/`, with its own router and sync
route.
[`2026-11-14-security-instance-members-roles-invitations.md`](2026-11-14-security-instance-members-roles-invitations.md)
is unchanged by it.

- **No shared surface.** The standalone serves no approvals, Auto or owner
  items. §5's Approvals tab, `AutoBanner` and "For you" are the platform
  app's, behind its device session.
- **Roles.** Owner items are answered through the platform app's device
  session (`POST /v0/app/items/<id>`), as approvals are. A learner's
  standalone has no route to them. Who may link a device is the members
  ADR's, unchanged here.

!547 needs no change.

### 2026-11-15: Reviewed against !556: they agree

[`2026-11-15-runtime-learner-practice-standalone-app.md`](2026-11-15-runtime-learner-practice-standalone-app.md)
(!556) gives the learner scan a 3-2-1 countdown and a manual shutter. It
changes the learner's scan screen and its `PigeonCore` model. §5 changes
the Approvals tab, `AutoBanner`, `Store`'s Auto and item calls, and
`AutoWindow`, `OwnerItem` and `Home` in `Models.swift`. Neither touches the
other's views or types. !547 needs no change.
