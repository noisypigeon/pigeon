## Connecting a provider account from a browser: one flow, an adapter per provider, and a narrowed granter at the end

* Date: 2026-11-14
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: security
* Governs: `src/pigeon-cli/crates/backplane/src/core/minting.rs`, `src/pigeon-cli/crates/backplane/src/worker/minting.rs`

## Context

The owner wants one mechanism to give pigeon access to an account at a
provider:

1. open a browser;
2. sign in to the provider;
3. choose an account;
4. grant pigeon access to it.

Cloudflare comes first. DigitalOcean, Scaleway and Backblaze reuse the same
mechanism later, unchanged, through an adapter each.

### What this rests on

- **Granters and minting.**
  [`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md)
  §1 and its amendments:
  - pigeon's own `granter` is in the platform's Secrets Store, made by the
    owner in the dashboard;
  - `core::minting` builds token bodies, settles and rotates;
  - `/grants/<id>` mints on the owner's proof;
  - `.config/grants.yaml` is the ledger, checked by `core::ledger`.
- **Profiles' granters.**
  [`2026-11-13-runtime-non-platform-profiles.md`](2026-11-13-runtime-non-platform-profiles.md):
  - §3: one granter per profile and provider, in D1 `grants` under
    `GRANT_KEY`, bounded by the profile's ceiling;
  - [`2026-11-14-runtime-accounts-profiles-instances.md`](2026-11-14-runtime-accounts-profiles-instances.md) (!419)
    renames that ADR's "profile" to **instance**. This ADR uses its terms:
    - an *instance* owns granters;
    - a *provider account* is the third party's, such as a Cloudflare
      account;
    - a pigeon *account* or *profile* is a person's, and owns no granter.
    D1's `profile_accounts` is `instance_accounts`, and the grants ledger's
    field is `instance:`;
  - §8, steps 3 and 4: a recipe, **Open Cloudflare**, and a paste;
  - §10, step 6: the owner enrolls the second Cloudflare account from the
    app.

  That ADR left "OAuth flows for third-party APIs" out of scope. This ADR is
  how its §8 steps 3 and 4 and its §10 step 6 happen.
- **The app.**
  [`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md):
  - §2: `ASWebAuthenticationSession` and the `pigeon://` callback, used to
    link a device;
  - §3: a passkey assertion per write, with a challenge from
    `POST /v0/app/challenge` bound to the operation and its target;
  - §4: routes under `/v0/app/`, each an `Instance` operation.
- **Passkeys.**
  [`2026-11-09-security-approvals-once-passkeys.md`](2026-11-09-security-approvals-once-passkeys.md)
  §4: relying party `platform.pigeon.dev`, challenges used once, five
  failures an hour lock.
- **Other providers' units.** The DigitalOcean, Scaleway and Backblaze
  provisioner ADRs of 2026-11-03 each hold one hand-made break-glass
  credential, and mint only narrow keys for workloads.

### What each provider supports today

Checked against the providers' documentation on 2026-11-14.
`developers.cloudflare.com` was read directly. The other providers' hosts
are outside this session's network allowlist, so their facts come from
search results that quote their docs. Each of those is marked *to confirm*,
and its adapter's first step confirms it before building on it.

**Cloudflare: OAuth clients, generally available.**

- **Who registers a client:** a Super Administrator, Administrator or
  *OAuth Client Write* member of an account.
  - In the dashboard: Manage Account, then OAuth clients.
  - Through the API: `POST /client/v4/accounts/{id}/oauth_clients`.
- **Endpoints**, from `https://dash.cloudflare.com/.well-known/openid-configuration`:
  - `https://dash.cloudflare.com/oauth2/auth`
  - `https://dash.cloudflare.com/oauth2/token`
  - `https://dash.cloudflare.com/oauth2/revoke`
  - `https://dash.cloudflare.com/oauth2/userinfo`
- **Grant types:** authorization code only.
  - A confidential client uses a secret, with PKCE optional.
  - A public client uses PKCE with `S256` and no secret.
  - Client credentials and implicit are not supported.
- **Visibility.** A new client is private: only members of its parent
  account may authorize it. Making it public is permanent, and needs a
  verified domain.
- **Scopes** are named after API token permissions, with `GET
  /client/v4/oauth/scopes` as the source of truth.
  - Every scope is required unless the client marks it optional.
  - The owner can untick optional ones at consent.
- **Consent chooses accounts.** The owner picks which of their accounts the
  client may reach. An account whose administrator turned off *Public OAuth
  App access* is not offered.
- **Revocation.** The owner sees and revokes every authorization at
  `https://dash.cloudflare.com/?to=/profile/access-management/authorization`.
- **Not documented:** the access and refresh tokens' lifetimes, refresh
  token rotation, and whether a private client may be granted an account
  other than its parent. Step 2 measures them.

**Cloudflare: a token-creation link with prefilled permissions.**

- **Account-owned tokens** open at
  `https://dash.cloudflare.com/?to=/:account/api-tokens&permissionGroupKeys=<url-encoded JSON>&name=<name>`.
  - `permissionGroupKeys` is `[{"key": …, "type": "read" | "edit" | …}]`.
    A permission group's ID is the reliable key; its `type` is then
    ignored.
  - The dashboard asks which account when the owner has several.
  - Zone and resource scope are still chosen by hand: an account link
    takes no `zoneId`.
- **Keys that don't resolve are dropped,** with a notice on the form. So
  the owner must check the form against the recipe.
- **The owner still creates and copies the token.** Nothing comes back by
  itself.

**Cloudflare: a pasted token.** This is today's §8 path of the profiles ADR,
and the fallback for both.

**DigitalOcean: OAuth applications** (*to confirm*).

- **The authorization server is `https://cloud.digitalocean.com/v1/oauth`,**
  with `/authorize`, `/token` and `/revoke`.
- **The token response:**
  - an access token with `expires_in` of 2,592,000 seconds (30 days);
  - a refresh token that works once. Using it issues a new pair and voids
    the old access token.
- **Scopes.** Granular `resource:action` scopes, plus the aliases
  `api:read` and `api:write`. Older apps use `read` and `read write`. It is
  unconfirmed whether an OAuth app may ask for granular scopes.
- **The docs disagree about refresh:** `/token` with
  `grant_type=refresh_token`, or `/refresh`.
- **Its API can't mint personal access tokens,** per the DigitalOcean
  provisioner ADR. It can mint Spaces keys scoped to buckets.

**Scaleway: no OAuth for third parties.**

- **An API key** is an access key and a secret key, carried by an IAM user
  or an IAM application. It inherits its bearer's policies.
- **Keys are created in the console,** under IAM and then API keys, or
  through the IAM API (`/iam/v1alpha1/api-keys`), and may carry
  `expires_at`.
- **The provisioner ADR's break-glass** is one key of an IAM application.
  With the right policy, that key can create expiring keys for other
  applications. That is the narrowing.

**Backblaze B2: no OAuth.**

- **Application keys** are made in the web UI or with `b2_create_key`.
  - Each holds capabilities.
  - Each can be limited to buckets and a name prefix.
  - Each can expire.
- **The master key** has every capability and never expires.
- **Auth tokens from `b2_authorize_account`** last 24 hours.
- **A key holding `writeKeys`** can mint narrower keys, so a pasted one
  narrows as Scaleway's does.

So there are two shapes:

- **`oauth`:** Cloudflare and DigitalOcean. A callback brings a code back.
- **`key`:** Scaleway, Backblaze, and Cloudflare's fallback. The owner
  makes a key on the provider's page and hands it back.

## Implementation plan.

### 1. One flow, two shapes

A **connection** is one run of the flow. It names:

- the **instance** it is for: `pigeon`, or another instance (§7);
- the **provider**, which picks the adapter (§2);
- the **purpose**: a **granter** (the default) or a **held** credential
  (profiles ADR §4);
- the **ceiling**: the capabilities the result may hold. It comes from the
  instance's ledger entry, or for `pigeon` from the grants ledger's grants for that
  provider.

**Its states,** in `core::connect`, pure and tested without a Worker:

```
started ──► awaiting-callback (oauth) ──► exchanging ──► choosing ──► granting ──► granted
   └──────► awaiting-key (key) ─────────────────────────► choosing ─┘
any state ──► failed | expired | cancelled
```

- **`started`:** made by `connect.start`, on the owner's proof (§4). It
  holds:
  - `state`: 32 random bytes;
  - a PKCE verifier: 64 characters;
  - the ceiling's digest;
  - a deadline: 10 minutes for `oauth`, 30 for `key`.
- **`awaiting-callback`:** the browser is at the provider.
- **`awaiting-key`:** the owner is on the provider's key page. The flow
  waits for the paste.
- **`exchanging`:** the Worker trades the code for tokens (`oauth`), or
  verifies the pasted key (`key`).
- **`choosing`:** the identity reaches more than one provider account (§5).
- **`granting`:** the adapter narrows (§6), stores the granter, and revokes
  what came back. Then it runs `instance.check`, or for `pigeon` a test
  mint.
- **Every state change is an event:** `connect.started`,
  `connect.callback`, `connect.choosing`, `connect.granted`,
  `connect.failed`, `connect.expired` and `connect.cancelled`. Each names
  the flow, the instance and the provider. None carries a value or an
  account ID beyond its last four characters.
- **One live flow per instance and provider.** A second `connect.start`
  cancels the first.

**Storage:**

- The flow lives in the `Instance` Durable Object, keyed by its ID, and is
  removed 24 hours after it ends.
- The PKCE verifier and any provider token stay in that storage until the
  flow ends.
- None of them is ever sent to a client, logged, or put in an event.

### 2. Adapters

An adapter is two halves:

- **literal metadata** in `catalog/<provider>/connect.yaml`, read by the
  deploy job and resolved into the Worker's catalog as
  `catalog/<provider>/capabilities.yaml` already is;
- **code** behind one trait in `core::connect`, with the HTTP calls in the
  Worker's shell.

```yaml
# catalog/cloudflare/connect.yaml
provider: cloudflare
shapes: [oauth, key]                 # tried in this order
oauth:
  authorize: https://dash.cloudflare.com/oauth2/auth
  token: https://dash.cloudflare.com/oauth2/token
  revoke: https://dash.cloudflare.com/oauth2/revoke
  pkce: S256
  client: { kind: public, id_secret: OAUTH_CLOUDFLARE_CLIENT }   # a Secrets Store secret's name
  scopes: { from: capabilities, add: [account-api-tokens.write] }  # step 2 fixes the names
key:
  page: "https://dash.cloudflare.com/?to=/:account/api-tokens&permissionGroupKeys={groups}&name={name}"
  groups: from-capabilities
  verify: https://api.cloudflare.com/client/v4/accounts/{account}/tokens/verify
accounts: https://api.cloudflare.com/client/v4/accounts
narrow: mint-granter
```

**The trait:**

| Method | Does |
|---|---|
| `authorize_url(flow)` | the provider's URL, with `state`, `code_challenge` and the scopes |
| `exchange(code, verifier)` | the token request; returns a provider credential, kept in the flow |
| `key_page(flow)` | the provider's key page, prefilled where it can be |
| `verify(credential)` | active, and which accounts it reaches |
| `narrow(credential, account, ceiling)` | makes the stored granter (§6) |
| `revoke(credential)` | revokes what came back, once narrowed |
| `renew(granter)` | rotation (§8) |

**The first adapter is Cloudflare,** with both shapes. DigitalOcean, Scaleway
and Backblaze are each a `connect.yaml` and an adapter, under §9's later
steps. None changes the flow.

### 3. Where the browser opens, and where the callback lands

**The redirect URI is `https://platform.pigeon.dev/connect/callback`.** There
is one for every provider, and the flow is found by `state`.

- **It is a `public_path`,** as `/v0/app/*` is. The browser that lands on it
  may hold no Access cookie: an ephemeral `ASWebAuthenticationSession`, or
  the CLI owner's default browser.
- **It carries no authority alone.** A request with an unknown, used or
  expired `state` is refused. The code is useless without the PKCE verifier,
  which never left the Worker.
- **The code is exchanged by the Worker.** No surface ever holds a provider
  token.

**Surfaces:**

- **The app:**
  1. **Connect** on an instance, Platform included, asks for Face ID
     with a challenge bound to `connect.start` (§4).
  2. `POST /v0/app/connect` answers the flow's ID and its `next` URL.
  3. **`oauth`:** the app opens `next` in `ASWebAuthenticationSession` with
     callback scheme `pigeon`. After the exchange, `/connect/callback`
     redirects to `pigeon://connected?flow=<id>`. The app then reads
     `GET /v0/app/connect/<id>` and shows the choice (§5) or the result.
  4. **`key`:** the app shows the recipe and opens `next` (the key page) in
     Safari. **Paste key** reads the clipboard once and sends it to
     `PUT /v0/app/connect/<id>/key`, with a fresh assertion bound to the
     value's SHA-256, as profiles ADR §8 step 4 does. It then overwrites
     the clipboard.
- **The web:** `/connect/<id>`, behind Access, beside `/approvals`.
  - Its proof is the passkey, served by `/passkey.js`.
  - It then sends the browser to the provider (`oauth`), or shows the
    recipe, the key page link and a paste field (`key`).
  - The callback ends on `/connect/<id>`, showing the choice or the result.
- **The CLI:** `pigeon backplane connect <provider> [--instance NAME]`.
  - It calls `connect.start`, which answers `/connect/<id>`, as
    `grant.request` answers its page.
  - It opens that page in the default browser, and prints it too.
  - It follows `connect.*` events for the flow until it ends.
  - **No loopback redirect.** The CLI never receives a code, a token or a
    pasted key. So a CLI on a remote machine works as well as a local one.
- **An agent may call `connect.start`** over MCP and is answered the page's
  link. Only the owner's proof moves the flow past `started`.

### 4. Who may start one, and the passkey binding

- **Who:** `may_connect(actor, instance)`, one predicate in the functional
  core.
  - Until the instance-roles ADR lands, it means the instance's
    Administrator, which today is the owner.
  - A User or a Service Account never may.

  The roles ADR owns the predicate's rule. This ADR only calls it, as
  agreed with that session and the accounts and instances session (!419).
- **Tier: `owner`,** every time. No Auto window or mandate covers a
  `connect.*` operation. Starting a flow sets a secret.
- **Passkey only.** There is no authenticator-code path, in line with the
  passkey-only session's change.
  - **Start:** the challenge is bound to `connect.start`, the instance, the
    provider and the ceiling's digest. The flow records the credential ID
    that proved it.
  - **Choosing an account** and **pasting a key** each take a fresh
    assertion. The first is bound to the flow and the chosen account ID's
    digest. The second is bound to the flow and the key's SHA-256. Each must
    come from the credential that started the flow.
  - **An `oauth` flow with one account** completes on the start's proof
    alone. The owner proved who they are before the browser left, and the
    callback can't be completed without the Worker's verifier.
- **One binding, not a new one.** Once the passkey-only ADR (!417) lands,
  each challenge is its `SHA-256("pigeon-proof-v1" …)` value, with:
  - `instance/provider` as `target`;
  - the digest as `head`.

  The page's proof goes through its `proof::on_page`. Whichever change
  merges second adapts.
- **Failures count toward the existing lock:** five an hour.

### 5. Choosing the provider account

- **After the exchange or the paste,** `verify` lists the provider accounts
  the credential reaches: their ID, name, and the owner's role in each.
- **Exactly one account:** it is used.
- **More than one:** the flow is `choosing`. The page or the app lists each
  account by name and the last four characters of its ID.
  - An account already bound to another instance is greyed, with that
    instance's name. One provider account belongs to one instance (profiles
    ADR §3, confirmed by !419).
  - A Cloudflare user token from the `key` shape that reaches several
    accounts is refused. The recipe makes an account-owned token, which
    reaches exactly one.
- **Ceiling zones must exist** in the chosen account, as profiles ADR §3
  checks. Otherwise the flow fails, naming the missing zones.

### 6. What comes back becomes a narrow granter, and what came back is revoked

The rule: **what the provider hands back is used once, to make the
narrowest standing credential the provider allows. It is then revoked.**

- **Cloudflare, `oauth`.**
  1. The OAuth access token, holding *Account API Tokens Write* plus the
     ceiling's scopes on the chosen account, mints one account-owned token
     through `core::minting::token_body`. That token:
     - holds *Account API Tokens Write* and the ceiling's permissions, and
       nothing more;
     - is limited to the ceiling's zones;
     - expires in 90 days.

     It is exactly the granter profiles ADR §3 describes. It is stored in
     D1 `grants` under `GRANT_KEY`.
  2. The refresh token is revoked at `/oauth2/revoke`. The authorization
     then leaves the owner's list in the dashboard.
  3. **Fallback to `key`.** Step 2 might find that *Account API Tokens
     Write* is not an OAuth scope, or that a private client can't reach a
     non-parent account. Either way the adapter's `shapes` become `[key]`.
     The flow is unchanged, and this ADR is amended with what was found.
- **Cloudflare, `key`.** The recipe's link asks for the same permissions.
  The pasted token is checked as profiles ADR §3 does.
  - **The pasted token is the granter only if it is no wider than the
    ceiling.** Otherwise it mints the narrow granter above, and is then
    revoked through its own *Account API Tokens Write*.
  - So a token the owner made too broad never stays.
- **DigitalOcean, `oauth`.**
  - **What it can narrow:** Spaces keys scoped to buckets. Nothing else.
  - **What is stored:** the refresh token, encrypted under `GRANT_KEY`.
  - **Its access token is renewed by the hourly cron** seven days before
    expiry, and handed to jobs sealed. The refresh is single-use, so only
    the cron refreshes.
  - **The scopes asked for are the ceiling's,** if DigitalOcean accepts
    granular ones for OAuth; otherwise `read` alone, or `read write` when
    the ceiling writes.
  - **This is a broad standing credential.** DigitalOcean's ADR step says so
    to the owner on the page.
- **Scaleway and Backblaze, `key`.**
  - The pasted key must be able to make keys: Scaleway's IAM API key
    rights, or B2's `writeKeys`.
  - It becomes the granter. Each run mints an expiring key from the
    ceiling, scoped to the projects or buckets.
  - It is never handed to a job.
- **The grants ledger gains `provider:`** on `mint` (default `cloudflare`).
  - Capabilities resolve against `catalog/<provider>/capabilities.yaml`.
  - The granter is chosen by the grant's `instance:` and its provider, never
    by the request.

  This lands as an amendment of the grants ADR, with each new provider's
  minting step.
- **`connect.revoke` and `grant.list`** treat a connected granter as any
  other: by name, token ID and expiry, never by value.

### 7. Which instance gets the grant

- **The instance is named at `connect.start`** and fixed for the flow. The
  result is the granter row for (instance, provider).
- **A managed instance.** The row is that instance's granter, and its
  ceiling is the instance's entry.
  - The profiles ADR's §8 steps 3 and 4 become **Connect Cloudflare**: the
    `oauth` flow, with the recipe and paste as its `key` fallback.
  - Its §10 step 6 is this flow, for `noisypigeon`. The route
    `PUT /v0/app/profiles/<p>/grants/<provider>` becomes a thin call into
    the flow's `key` shape.
- **`pigeon`.**
  - The Cloudflare `granter` stays in the platform's Secrets Store, made in
    the dashboard. The Worker can't write the store without a token that
    `granter` would mint.
  - pigeon's other providers' break-glass credentials become connected
    granters, retiring each `provisioner_token` (grants ADR §3).
- **A standalone instance** runs the flow on its own stack, with its own
  OAuth client and callback, or uses the `key` shape. pigeon never holds its
  credential (profiles ADR §7).

### 8. Refresh, rotation, revocation and expiry

| | Cloudflare | DigitalOcean | Scaleway | Backblaze |
|---|---|---|---|---|
| Stored | account-owned granter, 90 days | refresh token | key that makes keys | key with `writeKeys` |
| Renewed | the granter mints its successor and revokes itself, at 60 days | the cron refreshes the access token | the granter makes its successor key | the granter makes its successor key |
| Rotation tier | auto under `grants-rotate`, as nothing changes but the value | auto, refresh only | auto under `grants-rotate` | auto under `grants-rotate` |
| Revoked by `connect.revoke` | deletes the granter through itself | `/revoke` | deletes the key through itself | `b2_delete_key` through itself |
| Lost or refused | the flow again | the flow again | the flow again | the flow again |

- **A successor is checked before the old one goes,** as grants ADR §1's
  `settle` does. A failed renewal keeps the old one, and opens a
  `connect-failed` task.
- **Expiry is enforced by the provider** wherever it can be. The digest
  names a granter 14 days before it expires, or once a renewal has failed.
  - Its card is **Reconnect**: the same flow, prefilled.
- **A revoked or refused granter** sets the instance to `awaiting-grant` and
  stops its runs, as profiles ADR §3 revokes.
- **Revoked outside pigeon:** the daily `instance.check` finds it, with an
  event and a task.

### 9. Execution

Each step is one change, in order unless marked parallel. A step that adds
`core/connect/**`, `worker/connect.rs` or a `catalog/<provider>/connect.yaml`
adds it to `Governs` in the same change.

1. **This ADR.**
2. **The owner registers the OAuth client** in the pigeon account (owner
   step):
   - visibility private, authentication `none`, PKCE `S256`;
   - redirect `https://platform.pigeon.dev/connect/callback`;
   - scopes from step 3's list.

   Then the owner puts its ID in the Secrets Store as
   `OAUTH_CLOUDFLARE_CLIENT`. The planner then records, as an amendment:
   - `GET /oauth/scopes`;
   - whether *Account API Tokens Write* is offered;
   - token lifetimes and refresh rotation;
   - whether consent offers the second account.
3. **Brief, haiku: literal metadata and fixtures.**
   - `catalog/cloudflare/connect.yaml`, from §2;
   - fixtures of Cloudflare's token, accounts and verify responses under
     `src/pigeon-cli/crates/backplane/tests/fixtures/connect/`;
   - a `docs/` page for the flow.
4. **Brief, sonnet: `core::connect`,** in parallel with 3.
   - The state machine of §1, the adapter trait of §2 and the checks of §4
     and §5, pure, with tests for every transition:
     - refusals of unknown, used or expired `state`;
     - an assertion from another credential;
     - the choosing rules;
     - a ceiling breach.
   - No Worker code.
5. **Mine (owner tier, secrets): the Worker.**
   - `worker/connect.rs`, with `/connect/<id>`, `/connect/callback`,
     `POST /v0/app/connect`, `GET /v0/app/connect/<id>` and
     `PUT /v0/app/connect/<id>/key`;
   - the Cloudflare adapter's HTTP;
   - narrowing through `core::minting`, and the OAuth revoke;
   - `public_paths` gains `/connect/callback`;
   - `connect.*` operations and events.
6. **Brief, sonnet: the CLI,** `pigeon backplane connect`.
7. **Brief, sonnet: the app's Connect,** in `Pigeon/` and `PigeonCore/`.
8. **The owner connects `noisypigeon` from the app.** This is the profiles
   ADR's §10 step 6.
9. **Later, one ADR amendment and adapter each:** DigitalOcean, Scaleway,
   Backblaze.
   - Each first confirms the facts marked *to confirm*.
   - Each lands with the grants ADR's `provider:` amendment and its
     minting module.

### 10. Verification

- **`state`:**
  - an unknown, used or expired one is refused at the callback;
  - a replay of a good callback is refused;
  - a flow past its deadline is `expired`.
- **PKCE:** the token request carries the verifier, and the verifier never
  leaves the Worker. A test asserts that no response body or event contains
  it.
- **Passkeys:**
  - a choice or paste proven by another credential is refused;
  - so is one whose assertion is bound to another account's or key's
    digest.
- **Narrowing:**
  - a test with fake Cloudflare responses checks that the stored granter's
    body holds only the ceiling plus *Account API Tokens Write*, limited to
    the ceiling's zones;
  - the refresh token's revoke is called;
  - a too-broad pasted token is revoked after narrowing.
- **One account per instance:** an account already bound elsewhere can't
  be chosen.
- **Surfaces:**
  - the CLI never receives a provider credential;
  - the app overwrites the clipboard;
  - the callback page is reachable without Access, and does nothing without
    a live `state`.
- **`mise run ci` passes at every step.**
