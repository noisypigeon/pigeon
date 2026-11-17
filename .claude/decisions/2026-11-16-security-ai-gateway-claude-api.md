## Claude Platform API traffic goes through one authenticated Cloudflare AI Gateway, behind the pool's metering route

* Date: 2026-11-16
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: security
* Governs: `.config/pool.yaml`, `infra/pigeon/config.yaml`

## Context

The owner's goal: every call pigeon makes to the Claude Platform API goes
through Cloudflare AI Gateway. That gives one place for:

- authentication;
- attribution;
- a spend backstop;
- logs that carry no prompt bodies by default.

It builds on:

- [`2026-11-15-development-capacity-pools-hard-spend-cap.md`](2026-11-15-development-capacity-pools-hard-spend-cap.md)
  (the pool ADR), for the metering route, the ledger, task tokens and the
  four cap layers;
- [`2026-11-16-security-provider-calls-mcp-broker.md`](2026-11-16-security-provider-calls-mcp-broker.md)
  (the broker ADR), for `provider.call`, its §4 checks, its registry's
  `exempt:` rows and per-call `GRANTER` tokens;
- [`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md),
  for `GRANTER` and the grants ledger;
- [`2026-11-04-development-platform-instance-separation.md`](2026-11-04-development-platform-instance-separation.md),
  for the platform's own account and the `infra/pigeon` unit.

### Where Claude is called (artifacts/main, 2026-11-16)

| # | Caller | State | Endpoint setting |
|---|---|---|---|
| M1 | The pool's metering route, `POST /v0/pool/<pool>/v1/messages` and `…/count_tokens`, in the control Worker | designed (pool ADR §4); `core::pool` has `config`, `ledger` and `route`; the gateway adapter (b41) and `worker/pool.rs` (pool step 3) haven't merged | none yet: the upstream is fixed in code. This ADR makes it the gateway. |
| M2 | The `pool` job's `claude -p` in a `Checker` Container | designed (pool ADR §7) | `ANTHROPIC_BASE_URL`, already set to M1. It never reaches Anthropic or the gateway itself. |
| M3 | Harness sessions: planners and executors on claude.ai | live | none. They run on the owner's subscription, inside Anthropic's own service. They are out of reach and out of scope. |
| M4 | Workers, jobs and CI | none | `ci.yml`, the Dockerfile and every Worker call no model. |
| M5 | Workers AI and agents | none planned | — |

`.config/pool.yaml` has `anthropic.enabled: false`. Nothing holds an
Anthropic key.

So **M1 is the one caller.** A later caller is routed through M1's route or
the same upstream builder, never straight to `api.anthropic.com`.

### What AI Gateway offers (Cloudflare docs, read 2026-11-16)

- **The Anthropic path.**
  - It is `https://gateway.ai.cloudflare.com/v1/{account_id}/{gateway_id}/anthropic/v1/messages`.
  - It takes `x-api-key` and `anthropic-version` as Anthropic does.
  - A real `x-api-key` "bypasses stored keys".
  - Source: [Anthropic provider](https://developers.cloudflare.com/ai-gateway/usage/providers/anthropic/).
- **Authenticated gateway.**
  - Requests carry `cf-aig-authorization: Bearer <token>`.
  - The token is a Cloudflare API token with **AI Gateway Run**.
  - Run "cannot be restricted to a single gateway": any Run token reaches
    every gateway in the account. For isolation, the docs say to use
    separate accounts.
  - Requests through a Worker binding are "pre-authenticated".
  - Turning authentication on can't be undone.
  - Source: [Authentication](https://developers.cloudflare.com/ai-gateway/configuration/authentication/).
- **Stored keys (BYOK).**
  - Keys live in the account's Secrets Store, named
    `{gateway_id}_{provider}_{alias}`.
  - They are used only when the request carries no provider key.
  - `cf-aig-byok-alias` picks a key; `default` is used otherwise.
  - Rotating a key takes no code change.
  - Source: [BYOK](https://developers.cloudflare.com/ai-gateway/configuration/bring-your-own-keys/).
- **Custom metadata.**
  - It is sent in `cf-aig-metadata` as a JSON object.
  - At most **five** entries, each a string, number or boolean.
  - `cf.*` keys are reserved.
  - Logs can be searched and filtered by it.
  - Source: [Custom metadata](https://developers.cloudflare.com/ai-gateway/observability/custom-metadata/).
- **Spend limits.**
  - A rule is a dollar budget over a fixed or rolling window.
  - It can be scoped by provider, model or a metadata key, split per value
    or filtered to one value.
  - At most 20 rules per gateway.
  - Over a limit, the gateway answers `429`; the default action is block.
  - Limits are "eventually consistent": a burst can briefly pass a limit.
  - They apply to BYOK requests for models with known pricing.
  - Source: [Spend limits](https://developers.cloudflare.com/ai-gateway/features/spend-limits/).
- **Rate limiting** is per gateway, fixed or sliding, and answers `429`
  ([Rate limiting](https://developers.cloudflare.com/ai-gateway/configuration/rate-limiting/)).
- **Caching** is off by default and only matches identical requests
  ([Caching](https://developers.cloudflare.com/ai-gateway/features/caching/)).
- **Request handling.**
  - `cf-aig-request-timeout` bounds the time to the first byte, so streams
    aren't cut.
  - `cf-aig-max-attempts`, `cf-aig-retry-delay` and `cf-aig-backoff` retry
    up to five times.
  - Source: [Request handling](https://developers.cloudflare.com/ai-gateway/configuration/request-handling/).
- **Dynamic routing** takes only the OpenAI chat-completions shape.
  Anthropic Messages gets a `400`
  ([Dynamic routing](https://developers.cloudflare.com/ai-gateway/features/dynamic-routing/)).
- **Logging.**
  - Logs are on by default.
  - `cf-aig-collect-log: false` skips a request's log entry.
  - `cf-aig-collect-log-payload: false` keeps metadata (tokens, model,
    status, cost, duration) and drops the request and response bodies.
  - Source: [Logging](https://developers.cloudflare.com/ai-gateway/observability/logging/).
- **DLP** scans prompts and responses with Cloudflare One's detection
  profiles. It either flags (the `cf-aig-dlp` header and the logs) or blocks
  (a `400`) ([DLP](https://developers.cloudflare.com/ai-gateway/features/dlp/)).
- **Terraform.**
  - [`cloudflare_ai_gateway`](https://developers.cloudflare.com/api/terraform/resources/ai_gateway)
    covers:
    - `authentication`, `byok_only` and `store_id`;
    - `collect_logs`, `log_management` and `log_management_strategy`;
    - `cache_ttl` and `rate_limiting_*`;
    - `retry_*`;
    - `dlp`, `guardrails` and `spend_limits`.
  - So the gateway is declarative state.
  - Whether the pinned provider, 5.28.0, has every field is checked in
    Execution step 3.

## Implementation plan.

### 1. Boundaries with the broker

- **Configuring the gateway is tofu, never `provider.call`.**
  - The gateway's settings, rate limit, spend limits, logging and DLP are
    declarative.
  - They live in a catalog module, `ai-gateway`, instantiated by the
    `infra/pigeon` unit.
  - Only the pipeline applies them (§10).
  - No MCP tool writes gateway settings.
- **Reading the gateway goes through `provider.call`.**
  - Logs and analytics are read through Cloudflare's MCP server.
  - The tools are read-effect, pinned in `.config/providers/mcp.yaml`, and
    credentialed per call from `GRANTER` (the broker ADR §4.5).
- **Model traffic is not a `provider.call`.** `POST /v1/messages` is a data
  path, like the broker ADR's G2 and C5:
  - its bytes are prompts and streams;
  - its authority is the pool's task;
  - its spend is metered by the `Pool` object.

  The registry lists it as an `exempt:` row, `ai-gateway-messages`:
  - `what`: POST `/v1/messages` to the gateway's `anthropic` path;
  - `why`: inference traffic, metered by the pool ledger;
  - `evidence`: `worker/pool.rs`.

  The host is a literal `const` base, `"https://gateway.ai.cloudflare.com"`, in
  `worker/pool.rs`. The URL is built from that constant, never inline in a
  `format!`. So the coverage test (the broker ADR §6 and its step 9) finds
  the host there and only there, and fails on `api.anthropic.com` anywhere.
- **No argument scan on model traffic.** The broker refuses
  credential-shaped arguments (its §4 step 6). That check belongs to
  `provider.call` alone, because prompts are forwarded as text.
- **The verdict is shared, not copied.**
  - The pool route calls the broker's ticket and eviction check before each
    forward (§11).
  - That is b59's `core::provider::verdict::admit(caller, capability,
    change)`: `verdict`'s capability, ticket and change steps alone.
  - The pool route doesn't reimplement it.
  - The task's ticket state and its change's state map into `Caller` and
    `change` the same way `provider.call` maps them. The tier step is not
    run, so no grant or proof is needed per request.
  - It is cited as b59 revision 4 (!802) until !802 merges.

### 2. Topology: one gateway, on the platform's account

- **One gateway per platform deployment**, on the platform's own account,
  beside the control Worker that is its only caller.
  - Its name is the deployment's, with `-claude` appended.
  - Its URL is built at run time from the deployment's outputs, so the
    account ID is never written in the repository.
  - Blue and green stacks each get their own, and teardown removes it.
- **Instances and standalones get none** until one runs a pool, by
  amendment.
- **Isolation by account.**
  - Run can't be scoped to one gateway, so the platform's account holds
    exactly one gateway.
  - The control Worker's minted token (§3) is the only Run token on that
    account.
  - The grants policy refuses to mint AI Gateway Run for any other grant.

### 3. Authentication

- **The gateway is authenticated** (`authentication = true`). An
  unauthenticated request gets a `401` from Cloudflare, before Anthropic.
- **The Worker's token.**
  - The `Pool` object mints one token from `GRANTER`, with AI Gateway Run
    only.
  - It expires in **one hour** and is renewed at 50 minutes.
  - It lives only in that object's memory, and is revoked when the next one
    is minted.
- **It never leaves the Worker.** It is never in a Container, a job's
  `env`, a log, an event or a response.
- **The `pool` job never sees the gateway.** It calls M1 with its task token
  and the pool's Access service token, exactly as the pool ADR §4 says.
- **Minting Run widens what `GRANTER` mints,** so it is a credential grant:
  - asked with `owner_ask`;
  - granted on `/grants/<id>` with a passkey;
  - recorded in the ledger.

  It is the broker ADR §5's grant, plus AI Gateway Run if that grant hasn't
  merged first.
- **If the Worker binding is pre-authenticated, the token goes.**
  - Execution step 2 checks whether an `ai` binding's
    `gateway(id).getUrl("anthropic")`, fetched from workers-rs, is
    pre-authenticated.
  - If it is, the Worker uses the binding with no token, and this section
    is amended.

### 4. The Anthropic key

The owner decides between two options (owner item, §12):

- **A. Stored in the gateway (BYOK), recommended.**
  - The key is in the account's one Secrets Store as
    `<gateway>_anthropic_default`, with `byok_only = true` and `store_id`
    set to that store.
  - The owner adds it once in the dashboard: **AI Gateway > the gateway >
    Provider Keys**.
  - The Worker sends no `x-api-key`. It removes any the caller sent.
  - So no pigeon process ever holds the key.
  - With `byok_only`, a missing key fails with `2044`; nothing falls back to
    Unified Billing.
- **B. A Worker secret.** `POOL_ANTHROPIC_KEY`, as the pool ADR §10 has it,
  is sent in `x-api-key` through the gateway. The gateway forwards it.

Either way:

- the key is the pool ADR's workspace-bound key;
- the pool ADR's other secrets stay as they are;
- no new secret enters the repository;
- no key, key ID, workspace ID or account ID is written in a file, a
  commit, a branch name, a change's text or a message.

`POOL_ANTHROPIC_WORKSPACE` stays a Worker secret. The pool still checks each
response's `anthropic-workspace-id` against it. Step 2 checks that the
gateway passes that header through. If it doesn't, the pin moves to the
gateway's log metadata, by amendment.

### 5. Attribution: `cf-aig-metadata`

- **Every forwarded request carries five entries**, all set by the Worker,
  never by the caller:
  - `pool`: the pool's name;
  - `task`: the pool task's ID;
  - `change`: the change's number;
  - `brief`: `b<n>`;
  - `class`: `haiku` or `sonnet`.

  A caller's own `cf-aig-*` headers are removed.
- **Where they surface:**
  - **Logs.** The gateway's logs can be searched by task or change.
  - **Insights.** `insights::collect` reads, through `provider.call`, the
    gateway's hourly cost by `class`. It writes a `cost` row with `source`
    `aigateway`, whose `note` says it is the gateway's estimate.
  - **Comparison.** That row is shown beside the pool's own row, which the
    ledger metered.
  - **`digest_show`** has one `aigateway` line: the hour's gateway cost,
    the ledger's, and the difference. A difference over 10% and over $1
    writes `aigateway.drift`.
  - The ledger stays the meter that counts. Neither the gateway's figure nor
    Anthropic's is replaced by the other.

### 6. Caching, rate limits and spend limits

- **No gateway caching** (`cache_ttl = 0`, and `cf-aig-skip-cache: true` on
  every request).
  - Agent turns are never identical requests.
  - A cached reply would bypass the ledger's usage.
  - Anthropic's own prompt caching is untouched, and still priced by the
    pool.
- **A rate limit as a backstop for runaway loops:** 120 requests per
  minute, sliding. That is far above two concurrent tasks.
- **Spend limits, a fifth layer under the pool ADR's four.** It is set to
  the owner's choice (§12). The recommendation:
  - a gateway-wide rule of $100 a month, fixed, the same as the workspace
    limit;
  - a $10-a-day rule split by the `task` metadata key.

  Because the limits are eventually consistent, they are a backstop. Layers
  A to D stay the hard cap.
- **At any gateway `429` or `2044` the pool fails closed:**
  - The request is charged its reservation.
  - `classify` answers `GatewayCapped`, `GatewayLimited`, `GatewayNoKey` or
    `GatewayUnauthorized`, from the response's recorded bodies.
  - On `GatewayCapped`, `GatewayNoKey` or `GatewayUnauthorized`:
    - the pool goes `paused`;
    - an owner item is filed (`owner_ask`, `kind: action`), as the pool ADR's
      tripwires are;
    - the digest needs the owner.
  - `GatewayLimited` (the rate limit) passes the `429` through. The harness
    honours `retry-after`.
- **There is no bypass.** If the gateway is down or returns a `5xx`, the
  request fails. The Worker never falls back to `api.anthropic.com`.

### 7. Retries and fallbacks

- **The gateway never retries** (`retry_max_attempts = 1`, and no
  `cf-aig-max-attempts`).
  - A retry inside the gateway is spend the ledger never reserved.
  - The harness retries, and each of its retries is a new, reserved
    request.
- **No model fallback.**
  - Dynamic routes don't take the Messages format.
  - Pool admission already refuses any model the task didn't name.
- **`cf-aig-request-timeout: 120000`.** A first byte must come within two
  minutes; streams then run on.

### 8. Logging and retention

- **Logs are on** (`collect_logs = true`), with metadata only by default:
  - every request carries `cf-aig-collect-log-payload: false`;
  - no prompt or response body is stored.

  Storing bodies is the owner's choice (§12). It is a guarded edit of
  `.config/pool.yaml`'s `anthropic.gateway_payloads`, at owner tier.
- **Retention.** `log_management = 100000` entries,
  `log_management_strategy = DELETE_OLDEST`.
- Logpush, OpenTelemetry export and the Stripe integration are off.

### 9. DLP and guardrails

- **DLP flags and doesn't block.**
  - It uses Cloudflare One's predefined credentials profiles, on requests
    and responses, looked up by name in the unit, never by an ID in the
    repository.
  - A flagged response's `cf-aig-dlp` header writes `aigateway.dlp`
    (task, change, profile name), which shows in the digest.
  - Blocking would end tasks over test fixtures. The pool's job sandbox
    (the pool ADR §7) is what keeps real secrets out of a prompt.
- **Guardrails are off.** Content moderation doesn't fit code work.

### 10. Deploying it

- **A catalog module, `catalog/cloudflare/modules/ai-gateway/`,** wraps
  `cloudflare_ai_gateway`. It looks up the Secrets Store with the
  `secrets-store` module and the DLP profiles by name.
- **`infra/pigeon/config.yaml` gains `ai_gateway:`**, with the settings
  above and the owner's limits. Its outputs, the gateway's name and URL
  parts, join `DEPLOY_OUTPUTS`.
- **Only the pipeline applies it:** `pigeon infra plan` in a change, then
  MainRun's apply. Nobody runs `tofu apply` or `pigeon infra apply` by hand.
- **The Worker's binding is a `pigeon workers` deploy.** That covers the
  `ai` binding, if step 2 keeps it, and the `AI_GATEWAY` var from
  `$output`.
- **If 5.28.0 lacks a field** this ADR uses, `pigeon infra lock` raises
  the pin in its own change first.

### 11. Vetting a model call

Before forwarding, the metering route checks, in this order:

1. **The task token**, per the pool ADR §4: its signature, that it hasn't
   expired, and that its task is open.
2. **The broker's ticket and eviction check** (the broker ADR §4 step 2),
   by `core::provider::verdict::admit`:
   - the task's label (`pool:b<n>.<k>`) has a live ticket;
   - that label holds the task's change;
   - the change isn't evicted or closed.

   A refusal closes the task as `stopped (evicted)` and answers `403`,
   `pool: change <n> evicted`.
3. **Admission and reservation**, per the pool ADR §4.

Model calls need no tier beyond the route's capability. Their spend is the
pool's, and the pool's switch is the owner's.

### 12. What needs the owner

- **Decisions,** filed with `owner_ask` on the change for this ADR:
  1. where the key lives (§4, A or B);
  2. the gateway's spend limits (§6);
  3. whether bodies are logged (§8).
- **The grant:** AI Gateway Run for `GRANTER` (§3), on `/grants/<id>`.
- **With A, the key:** added in the dashboard's Provider Keys, after the
  gateway exists. This replaces putting `POOL_ANTHROPIC_KEY` in pool step
  §10.8.
- **Approving the guarded and Worker changes,** with a passkey: steps 3, 4
  and 6.

### Execution

1. **This ADR.** (planner)
2. **A spike: `tooling/spikes/aig/`.** It runs after step 3 deploys a
   gateway, with no Anthropic key, and records:
   - an unauthenticated request's status and body;
   - a request with a Run token and no key: `2044` under `byok_only`;
   - a spend-limit `429` and a rate-limit `429`, from a $0.01 rule on a
     scratch model filter;
   - whether `anthropic-workspace-id` passes through (it needs a key, so
     after §12's key, with one `count_tokens` call);
   - whether workers-rs's `ai` binding is pre-authenticated.

   The bodies become `classify`'s fixtures. An amendment records the
   results. (planner)
3. **The `ai-gateway` module and its `infra/pigeon` entry**, disabled for
   traffic until step 6:
   - `pigeon infra validate`, `render` and `plan`;
   - the provider pin checked.

   (brief, `sonnet`; the config values by the planner after the owner's
   answers; owner approves)
4. **`core::pool::aigateway`, pure:**
   - the upstream URL;
   - the forwarded header set: `cf-aig-metadata`, `skip-cache`,
     `collect-log-payload`, `request-timeout`, and `x-api-key` stripped
     under BYOK;
   - the caller's `cf-aig-*` headers stripped;
   - `classify` for the gateway's signals;
   - table tests.

   (brief, `sonnet`; the fixtures from step 2 are added by amendment)
5. **The Worker:** `worker/pool.rs` forwards through the gateway, with the
   token's minting and renewal and the shared ticket check. It is held
   until:
   - pool step 3;
   - b59 (`core::provider::verdict::admit`);
   - the src/ restructure's step 12 items a7, (e) and k3.

   (brief; Worker, owner approves)
6. **Insights and the digest:** the `aigateway` cost source, the
   `aigateway.drift` and `aigateway.dlp` events, and the digest line,
   through `provider.call`. It is held until the broker's step 5.
   (brief, `sonnet`)
7. **Live:** one pool smoke task (the pool ADR's step 9) through the gateway.
   Its log shows the five metadata entries and no body, and the ledger and
   the gateway agree within one cent. (planner)
8. **This ADR is amended with the outcome.** (planner)

## Amendments

### 2026-11-16: The owner's answers: BYOK, no gateway spend limits, metadata-only logs

The owner answered §12's three decisions, owner items #16 to #18 on !807.

- **#16, the key: A, stored in the gateway (BYOK).**
  - §4's option A applies: `byok_only = true`, and `store_id` is the
    account's one Secrets Store.
  - The Worker sends no `x-api-key`, and removes any the caller sent.
  - `POOL_ANTHROPIC_KEY` is never declared or put. The pool ADR's §10.8 drops
    that one secret. In its place, the owner adds the key under the gateway's
    **Provider Keys**, filed as an owner item once step 3 has deployed the
    gateway.
  - Option B is not built.
- **#17, spend limits: none at the gateway.**
  - §6's spend-limit rules are not set: `spend_limits` is absent or
    `enabled = false`.
  - The pool ADR's four layers (billing, ledger, task budget, harness) are
    the hard cap. The gateway adds no fifth.
  - `GatewayCapped` is dropped from `classify`. A gateway `429` is
    `GatewayLimited`, the rate limit, and passes through for the harness's
    `retry-after`.
  - `GatewayNoKey` and `GatewayUnauthorized` still pause the pool and file an
    owner item.
  - The 120-requests-per-minute sliding rate limit stays, as the backstop
    for runaway loops.
  - Step 2's spike drops its spend-limit case.
- **#18, logging: metadata only.**
  - §8 stands: `collect_logs = true`, and every request carries
    `cf-aig-collect-log-payload: false`.
  - `anthropic.gateway_payloads` isn't added to `.config/pool.yaml`. Turning
    bodies on later takes an amendment and an owner-tier change.

Steps 3 and 4 take these values.

### 2026-11-16: The gateway is an optional `ai_gateways` list on the `pigeon/platform` kind, and DLP gets its own step

Reading the platform kind for step 3's brief corrected §10.

- **Where it is configured.**
  - The platform's units are in the root `config.yaml`: `platform/blue`, of
    kind `pigeon/platform`. `infra/pigeon/config.yaml` is the account's
    bootstrap, so it gains nothing.
  - The kind's schema, `catalog/pigeon/kinds/platform/schema.json`, gains an
    optional property, `ai_gateways`: a list of at most one
    `{name, rate_limit_per_minute}`.
  - The kind expands it with `x-pigeon.each` into
    `catalog/cloudflare/modules/ai-gateway`:
    - `store_id` comes from the existing `secrets_store` singleton;
    - the output `ai_gateway_names` joins `DEPLOY_OUTPUTS`.
  - A unit with no `ai_gateways`, such as an app standalone, gets no
    gateway. That is §2's "instances get none".
  - `platform/blue` gets
    `ai_gateways: [ { name: pigeon-claude, rate_limit_per_minute: 120 } ]`.
- **The module sets §3, §6 to §8 and the first amendment:**
  - `authentication = true`;
  - `byok_only = true`, with `store_id`;
  - `collect_logs = true`;
  - `cache_ttl = 0`;
  - the rate limit, sliding over 60 seconds;
  - `retry_max_attempts = 1`;
  - `log_management = 100000`, `DELETE_OLDEST`;
  - `logpush = false`;
  - no `spend_limits`, `dlp`, `guardrails`, `otel` or `stripe`.
- **DLP moves to a new step 3b, after step 2.**
  - Looking up Cloudflare One's predefined profiles by name, with the pinned
    provider, is unverified. Writing a profile's ID into the repository is
    what §9 rules out.
  - Until 3b merges, DLP is off. §9's flag-only design stands for it.
- **`Governs` gains `catalog/cloudflare/modules/ai-gateway/**`** when step 3
  merges, by this planner.

### 2026-11-16: The gateway is provider-neutral, `pigeon-models`, with Anthropic as its first provider

The owner asked that the gateway be named generally, since models from
other providers may join it later.

- **The name.** The gateway is `pigeon-models`. It is no longer the
  deployment's name with `-claude` appended (§2). `platform/blue`'s
  `ai_gateways` entry carries it (!843, brief b62 revision 5).
- **What was already neutral stays as it is:**
  - the module `catalog/cloudflare/modules/ai-gateway`;
  - the kind's `ai_gateways` property and its `ai_gateway_names` output;
  - the `aigateway.run` capability (!848);
  - the `cf-aig-metadata` keys (`pool`, `task`, `change`, `brief`,
    `class`), which name no provider.
- **Anthropic is the first provider, not the only one.**
  - Its key is the gateway's stored key under provider `anthropic`, alias
    `default`, per the first amendment.
  - A later provider adds:
    - its own stored key in the same gateway;
    - its own `Path` values in `core::pool::aigateway`;
    - its own adapter behind the capacity-pools ADR's `Gateway` trait.

  The gateway, its token, its metadata and its logging rules are shared.
- **This ADR's title and filename keep "Claude API".** They record the
  decision as made. A merged ADR's text is never rewritten.
- **Owner item #23 is reissued** with the new name, and its dashboard path
  becomes **AI Gateway > pigeon-models > Provider Keys**.
