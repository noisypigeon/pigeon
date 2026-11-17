## Capacity pools: finite agent tasks on paid model gateways, the Claude Platform API's included credit first, under a hard spend cap

* Date: 2026-11-15
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/**/jobs.rs`, `src/**/brief/mod.rs`, `src/**/insights/mod.rs`, `tooling/session/models.json`, `src/pigeon-core/src/pool/**`

## Context

The owner's goal:

- Use the Claude API credit that comes with their Max plan as an extra pool
  of capacity for pigeon's execution work, beside their subscription
  sessions.
- Use it for finite tasks that run, complete and exit: executor briefs,
  review passes, checks and measurements.
- **A hard cap with no exceptions:** spend never exceeds the included
  credit. No card charge, no auto-reload, no overage, ever.
- **Other gateways later.** The Claude Platform API is the first use of a
  mechanism that other model gateways can join.

### What is proved: the premise, checked on 2026-11-15

The sources are [API credits for Max, Ultra, and Team plans](https://platform.claude.com/docs/en/about-claude/api-credits-for-subscribers),
the help article [Monthly API credits](https://support.claude.com/en/articles/17154008),
[Rate limits](https://platform.claude.com/docs/en/api/rate-limits),
[Workspaces](https://platform.claude.com/docs/en/manage-claude/workspaces),
[Pricing](https://platform.claude.com/docs/en/about-claude/pricing) and
[Agent SDK cost tracking](https://code.claude.com/docs/en/agent-sdk/cost-tracking).

| Question | Answer |
| --- | --- |
| Does Max include API credit? | Yes: **$100 USD a month**. Ultra gets $200; Pro gets none. |
| Is it automatic? | No. The subscriber links one Console organization, in claude.ai **Settings > Billing > API credits**, after 7 days on the plan, accepting the credit terms. Each plan links one organization, and each organization takes credit from one plan. **The link can't be changed by the owner; only support can change it.** |
| Does it renew, and does it expire? | Credit arrives each billing cycle, shortly after the plan's payment. Unused credit **expires at the cycle's end and doesn't roll over**. |
| What does it cover? | The Claude API (Messages, Message Batches), Managed Agents, the Agent SDK, the Console playground. `claude -p` and the Agent SDK count **when run with an API key from the linked organization**. |
| What doesn't it cover? | Interactive Claude Code, extra usage in the apps, and Bedrock, Vertex and Foundry. |
| What happens when it runs out? | It is spent before any purchased credit. With no other balance, **requests stop** ("Your credit balance is too low…") until the next credit. Usage is never charged to the Claude plan. **No payment method is needed.** |
| Is there auto-reload? | Only on the *purchased* balance, which it refills from a card. With no purchased balance and auto-reload off, nothing can be charged. |
| Can spend be limited per workspace? | Yes. A workspace has a monthly spend limit, Console **Settings > Spend limits > Workspace limits**. The Default Workspace can't have one. Organization limits always apply too. Over a limit, requests get a 400, "You have reached your specified workspace API usage limits". |
| When do limits reset? | **Spend limits reset on the 1st of the calendar month, UTC. Credit follows the billing cycle.** The two windows don't line up. |
| Are API keys scoped? | A key may be bound to one workspace. Each response names it in `anthropic-workspace-id`. |
| Is the SDK's cost authoritative? | No. `total_cost_usd` is a client-side estimate, and a step's `output_tokens` is a placeholder. Only each response's own `usage` is the API's count. |
| What are the terms? | Credits can't be transferred and are "used only by the holder of the account". pigeon runs the owner's own work. |

**So the premise holds, with five differences that shape this design:**

1. the owner must link an organization, once and for good;
2. the credit is use-it-or-lose-it each cycle;
3. it covers the API and Agent SDK with an API key, never interactive Claude
   Code, so pool tasks run headless with a key;
4. the strongest cap is the provider's own stop at a zero balance, with no
   purchased credit and auto-reload off;
5. spend limits run on the calendar month while credit runs on the cycle.
   That is why the workspace limit is $50, not $100 (§2).

Linking makes the organization at least the Start tier: a $500 tier cap, and
4,000 RPM for Sonnet 5.5 and Haiku 5.5.

**Prices, per million tokens, 2026-11-15.**

| Model | Input | Cache write 5m | Cache write 1h | Cache read | Output |
| --- | --- | --- | --- | --- | --- |
| `claude-sonnet-5-5` | $2.00 | $2.50 | $4.00 | $0.10 | $10.00 |
| `claude-haiku-5-5`, prompts up to 100,000 tokens | $0.10 | $0.125 | $0.20 | $0.01 | $0.50 |
| `claude-haiku-5-5`, prompts over 100,000 tokens | $0.50 | $0.625 | $1.00 | $0.05 | $2.50 |

These add to the table:

- **US-only inference** (`inference_geo: "us"`) is ×1.1.
- **Fast mode** is a premium, and only on Opus and Fable.
- **Web search** costs $10 per 1,000 searches.

### What exists

- **Job Containers.** `Checker` (standard-4) and `Runner` (standard-1) run
  `pigeon backplane jobs`, signed under `BACKPLANE_TOKEN`.
  - A job is a `control::JobSpec` whose `env` carries that job's credentials.
  - `jobs::run_judged` dispatches on `spec.job`, and the `Report` comes back
    through `ChangeRun`'s poll.
  - A Container has internet, and its environment holds only
    `BACKPLANE_TOKEN` and the build variables.
  - The [job images ADR](2026-11-07-development-job-images-on-cloudflare.md)
    and the [start backoff ADR](2026-11-15-development-job-start-failures-backoff.md)
    govern them.
- **Executors are harness sessions** a planner starts with `create_session`,
  per the [model-tiered briefs ADR](2026-11-13-development-model-tiered-agent-briefs.md).
  A brief's `budget` counts heads and hours, never tokens or money.
- **Nothing in pigeon calls a model API or holds a model key.**
  [`2026-11-15-development-archive-cleanly-ended-sessions.md`](2026-11-15-development-archive-cleanly-ended-sessions.md)
  refused an Anthropic credential that had no scope of its own. This ADR gives
  one key one scope: a workspace whose spend the owner capped.
- **Grants mint Cloudflare tokens only**
  ([`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md)).
  A provider's key enters as a Worker secret the owner puts, as
  `GITHUB_APP_KEY` does.
- **Insights** keeps hourly `cost` rows by `source` and `kind`
  ([`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md)
  §7).
- **The digest** has a precedent for a monthly cap: macOS minutes.
- **Access's Workers application has no path field,** so a route can't be
  bypassed alone. Job Containers read `/builds/` with a dedicated service
  token that `narrow_token` refuses every other path.

### What this rests on

- **Standing Auto and owner items**
  ([`2026-11-15-security-standing-auto-owner-flags.md`](2026-11-15-security-standing-auto-owner-flags.md)):
  - guarded paths leave Auto;
  - an owner-only act is filed with `owner_ask`.
- **When sessions stop**
  ([`2026-11-15-development-sessions-stop-only-for-oversight.md`](2026-11-15-development-sessions-stop-only-for-oversight.md)):
  - a spend or credential grant is the owner's;
  - a `wait:` refusal is not a stop.
- **Briefs** (the model-tiered ADR §3 to §6): the planner writes them,
  executors follow them to the letter, and the planner reviews before Auto
  approves.

### Threat model

- **Mistakes, not adversaries:**
  - a bug in the meter;
  - a stale price;
  - a loop that keeps calling;
  - a harness that retries;
  - a model choosing an expensive tool or model.
- **A model running arbitrary shell commands in a Container.** It may try to
  reach every credential in that Container, and the network.
- **The cap must hold when any one layer is wrong.**

## Implementation plan.

### 1. Pools and gateways

- **A pool** is one paid account at one model gateway, with a finite budget.
  It has a name, its gateway, its key (a Worker secret), its limits (§2) and
  its task budgets (§5).
- **A gateway adapter** is what differs between gateways. It is a Rust
  trait, `core::pool::Gateway`, with pure, tested methods:
  - `admit(request) -> Result<Admitted, Refusal>`: the endpoint, the model,
    and refused parameters and tools (§4);
  - `reserve(admitted) -> Micros`: a pessimistic upper bound on the
    request's cost;
  - `usage(response or stream events) -> Usage`: the gateway's own count;
  - `cost(usage, prices) -> Micros`;
  - `classify(status, body, headers) -> Signal`: for example `Exhausted`,
    `Limited`, `Unauthorized`, `WrongAccount`, `RateLimited` or `Ok`;
  - `inject(request, key)`: puts the key where the gateway wants it.
- **The first adapter is `anthropic-messages`,** the Claude Platform API's
  Messages API. A later gateway adds an adapter, a price table, a pool entry
  in `.config/pool.yaml`, a key, and a harness that speaks its format. The
  ledger, budgets, routing, Insights and digest don't change.
- **The harness is separate from the pool.** A task names a harness:
  - today, `claude-code`, which is `claude -p`, for any pool whose gateway
    speaks Anthropic's Messages format;
  - a gateway in another format needs its own harness entry.

  The router (§6) pairs a task only with pools its harness can use.
- **Money is integer micro-dollars (`Micros`),** never floats. Prices are
  kept in USD per million tokens, which equals micro-dollars per token.
- **One `Pool` Durable Object per pool** keeps its ledger in SQLite-backed
  storage.
  - It serialises every reservation, so concurrent requests can't overspend.
  - It is a new class with a `v4` migration tag.
  - Its pure logic is in `core/pool/`.

### 2. The hard cap: four layers, any one of which stops spend

| Layer | Who | What stops spend |
| --- | --- | --- |
| **A. Billing** | the owner, once (§10) | No purchased balance and auto-reload off, so the provider refuses at zero. A workspace limit of $50 a month. A key bound to that workspace. |
| **B. Pool ledger** | the `Pool` object | No new task above the **admit line**, 90% of a window. No request above the **stop line**, 97% of a window. |
| **C. Task budget** | the `Pool` object | No request beyond a task's dollars, tokens, requests or deadline (§5). |
| **D. Harness** | the job | `--max-budget-usd`, `--max-turns`, and the job's wall-clock timeout. |

- **Why $50, not $100.**
  - The workspace limit resets on the 1st of the month; credit resets on the
    billing cycle, which straddles two months.
  - If a purchased balance ever existed, a $100 limit could let $100 through
    in each month's part of one cycle: $200 against a $100 credit.
  - At $50, no cycle can pass $100 at the workspace.
  - pigeon can still use up to about $90 a cycle: $45 in each month's part.
- **Why the stop line is 97%.** It leaves room for rounding and a late price
  change. The provider's zero balance remains the final stop.
- **The windows.**
  - **`cycle`** runs from 00:00 UTC on the day after the date the Console
    shows as the credit's expiry, monthly. It starts no earlier than the
    credit, so an early reset only fails requests and charges nothing. Its
    limit is the credit, $100.
  - **`month`** is the calendar month, UTC. Its limit is the workspace limit,
    $50.

  A reservation must fit every window.
- **States:**
  - `open`;
  - `full`: past the admit line; in-flight tasks finish;
  - `capped`: at the stop line; every request is refused;
  - `exhausted`: the gateway said the credit is gone, until the cycle ends;
  - `limited`: the gateway said a spend limit was hit, until the month ends;
  - `paused`: by anyone, or by a tripwire;
  - `disabled`: by `.config/pool.yaml`.
- **Tripwires pause the pool and file an owner item** (`owner_ask`, `kind:
  action`):
  - the gateway reports `Exhausted` or `Limited` while the ledger's window is
    under half spent: the meter disagrees with the provider;
  - a response names a workspace other than the pinned one (§4);
  - a 401 or 403;
  - a reconciled cost above its reservation (`pool.overrun`): the estimate is
    wrong, so a bug is assumed.

### 3. The ledger: reserve, then reconcile

- **Per request.**
  - `reserve` happens before forwarding: the request's upper bound (§4) is
    added to `held`, and is refused unless every window and the task still
    fit.
  - After the response, `reconcile` replaces the hold with the cost
    computed from the response's `usage`.
  - **A response without a final usage**, such as a broken stream or an
    error after headers, is charged its full reservation.
- **Per task.** `admit_task` holds the task's whole dollar budget against the
  admit line at dispatch. A task's requests draw within its budget, so
  committed spend never passes 90% while the meter is right. `close_task`
  releases the unspent part.
- **Stale holds are charged.**
  - The object's alarm charges any request hold older than 15 minutes at its
    full amount.
  - It closes any task 10 minutes past its deadline, charging its open
    holds.
  - Nothing a crash leaves behind is ever forgotten as zero.
- **The record kept.**
  - Per window: `spent` and `held`.
  - Per task: budget, spent, held, tokens, requests, deadline, state.
  - Per hour and model: spent and tokens, for Insights. Rows are kept 90
    days.
  - Holds live in the object only.

### 4. The metering gateway: the only holder of the key

- **The route is `POST /v0/pool/<pool>/v1/messages`** and
  `…/v1/messages/count_tokens`, on the control Worker. Every other path under
  a pool is refused, including Batches and Files.
- **Who may call it.**
  - Access admits a dedicated service token, `POOL_ACCESS_ID` and
    `POOL_ACCESS_SECRET`. `narrow_token` refuses that token every path but
    `/v0/pool/`, and refuses `/v0/pool/` to any other identity.
  - The caller's `x-api-key` is a **task token**:
    `<pool>.<task>.<expires>.<HMAC>`, signed under a key derived from
    `SESSION_KEY` with the label `pigeon-pool-task-v1`. A task token is
    valid only while its task is open.
- **The key.**
  - The Worker puts the pool's key, `POOL_ANTHROPIC_KEY`, in `x-api-key`.
  - It forwards only `anthropic-version` and `anthropic-beta` from the
    caller.
  - The key is never in a Container, a job's env, a log, an event or a
    response.
- **`anthropic-messages` admission refuses:**
  - a `model` not in the pool's price table, or not the task's model or the
    class's helper model (`claude-haiku-5-5`);
  - `speed`;
  - `inference_geo` other than `global`;
  - `fallbacks`;
  - `mcp_servers` and `container`;
  - any tool with a `type` other than `custom`, which means no server tools;
  - `max_tokens` above 64,000.
- **The reservation.**
  - Input tokens are bounded by the body's byte length plus 4,096. A text
    token covers at least a byte; images bill far below their base64 size;
    the 4,096 covers the server's tool and system preamble.
  - Input is priced at the model's 1-hour cache-write rate, the highest
    input rate, and at Haiku's over-100,000 table when the bound passes
    100,000.
  - Output is `max_tokens` at the output rate.
- **The cost**, from `usage`:
  - `input_tokens`;
  - `cache_creation.ephemeral_5m_input_tokens` and `ephemeral_1h_input_tokens`
    (with no split, all at the 1-hour rate);
  - `cache_read_input_tokens`;
  - `output_tokens`;
  - Haiku's long-context table when input, cache reads and cache writes
    together pass 100,000;
  - ×1.1 if `inference_geo` is `us`;
  - $0.01 for each `server_tool_use` request.

  The last two are admitted never, and priced anyway.
- **A response's model is priced as named.** A model outside the table is
  priced at the table's most expensive model, and pauses the pool.
- **Streams pass through unchanged.** The Worker tees the SSE and reads
  `message_start` and the last `message_delta` for `usage`.
- **The pinned workspace.**
  - Every response's `anthropic-workspace-id` must equal the Worker secret
    `POOL_ANTHROPIC_WORKSPACE`.
  - A mismatch is charged, returned, and trips §2.
  - Until the secret is set, the pool refuses every request.
- **Signals** from errors:
  - "credit balance is too low" is `Exhausted`;
  - "specified workspace API usage limits", "specified API usage limits" and
    `enforced_spend_limit_reached` are `Limited`;
  - a 429 rate or ramp limit passes through to the harness, which honours
    `retry-after`.

### 5. Per-task budgets

- **Every pool task has four bounds:**
  - **dollars:** a hard cap on spend;
  - **fresh tokens:** uncached input, cache writes and output; cache reads
    are counted in dollars;
  - **requests:** the harness's turns, counted at the gateway;
  - **minutes:** a deadline, after which the task token stops working.
- **Defaults, in `.config/pool.yaml`, by class:**

  | Class | Dollars | Fresh tokens | Requests | Minutes |
  | --- | --- | --- | --- | --- |
  | `haiku` | $0.75 | 3,000,000 | 120 | 30 |
  | `sonnet` | $4.00 | 3,000,000 | 200 | 40 |

  A brief may lower them. It may raise dollars and minutes up to a
  per-class maximum: $8 and 60 minutes for `sonnet`, $2 and 45 minutes for
  `haiku`.
- **Opus never runs on a pool.** The planner is opus and stays on the
  subscription. A change to that is an amendment that shows the cap still
  holds.
- **On breach** the gateway refuses the request with
  `pool: task <id> spent its <bound>`. Claude Code exits on the error. The
  job's runner also kills the harness at the deadline. The task ends
  `stopped`, with its bound named (§7).

### 6. What runs on a pool, and routing

- **What is eligible:**
  - an executor brief of class `haiku` or `sonnet`;
  - later, read-only `review` and `measure` tasks (Execution step 8).

  Planner work, ADRs, owner-tier and guarded changes never run on a pool:
  they are never briefs (the model-tiered ADR §2).
- **A brief gains `runs_on`:** `any` (the default), `session` or `pool`. The
  planner sets it in `brief_create`. A brief that needs things only a
  harness session has, such as tools outside the Container or a person
  watching, says `session`.
- **The router** (`core::pool::route`, pure) runs at `brief_create` and when
  a brief's task opens again. It answers `pool:<name>` or `session`, with
  why:
  1. `runs_on: session`, or a class not allowed, means `session`.
  2. No pool is `open` for the class's harness, or none can admit the task's
     budget, means `session` (`pool full until <date>`).
  3. Every pool at its `concurrency` (default 2 running tasks) means
     `session` for `any`; a `pool` brief waits for a slot instead.
  4. The owner's `prefer` in `.config/pool.yaml` decides:
     - **`subscription`** (the default): use a pool when live executor
       sessions are at `subscription_slots` (default 3), or the cycle ends
       within `drain_days` (default 7) with more than its admit headroom
       left. Subscription usage is already paid; the credit expires
       unspent.
     - **`pool`**: use a pool whenever one admits the task.
  5. Among pools that fit, the one listed first in `.config/pool.yaml` wins.
     That is the owner's order.
- **`brief_create` answers `route`.**
  - On `session`, the planner starts an executor as today.
  - On a pool, the control plane runs it (§7), and the planner follows its
    events as for any executor.

### 7. Where a pool task runs: the `pool` job

- **The control plane is the executor's launcher.** For a brief routed to a
  pool, it:
  1. starts a ticket for the label `pool:b<n>.<k>`, with the brief's class,
     the brief's planner as `planner` and `parent`, and the brief;
  2. claims the brief's task, and proposes its change with `brief`. The
     change is the brief's, as any executor's is (the model-tiered ADR §4);
  3. opens the task in the `Pool` object (`admit_task`);
  4. dispatches a `pool` job on `Checker`.
- **The job's spec** carries in `env`:
  - the task token;
  - the pool's URL;
  - `POOL_ACCESS_ID` and `POOL_ACCESS_SECRET`;
  - the model ID from `tooling/session/models.json` at the brief's `base`;
  - the brief's latest revision;
  - its budgets;
  - a write token minted for this job's push to the change's branch, as the
    restack job's is.
- **The runner, `pigeon backplane pool-run`** (`pool_job.rs`), in order:
  1. Checks out the change's branch.
  2. Runs `claude -p` under a **separate uid in a user namespace**, with
     `newuidmap` as rootless BuildKit uses it. That uid can't read the
     runner's or the server's `/proc/<pid>/environ`, so `BACKPLANE_TOKEN`
     and the git token are out of the model's reach.
     - Its environment is cleared, then set to only:
       - `ANTHROPIC_BASE_URL` (the pool's URL);
       - `ANTHROPIC_API_KEY` (the task token);
       - `ANTHROPIC_CUSTOM_HEADERS` (the Access token);
       - `ANTHROPIC_MODEL`;
       - `ANTHROPIC_DEFAULT_HAIKU_MODEL=claude-haiku-5-5`;
       - `CLAUDE_CODE_MAX_OUTPUT_TOKENS=32000`;
       - `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`;
       - `HOME`, set to a temporary directory.
     - Its flags are `--max-turns`, `--max-budget-usd`,
       `--output-format stream-json`, `--permission-mode bypassPermissions`,
       `--disallowedTools WebSearch,WebFetch,Task`, and
       `--mcp-config` with one stdio server, `pigeon backplane pool-tools`.
     - That server's tools are `brief_show` (the spec's copy) and
       `pool_escalate {question, tried, evidence}`, which records an
       escalation and ends the run. It reaches no network.
  3. Its prompt is the executor rules of `CLAUDE.md` and the model-tiered ADR
     §4 for this brief, minus what the runner does itself:
     - it doesn't propose, push, run checks for the record, or call pigeon
       tools;
     - it escalates through `pool_escalate`.
  4. When `claude` exits, the runner runs the brief's `checks` itself,
     commits with the brief's `commit` and trailers, and pushes. Code
     decides that, never the model.
  5. Reports `Report.pool`: `outcome` (`done`, `escalated`, `stopped` or
     `failed`), `why`, `turns`, the harness's own `total_cost_usd` for
     comparison only, and the checks' results.
- **The job's timeout** is the task's minutes plus 15 for checks and push,
  under `cobot::ci::TIMEOUT`.
- **The image.** The `ci` image gains Claude Code at one pinned version
  (`npm install -g @anthropic-ai/claude-code@<x.y.z>`), a Dockerfile change
  at the deploying tier. Its version is raised by a change, never at run
  time, and auto-updates are off (`DISABLE_AUTOUPDATER=1`).
- **What each outcome does:**
  - `done`, checks green: the change follows its stages. The planner
    reviews (`brief_review`), and Auto approves on `accept`, as for any
    executor.
  - `escalated`: the control plane calls `brief.escalate` as the task's
    label, with the model's question. The planner answers as the model-tiered
    ADR §6 says. `brief_amend` dispatches a new pool task on the same change;
    `take` hands the change to the planner.
  - `stopped` (a bound, the cap, a tripwire) or `failed`: the runner pushes
    whatever was committed as `wip:` commits, and the change is
    `escalated` to the planner with the bound named. Nothing is lost, and
    nothing unreviewed merges.
  - `revise`: a new pool task, admitted afresh, on the same change. It
    counts against the brief's `heads`, as any revision does.
- **Each pool task's label ends** (`session.end`) when its task closes, so
  tickets don't pile up. Pool labels are not harness sessions, so there is
  nothing to archive.

### 8. Oversight: the same as an executor session

- **Tickets:** a pool task acts under its own label and class. Every check
  that refuses an executor refuses it (the model-tiered ADR §2).
- **Briefs:** it does a brief's steps and only them. Its change carries the
  brief's title, plan and scope. `bound` refuses a path outside it.
- **Review:** the planner reviews before Auto approves (the model-tiered ADR
  §5).
- **The stop rules:** a pool task never asks the owner anything. It
  escalates to its planner, which follows the oversight ADR §1.
- **The owner's passkey** is still asked for everything owner-tier. A pool
  task never reaches owner-tier work, since briefs never do.
- **Spend is the owner's.** Turning a pool on, raising its limits, its
  budgets or `prefer`, and resuming it after a tripwire are edits to
  `.config/pool.yaml` (guarded) or `pool.resume` (owner tier, passkey).
  Pausing is open to anyone, since it only takes power away.

### 9. Seeing it

- **`pool_status`** (read) shows each pool's state, windows (spent, held,
  admit and stop lines, reset dates), open tasks and their bounds, and the
  last 20 closed tasks.
- **`pool_pause {pool, why}`** is `auto` tier. **`pool_resume`** is `owner`
  tier, proved on the approval page with a passkey.
- **Insights.**
  - `insights::collect` reads each `Pool` object's hourly totals into
    `cost` rows: `source` is the pool's name, `anthropic` today, and `kind`
    is the model class.
  - Its `note` says the figure is metered from each response's usage at
    list price, not read from Anthropic's invoice.
  - `SOURCES` gains each pool's name. The app shows the source in the
    existing Cost tile and chart. If its source list is fixed, a `sonnet`
    brief for the app adds it.
- **The digest's `pool` lines**, one per enabled pool, for example: "pool
  anthropic: $12.40 of $100 this cycle (to 4 Dec), $8.10 of $50 this month,
  held $4.00; 6 tasks: 4 done, 1 escalated, 1 stopped (dollars)".
  - `needs_owner` is true, so the digest is pushed, only while a pool is
    `capped`, or a tripwire paused it.
  - `full`, `exhausted` at month's end, and every routine line are shown
    and never pushed.
- **Events:**
  - `pool.task.opened` and `pool.task.closed` (outcome, spent, bound) wake
    the planner;
  - `pool.state` (any state change);
  - `pool.overrun`.

### 10. Secrets and the owner's steps

- **New Worker secrets on `pigeon-control`,** each declared in `worker.yaml`
  with a comment, so the owner approves the list's growth:
  - `POOL_ANTHROPIC_KEY`: the workspace's API key;
  - `POOL_ANTHROPIC_WORKSPACE`: its workspace ID;
  - `POOL_ACCESS_ID` and `POOL_ACCESS_SECRET`: the pool's Access service
    token.
- **Each is put by the owner** with
  `pigeon cobot workers secrets put pigeon-control <KEY>`, the value on
  stdin. It is never in the repository, a change, a log or a chat. No
  operation returns it.
- **The owner's steps, in order,** each filed as an owner item when its
  turn comes (Execution):
  1. **Link the credit.** On claude.ai in a web browser: **Settings >
     Billing > API credits > Link organization**. Create a new Console
     organization used for nothing but pigeon, or pick one that is. The link
     is permanent unless support changes it. It needs 7 days on Max.
  2. **Check billing.** In the Console, **Settings > Billing**:
     - the $100 shows under **Promotional credits**;
     - the purchased balance is $0;
     - **auto-reload is off**;
     - no payment method is on file. Remove one if present.

     Tell the planner the credit's expiry date. It isn't secret; it sets
     `cycle` (§2).
  3. **Make the workspace.** **Settings > Workspaces > Create**:
     `pigeon-pool`.
  4. **Limit its spend.** **Settings > Spend limits > Workspace limits >
     `pigeon-pool` > Monthly spend limit: $50.** Also set the
     organization's **Monthly spend limit to $50**, since the organization
     is pigeon's alone.
  5. **Optionally, limit its models.** **Settings > Limits > Rate limits**,
     workspace `pigeon-pool`: set every model but Claude Sonnet 5.5 and
     Claude Haiku 5.5 to the lowest value the Console allows.
  6. **Make the key.** **Settings > API keys > Create key**, in workspace
     `pigeon-pool`. Copy it once.
  7. **Make the Access service token.** Zero Trust, **Access > Service
     credentials > Service Tokens > Create**: `pigeon-pool`. Add it to the
     `pigeon-control` application's policy as the builds token is
     (**Service Auth**).
  8. **Put the secrets,** from a pigeon checkout with `CLOUDFLARE_API_TOKEN`
     holding `pigeon cobot workers permissions`. Each prompts for the value
     on stdin, ended with Ctrl-D:

     ```
     pigeon cobot workers secrets put pigeon-control POOL_ANTHROPIC_KEY
     pigeon cobot workers secrets put pigeon-control POOL_ANTHROPIC_WORKSPACE
     pigeon cobot workers secrets put pigeon-control POOL_ACCESS_ID
     pigeon cobot workers secrets put pigeon-control POOL_ACCESS_SECRET
     ```

     The workspace's ID is on its page in the Console, `wrkspc_…`.
  9. **Approve turning it on:** the guarded change that sets
     `enabled: true` and `cycle_day`, with the owner's passkey.

### Execution

Each step is its own change, behind the one before unless marked parallel.
Steps 3 and 9 are the owner's passkey: secrets, guarded paths, and the
spend switch.

1. **This ADR.** `docs(adr)`, by this planner.
2. **`feat(backplane)`: the pure pool core**, in `core/pool/`, with its
   tests:
   - `config` (`.config/pool.yaml`'s types and validation);
   - `ledger` (windows, holds, tasks, admit and stop lines, the alarm's
     sweep);
   - `gateway` (the trait), and `anthropic` (admission, reservation, usage
     from JSON and SSE, cost, signals);
   - `route` (§6).

   Two `sonnet` briefs, with signatures and cases written by the planner:
   2a, `config`, `ledger` and `route`; 2b, `gateway` and `anthropic`.
   `Governs` gains `core/pool/**`.
3. **`feat(backplane)`, owner tier, by this planner:**
   - `.config/pool.yaml`, with `enabled: false`, and its policy
     `.config/policies/pool.yaml`, which pins its keys and the class
     maxima;
   - `.config/approvals.yaml`'s `guarded` gains `.config/pool.yaml`;
   - `worker.yaml`'s four secrets and the `Pool` class (`v4`);
   - `worker/pool.rs`: the `Pool` object and the gateway route;
   - `narrow_token` for `POOL_ACCESS_ID`;
   - `pool_status`, `pool_pause` and `pool_resume` in `wire.rs`.

   Disabled, it spends nothing. `Governs` gains `worker/pool.rs`,
   `.config/pool.yaml`.
4. **A spike, `tooling/spikes/pool-harness/`.** It checks that a `Checker`
   Container can run a pinned `claude -p` under a separate uid in a user
   namespace, unable to read the runner's `/proc/<pid>/environ`. It uses no
   key: `ANTHROPIC_BASE_URL` points at a local stub. The spike is read, then
   closed, as the images ADR's are. An amendment records the result. If the
   uid split fails, the plan stops here for an amendment.
5. **`feat(backplane)`: the `pool` job.**
   - `pool_job.rs`, `pool-run` and `pool-tools`;
   - `control.rs`' `POOL` job, its spec and `Report.pool`;
   - the Dockerfile's pinned Claude Code (deploying tier).

   One `sonnet` brief for the runner; the Dockerfile by this planner.
6. **`feat(backplane)`: routing and dispatch.**
   - `runs_on` in `core/brief`;
   - the router's call at `brief_create`;
   - the control plane's ticket, claim, proposal and dispatch for a pool
     task;
   - outcomes to `brief.escalate`.

   A `sonnet` brief. Parallel with step 7.
7. **`feat(backplane)`: Insights and the digest.**
   - the `cost` source per pool;
   - `Digest.pool` and its lines;
   - `needs_owner`.

   A `sonnet` brief, and an app brief if the app's sources are fixed.
8. **The owner's steps** §10.1 to §10.8, filed in order as owner items
   once step 6 merges. The planner withdraws each when it sees it done:
   - a request through the gateway answers, from step 8;
   - the expiry date, which the owner tells the planner, from step 2.
9. **`feat(backplane)`, owner tier: turn it on.** `enabled: true` and
   `cycle_day`. Then one live smoke task: a `haiku` brief with a $0.25
   budget, which edits one docs line. Its ledger is compared with the
   Console's **Cost** page for that hour.
10. **Later, by amendment:** `review` and `measure` tasks, read-only, whose
    report is a change note; and a second gateway.

### Verification

- **Core tests:**
  - a reservation fits only if every window and the task fit;
  - concurrent reservations never pass the stop line;
  - a broken stream is charged its reservation;
  - a stale hold is charged in full;
  - a task's unspent budget comes back on close;
  - a Haiku request over 100,000 tokens is priced at the higher table;
  - each refused parameter and server tool is refused;
  - each signal is classified from a recorded body;
  - the router's every branch;
  - `month` and `cycle` reset on their own dates.
- **Live, disabled:** with `enabled: false`, the route refuses every request,
  and no Insights row or digest line appears.
- **Live, the smoke task:** it runs, checks, pushes and stops at `in-review`.
  Its pool spend matches the Console's Cost page within one cent. A second
  task with a $0.01 budget ends `stopped (dollars)`.
- **The cap:** a test pool whose limit is set below one task's budget
  admits no task. A request past the stop line is refused with
  `pool: capped`.
- **Isolation:**
  - the spike shows the model's uid can't read the runner's environment;
  - no Container, log, event or report holds `POOL_ANTHROPIC_KEY`, checked
    by `grep` of the R2 logs for its prefix;
  - a task token stops working at its task's close.

## Amendments

### 2026-11-15: The owner is on Ultra: a $200 credit, $100 Console limits, and step 2 in three briefs

- **What changed.** The owner moved from Max to Ultra. Ultra's included
  Claude Platform credit is **$200 USD a month**, against Max's $100
  ([API credits for Max, Ultra, and Team plans](https://platform.claude.com/docs/en/about-claude/api-credits-for-subscribers)).
  - On an upgrade from Max, a prorated credit arrives at once, then $200 each
    billing cycle.
  - A linked organization stays linked.
  - Credit of exactly $200 isn't "over $200", so the organization stays at
    least the Start tier, with a $500 tier cap.
- **The numbers.** The rules of §2 are unchanged; their values follow the
  credit.

  | | Max (as written) | Ultra (now) |
  | --- | --- | --- |
  | `cycle` limit: the credit | $100 | **$200** |
  | Admit line, 90% of `cycle` | $90 | **$180** |
  | Stop line, 97% of `cycle` | $97 | **$194** |
  | Workspace monthly spend limit (`month`) | $50 | **$100** |
  | Organization monthly spend limit | $50 | **$100** |
  | `month`'s admit and stop lines | $45, $48.50 | **$90, $97** |

  - The workspace limit is half the credit for §2's reason: two calendar
    months fall in one billing cycle.
  - pigeon can use up to about $180 a cycle, about $90 in each month's part.
- **The hard rule holds.** Spend never exceeds the included credit.
  - Layer A is unchanged: no purchased balance, auto-reload off, no payment
    method. The provider stops at zero whatever pigeon's numbers say.
  - **The first, prorated cycle** may hold less than $200. pigeon's ledger
    assumes $200 from the first full cycle; step 9 turns the pool on no
    earlier than that. If it's on sooner, the provider stops at the real
    balance. A stop under half the ledger's limit trips §2's
    meter-disagrees tripwire, which pauses the pool and files an owner item.
  - **A later downgrade** to Max is the same case until `.config/pool.yaml`
    is changed back to $100 and $50, an owner-tier edit.
- **`.config/pool.yaml`.**
  - `anthropic.credit_usd` becomes `200` and `anthropic.month_limit_usd`
    becomes `100`.
  - The change is in step 3, the owner-tier change that makes the file
    guarded, so the new spend limit carries the owner's passkey.
  - !589 merged the file with Max's values. It is disabled and reads no
    key, so nothing can spend at either value until step 9.
- **The owner's steps, §10, with the new numbers:**
  - step 2 now expects **$200** under Promotional credits;
  - step 4 sets the workspace's **and** the organization's Monthly spend
    limit to **$100**.

  Nothing else changes.
- **Step 2 splits in three, and its first part lands.**
  - **2a:** b37, `core/pool/mod.rs` and `config.rs`, merged as !589.
  - **2a2:** b40, `ledger.rs` and `route.rs`, accepted as !615.
  - **2b:** b41, `gateway.rs` and `anthropic.rs`, after !615.

  Each later brief is stacked on the one before with `after`, so the merge
  queue holds them in order. `Governs` gains `core/pool/**` when 2b merges.

### 2026-11-16: Governs globs match the old and the new crate layout

Per [`2026-11-16-development-src-crates-neutral-names.md`](2026-11-16-development-src-crates-neutral-names.md) §6, each
glob under `src/pigeon-cli/` or `src/site-templates/` becomes `src/**/<tail>`
(or `**<file>` for a workspace root file), which matches the file both before
and after the restructure moves it. That ADR's step 12 tightens them to the
new paths.

### 2026-11-17: Step 3 is rebuilt on the restructured crates and the AI Gateway, in three changes

b41's change, !622, was approved and then closed unmerged when the src/
restructure moved `core/pool/`, and step 3 was never started. The
[AI Gateway ADR](2026-11-16-security-ai-gateway-claude-api.md) and the
[broker ADR](2026-11-16-security-provider-calls-mcp-broker.md) have since
changed what step 3 builds. This amendment restates it.

- **What is replaced.**
  - **The upstream.** §4's route forwards to the AI Gateway, never to
    `api.anthropic.com`, through `core::pool::aigateway` (the AI Gateway ADR
    §1, §5 to §7). The host is the `const` base in `worker/pool.rs`.
  - **The key.** The owner chose BYOK (the AI Gateway ADR's first
    amendment). `POOL_ANTHROPIC_KEY` is never declared or put; §4's "the
    Worker puts the pool's key" and §10.8's first line are void. The Worker
    sends no provider key, and drops the caller's `x-api-key`, which is the
    task token.
  - **The Worker's credential** is a one-hour AI Gateway Run token the `Pool`
    object mints from `GRANTER` (the AI Gateway ADR §3), held in memory only.
  - **The migration tag** is `v5`: `v4` went to `Bench`.
  - **Paths.** `core/pool/` is `src/pigeon-core/src/pool/`; the Worker's
    files are under `src/pigeon-agent/src/worker/`. `Governs` gains
    `src/pigeon-core/src/pool/**` now, and `worker/pool.rs` when 3b adds it.
- **The vetting order** is the AI Gateway ADR §11: the task token; then
  `core::provider::verdict::admit` (b59) with the capability `pool.messages`,
  the task label's ticket and the task's change; then `anthropic-messages`
  admission and the reservation. The `Caller` is built from the label's
  ticket (`Live`, `Missing`, `Lapsed`, `Revoked`) and the changes it holds;
  `platform` is false. A refusal at the ticket or change step closes the
  task `stopped (evicted)` and answers `403`.
- **It fails closed.** No gateway output, no Run token, a fetch error or a
  `5xx` from the gateway answers `502`, `pool: gateway unavailable`, and
  charges the reservation. There is no other upstream.
- **Signals.** A non-2xx response is classified by `aigateway::classify`
  first. `Upstream` is Anthropic's own error and goes to
  `anthropic::Messages::classify` (§4's signals). `GatewayNoKey`,
  `GatewayUnauthorized` and any other gateway code pause the pool and file
  an owner item; `GatewayLimited` passes the `429` through.
- **Step 3 splits in three, in order:**
  - **3a, `feat(core)`: the adapter.** b41, amended to revision 2:
    `src/pigeon-core/src/pool/gateway.rs` and `anthropic.rs`, as !622
    accepted them, on current `main`, after !827 (`aigateway.rs`). A
    `sonnet` brief.
  - **3b, `feat(agent)`, owner tier, by this planner:**
    - `worker/pool.rs`: the `Pool` object (`v5`), its ledger in storage, the
      alarm's sweep, the route `POST /v0/pool/<pool>/v1/messages` and
      `…/count_tokens`, the SSE tee, the workspace pin, and the vetting
      above;
    - `narrow_token` for `POOL_ACCESS_ID`;
    - `pool.status`, `pool.pause` and `pool.resume` (owner tier);
    - `worker.yaml`: the `Pool` class and three secrets,
      `POOL_ANTHROPIC_WORKSPACE`, `POOL_ACCESS_ID` and `POOL_ACCESS_SECRET`;
    - `.config/pool.yaml` gains the Ultra values (the first amendment) and
      joins `guarded`, with its policy `.config/policies/pool.yaml`.

    The Run token is asked through one function, `gateway_token`, which
    answers none until 3c, so every request is refused
    `pool: no gateway token`. With `enabled: false` it spends nothing
    either way. It touches `src/pigeon-agent/**`, so it is pushed only once
    the restructure's (e) and k3 have merged.
  - **3c, `feat(agent)`, the gate:** minting the Run token in
    `worker/minting.rs`, and its row in `.config/grants.yaml`. Both are gate
    files, so the owner's passkey approves it; it waits for owner decision
    #22 (the Run grant for `GRANTER`).
- **The AI Gateway ADR's step 5** is then 3b and 3c. Its planner keeps the
  live wiring and the spike's fixtures.
- **Verification** adds: a gateway `5xx` and a fetch error are refused and
  charged; no request reaches any host but the gateway's; a caller's
  `x-api-key` and `cf-aig-*` headers never reach it.
