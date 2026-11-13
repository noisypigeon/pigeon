## `pigeon session`: a Rust follower whose polls stay under the proxy's 30-second cut, and the harness's hooks in Rust

* Date: 2026-11-13
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `tooling/session/session.py`, `.claude/settings.json`, `src/pigeon-cli/crates/backplane/src/worker/events.rs`

## Context

A cloud agent session learns that the control plane needs it from
`tooling/session/session.py follow`, armed in `Monitor`. The follower and the
hooks are
[`2026-11-13-development-session-stages-subscriptions-breaker.md`](2026-11-13-development-session-stages-subscriptions-breaker.md)
§2 and §3; the long poll is
[`2026-11-08-development-events-tickets-digest-mandate.md`](2026-11-08-development-events-tickets-digest-mandate.md)
§2. This ADR supersedes the stages ADR's §2 *follower script* and §3 *scripts*;
what they show, and when, stays.

### What was measured (2026-11-13, from a cloud session)

- **The cut is the session's own proxy.** The agent proxy terminates TLS to
  inject the host's credentials. A request it holds past ~30 s answers
  `502`, `text/plain`, body `upstream request failed`, with no `Cf-Ray`. A
  poll that finds events answers `200` at once with Cloudflare's headers.
  Cloudflare's own limit is ~100 s.
- **`seconds` is the only hold parameter.** `GET /v0/events` passes its
  query to `events::wait`, which reads `seconds` (default and cap
  `WAIT_MAX`). `wait`, which `session.py` sent until !365, was never read, so
  every quiet poll held 50 s and was cut: the "5 failed polls in a row".
- **!365 set `WAIT_MAX` to 25,** and `session.py` asks 25. Re-measured:
  `seconds=0` 0.5 s, `20` 20.8 s, `25` 25.9 s, `28` 25.8 s, all `200`. The
  ~0.9 s over is the deadline's whole seconds (`now()`) and a last look that
  sleeps a full 5 s past it. That leaves ~4 s under the cut.
- **No `pigeon` binary is in a session,** but `GET /builds/<sha>/pigeon`
  through the proxy fetched `main`'s 20 MB build in ~1 s, and its SHA-256
  matched `deploy.status`'s `latest_build.sha256`. It runs as is.
- **ureq 3,** already a workspace dependency, takes the proxy from
  `HTTPS_PROXY`/`NO_PROXY` by default and parses PEM roots, so the proxy's CA
  bundle (`SSL_CERT_FILE`) is enough. No `curl`, no token.

## Implementation plan.

### 1. The hold

- **`seconds` is the parameter.** Nothing adds `wait`.
- **The follower asks 20; the server caps at 25,** as now. A hold of `s`
  answers within `s` plus one call's latency, so 20 leaves ≥ 8 s under the
  cut, for one more request every ~100 s than 25.
- **The server's deadline is exact.** `events::wait` keeps a millisecond
  deadline, and its last look sleeps only until it (`events::look`: the
  next delay, or none when due). Today's whole seconds and fixed 5 s go.
- **No heartbeat.** The answer is one JSON document. A heartbeat would mean
  streaming it, and whether the proxy times idle or total is unknown. At the
  bound the server answers `events: []`, an ordinary answer.

### 2. `pigeon session follow`

**Where the code lives.**

- **Core:** `backplane/src/core/session/`, pure per
  [`2026-11-10-development-factory-crates-functional-core.md`](2026-11-10-development-factory-crates-functional-core.md)
  §3: the answer's parse, classification, backoff, the hold, and every line
  `follow`, `brief`, `statusline` and `stop` print. Time and random bytes are
  inputs.
- **Shell:** `pigeon`'s `cli/session.rs` and `commands/session.rs`, behind a
  `session` feature (`backplane`, `serde_json`, `ureq`), in the defaults.
  It reads files, the clock and the network, and calls the core.

**Transport.** ureq through `HTTPS_PROXY`, trusting `SSL_CERT_FILE`'s PEM
roots when it is set, else webpki's. The proxy injects the credentials, so
the binary holds none. `PIGEON_HOST` overrides `platform.pigeon.dev`.

**The loop.**

- `GET /v0/events?session=<label>&after=<cursor>&wake=agent&seconds=<hold>`,
  with a timeout of `hold + 15` s. The cursor starts at `session`, then is
  the last id printed, or `last` while nothing has been printed: at least
  once, as the stages ADR's step-2 amendment has it.
- **Each answer is classified** (`core::session::classify`, from status,
  `Content-Type`, whether `Cf-Ray` is present, and the body):

  | Class | When | Then |
  | --- | --- | --- |
  | `Answer` | `200`, JSON with `last` and `events` | print, poll again at once; empty is no events, not a failure |
  | `Cut` | `502`, no `Cf-Ray`, body `upstream request failed` | the hold halves, at least 5 s, and says so once |
  | `Refused` | `400`, `401`, `403`, `404` | backoff; unrecoverable after 10 minutes of nothing else |
  | `Server` | any other status | backoff |
  | `Unreachable` | a transport error, a timeout, or a `200` that isn't the answer | backoff |

- **Backoff** is full jitter: the n-th failure in a row waits a random
  `1..=min(60, 2^n)` s. The shell's randomness is std's `RandomState`.
- **Lines on stdout,** one per actionable event, exactly as `session.py`'s:
  `<id> !<n> <stage> needs=<who> <kind>: <summary>`. Anything else starts with
  `events:`, so a `Monitor` consumer can tell them apart:
  - the 5th failure in a row, then every 60th:
    `events: <k> failed polls in a row (<class>: <detail>); still trying`;
  - the first answer after such a line: `events: following again`;
  - a lowered hold: `events: the proxy cut a <s>s poll; holding <s/2>s`.
- **The status cache.** After an answer with events, or when the cache is
  over 60 s old, `session.status` (`POST /v0/run`) is written atomically to
  `~/.cache/pigeon/status.json`, in today's format.
- **Exit codes:** `1` with no `~/.config/pigeon/session.json` (printing the
  first start step, as today); `3` after 10 minutes of `Refused` and nothing
  else, printing why; `0` on SIGINT or SIGTERM. Nothing else ends it.

### 3. `brief`, `statusline` and `stop` move too

- `pigeon session brief|statusline|stop` print what `session.py`'s do, from
  the same cache, rendered by the core. `brief` asks the control plane for at
  most 10 s, then falls back to the cache.
- **Why move them:** one implementation and one cache format, tested
  without a network. `statusline` runs every few seconds, and a binary
  starts faster than Python.
- **Each exits 0 on any error,** so a hook never stops a session.

### 4. The binary in a session: `session.sh` and `session.py install`

- **`tooling/session/session.sh <command>`,** POSIX `sh`, is what the
  hooks and `Monitor` run. It execs the first of:
  1. `~/.cache/pigeon/bin/pigeon session <command>`;
  2. `pigeon session <command>` on `PATH`, for a developer's machine;
  3. `python3 -I tooling/session/session.py <command>`.

  A binary counts only if `pigeon session --help` succeeds, so a build from
  before step 3 falls through.
- **`session.py install`** (stdlib): reads `deploy.status`'s
  `latest_build`, fetches `/builds/<sha>/pigeon` through the proxy, checks its
  SHA-256, and renames it into `~/.cache/pigeon/bin/pigeon`, with the SHA
  beside it in `pigeon.sha`. It does nothing when that SHA is already there.
- **`session.sh brief`** runs `install` first, bounded to 10 s, inside the
  `SessionStart` hook's 20 s. Each start, resume or compaction takes `main`'s
  newest build.
- **Trust.** It is the build job Containers run, per
  [`2026-11-07-development-job-images-on-cloudflare.md`](2026-11-07-development-job-images-on-cloudflare.md)
  §3, served behind Access and checked against the control plane's SHA-256.
  Only `main`'s merges are built.

### 5. The switch

- **`CLAUDE.md`:** arm `Monitor` with `sh tooling/session/session.sh follow`.
- **`.claude/settings.json`:** the hooks and the status line run
  `sh "$CLAUDE_PROJECT_DIR/tooling/session/session.sh" <command>`, with the
  same timeouts. Guarded: the owner's, with the code.
- **`session.py` keeps working** throughout: it is the fallback, and its
  `follow` asks `seconds=20` once step 2 lands. Seven days after the switch,
  its four commands go, leaving `install`. The shim's fallback then prints
  the first start step, or nothing for a hook.

### Execution

Each step is one change.

1. **`docs(adr)`:** this ADR, and a pointer amendment in the stages and
   events ADRs.
2. **`fix(backplane)`: the exact deadline.** `events::look` and its tests;
   `worker/events.rs` uses it. `session.py` asks `seconds=20`.
3. **`feat(pigeon)`: `pigeon session`.** `core/session/` with table tests
   (§2's classes, backoff bounds, the hold, each line against
   `session.py`'s), and the shell. `Governs` gains their paths.
4. **`feat(tooling)`: `session.sh`, `session.py install`, and `CLAUDE.md`'s
   `Monitor` line.** After step 3's build is on `main`.
5. **`chore(claude)`: `.claude/settings.json` through `session.sh`.**
   The owner, with the code.
6. **`chore(tooling)`: `session.py` keeps only `install`,** seven days after
   step 5.

### Verification

- **Core, without a network:** the measured proxy `502` is `Cut`; a `502`
  with `Cf-Ray` is `Server`; `200` with `events: []` is an `Answer` with
  none; a `200` HTML page is `Unreachable`; `401` and `404` are `Refused`.
  Backoff stays in `1..=60`; the hold never drops under 5. Event lines match
  `session.py`'s for the same events.
- **Server:** `seconds=20` answers in under 21 s, and `seconds=99` in under
  26 s.
- **Live, in a cloud session:**
  - 30 minutes of `pigeon session follow` with nothing happening print
    nothing, and `session_status` shows it listening;
  - a `change_note` on the session's change prints within 6 s;
  - killed and re-armed, it prints what it missed, once.
- **Harness:** after step 5, a fresh session's `brief` comes from the Rust
  binary within the hook's 20 s, and with `~/.cache/pigeon` removed and the
  network blocked, the hooks fall back to `session.py` and exit 0.

### 2026-11-13: Review of !373 (grants) against this ADR

!373 carries out step 1c of
[`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md).
In `events.rs`, it adds one kind to those that wake an agent:
`grant.failed`.

- **The two agree.** This ADR decides how the follower reaches the event log
  and prints what wakes it. It doesn't decide which kinds wake. A
  `grant.failed` line prints in §2's format like any other.
- **A `grant.failed` event names no change and no session.** So a follower
  asking for one session's events doesn't see it. It reaches the owner, the
  digest and `grant-failed` tasks, as the grants ADR intends.
- **Nothing in !373 changes this ADR.**
