## Faster merges: measured, the release bot takes a merge turn, and the head's jobs overlap

* Date: 2026-11-16
* Model: Claude, planner session
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/**/worker/main_run.rs`, `src/**/worker/merges.rs`, `src/**/worker/github.rs`, `src/pigeon-agent/src/*/ci.rs`, `src/**/release/**`, `src/**/change/queue.rs`, `src/**/macos/job.rs`
* Governs: `src/**/worker/main_run.rs`, `src/**/worker/merges.rs`, `src/**/worker/github.rs`, `src/**/checks/ci.rs`, `src/**/release/**`

## Context

The owner asked how merges can be faster. On 2026-11-16 about 11 approved
changes sat in the one merge queue, and a batch took 30 to 45 minutes. This
ADR measures first, then decides.

It builds on
[`2026-11-08-development-concurrent-agent-sessions.md`](2026-11-08-development-concurrent-agent-sessions.md)
(the queue, restacks, carried approvals, kept turns),
[`2026-11-15-development-code-checks-by-reach.md`](2026-11-15-development-code-checks-by-reach.md)
(the gate's set) and
[`2026-11-16-development-src-crates-neutral-names.md`](2026-11-16-development-src-crates-neutral-names.md)
§7 (the release bot).

### What was measured

- **Source.** The event log (`GET /v0/events`), events 33758 to 35769,
  16:53 to 20:53 UTC: 20 merges, !770 to !810. Earlier events, !740 to !769,
  are past the log's retention; `main`'s history adds only their cadence.
  GitHub's own job times come from the Actions API, for !810's gate run and
  the 66 change runs from 15:43 UTC.
- **Head of the queue** means a change in `restacking` or `merging`: the
  serial part every queued change waits behind.

### Where the minutes go

- **Cadence.** Merges came a median 493 s apart (mean 680 s). From first
  approval to merge: median 675 s, p75 2744 s, longest 3670 s. Over the whole
  day, `main` took 37 merges and 13 `chore(release)` commits.
- **The head was busy 8008 s of the 13149 s window**, 400 s per merge. A
  change sat `queued` while the head was idle for another 1988 s.
- **The head's 8008 s, by job, as wall clock** (categories overlap; some job
  ran for 6637 s of it):

  | At the head | Jobs | Wall clock |
  | --- | --- | --- |
  | gate jobs (`gate: test`, `gate: clippy …`) | 28 | 3527 s |
  | the head's checks again, after a clean restack | 136 | 2107 s |
  | `bound` and `mirror`, one after the other | 49 | 1370 s |
  | `merge` (squash and push), 8 of them failed | 28 | 1017 s |
  | `restack` | 8 | 194 s |
  | no job running | | 1371 s |

  `gate: test` ran 9 times, median 248 s, longest 331 s.
- **The release bot races the holder.** It commits to `main` after each
  passed deploy, outside the queue (`control::Releasing`). The holder keeps
  `main` still only against other merges (the concurrent-sessions ADR's
  2026-11-11 amendment).
  - All 8 failed `merge` jobs (!772, !777, !778, !780 twice, !782, !795,
    !799) followed a `release.passed` that pushed a version or changelog
    commit. Each restacked, re-checked and gated again.
  - The lost attempts of !772, !777, !780, !795 and !799 held the head 399,
    418, 358, 443 and 483 s: 2101 s, 26% of the head's time. !778's and
    !782's ran 914 s and 1838 s, across reconciliations.
  - The bot's commits change `Cargo.lock` and a crate's `Cargo.toml`. Three
    reconciliations (!783, !786, !790) were for conflicts in `Cargo.lock`
    or `.config/releases.yaml`, and a `Cargo.toml` bump makes "main changed
    its files" for any change touching that crate.
- **The head waits on its own plumbing.**
  - `bound` (median 26 s) then `mirror` (median 25 s) run one after the
    other, and checks are dispatched only after `mirror`.
  - On GitHub, !810's gate jobs ran 42 to 49 s each: 8 to 14 s of clippy,
    the rest setup. `gate: test` ran 218 s, 175 s of it tests.
  - The control plane recorded those results 31 to 46 s after GitHub
    finished them. Dispatch itself is quick: runs are created within a
    second.
  - The 66 change runs took a median 219 s on GitHub, p90 387 s.
  - `ios` (macOS) ran 3 times: median 524 s, longest 979 s. It runs only for
    app changes.
- **Owner waits.**
  - 15 `change.approve` asks waited a median 362 s for the owner's tap (sum
    6878 s); 6 `change.merge` asks a median 809 s (sum 6918 s).
  - !782, !785, !789 and !794 were each approved 2 or 3 times, as restacks
    changed their own lines: 6 extra taps.
- **Agent latency.** 26 changes sat `approved` with `next` "ask to merge":
  median 97 s, sum 5855 s. It isn't head time, but each change waits it.
- **Deploys** don't hold merges. They reach the merge path only through the
  release bot's commits.
- **Docs-only changes** are already short: !805 held the head 102 s
  (restack 21, bound 21, mirror 22, merge 63).

### Options, against the data

Estimates are for the window's 20 merges.

| Option | Saves | Risk | Decision |
| --- | --- | --- | --- |
| Release bot takes a merge turn, coalesced while the queue is busy | ~1750 s of head time (2101 lost, less ~350 for 7 turns of ~50 s), and most bot-made conflicts | Releases lag up to 30 min under load | §1 |
| A restack resolves a `Cargo.lock` conflict that is only workspace version lines | up to one reconciliation each (3 today) | A wrong resolution; checked by the head's checks | §2 |
| `bound` and `mirror` overlap; GitHub polled every 10 s while a merge holds the turn | ~60 s per head attempt, ~1700 s over 28 | GitHub minutes spent when `bound` fails | §3 |
| A merge asked once is asked again when the head is approved again | median 97 s per change, not head time | None: approval is unchanged | §4 |
| One approval covers the merge on an unchanged head (owner tier) | median 809 s per owner-tier merge | Approval security | owner, §5 |
| Checks carry across a clean restack when `main`'s change is outside the change's reach | up to 2107 s of head time | Check coverage: an interaction outside the test map is caught only on `main` | owner, §5 |
| Merge trains: N queued changes gated together, bisected on failure | after §1–§3, a burst of 11 from ~41 to ~15 min | Complex; interacts with carried approvals and reconciliation | deferred, §6 |
| Gates on Cloudflare benches | up to ~30 s of setup per job | No data: benches are down until !810 deploys | deferred, §6 |
| No checks for docs/ADR-only changes | nothing: they already run none | | already so |
| Skipping `gate: test` when the test map clears the restack | up to 2460 s | Check coverage | not chosen without the owner; see §5 |

With §1 to §4 the head holds about 225 s per merge instead of 400 s: a batch
of 11 takes about 41 minutes instead of 73, before the owner's decisions.

## Implementation plan.

### 1. The release bot takes a merge turn

- `MainRun`'s release starts only while the merge queue grants it a turn,
  as a holder with no change (`merges::release_turn`). No merge starts its
  restack while it holds the turn, and it frees it when the release job ends,
  passed or failed, or after `HOLD`.
- The bot asks for a turn when its target has waited 30 minutes, or when the
  queue is empty. `Releasing` already keeps only the newest passed deploy, so
  a busy queue coalesces releases.
- So `main` moves only through the queue, and a holder's `merge` job is no
  longer refused for a bot commit.

### 2. A restack resolves a `Cargo.lock` conflict over version bumps

- When a restack conflicts only in `Cargo.lock`, and `main`'s side of that
  conflict changes only `version = ` lines of workspace packages, the
  restack takes the change's `Cargo.lock` and sets those lines to `main`'s.
- Anything else in the conflict stays a conflict, reconciled as today.
- The new head is checked as any clean restack's is.

### 3. The head's jobs overlap, and results arrive sooner

- After a restack, `bound` and `mirror` start together, and code checks are
  dispatched when `mirror` passes. A failed `bound` cancels them.
- While a merge holds the turn, the control plane polls its GitHub run every
  10 s, instead of the alarm's usual interval.

### 4. A merge asked once is asked again

- When a change whose session asked `change_merge` is approved again, by a
  window, a mandate or the owner, the control plane asks its merge again
  itself, as the 2026-11-15 kept turn does.
- The session still asks first; nothing merges that wasn't asked to.

### 5. Owner decisions

Filed with `owner_ask`; each becomes an amendment if she agrees.

- **One approval covers the merge** of an owner-tier change whose head and
  tier are unchanged since the approval. Approve and merge stay separate
  until she decides. Coordinated with the approval-policy session.
- **Checks carry across a clean restack** when no check the change selects
  has an input among `main`'s changed files, by the test map and
  `pigeon dev scope`. Until she decides, a clean restack re-runs them.
  Coordinated with the test-strategy planner.

### 6. Deferred, with what reopens them

- **Merge trains**: when, a day after §1 to §3 merge, a burst of 10 queued
  changes still takes over 30 minutes.
- **Gates on benches**: once !810 has deployed, when a bench's measured
  `gate: test` plus setup beats GitHub's 218 s.

### Execution

Steps 1 to 4 edit `src/pigeon-agent/**`, so they are held until the
restructure planner reports that step 12's a7, (e) and k3 have merged. !803
moves the agent's `cobot` module, so the briefs are written against `main`
then.

1. **This ADR**, with §5's two `owner_ask` items.
2. **Brief: the release turn** (§1), sonnet.
3. **Brief: `Cargo.lock` version conflicts** (§2), sonnet.
4. **Brief: overlap and polling** (§3), sonnet.
5. **Brief: the merge asked again** (§4), sonnet.
6. **Amendments** for the owner's answers to §5, each with its own brief.
7. **After.** A day after steps 2 to 5 merge, an amendment records the same
   measures: head time per merge by category, failed `merge` jobs, gap
   between merges, and approval to merge.

### Verification

- Replays of the window's sequences in the agent suite: a release during a
  held turn waits; !782's `Cargo.lock` conflict restacks clean.
- The step 7 measures, against this ADR's.

## Amendments

### 2026-11-16: The owner's answers to §5

The owner agreed to both of §5's decisions (items #19 and #20).

- **#19, one approval covers the merge of an unchanged head.**
  - The 2026-11-12 amendment to
    [`2026-11-09-security-approvals-once-passkeys.md`](2026-11-09-security-approvals-once-passkeys.md)
    already decided this, but every merge tap measured here was a
    `change.merge` ask for the head just approved.
  - That ADR's 2026-11-16 amendment records the defect and its fix. No carry
    rule widens.
- **#20, checks carry across a clean restack outside the change's reach.**
  Reach is computed as selection already computes it, per
  [`2026-11-16-development-one-test-binary-per-crate.md`](2026-11-16-development-one-test-binary-per-crate.md):
  - **Units are the test map's.** Since version 2, each crate's `suite` is
    mapped per top-level module, and `TestMap::select` resolves by module.
    A check is reached when `main`'s diff since the change's old base
    touches a file of any unit the check runs.
  - **`tests/suite/main.rs` and `tests/suite/harness/**`** reach every
    module of their crate's suite.
  - **These files always reach:**
    - any non-Rust file, unless `pigeon dev scope` maps it to no check of
      the change;
    - `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml` and `build.rs`;
    - a file the map doesn't know, which reaches its whole crate, as
      selection falls back.
  - **A reached check runs again,** and so does the gate's set for it.
    Checks nothing reaches keep their results, recorded as carried with both
    SHAs. Everything else about a clean restack is unchanged.
  - **A failed `test map` on `main` stops carrying,** as it makes gates full,
    until a run passes.

**Execution,** added to the steps, held as they are:

- **6a. Brief: the owner's merge ask** (sonnet), per the approvals ADR's
  2026-11-16 amendment. It goes after k3, with the owner's passkey.
- **6b. Brief: checks carried by reach** (sonnet), in the restack's carry
  rule and the gate's set. Replays: a restack over an ADR-only merge keeps
  every check; one over a `Cargo.lock` change re-runs them all. The tests
  planner reviews the brief.

### 2026-11-16: Review of !818 against !801: they agree

!801 amended
[`2026-11-15-security-owner-approvals-always-listed.md`](2026-11-15-security-owner-approvals-always-listed.md):
`ChangeRun` hands out the same page link while the head, tier and operation
still apply.

- **They agree.** This ADR changes nothing in that ADR's files
  (`worker/approve.rs`, `app/approvals.rs`, `approval/listed.rs`,
  `worker/listed.rs`), and no step here mints or lists an approval.
- **They meet at #19.** !801 makes one pending ask answer one link. #19 (the
  approvals ADR's 2026-11-16 amendment) removes the merge ask itself on a head
  the owner just approved. Its fix must leave !801's `wait` mapping as it is:
  `change.merge` on an unapproved checked head is shown as `change.approve`.
- **§4's re-ask** goes through `change.merge`, like a session's ask. When it
  needs the owner, it lists through !801's `Grants::page`, so it reuses the
  open link and mints none.
### 2026-11-16: Review of !812 against this ADR: they agree

- !812 adds `approvals.yaml`'s `gate:` under
  [`2026-11-15-security-standing-auto-owner-flags.md`](2026-11-15-security-standing-auto-owner-flags.md):
  Auto covers every guarded and eviction path outside the gate, and the gate
  (the approval core, `approve.rs`, `access.rs`, the Worker's `elicit.rs`,
  `mandates.rs` and passkey files, `approvals.yaml`, the secrets pins and
  `.github/**`) keeps the owner's passkey.
- This ADR governs `worker/main_run.rs`, `worker/merges.rs`,
  `worker/github.rs`, `cobot/ci.rs` and `release/**`. None of them is guarded
  or in the gate, so !812 changes no tier there. Its steps 1-4 stay as
  written, and Auto covers them as before.
- §5's first owner decision (one approval covers an owner-tier merge) would
  change the approval core and `approvals.yaml`, both in the gate, so that
  change takes the owner's passkey whatever she decides. It still needs her
  answer first, and it lands after the restructure's k3, which next edits
  `tiers.rs`.

### 2026-11-16: Step 2 lands, and two more steps: a waiting macOS gate yields the head, and a re-push keeps its place

- **Step 2 landed as !821.** The release bot takes a merge turn
  (`merges::release_turn`).
  - Its first turn at the head restacked it onto !797's library rename.
    The new test module still named the old library, so it failed clippy and
    gave its turn back. A one-line fix followed.
  - That re-push queued it last, so the owner moved it to the front a second
    time.
  - A brief that adds a test module now checks for an open rename of what it
    imports.
- **Measured tonight, after this ADR:**
  - **!804's merge-gate `ios` held the head from about 21:40 until it yielded
    by hand** (events 36527–36547).
    - GitHub gave no runner, Bitrise was refused for an unset
      `BITRISE_TOKEN`, and Buildkite for a token without `read_pipelines`.
    - The job then retried Buildkite every ~20 s while holding the head.
  - **A re-push sends a queued change to the back,** and loses the owner's
    front. That happens even when the re-push is only a rebase onto a parent
    that just merged, or a fix after a head failure. !803 was fronted 3 times
    and !821 twice, a passkey tap each.

#### 7. A merge-gate macOS check that waits gives up the head

- **A gate check waiting for a machine yields the turn, as `change_yield`
  does.** The change keeps its place, and takes the turn back when the check
  has a machine.
- **A credential refusal from every provider in the order makes the change
  `needs: owner`,** naming the missing secret or scope. Per !829's step 3 in
  [`2026-11-12-development-macos-builds-provider-failover.md`](2026-11-12-development-macos-builds-provider-failover.md),
  a provider refused for a missing secret is skipped, not counted as tried.

#### 8. A re-push keeps its place

- **A re-push keeps the change's place in the queue, and a front the owner
  gave it,** in either of two cases:
  - its own lines are unchanged apart from the restack, by the 2026-11-13
    `own_lines` rule of
    [`2026-11-08-development-concurrent-agent-sessions.md`](2026-11-08-development-concurrent-agent-sessions.md):
    a rebase onto `main`, or onto a parent that just merged;
  - it is the first push after a head failure lost the change its turn.
- **Any other re-push goes to the back,** as today.
- **A remembered place is used once,** and lapses after `RETURN` (60
  minutes: a fix, its checks and a re-approval can outlast `HOLD`). A second
  failure after a fix queues its next fix at the back.
- **This amends the concurrent-sessions ADR's 2026-11-16 rule** that a
  failed change's later push merges from the tail. Its first push after the
  failure keeps the place; a `ci_rerun` still queues at the tail.

**Execution,** added to the steps:

- **8. Brief: the macOS gate yields** (§7), sonnet.
- **9. Brief: a re-push keeps its place** (§8), sonnet.

Steps 3 to 5, 6b, 8 and 9 are briefed against `main` now. Those that edit
`src/pigeon-agent/**` are built and tested on their branches but pushed only
once the restructure's (e) and k3 have merged. A step whose files reach the
gate list (`worker/change.rs` once !812's follow-up merges) is carried by a
planner session, with the owner's passkey.

#### Corrections to §2 and §3, found while briefing

- **§2: the lock is regenerated, not edited.**
  - A package rename moves its whole `[[package]]` block, since
    `Cargo.lock` sorts by name, so setting version lines doesn't hold.
  - When `Cargo.lock` is the only conflicted path, the restack takes the
    change's lock and runs `cargo update --workspace --offline`, which
    rewrites only workspace entries. `cargo generate-lockfile` re-resolves
    registry packages, and `cargo metadata --offline` fails on uncached ones.
  - It accepts the result only when every registry and git package equals a
    three-way merge of the fork's, `main`'s and the change's locks. Anything
    else stays a conflict.
  - The pure check is `change::lockfile` in pigeon-core. `Governs` gains it
    once it lands.
- **§3: the overlap of `bound` and `mirror` is dropped.**
  - Mirroring before `bound` would publish a head `bound` hasn't checked.
    [`2026-11-15-development-code-checks-by-reach.md`](2026-11-15-development-code-checks-by-reach.md)
    forbids that: a head `bound` refused never leaves Cloudflare.
  - Its saving, about 25 s per attempt, doesn't justify loosening that gate.
    `bound`, then `mirror`, then checks keep today's order.
  - Only the polling remains: a change holding the merge turn finds and
    polls its GitHub run every 10 s (`actions::POLL_TURN`).
- **6b (#20): reach also decides a disjoint restack.**
  - Today checks carry across a disjoint restack without a look at `main`'s
    files. That can't re-run anything when `main` changed `Cargo.lock` and
    the change didn't.
  - So the reach rules above decide disjoint restacks too, which narrows the
    concurrent-sessions ADR's "checks carry across a disjoint restack".
  - A restack job too old to report reach keeps today's behaviour.
- **6b (#20): "a failed test map stops carrying".** `TestMaps::red` is set by
  a failed run on `main` and cleared by the next passed one. While it is set,
  or with no map at all, every check is reached.

### 2026-11-17: Docs-only changes take one turn together and skip `mirror`; GitHub load, trains and bench gates come back

The owner decided three things, relayed by the status session:

1. Docs-only changes skip CI and stop costing a full queue turn each.
2. GitHub load comes down by running fewer and cheaper checks, not bigger
   runners.
3. Merge trains and bench gates are designed now, not deferred.

- **Measured** (events to 37800, 43 merges, 16204 s of head time):
  - Today 23 of 65 merges were docs-only.
  - In the window, 11 docs-only merges held the head a median 127 s each
    (restack ~21 s, `bound` ~21 s, `mirror` ~22 s, merge job ~60 s): 2020 s,
    12% of head time.
  - They already run no GitHub check, no merge gate and no re-check.

#### 9. Docs-only changes share a turn

- **A docs-only change** is one whose bound scope selects no code check,
  only `policy.check`. Every change under `.claude/decisions/**` qualifies,
  and so does a generated `review-<n>-<m>`.
- **A head with no code checks skips `mirror`.** `main`'s own mirror after
  the merge still runs.
- **When a docs-only change takes the turn,** every other approved docs-only
  change waiting in the queue merges in the same turn, in queue order.
  - Each one is still restacked and `bound`-checked on its own.
  - A failure drops that change alone.
  - The turn ends when the last one has merged.
- **`bound` stays.** It is the secrets scan on a public repository, the
  scope check and the decisions policy (Governs globs, ADR format), at about
  20 s. Dropping it isn't safe.
- **Later:** a docs-only merge lands while a code change holds the turn.
  That waits for step 6b, because the holder's merge job then needs reach to
  skip re-gating over docs-only commits.
- **Expected:** today's 11 docs turns become about 3, saving about 1500 s of
  head time per window like this one.

#### 10. GitHub load

Briefed with the tests planner after step 6b:

- **The merge gate runs what reach selects.** It stops adding a full
  `cargo test --workspace` because `main` changed the change's files. Today
  that `gate: test` ran 9 times, median 248 s.
- **A clippy feature variant runs only when the change's files reach that
  feature.**
- **Fewer, cheaper jobs.** One test binary per crate, and several clippy
  variants in one job, cut the ~30 s of setup per GitHub job (clippy itself
  takes 8 to 14 s).
- **The amendment after it records GitHub minutes and wall-clock,** before
  and after.

#### 11. Merge trains and bench gates

No longer deferred (§6). Their design, safety argument and measured savings
come in their own amendment, once benches have been measured since !810, and
are briefed after steps 3 to 10.

**Execution,** added: **10. Brief: docs-only changes share a turn** (§9),
sonnet, core first.

### 2026-11-17: A transient push refusal keeps the turn

#### 12. A merge's push retries

- Per the concurrent-sessions ADR's 2026-11-17 amendment, the merge job
  retries a transient push refusal. A change still refused keeps its turn,
  and its merge runs again with no restack and no gate.
- **Measured:** !842 lost a passed gate run and its place to one
  `receive_pack_busy`.

**Execution,** added: **12. Brief: a merge's push retries** (§12), sonnet.
### 2026-11-17: Review of !851 against this ADR: they agree

!851 carries out step 3 (session check) of
[`2026-11-15-runtime-one-mcp-entry-point.md`](2026-11-15-runtime-one-mcp-entry-point.md).

- **What it touches.** v3 MCP session IDs name their door and caller, and
  `/mcp` refuses a mismatched ID. It changes `mcp.rs`, `worker/api.rs` and
  their tests.
- **What it leaves alone.** It touches neither `change/queue.rs` nor
  `macos/job.rs`, and changes no merge turn, queue place, yield or macOS
  gate.

Neither ADR changes.

### 2026-11-17: Review of !812 against !820: they agree

- !820 records the owner's answers above, and amends
  [`2026-11-09-security-approvals-once-passkeys.md`](2026-11-09-security-approvals-once-passkeys.md):
  an owner approval covers its head's merge, with no `change.merge` ask
  for the same head. It changes no code.
- !812 adds `approvals.yaml`'s `gate:` under
  [`2026-11-15-security-standing-auto-owner-flags.md`](2026-11-15-security-standing-auto-owner-flags.md).
  The gate keeps the owner's passkey on the approval core, `approve.rs`,
  `access.rs`, the Worker's `elicit.rs`, `mandates.rs` and passkey files,
  `approvals.yaml`, the secrets pins and `.github/**`.
- **6a** (the owner's merge ask) touches the approval code and so the gate. It
  takes the owner's passkey, as its step already says. Once it lands, an
  owner-tier change such as !812 asks once, not twice. !812 itself needed
  both taps.
- **6b** (checks carried by reach) lives in the restack's carry rule and the
  gate's set. That code is neither guarded nor in the gate, so Auto covers it
  as before.
- Neither changes what the other decides.
