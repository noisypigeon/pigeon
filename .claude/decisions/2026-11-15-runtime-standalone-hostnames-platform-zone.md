## Standalone hostnames: each standalone is `<label>.pigeon.dev`, served through the platform's wildcard route

* Date: 2026-11-15
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `src/pigeon-cli/crates/backplane/src/core/platform/**`, `src/pigeon-cli/crates/backplane/src/worker/mod.rs`, `src/pigeon-cli/crates/backplane/src/cobot/workers.rs`, `catalog/cloudflare/kinds/zone/schema.json`, `catalog/pigeon/kinds/platform/schema.json`, `infra/pigeon/workers/control/worker.yaml`

## Context

A standalone instance has no zone. It serves only its `workers.dev` host,
which names its Worker and its account's subdomain, and changes if either
does:

- [`2026-11-13-runtime-non-platform-profiles.md`](2026-11-13-runtime-non-platform-profiles.md)
  §7 serves a standalone on `workers.dev`, and §8 lists its base URL.
- [`2026-11-15-runtime-learner-practice-standalone-app.md`](2026-11-15-runtime-learner-practice-standalone-app.md)
  §1 and its first 2026-11-15 amendment build the `shape: app` stack, its
  `instance.standup` and its private listing, `instances.private` (§2).
- [`2026-11-13-runtime-zone-vendor-dns-placements.md`](2026-11-13-runtime-zone-vendor-dns-placements.md)
  keeps `zones/pigeon.dev` as a zone unit with no records, applied by the
  owner with `platform.apply`.
- [`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md)
  §1 mints every platform token from `granter`; the zone's DNS token is the
  owner's, per apply.

The owner asked for each standalone to get a real hostname, `<label>.pigeon.dev`,
as its identifier and base URL, keeping:

- each standalone in its own Cloudflare account, with its own Worker, Durable
  Object and bucket;
- the `shape: app` stack.

**Privacy is a hard rule.** A label can name a person. No label or hostname
of a real instance is ever written to the repository, a change's title or
plan, a branch, a log or a fixture. Code and tests use `example.pigeon.dev`.

### How a pigeon.dev host can reach another account's Worker

| Option | TLS | Account boundary | Per instance | Verdict |
| --- | --- | --- | --- | --- |
| Cloudflare for SaaS custom hostname | a certificate per host, published in Certificate Transparency logs | the hostname must sit outside the SaaS zone, and a zoneless account has no zone to be one | a hostname, a certificate and $0.10 a month past 100 | no |
| CNAME to the stack's `workers.dev` host | proxied: refused across accounts (error 1014); DNS only: the certificate is `*.workers.dev`, so TLS fails | `workers.dev` routes by its own hostname and doesn't know the name | a DNS record | no |
| A platform Worker route that proxies | the zone's Universal SSL already covers `*.pigeon.dev` | the platform forwards bytes and holds no credential of the stack | nothing | **yes** |

### The decisions

- **One wildcard, made once.** A proxied `*` record and a route `*.pigeon.dev/*`
  on the platform's control Worker serve every standalone. Standing one up
  writes no DNS record, route or certificate, so it needs no pigeon.dev
  credential.
- **The hostname lives in one place:** the standalone's entry in
  `instances.private`. No per-host record or certificate means the name
  appears in no public log.
- **The platform proxies; the standalone trusts nothing it adds.** Device
  signatures and passkey proofs are checked end to end by the standalone.

**Out of scope:** hostnames for `shape: full` or managed instances, more than
one hostname per instance, a vanity apex per instance, and renaming an
instance's label.

## Implementation plan.

### 1. The wildcard

- **`zones/pigeon.dev`** gains one record:
  `instances: { name: "*", type: AAAA, content: "100::", proxied: true }`.
  - Exact records win over a wildcard in DNS, so every name the platform
    serves is unchanged.
  - The zone schema's `$defs/name` allows a bare `*`; today it allows only
    `*.` followed by a label.
- **The `pigeon/platform` kind gains `instance_zone`,** optional: a zone in
  the manifest. A Worker whose unit sets it also serves the route
  `*.<instance_zone>/*`.
  - At most one unit in a manifest sets it, so only the live color carries
    the route. A blue-green cutover moves it in the same change that moves
    the platform's hostname.
  - `platform/blue` sets `instance_zone: pigeon.dev`.
- **`worker.yaml` routes gain `subdomains_of: <zone>`,** beside
  `custom_domain`. It renders the route `*.<zone>/*` on that zone and the var
  `INSTANCE_ZONE`. The control Worker adds
  `subdomains_of: $deployment.instance_zone`; a stack that leaves it unset gets
  neither.
  - Custom domains take precedence over routes, so the platform's stay put.
  - The deploy token already holds *Workers Routes Write* on the zone
    (`pigeon cobot workers permissions`).

### 2. Labels

`core::platform::hosts::check_label(label, taken)` is the one check, in the
functional core. The app's form, `instance.standup` and the MCP tool call it.

- **DNS-safe:** 3 to 32 characters of `[a-z0-9-]`, starting with a letter,
  not ending with `-`, no `--`, and never starting `xn--`.
- **Reserved, compiled in:** `platform`, `www`, `api`, `app`, `blue`,
  `green`, `git`, `docs`, `control`, `rescue`, `users`, `mail`, `mx`, `smtp`,
  `imap`, `autoconfig`, `autodiscover`, `admin`, `auth`, `login`, `id`,
  `status`, `static`, `cdn`, `assets`, `ns1`, `ns2`, `pigeon`, `noisypigeon`.
- **Taken:** every first label of a record name in the zone unit, of a
  `pigeon/platform` domain in the zone, and of every existing instance's
  host.
- **The answer names the rule, never another instance's label.** "taken"
  says only that.

### 3. The proxy, first in `fetch`

`worker/hosts.rs`, the control Worker's first step, before `/builds/`, Access
or any other route:

- **It runs only where `INSTANCE_ZONE` is set,** a var the control Worker
  takes from `instance_zone`. A standalone, built from the same crate, never
  has it.
- **The platform's own hosts** (`PASSKEY_RP_ID`, `PUBLIC_HOST`), and any host
  not directly under `INSTANCE_ZONE`, go on to the existing routes.
- **Any other host** is looked up with the `Instance` operation
  `/instances/host`, which answers the entry's `origin` from
  `instances.private`, or nothing.
  - An isolate caches answers for 60 seconds, misses included.
  - An unknown host, or a known host on a path outside `/v0/app/*`, is a
    bare 404 with no body.
- **A known host on `/v0/app/*`** is forwarded to `origin`:
  - the method, path and query, body, `Content-Type`, `Accept` and
    `X-Pigeon-*` headers pass; cookies, `Authorization`, `Cf-*` and `X-Forwarded-*`
    do not;
  - the answer is streamed back unchanged.
  - Device signatures cover the method, path and body, not the host, so they
    verify on the stack as they do directly.
- **Nothing logs a host.** `hosts.rs` never writes one to the console, an
  event or the digest, and insights never break down by hostname.

The standalone checks every request itself. The proxy holds none of its
keys, and an altered request fails its device signature.

### 4. Passkeys and app links

- **The relying party stays `platform.pigeon.dev`** (the learner ADR §7, the
  passkey-only ADR §5). The app's assertions are native, through the
  associated domain `webcredentials:platform.pigeon.dev`, and the client data's
  origin is the relying party's. The base host never enters an assertion.
- **The standalone checks the relying party's hash and client data as now.**
  It never compares the origin with its own host.
- **A browser page on `<label>.pigeon.dev` could not assert** for
  `platform.pigeon.dev`, but a standalone serves only `/v0/app/*`, so there is
  none.
- **Associated domains are unchanged.** Universal links stay on
  `platform.pigeon.dev`; a join link is `pigeon://join?base=<base URL>&…`
  (the learner ADR's 2026-11-15 amendment), which carries the host as data.

### 5. The listing, the stand-up and the app

These are hooks in the learner ADR's steps 5b and 6, built by whichever
change carries those steps:

- **`instance.standup` takes `label`,** typed by the owner at stand-up as
  runtime data (the app's form or an `owner_ask` answer), checked by §2.
- **An `instances.private` entry gains `host`** (`<label>.<instance_zone>`)
  **and `origin`** (the stack's `workers.dev` URL).
  - `base_url` is `https://<host>`.
  - `origin` never leaves the `Instance` Durable Object, and
    `GET /v0/app/instances` doesn't answer it.
- **The stack's `BASE_URL` var** is set by the stand-up job at deploy, from
  the same runtime input, for its join links. It is never rendered into the
  repository.
- **The app shows `host` as the instance's identifier** in `InstancesSheet`
  and the instance chip, and uses `base_url` for every call.
- **Teardown removes the entry,** which is the hostname's only record; the
  wildcard stays.

### 6. Steps

1. **This ADR.**
2. **`feat(cobot)`: the wildcard's pieces.** `subdomains_of` routes in `worker.yaml`
   and their rendering, the zone schema's bare `*`, and `instance_zone` in
   the platform kind with its at-most-one check; tests with
   `example.pigeon.dev`. A brief.
3. **`feat(backplane)`: labels and the proxy.** `core::platform::hosts` (the
   label check, header filtering, the path rule) with tests; `worker/hosts.rs`,
   added to what this ADR governs, first in `fetch`; `/instances/host` on
   `Instance`. It reads the learner ADR's entries, after its step 5b.
4. **`feat(infra)`: turn it on.** `instance_zone: pigeon.dev` on
   `platform/blue`, the `*` record in `zones/pigeon.dev`, and the control
   Worker's `subdomains_of` route. Owner tier.
5. **The hooks of §5,** with the learner ADR's steps 5b and 6.
6. **Owner steps:** approve step 4; run `platform.apply` for `zones/pigeon.dev`
   so the wildcard record exists; stand up the first standalone, typing its
   label in the app.

### 7. Verification

- **Labels:** each reserved word, a taken label, `xn--` and `--` forms, a
  leading digit, 2 and 33 characters are refused; `example` passes.
- **Proxy:** with a fake `Instance`, `example.pigeon.dev/v0/app/instances`
  reaches its origin with the method, path, query, body and device header
  intact, and without cookies or `Cf-*`; `/builds/x` and `/healthz` on it are
  404; an unknown host is 404; `platform.pigeon.dev` still reaches the
  platform's routes.
- **End to end, after step 6:** the owner's app lists the instance by its
  hostname, and a signed `GET /v0/app/instances` through it answers; `dig`
  for an unused label resolves to the same proxied address and answers 404.
- `mise run ci` passes at every step.

## Amendments

### 2026-11-15: !561 agrees, and this ADR carries §5's hooks itself

!561 amended
[`2026-11-15-runtime-learner-practice-standalone-app.md`](2026-11-15-runtime-learner-practice-standalone-app.md)
and cites
[`2026-11-14-security-instance-members-roles-invitations.md`](2026-11-14-security-instance-members-roles-invitations.md),
which it leaves unchanged.

- **They agree.** Neither changes a decision of the other:
  - its app-only router answers `/v0/app/*` and 404s the rest, which is all
    §3's proxy forwards;
  - its join link, `pigeon://join?base=<base URL>&token=…`, carries the base
    as data, and `/v0/app/join*` reaches the stack through the proxy;
  - its native passkey registration keeps relying party `platform.pigeon.dev`,
    as §4 does;
  - its stand-up job's log names no instance, account or host, and every app
    stack is `learner` and `pigeon-learner` in its own account;
  - the members ADR's platform gate and sign-in cookie stay unbuilt, and this
    ADR needs neither.
- **§5's hooks are this ADR's, not the learner ADR's.** The learner planner
  keeps its steps 5b and 6 as they are. One change, built after 5b, adds:
  - the entry's `label`, `host` and `origin`. Its `name` stays the owner's
    display text, not a label;
  - `label` on the stand-up, checked by `check_label`;
  - the job's `workers.dev` URL recorded as `origin`, and `base_url` as
    `https://<host>`;
  - `/instances/host` on `Instance`, answering `origin` for a host;
  - `GET /v0/app/instances` answering `host` and `base_url`, never `origin`.
- **Two places assume `workers.dev`, and change with it:**
  - the standalone builds invite links from the request's origin, which behind
    the proxy is its `workers.dev` host. It uses `BASE_URL`, a var in
    `catalog/pigeon/kinds/platform/app-worker.yaml` set at deploy, instead;
  - the app's `JoinLink` accepts only `https://*.workers.dev` bases. It also
    accepts `https://<label>.pigeon.dev`, a label passing `check_label`'s
    shape.
- **Order.** The proxy (step 3's Worker half) and these hooks build after
  5b. Step 4 turns the route on only once the proxy is deployed, so no
  `*.pigeon.dev` request reaches the platform's own routes.

### 2026-11-15: the proxy lives in `worker/hosts.rs`, and asks `Instance` at `/hosts/origin`

- This amendment adds `src/pigeon-cli/crates/backplane/src/worker/hosts.rs` to
  what this ADR governs.
- The `Instance` operation that answers a host's `origin` is `/hosts/origin`,
  not `/instances/host`, so it never meets the learner ADR's `/instances/`
  operations. It reads `instances.private` and answers `{origin}` or `{}`.
- **Any host under `INSTANCE_ZONE`** but the platform's own goes to the proxy,
  so a reserved or unknown name is a bare 404, never the platform's routes.
- Until step 4 sets `INSTANCE_ZONE`, the dispatch does nothing; until the
  hooks after the learner ADR's step 5b add `host` and `origin`, every host is
  a 404.
### 2026-11-15: Review of !593 against !582: they agree

[`2026-11-15-runtime-architecture-coverage-agents-deploys.md`](2026-11-15-runtime-architecture-coverage-agents-deploys.md)
§1 derives the Worker's routes and hostnames from the tree.

- **They agree.** Neither changes a decision of the other:
  - the `subdomains_of` route is drawn as `*.pigeon.dev`, "standalone instance
    (own account)", and no standalone's hostname or account is shown, as §3
    here requires;
  - !593's dispatch in `fetch` is a call, `hosts::ours`, not a path literal,
    and it forwards only `/v0/app/*`, which the `app` client already covers;
  - `/hosts/origin` is an `Instance` operation, not a Worker route, so it adds
    no client.
### 2026-11-15: Reviewed against !563, the app shape: they agree

!563 carries step 5a of
[`2026-11-15-runtime-learner-practice-standalone-app.md`](2026-11-15-runtime-learner-practice-standalone-app.md).
In `catalog/pigeon/kinds/platform/schema.json` it adds `shape: full | app`
and lets an app shape omit `domains`. It also adds `app-worker.yaml` beside
the schema, the app-only router and the hand-over seed.

- **The schema changes don't meet.** `shape` and this ADR's `instance_zone`
  are separate properties. An app shape still has no `domains`: its hostname
  is reached through the platform's wildcard (§1), not through a route of its
  own.
- **The stack itself doesn't change.**
  - It still serves `workers.dev`, which becomes §5's `origin`.
  - Its router already answers `/v0/app/*` alone and 404s every other path
    first, which is what §3's path rule expects of the origin.
- **§5's hooks land in one change after the learner ADR's step 5b,** made by
  this ADR's planner, as the two planners agreed:
  - the entry's `label`, `host` and `origin`;
  - `/instances/begin` taking `label`;
  - `BASE_URL` in `app-worker.yaml`, used for invite links;
  - `JoinLink` accepting `https://<label>.pigeon.dev`.

  Step 5b lands the entry without them, with `base_url` the `workers.dev` URL
  until then.

Neither ADR changes.
### 2026-11-15: `BASE_URL` is a secret of the stack, and the listing's entry is named after it begins

- **`BASE_URL` is a Worker secret, not a var.** The stand-up job puts it as
  it puts `HANDOVER`, so the `wrangler.jsonc` it renders never holds a host.
  A stack without one builds join links from its request's origin, as before.
- **`Listing::named`** gives a begun entry its `label` and `host`, after
  `begin`, only when the control Worker has `INSTANCE_ZONE`. A resumed entry
  keeps its label. `Listing::shown` is what `/instances/list` answers: every
  entry without `origin`.

### 2026-11-15: `noisypigeon` is not a reserved label

- §2's reserved labels lose `noisypigeon`. Only the owner names an instance
  (stand-up is administrator-only and passkey-proved), so the reservation
  guarded nothing, and it kept the noisypigeon standalone of
  [`2026-11-15-runtime-zoned-standalone-account-imports.md`](2026-11-15-runtime-zoned-standalone-account-imports.md)
  from its own label. `pigeon` stays reserved.

### 2026-11-15: the `*` record was made in the dashboard, and is adopted later

- **The owner created §1's record by hand**, in the Cloudflare dashboard:
  `*` AAAA `100::`, proxied, in pigeon.dev. `infra.plan` isn't dispatched
  yet, and an apply by hand is barred, so this was the owner's choice.
- **It is adopted, not created again.** Once
  [`2026-11-15-runtime-zoned-standalone-account-imports.md`](2026-11-15-runtime-zoned-standalone-account-imports.md)
  §5's `plan --adopt` for DNS records (!640) is on `main` and `infra.plan`
  runs, `zones/pigeon.dev`'s first plan adopts the record by name, type and
  content. That plan must show no other change; one that would create a
  second `*` record is refused.
- **The live check needs the owner.** A cloud session's network proxy
  refuses hosts outside its allowlist, so it can't reach an unused label.
  From any machine:
  `curl -sI https://example-check.pigeon.dev/v0/app/instances` answers 404
  with an empty body, and `curl -sI https://example-check.pigeon.dev/healthz`
  answers 404 too, not the platform's health.

### 2026-11-15: `*.pigeon.dev/v0/app` is reached without an Access login

- **The owner's live check found Access in front of the proxy.**
  `example-check.pigeon.dev/healthz` redirected to the Access login. The
  owners-only application has a `worker` destination, the control Worker's
  ID, so it covers every hostname the Worker serves, the wildcard route
  included.
- **`*.pigeon.dev/v0/app` gets a bypass,** as `platform.pigeon.dev/v0/app`
  has: a `public_paths` entry on `platform/blue`, its own application with a
  bypass policy, more specific than the Worker-wide one. `public_paths` may
  name a host as `*.` and one label.
- **Nothing else opens.** Every other path on `*.pigeon.dev` stays behind the
  owners' login; there the proxy answers only 404 anyway. The platform's own
  hosts are unchanged.
- **It applies through the pipeline:** `platform.plan` on `platform/blue`,
  then `platform.apply`, which the owner grants with a passkey. The plan
  should create one Access application and one policy, and change nothing
  else.
