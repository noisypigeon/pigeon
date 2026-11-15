## The owner sets the instance's open-change cap with `limits_set`, granted with a passkey

* Date: 2026-11-15
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/pigeon-cli/crates/backplane/src/sessions.rs`

## Context

The instance's cap on open changes is `backplane::sessions::MAX_OPEN`, a
constant. Each change to it has taken an amendment to
[`2026-11-08-development-concurrent-agent-sessions.md`](2026-11-08-development-concurrent-agent-sessions.md)
and a code change: 12, then 20 (2026-11-10), then 50 (2026-11-15, !599).

The owner intends to raise the cap in steps, each after its cost is read,
toward no cap. The 2026-11-15 amendment lists what each step watches. At one
ADR and one merge per step, a step takes hours of queue and a planner session.
The owner asked for a tool instead.

The cap's value is a spend decision. Per
[`2026-11-15-development-sessions-stop-only-for-oversight.md`](2026-11-15-development-sessions-stop-only-for-oversight.md),
a spend is the owner's act, so no session, window or mandate sets it.
`ci.window`
([`2026-11-10-development-code-checks-github-actions.md`](2026-11-10-development-code-checks-github-actions.md),
2026-11-11 amendment) already has this shape. An agent asks, the `Instance`
keeps an offer, the owner grants it on its page with a passkey, and the
`Instance` keeps the result.

## Implementation plan.

### 1. The cap is a value the `Instance` keeps

- `limits.rs` holds `Limits { max_open: Option<u32>, by, at }`. It is kept by
  the `Instance` beside the CI window. `None` means the default.
- **`MAX_OPEN` becomes `DEFAULT_MAX_OPEN` (50).** `sessions::limit` takes the
  cap as an argument; `propose` in `worker/mod.rs` reads it from the
  `Instance`.
- **`CEILING` (200) bounds it in code.** A value above the ceiling, or no cap,
  is refused with "takes an ADR". Removing the cap is the ADR that raises or
  drops `CEILING`.
- The floor is the number of changes open now. Lowering the cap never strands
  a change: it only refuses new proposals.
- `MAX_PER_SESSION` is not covered. It stays a constant.

### 2. `limits.set` and `limits.show`

| Operation | Effect | Who |
|---|---|---|
| `limits.set {max_open, why}` | Plan: stores an offer, answers its page's link | anyone may ask; the owner grants with a passkey |
| `limits.show` | Read: the cap, the default, `CEILING`, open now, and the last 10 sets | anyone |

- **The tier is owner, always.** No window, mandate or grant covers it. The
  page shows the cap now and the cap asked for, `why`, the open count, and the
  cost snapshot (§3). Its button is "Set the cap to N".
- An offer expires like a window's, after 2 hours.
- The MCP tools are `limits_set` and `limits_show`.

### 3. Each set records its cost snapshot

- **Event:** a granted set records `limits.set` with `{from, to, by, why}`.
- **Snapshot:** the event also carries a snapshot of the hour before:
  - the macOS provider minutes;
  - the GitHub Actions dispatches and their minutes;
  - job Container starts and failed starts;
  - `change.behind` events per `change.merged`;
  - Workers and Durable Object requests, where Insights has them.
- **Digest:** the next digest has a `limits` line with the change and its
  snapshot. It is shown, never pushed on its own.
- **Comparison:** `limits.show` compares the last set's snapshot with the hour
  since. This answers "what did this step cost" without an amendment.

### 4. What still takes an ADR

- going past `CEILING`, or removing the cap;
- changing `MAX_PER_SESSION`;
- changing what counts against the cap (`sessions::counts`).

### 5. Execution, as briefs

1. Sonnet: `limits.rs` (`Limits`, `Offer`, `CEILING`, the set's checks),
   `sessions::limit` taking the cap, and their tests.
2. Sonnet: `wire.rs`, `control.rs` and `worker/limits.rs`: the two
   operations, the `Instance`'s offer and grant, the page, and the owner tier
   in `core/approval/tiers.rs`.
3. Sonnet: the `limits.set` event, the snapshot, the digest line and the
   `limits.show` comparison.
4. Opus: set the cap once live through the tool, from 50 to 50 as a no-op
   grant, and amend this ADR with the result. Each later step is a
   `limits_set`, not an amendment.
