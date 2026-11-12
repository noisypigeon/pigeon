## The Pigeon iOS app, phase one: insights, approvals with a passkey, Auto windows, and the backplane it needs

* Date: 2026-11-12
* Model: Claude
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `src/pigeon-ios/Pigeon/**`, `src/pigeon-ios/project.yml`, `src/pigeon-cli/crates/backplane/src/core/app/**`, `src/pigeon-cli/crates/backplane/src/devices.rs`, `src/pigeon-cli/crates/backplane/src/worker/devices.rs`

## Context

The app at `src/pigeon-ios` shows "Pigeon" and ships to TestFlight through
the release pipeline
([`2026-11-11-release-ios-app-buildkite-testflight.md`](2026-11-11-release-ios-app-buildkite-testflight.md)).
The owner approves changes today on the approval pages, with the
authenticator code or a passkey, and is told by Web Push
([`2026-11-09-security-approvals-once-passkeys.md`](2026-11-09-security-approvals-once-passkeys.md),
[`2026-11-08-development-events-tickets-digest-mandate.md`](2026-11-08-development-events-tickets-digest-mandate.md)).

A mockup settled phase one's screens:

- **Home, one screen:**
  - the app icon, top left, opens Profiles;
  - **Insights**: Cost and Errors side by side, today by hour;
  - **Approvals**: an `Accept edits` / `Auto` switch, the waiting
    approvals, then what happened today.
- **An expanded chart** over Home: 1d / 7d / 30d, one source at a time
  (Cloudflare, GitHub, Buildkite), stacked by kind, each segment tappable.
  - Cost kinds: Cloudflare's Workers, Containers, and R2 with D1;
    Buildkite's hosted macOS minutes; GitHub's Actions minutes.
  - Error kinds: Cloudflare's 5xx responses, Worker exceptions and CPU limit
    exceeded; GitHub's failed checks, cancelled runs and dispatch errors;
    Buildkite's failed sign steps, cancelled builds and lost agents.
- **An approval card** over Home: the operation, tier, head, scope and
  checks, `Keep the merge for me`, and Face ID, with the code as a fallback.
  Approving turns the card into the merge's progress.
- **Auto:** choosing it asks for 1, 4, 8 or 24 hours and Face ID. While it
  runs, a banner shows when it ends, and owner-tier approvals still ask.
- **Profiles:** Platform only, with Settings, whose Appearance is System,
  Light or Dark and follows the device by default.
- **The look:** a dot grid, offset block shadows, SF Pro and SF Mono, light
  and dark.

**Not in phase one:** the architecture view, change and deploy lists, other
instances' profiles (noisypigeon's blue and yellow), iPad, widgets.

Every number the app shows needs a source the control plane doesn't keep
yet, and every action needs a way for a phone to authenticate and prove the
owner. That backplane work is most of this ADR.

## Implementation plan.

### 1. The app

- **SwiftUI, iOS 18, no third-party packages.** Two targets in
  `project.yml`:
  - **`PigeonCore`**, a framework of pure Swift: the API's types, request
    signing's canonical form, the screens' state as values with pure
    transitions, chart geometry, and time formatting. Swift Testing covers
    it, and `ios.check` runs it.
  - **`Pigeon`**, the app: views, the Keychain, `AuthenticationServices`,
    and notifications. It holds no logic a test needs.
- **Design tokens** are one Swift file: the light and dark colors, the dot
  grid, the 3 pt offset shadow, and the chart palettes. The palettes are the
  mockup's, checked for colour-blind separation and contrast on both
  surfaces:
  - providers: light `#B04A78` `#B8862B` `#3A78B5`, dark `#C65A8C` `#B88A2E`
    `#4C86C6`;
  - kinds: light `#6A55C9` `#C2571F` `#2D8C6A`, dark `#8471DC` `#D06A35`
    `#2F9C78`.
- **Appearance** is stored in `UserDefaults` as `system`, `light` or `dark`,
  `system` by default, applied with `preferredColorScheme`.
- **Charts** are drawn with SwiftUI shapes, not Swift Charts, so a segment is
  its own button with an accessibility label ("14:00, Workers, $0.04").
- **The release** is unchanged: `ios.release` archives, signs and uploads
  each build.

### 2. Signing in: a device session, bound to a key on the phone

A phone can't hold Access's browser cookie, and the API mustn't trust a
bearer token alone.

1. **The app makes a key** in the Secure Enclave: P-256, not exportable,
   usable only after first unlock.
2. **It opens `https://platform.pigeon.dev/app/link`** in
   `ASWebAuthenticationSession`, which is behind Access as every page is. The
   page shows the device's name and key fingerprint, and asks for a passkey or
   the code: linking a device is `owner` tier, as registering a passkey is.
3. **The page redirects to `pigeon://linked?code=…`** with a one-time code,
   valid for 2 minutes.
4. **The app redeems it** at `POST /v0/app/session` with its public key and a
   signature over the code. The Worker stores the device, in `Instance`
   storage:
   - its ID, name and public key, as SPKI;
   - when it was linked and last seen;
   - its APNs token, once it has one (§6).
5. **Every API call is signed** with the device key over
   `method`, `path`, `timestamp` and the body's SHA-256, in an
   `X-Pigeon-Device` header. The Worker refuses a timestamp more than
   60 seconds off, and a signature it has seen in that window.
6. **`/devices`**, behind Access, lists linked devices and unlinks one with a
   proof. A device unused for 30 days is unlinked.

A stolen session is useless without the phone's Secure Enclave. A stolen
phone still needs Face ID for every action that changes anything (§3).

### 3. The passkey in the app

- **The app proves the owner with the same passkey** as the pages, through
  `ASAuthorizationPlatformPublicKeyCredentialProvider` with relying party
  `platform.pigeon.dev`
  ([`2026-11-10-runtime-platform-hostname-dashboard-observability.md`](2026-11-10-runtime-platform-hostname-dashboard-observability.md) §2).
- **That needs associated domains:**
  - the app's entitlement `webcredentials:platform.pigeon.dev`;
  - `GET /.well-known/apple-app-site-association`, served by the Worker
    without Access, naming `<team>.dev.pigeon.app`. The team ID comes from
    the `APPLE_TEAM_ID` secret at request time, so it isn't in the
    repository.
- **The Worker's verifier is unchanged.** A native assertion's client data
  carries the origin `https://platform.pigeon.dev`, which it already accepts.
- **Every write asks for one:** approving, starting Auto, ending Auto early,
  and unlinking. The app gets a challenge from `POST /v0/app/challenge`,
  bound to the operation and its target, and sends the assertion with the
  operation. Failures count toward the existing lock: five in an hour.

### 4. The app's API, under `/v0/app/`

Access lets `/v0/app/*` and the association file through, as it does
`/v0/apple/release`
([`2026-11-08-runtime-blue-green-self-standup.md`](2026-11-08-runtime-blue-green-self-standup.md)'s
`public_paths`). The device signature is the only authentication.

| Route | What it answers or does |
|---|---|
| `POST /v0/app/session` | redeems a link code (§2) |
| `GET /v0/app/home` | the two tiles' 24 hourly stacks, the waiting approvals, today's history, Auto's state |
| `GET /v0/app/insights?metric=&source=&range=` | one source's stacks by kind, for 1, 7 or 30 days |
| `GET /v0/app/approvals/<grant>` | one waiting approval: operation, tier, change, head, scope, checks, expiry |
| `POST /v0/app/challenge` | a passkey challenge for one operation and target |
| `POST /v0/app/approvals/<grant>` | redeems the grant with an assertion; `hold` keeps the merge |
| `POST /v0/app/auto`, `DELETE /v0/app/auto` | starts an Auto window for 1, 4, 8 or 24 hours, or ends it |
| `PUT /v0/app/device/push` | the device's APNs token |
| `GET /v0/app/profiles` | the instances this device may switch to: Platform only for now |

- **Each route is an `Instance` operation** in the functional core, so its
  parsing, checks and answers are tested without a Worker. The routes call
  the same code as the approval pages and the MCP tools.
- **Reads hold nothing secret.** They show what `change_status` and
  `deploy_status` already show, scrubbed as the observability tools are.
- **Approving redeems the existing grant,** so tiers, the change grant, and
  approval-queues-merge (the approvals ADR's 2026-11-12 amendment) apply
  unchanged.

### 5. Auto: an owner's window

**What Auto is.** For 1, 4, 8 or 24 hours, every change whose merge tier is
`code` or lower is authorized, approved and merged as it passes, without
asking. It is a new kind of mandate, `window`, beside task mandates.

| | Task mandate | Auto window |
|---|---|---|
| Proposed by | the agent, citing an ADR | the owner, in the app |
| Granted with | the code | a passkey |
| Covers | a scope of globs, guarded paths cut out | every change at `code` or lower |
| Lasts | up to 72 hours, 10 merges | 1, 4, 8 or 24 hours, 20 merges |

- **What it never covers:** the `owner` tier. Secrets, destruction and power
  still ask, each time.
- **It pauses itself,** and pushes, as a task mandate does: a deploy fails or
  rolls back, a check fails twice on one head, the budget runs out.
- **Each approval is recorded as `window:<id>`,** with the owner who granted
  it, in the event log and the digest.
- **Starting one is `owner` tier,** as granting a mandate is. Ending one early
  needs a passkey too, so a stolen session can't start or end it.
- **The residual risk.** Auto approves guarded, code-tier changes the owner
  hasn't seen. Its limits are its length, its budget, the pauses, the owner
  tier, and the record of every merge.
- **`Accept edits`** is today's behaviour: each approval asks.

### 6. Push: APNs beside Web Push

- **Token-based APNs:** two new Worker secrets, `APNS_KEY` (the `.p8`) and
  `APNS_KEY_ID`, with the existing `APPLE_TEAM_ID`. Adding them is `owner`
  tier, put with `pigeon cobot workers secrets put`. The Worker signs the
  ES256 JWT with the WebCrypto code `apple.rs` already uses, and reuses it for
  up to 50 minutes.
- **What is pushed** is what Web Push already sends: an approval waiting, a
  mandate or window paused, a deploy failed, the digest. Each notification
  opens `pigeon://approval/<grant>` or Home.
- **The payload holds no secret:** the change's number and title, and the
  operation.
- A token APNs reports gone is dropped from its device.

### 7. Insights: what's counted, and where it's kept

**Storage.** A D1 table, `insights`, migration `0005_insights.sql`:

- `hour` (Unix seconds at the hour), `metric` (`cost` or `errors`),
  `source`, `kind`, `value`;
- hourly rows kept 48 hours, daily sums kept 90 days.

**Collection.** `MainRun`'s alarm fills the last complete hour once an
hour, and fills a missed hour when it next runs. Each source fails alone: a
source that can't be read leaves a gap, and the app shows it as one.

| Source | Errors | Cost | Needs |
|---|---|---|---|
| Cloudflare | GraphQL Analytics: Workers invocations by status (`scriptThrewException`, `exceededResources`), and 5xx responses on `platform.pigeon.dev` | an **estimate**: the same API's usage (Workers requests and CPU, Durable Objects, R2 operations and storage, D1 rows, Containers' vCPU and memory seconds) times the prices in `.config/insights.yaml`, after the plan's included amounts | `OBSERVABILITY_TOKEN` (Account Analytics Read), from the observability ADR |
| GitHub | `runs` rows: failed and cancelled checks; failed dispatches from `actions.rs` | Actions minutes from the same rows, at a price of 0 while the repository is public | nothing new |
| Buildkite | `ios_releases` and the Buildkite API: failed sign steps, cancelled builds, lost agents | each job's minutes times the hosted macOS rate in `.config/insights.yaml` | `BUILDKITE_TOKEN`, as today |

- **Cost is labelled an estimate** in the app and the API. The invoice isn't
  read; Cloudflare's billing API needs a broader token than this is worth.
- **`.config/insights.yaml`** holds public unit prices and included amounts,
  no secret. A sentinel policy checks its shape.
- **The same data serves `stats_show`** and the dashboard's panels, so the
  app and the web read one set of numbers.

### 8. Profiles

- **`/v0/app/profiles` answers Platform alone** in phase one. A profile is an
  instance: its name, base URL, accent and icon.
- **The switcher shows only the profiles that exist:** Platform and Settings,
  with no placeholders for later ones.
- **Later, each noisypigeon colour is a profile,** with its own device session
  and accent, so it's always clear which instance an approval is for. That is
  a later ADR.

### Execution

Each step is its own change, behind the one before.

1. **This ADR.**
2. **The owner's Apple settings,** by hand: enable Associated Domains and Push
   Notifications on the App ID `dev.pigeon.app`, regenerate the App Store
   profile (the grant fetches the active one), and create an APNs key.
3. **Device sessions and the association file:** §2's link page, session
   route and signature check; `/devices`; the association route; `/v0/app/*`
   and the association file added to `public_paths` (`platform.apply`, owner).
4. **Insights:** the migration, `.config/insights.yaml` and its policy,
   collection on `MainRun`'s alarm, and `GET /v0/app/home` and `insights`.
   It needs `OBSERVABILITY_TOKEN`; until the observability ADR's step adds it,
   Cloudflare's rows are gaps.
5. **Approvals in the app:** the challenge and approval routes and the
   approval detail.
6. **Auto windows:** the `window` mandate in the core, its routes, and its
   pauses.
7. **APNs:** the secrets (owner), the sender, and the device token route.
8. **The app:** `PigeonCore` and its tests, then the screens, sign-in, the
   passkey, push and the entitlements. Then a release.

Steps 3 to 7 each add the backplane files they create to this ADR's
`Governs`, with an amendment saying so.

### Verification

- **Signing in:** linking a device needs the owner's proof; a request with a
  missing, stale, replayed or wrongly signed `X-Pigeon-Device` header is
  refused; an unlinked device is refused at once.
- **The passkey:** an assertion made in the app redeems a grant; one replayed,
  or made for another grant or operation, is refused.
- **Auto:** a code-tier change merges unasked inside a window; an owner-tier
  one still asks; a failed deploy pauses the window and pushes; the window
  ends on time.
- **Insights:** a test with recorded GraphQL, GitHub and Buildkite answers
  fills the expected rows; a source that fails leaves a gap and the others
  fill.
- **Push:** an approval waiting reaches the phone, and opens its card.
- **The app:** `PigeonCore`'s tests pass in `ios.check`; build N in TestFlight
  signs in, shows today's insights, and approves a change with Face ID.

## Amendments

### 2026-11-12: Step 3 lands: device sessions, the link page and the association file

- **Paths.** `Governs` gains the files step 3 adds:
  `src/pigeon-cli/crates/backplane/src/core/app/**`, `devices.rs` and
  `worker/devices.rs`.
- **The header** is `X-Pigeon-Device: id=<fingerprint>;t=<unix seconds>;s=<signature>`.
  The device signs `pigeon-device-v1`, the method, the path, the time and the
  body's SHA-256, one per line. The signature may be raw `r‖s`, as CryptoKit
  gives it, or DER.
- **A device's ID is its key's fingerprint:** the first 16 bytes of the
  point's SHA-256, hex. The link page shows it, so the owner can check it
  against the app.
- **Redeeming a link** signs `pigeon-link-v1` and the code. A code is used up
  even when the redemption is refused.
- **`GET /v0/app/whoami`** answers a signed request's device, so the app and a
  test can check signing end to end before step 4's routes.
- **Unlinking on `/devices`** takes the code. The passkey there waits for the
  app's own unlink in step 5.

### 2026-11-12: Step 3 reconciled with the CI window and the docs gate

`main` gained the owner's CI window (!214), the full merge gate on request
(!256) and the docs site behind a passkey (!280) while step 3 waited.

- **Both sides stand.** `worker/mod.rs` serves `devices`, `ci_window` and
  `docs` routes side by side, and the `Instance` keeps `/devices/*` beside
  the rest.
- **The passkey challenge accepts three purposes besides registering:**
  `grant:`, the docs view, and `device:` for the link page.
