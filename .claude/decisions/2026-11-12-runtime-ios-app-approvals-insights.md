## The Pigeon iOS app, phase one: insights, approvals with a passkey, Auto windows, and the backplane it needs

* Date: 2026-11-12
* Model: Claude
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `src/pigeon-ios/Pigeon/**`, `src/pigeon-ios/PigeonCore/**`, `src/pigeon-ios/PigeonCoreTests/**`, `src/pigeon-ios/project.yml`, `src/pigeon-cli/crates/backplane/src/core/app/**`, `src/pigeon-cli/crates/backplane/src/devices.rs`, `src/pigeon-cli/crates/backplane/src/worker/devices.rs`, `src/pigeon-cli/crates/backplane/src/core/insights/**`, `src/pigeon-cli/crates/backplane/src/insights.rs`, `src/pigeon-cli/crates/backplane/src/worker/insights.rs`, `src/pigeon-cli/crates/backplane/migrations/0005_insights.sql`, `src/pigeon-cli/crates/backplane/migrations/0007_insights_sources.sql`, `.config/insights.yaml`, `src/pigeon-cli/crates/backplane/src/worker/app.rs`
* Governs: `src/pigeon-ios/Pigeon/**`, `src/pigeon-ios/project.yml`, `src/pigeon-cli/crates/backplane/src/core/app/**`, `src/pigeon-cli/crates/backplane/src/devices.rs`, `src/pigeon-cli/crates/backplane/src/worker/devices.rs`, `src/pigeon-cli/crates/backplane/src/core/insights/**`, `src/pigeon-cli/crates/backplane/src/insights.rs`, `src/pigeon-cli/crates/backplane/src/worker/insights.rs`, `src/pigeon-cli/crates/backplane/migrations/0005_insights.sql`, `.config/insights.yaml`, `src/pigeon-cli/crates/backplane/src/worker/app.rs`, `src/pigeon-cli/crates/backplane/src/worker/apns.rs`

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

### 2026-11-12: Step 4 lands: insights from GitHub and Buildkite, Cloudflare later

- **Paths.** `Governs` gains `core/insights/**`, `insights.rs`,
  `worker/insights.rs`, `migrations/0005_insights.sql` and
  `.config/insights.yaml`.
- **Collection runs on the hourly cron,** beside the digest, not on
  `MainRun`'s alarm, which only runs while a deploy or release is busy. It
  fills each complete hour since the newest filled, at most 48, and drops rows
  older than 90 days. Hourly rows are kept the whole 90 days; there is no
  separate daily table.
- **GitHub's errors come from D1's `jobs`,** the table that exists, not the
  observability ADR's `runs`. Failed and cancelled code checks count; the
  control plane's own jobs (`bound`, `mirror`, `restack`, `merge`) don't.
  Dispatch errors aren't recorded anywhere yet, so that kind waits.
- **Buildkite's errors** come from `ios_releases` that failed on Buildkite,
  sorted by their error into failed sign steps, cancelled builds and lost
  agents. Its cost is each sign build's jobs, rounded up to the minute, at
  `buildkite_macos_minute`: $0.12, the M4 medium agent.
- **Cloudflare waits for `OBSERVABILITY_TOKEN`.** Its GraphQL fields are
  checked against the live API when the token is added, in their own change.
  Until then Cloudflare's rows are gaps, and the app says so.
- **`.config/insights.yaml`** is flat, `github_actions_minute` and
  `buildkite_macos_minute`, so a policy can check each is a number or `null`
  and that nothing else is in it. That policy is a sentinel change, so it
  follows in its own change, as the factory ADR's §2 requires.

### 2026-11-12: Step 5 lands: approvals in the app

- **Paths.** `Governs` gains `worker/app.rs`; the core is in `core/app/`.
- **An approval's ID is its grant's key,** the SHA-256 of its token. The app
  never holds a token, and the passkey challenge's purpose is the same
  `grant:<key>` the approval page uses.
- **Routes,** each behind the device signature: `GET /v0/app/approvals`,
  `GET /v0/app/approvals/<id>`, `POST /v0/app/challenge` and
  `POST /v0/app/approvals/<id>`. `GET /v0/app/home` lists the waiting ones
  beside the insights.
- **Approving checks the passkey against `PASSKEY_RP_ID`,** the origin a
  native assertion carries, then redeems the grant and runs its operation as
  the owner, `APPROVER`, exactly as the approval page does. The app always
  sends a passkey, even for a confirm-tier grant.
- **Phase one serves change approvals.** Platform, mandate and CI-window
  grants stay on their pages.

### 2026-11-12: The prices file's policy lands

- **`.config/policies/insights.yaml`** checks that `.config/insights.yaml`
  holds `github_actions_minute` and `buildkite_macos_minute`, in that order,
  each a number or `null`, and nothing else after its comments.
- **The sentinel counts 16 policies,** in `crates/sentinel/tests/policies.rs`.

### 2026-11-12: Step 6 lands: Auto windows

- **A window is its own record,** `Windows` in `core/approval/windows.rs`,
  beside mandates rather than a new kind of `Mandate`: it has no ADR, scope or
  plan to cite, and only one runs at a time.
- **What it lets through:** authorize, approve and merge whose tier, by
  `tier_of`, is below `owner`. The owner tier, break-glass and anything else
  still ask. Each is run as `window:<id>`, after a change's mandate and before
  policy.
- **Its limits:** 1, 4, 8 or 24 hours; 20 merges, after which it ends; a
  failed deploy pauses it beside mandates. A paused window approves nothing
  until the owner ends it or starts a new one.
- **Starting and ending** each take a passkey for `window:start` or
  `window:end`, through `POST` and `DELETE /v0/app/auto`.
  `GET /v0/app/home` shows the window running or paused.
### 2026-11-12: Step 8: the app

- **Paths.** `Governs` gains `src/pigeon-ios/PigeonCore/**` and
  `PigeonCoreTests/**`.
- **Swift modes.** `PigeonCore` builds in Swift 6. The app target builds in
  Swift 5 mode, because `AuthenticationServices`' delegates aren't annotated
  for Swift 6's isolation; it has no logic a test needs.
- **The archive is ad-hoc signed** (`CODE_SIGN_IDENTITY=-`), not unsigned, so
  it carries `Pigeon.entitlements`: associated domains for the passkey, and
  `aps-environment`. The sign step's export re-signs it with the App Store
  profile, which has both capabilities.
- **`check.sh` tests `PigeonCore`** on the first iPhone simulator Xcode has,
  after the unsigned simulator build.
- **Linking** opens `/app/link` in `ASWebAuthenticationSession`, which keeps
  the Safari session, so a passkey or Access login already there is reused.
- **The avatar** is the app icon at 240 px, as `Avatar` in the asset catalog.

### 2026-11-12: Step 7 lands: APNs

- **Paths.** `Governs` gains `worker/apns.rs`; the pure part is
  `core/app/push.rs`.
- **The secrets are `APNS_KEY` and `APNS_KEY_ID`,** with `APPLE_TEAM_ID` as
  the issuer. Without them nothing is sent, and Web Push carries on.
- **The provider token** is signed with the WebCrypto ES256 `worker/apple.rs`
  already uses, and kept in the `Instance` for 50 minutes; APNs refuses one
  older than an hour.
- **Each alert** carries the title, the body and a link: an approval page's
  path becomes `pigeon://approval/<grant key>`, anything else
  `pigeon://home`. No grant token leaves the Worker.
- **`PUT /v0/app/device/push`** sets the signed device's token. A token APNs
  answers 410, or 400 `BadDeviceToken`, for is dropped.
- **Production APNs only,** `api.push.apple.com`: TestFlight builds use it,
  and phase one has no development builds.

### 2026-11-13: A source says why it's empty

- **`GET /v0/app/insights` answers `note`** when a source isn't collected,
  or only partly, for the metric: Cloudflare until `OBSERVABILITY_TOKEN`;
  GitHub's cost, $0 while the repository is public; GitHub's dispatch errors,
  not yet recorded. The chart shows it under the total. Home's tiles carry
  none.
- **Totals sum from +0.0.** An empty range read `$-0.00`, because `f64`'s
  `Sum` starts at -0.0.

### 2026-11-14: §8's profiles are instances

[`2026-11-14-runtime-accounts-profiles-instances.md`](2026-11-14-runtime-accounts-profiles-instances.md)
renames §8's "profiles" to **instances**: `/v0/app/instances`, and the
**Instances** sheet, opened from Home's instance chip. "Profile" now means
the signed-in person's own: name, avatar, appearance, passkeys and devices,
behind Home's avatar.

- **Appearance moves** from the phone's storage to the profile.
- **Name, avatar and appearance** take only the device signature. Every
  other write still asks for a passkey, as §3 decides.

### 2026-11-14: A window never approves a brief's head awaiting review, or escalated

Per
[`2026-11-13-development-model-tiered-agent-briefs.md`](2026-11-13-development-model-tiered-agent-briefs.md)
§5 and §6:

- **A change carrying out a brief** is approved by a window only once its
  brief's planner has accepted that head (`brief_review` with `accept`). It
  is never approved by one while it is escalated to the planner.
- **The gate is in the lifecycle,** not in `windows.rs`. `Change::approve`
  refuses any approver named `window:`, `mandate:` or `policy:` for such a
  head. A window that would allow it approves nothing, and the head stays
  `in-review` or `escalated`.
- **Once the head is accepted,** a window approves it as any other: on the
  next ask, or by re-approving it after a restack, per the session-stages
  ADR's 2026-11-14 amendment.
- **The owner's own approval is unchanged.** Approving without a review is
  theirs to do.

### 2026-11-15: Three tabs: Tracker, Approvals and Insights

The owner approved a mockup of three tabs and an approval push's deep link.
Home's single screen becomes a `TabView`:

| Tab | What it shows | Route |
|---|---|---|
| **Tracker**, the default | each live session and its changes, each on a stage rail | `GET /v0/app/tracker`, live by `GET /v0/app/events` |
| **Approvals** | Accept edits / Auto, everything waiting on the owner, today's history | `GET /v0/app/home`, unchanged |
| **Insights** | deploys, the merge queue, providers, throughput, approval wait, failed jobs, cost and errors | `GET /v0/app/ops`, and `GET /v0/app/insights` for cost and errors |

Each route is an `Instance` operation per §4, behind the device signature,
with its pure part in the core and tested there.

**The rail.** A change's stage maps to one of ten stops:

| Stop | Stages |
|---|---|
| 0 proposed | `proposed` |
| 1 authorized | `authorized` |
| 2 pushed | `bounding`, `conflicted`, `restacking`, `refused` |
| 3 checking | `checking`, `checking n/m`, `failed` |
| 4 checked | `checked`, `in-review`, `escalated` |
| 5 approved | `approved` |
| 6 queued | `queued n/m` |
| 7 merging | `merging` |
| 8 merged | `merged` |
| 9 deploying | `deploying`, `done`, `deploy-failed` |

- **Each rail answers `stop`, `stage` and a `mark`:**
  - `failed`, for `refused`, `failed` and `deploy-failed`;
  - `stalled`, when `needs` is `agent` and the session hasn't polled for
    30 minutes (`tasks::stalled`);
  - `done`, for `done`;
  - `ok` otherwise.

  The app draws a mark with a shape and a word as well as a colour.
- **`closed` changes are left out.** A merged change stays on its session's
  card until its stage is `done`, at most an hour after its merge.
- **A row carries:**
  - from the stage: `number`, `title`, `needs`, `waits_on`, `next` and
    `since`;
  - from `briefs::all`: its brief's `id`, `title`, `adr` and `step`.
- **A session carries `label`, `model`, `role`, `planner`, `brief` and
  `listening`,** from `push::build`'s session lines and the tickets.
  - A live session with no open change is shown, so an executor that hasn't
    proposed yet is visible.
- **`counts`** sums the changes that need the owner, the stalled ones and
  those waiting on a provider. The chips at the top filter by them.

**Live.**

- `GET /v0/app/events?after=&seconds=` calls `events::wait` as the owner.
  - It passes no `session`, so no agent's cursor moves.
  - `seconds` is at most 25. It answers `{events, after}`.
- The app long-polls it while the Tracker is on screen. On any event it
  reads `/v0/app/tracker` again, at most once every 2 seconds.
- On an error it falls back to reading the tracker every 30 seconds. It also
  reads again when the app returns to the foreground.

**Operational insights: `GET /v0/app/ops?range=1|7|30`.**

| Panel | Answer | Source |
|---|---|---|
| Deploys | passed of total in range; the newest 5 with `sha`, `outcome`, `at` | `MainRun`'s `Pipeline.history`; no durations are kept |
| Merge queue | `depth` now and the holder; minutes from `change.approved` to `change.merged` per merge, with p50 and max | the `Instance`'s queue; the event log |
| Approval wait | minutes from `change.checked` to an owner's `change.approved`, with p50 and p95 and how many | the event log |
| Throughput | merges per day; proposed-to-merged p50; merges by approver: `owner`, `window`, `mandate`, `policy` | D1 `changes` |
| Providers | each macOS provider's health, minutes and cap | `macos::status`, as `deploy_status.macos` |
| Failed jobs | failed and cancelled jobs, newest 10, with `name`, `change` and `at`; macOS jobs `lost`, `unassigned` or `refused` | D1 `jobs` and `macos_jobs` |

- **The event log keeps seven days.** A 30-day range answers the queue and
  approval panels over seven, with a `note` saying so.
- **GitHub Actions minutes aren't recorded,** so the GitHub provider row
  shows health alone.
- **Worker errors and Cloudflare's cost** stay gaps until the grants ADR's
  `observability` grant puts `OBSERVABILITY_TOKEN`.
- **Job Containers that fail to start** join the failed jobs once the
  reliability ADR (!469) records them. Until then they aren't counted.

**The deep link.**

- A push tap is handled by `AppDelegate`, which keeps the link until the
  tabs are on screen, so a cold start doesn't drop it.
- `pigeon://approval/<id>` selects Approvals and opens that card.
  `pigeon://home` selects Tracker.
- The card stays passkey-only, as the passkey-only cutover requires.

**Execution, as `sonnet` briefs, each its own change, in order:**

1. The tracker and events routes: `core/app/tracker.rs`, `worker/app.rs`
   and `worker/devices.rs`.
2. The ops route: `core/insights/ops.rs`, `worker/insights.rs` and
   `worker/devices.rs`, built on 1.
3. `PigeonCore`: the tracker's and ops' models, the rail, and
   `PigeonCoreTests`.
4. The app: the tabs, the Tracker and Insights views, live updates and the
   deep link, built on 3.

Then an `ios.release`.

### 2026-11-15: A failed deploy holds Auto, and a passed deploy resumes it

The owner asked that a failed deploy stop ending Auto's usefulness: work that
could make things worse waits, the fix goes through, and Auto resumes by
itself. §5's "it pauses itself… a deploy fails or rolls back" and step 6's
"a failed deploy pauses it" are replaced by a **hold**.

- **`held` is a state of its own,** beside `active`, `paused` and `ended`, with
  a `hold`: the failed SHA, why, when it began, and how many deploys have
  failed in it. A failed or rolled-back deploy of `main` holds every active
  window and mandate. A paused one stays paused.
- **During a hold, only merges wait.** A merge is the one operation that
  deploys.
  - Authorizing, approving and closing go through as before, so work reaches
    `approved` and is ready.
  - A merge goes through only for a change whose title's type is `fix` or
    `revert`. `MainRun` deploys the newest push, so that merge is the next
    deploy.
  - Any other merge answers `held`: it isn't run, isn't listed for the owner,
    and its change's stage says it waits on a deploy.
  - Merges already queued when the hold begins still run; they were approved
    before the failure.
  - Why the title: it is deterministic and already checked by policy, and
    agents already name fixes so. The window trusts the agent at the code tier
    anyway, so a mislabelled change gains nothing a window doesn't already
    grant. The digest names every merge made during a hold.
  - Why not "paths in the failed deploy's diff": a fix often lies elsewhere,
    such as a token or a Worker binding, and tonight's needed no code at all.
- **Any passed deploy of `main` resumes every held window and mandate,** a
  `deploy_retry` with no new code included.
  - It is the same window, with its budget and merges untouched.
  - Its expiry moves later by the time it was held. The owner bought hours of
    unattended work, and a hold gives none, so the hold costs nothing. The cap
    below bounds the stretch.
- **A hold ends the window or mandate when it reaches 6 hours or 3 failed
  deploys,** whichever comes first, pushing why. A main that can't deploy for
  that long needs the owner, not more fixes.
- **Time doesn't run during a hold.** A held window never expires; its expiry
  is moved on resume. The budget running out still ends it.
- **Unchanged:**
  - the `owner` tier always asks, held or not;
  - the owner may end a held window (`DELETE /v0/app/auto`), and starting a
    new one ends a held one, as it does a paused one;
  - only a deploy failure holds. A check failing twice on one head isn't
    implemented yet; when it is, it pauses, since it is one change's fault,
    not `main`'s.
- **The owner is told.**
  - On a hold, a push, "Auto held": "window `<id>` held: the deploy of `<sha>`
    failed: `<why>`; it resumes when a deploy passes".
  - On a resume, "Auto resumed", with the time held and what is left.
  - On a capped end, "Auto ended".
  - Events `auto.held`, `auto.resumed` and `auto.ended`, and a digest line for
    each held window or mandate.
- **The app** gets `state: "held"` and `hold` on `GET /v0/app/home`'s window
  and shows "Held". The tracker's rail maps the stage `held` to stop 5,
  approved, with a `held` mark.

**Execution,** each its own change, at the owner's tier:

1. Core: `State::Held`, `Hold`, `Decision::Hold`, hold, resume and the cap in
   `windows.rs` and `mandates.rs`, with tests for the hold, the fix's merge,
   the resume, repeated failure and expiry.
2. Worker: `hold_all` and `resume_all` in `worker/mandates.rs`, called from
   `MainRun` on a failed and a passed deploy; a `held` decision answers the
   agent without listing an approval; the pushes and events.
3. Stages and digest: the `held` stage, `auto.resumed` waking agents, the
   digest's held lines, and the rail's mark.

### 2026-11-15: Cloudflare is collected, and a source's note says what happened

The `observability` grant put `OBSERVABILITY_TOKEN` on `pigeon-control` at
04:41 UTC, but the app still said Cloudflare waits for it. Step 4's amendment
deferred Cloudflare's collection to "its own change, when the token is added",
and that change was never made. `note` returned a constant, and nothing read
the token. Redeploying couldn't fix it. This amendment is that change.

- **Collection.** The hourly cron reads Cloudflare's GraphQL Analytics
  (`/client/v4/graphql`) with `OBSERVABILITY_TOKEN`, for the account the
  `ARTIFACTS_TOKEN` sees. It sends one query per dataset, so a field the live
  API refuses fails only that dataset:

  | Dataset | Rows |
  |---|---|
  | `workersInvocationsAdaptive`: `requests`, `cpuTimeUs` by `status` | errors: `Worker exceptions` (`scriptThrewException`), `CPU limit exceeded` (`exceededResources`); cost: `Workers` |
  | `durableObjectsInvocationsAdaptiveGroups`: `requests`; `durableObjectsPeriodicGroups`: `activeTime` | cost: `Durable Objects` |
  | `d1AnalyticsAdaptiveGroups`: `rowsRead`, `rowsWritten` | cost: `R2 and D1` |
  | `r2OperationsAdaptiveGroups`: `requests` by `actionType` | cost: `R2 and D1` |

  - Cloudflare fills its own hours, from the newest hour it filled, at most
    48. An hour fills with the datasets that answer, and the run records the
    ones that failed. If none answers, the hour stays due and the next run
    retries it.
  - Not yet counted, and said so in the note: 5xx responses on the platform
    hostname, which need Zone Analytics; Containers; storage; and the plan's
    included amounts. The cost is gross usage times the unit prices, an
    estimate as §7 says, and it reads high while usage is inside the plan.
- **Prices.** The Workers Paid list prices, in dollars per million units, are
  a constant in `core/insights/cloudflare.rs`: `cloudflare_workers_requests`,
  `cloudflare_workers_cpu_ms`, `cloudflare_do_requests`,
  `cloudflare_do_gb_seconds`, `cloudflare_d1_rows_read`,
  `cloudflare_d1_rows_written`, `cloudflare_r2_class_a` and
  `cloudflare_r2_class_b`. They aren't in
  `.config/insights.yaml`, because that file's policy (`only: true`) refuses
  new keys. Changing the policy is a sentinel change of its own, and whichever
  of the two merged first would leave `main` failing it. Moving the prices
  there is a follow-up. Durable Objects' duration bills at 128 MB, so
  GB-seconds are active seconds × 0.125.
- **Each run's outcome is kept.** Migration `0007_insights_sources.sql` adds
  `insights_sources`: `source`, `at`, the newest `hour` filled, and `failed`,
  the datasets that failed and Cloudflare's words, scrubbed of the token.
  This is expand-only.
- **The note is the truth at read time,** in this order:
  - no `OBSERVABILITY_TOKEN` on the Worker: "Not collected: the Worker has no
    OBSERVABILITY_TOKEN.";
  - the token is present but no run has finished: "Token present; the first
    collection runs at :17 past the hour.";
  - the newest run failed: "Token present; Cloudflare's analytics query failed
    at HH:MM UTC: <datasets: why>.";
  - otherwise, what isn't counted, from the list above.
- **`OBSERVABILITY_TOKEN` is declared** in `worker.yaml`'s `secrets`, beside
  the others. The grant puts it, so `secrets put` is never needed for it.
- **The app is unchanged.** It shows whatever `note` the route answers, so no
  TestFlight build is needed.

**Execution:** one change. The core's query, parse, pricing and note, with a
recorded answer in the tests; the migration; the Worker's collection and the
note at read time; and `worker.yaml`. After it deploys, the first
cron run's note shows whether the live API accepted each field. A field it
refuses is fixed in a follow-up `fix` change.
