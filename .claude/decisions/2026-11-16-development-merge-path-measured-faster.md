## Faster merges: measured, the release bot takes a merge turn, and the head's jobs overlap

* Date: 2026-11-16
* Model: Claude, planner session
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/**/worker/main_run.rs`, `src/**/worker/merges.rs`, `src/**/worker/github.rs`, `src/**/cobot/ci.rs`, `src/**/release/**`

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
