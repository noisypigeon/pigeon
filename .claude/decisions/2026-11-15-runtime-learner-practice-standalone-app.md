## Learner practice: the first app feature, on an app-only standalone instance listed privately

* Date: 2026-11-15
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `src/pigeon-ios/Pigeon/**`, `src/pigeon-ios/PigeonCore/**`, `.config/roles.yaml`, `catalog/pigeon/kinds/platform/schema.json`, `infra/pigeon/workers/control/worker.yaml`, `src/pigeon-cli/crates/backplane/src/worker/members.rs`

## Context

The owner asked for a practice app for one learner preparing for the BC Class 7L knowledge test, inside the pigeon iOS app:

- it runs as the learner's own instance, in its own zoneless Cloudflare account, with its own R2 state for their progress and every question they have answered;
- it shows in the owner's instance list, so the owner can see their progress;
- the learner gets the app from TestFlight and signs in with their own passkey.

The pieces this builds on, and what they leave open:

- **Standalone instances.** [`2026-11-13-runtime-non-platform-profiles.md`](2026-11-13-runtime-non-platform-profiles.md) §1 and §7 (renamed to instances by its 2026-11-14 amendment) design a zoneless instance as a full copy of the platform stack. That needs Containers and the Workers Paid plan; a quiz needs a Worker, a Durable Object and a bucket. Its §8 lists every instance in `.config/instances.yaml`, a public file. None of it is built.
- **Members.** [`2026-11-14-security-instance-members-roles-invitations.md`](2026-11-14-security-instance-members-roles-invitations.md) §2 gives role `user` no capability, and §5 hands a standalone only its administrators' keys at stand-up. Adding a member to a standalone afterwards is not designed. Its gate and `/join` (steps 5a to 6b) are not built.
- **Passkeys.** [`2026-11-14-security-passkey-only-owner-proofs.md`](2026-11-14-security-passkey-only-owner-proofs.md) §5: one relying party, `platform.pigeon.dev`; a passkey belongs to one account. A `workers.dev` host can verify an assertion for it but cannot register one.
- **The app.** [`2026-11-14-runtime-accounts-profiles-instances.md`](2026-11-14-runtime-accounts-profiles-instances.md) §4 gives Home an instance chip and `InstancesSheet`, which today hard-codes Platform. The app has no per-instance feature screens.
- **Distribution.** [`2026-11-11-release-ios-app-buildkite-testflight.md`](2026-11-11-release-ios-app-buildkite-testflight.md) §8 ships to an internal TestFlight group and leaves external groups out.
- **Privacy.** The repository is public. A membership is kept out of it because it names a person (the members ADR, "Step 2 lands"). An instance made for one person names that person the same way.

The owner chose an app-only stack and a private listing.

**Out of scope:** other tests or provinces, more than one learner per instance, handwriting recognition, Android, and any change to how the platform instance itself runs.

## Implementation plan.

### 1. An app-only standalone

- The `pigeon/platform` kind gains `shape: full | app`, default `full`. `shape: app` allows `domains` to be empty and requires `instance` to be set.
- An `app` stack has the control Worker, its `Instance` Durable Object and one private R2 bucket, `<instance>-<random suffix>`. It has no Containers, `Checker`, `ChangeRun`, `MainRun`, D1, Artifacts namespace or Access application.
- The Worker serves on `workers.dev` (`worker.yaml`: `workers_dev` is true when `domains` is empty) and answers only `/v0/app/*`. Every other path is 404.
- Device signatures and passkey proofs are its whole authentication. With no Access in front, every route checks `X-Pigeon-Device` before it does anything else.
- It fits Cloudflare's free plan: Workers, SQLite-backed Durable Objects and R2's free tier.
- Stand-up is `instance.standup` (the instances ADR §8) with `shape: app`. It creates the bucket and the Worker and generates `STATE_KEY` into it. As there, pigeon keeps no credential for the stack and hands over only public keys.

### 2. A personal instance is listed privately

- A personal instance is recorded in the platform's `Instance` Durable Object under the key `instances.private`. It is never written to `.config/instances.yaml` or any other repository file, and never into a change, commit, branch name or log.
- An entry is `{name, mode: standalone, shape: app, base_url, accent, features, created}`. Its name, the Cloudflare account's name and the `workers.dev` host may name a person; none of them leaves the Durable Object.
- Adding, standing up or removing one is an owner-tier operation proved by passkey. It does not open a change, because there is nothing for the repository to hold.
- `GET /v0/app/instances` returns the ledger's instances plus the private ones the caller is a member of.

### 3. The `learner` feature

- An instance's `features` may include `learner`. This is the first app feature: a screen set in the app plus routes on the instance's Worker, released with the platform. It is not a job kind.
- **Questions.** A bank of original four-choice questions written from ICBC's *Learn to Drive Smart* and official BC sources lives at `catalog/learner/bc-7l/questions.json`. Each question has one correct answer, a topic, a one-line explanation and its source section.
  - Where more than one statement is true, the single answer is "All of the above" or "Both A and C".
  - A question whose fact is unconfirmed carries `unverified` and is never printed.
  - The bank is public content and holds nothing about any person.
- **Forms.** A form is 25 questions: about a fifth sign questions, a tenth combined-answer questions, and the rest round-robin across topics. Topics the learner misses more often are drawn more often.
  - Its number is 10 bits and its page 2 bits. The form is stored (§5) before it prints, so the key is never recomputed.
- **The page.** One US Letter page per 25 questions:
  - four corner anchors, the top-left one larger, so a reader can tell which way is up;
  - a 12-bit code strip giving the form and page;
  - a date box and a score box;
  - two columns, each question's answers in a 2 × 2 grid, with a 3.7 mm circle beside each answer.
  - `PigeonCore` owns this geometry once. The renderer and the reader both use it.
  - The app renders the PDF and prints it through AirPrint at 100%.

### 4. Scoring from a photo, on the phone

- The camera finds the four anchors, takes the picture once they are steady, straightens it (`CIPerspectiveCorrection`), reads the code strip, and samples every circle at its known position.
- A circle that is 55% or more dark is marked. One between 20% and 55% is unclear.
- Two marks on one question score as wrong. Unclear marks are shown to the learner to confirm before scoring.
- **The photo never leaves the device** and is discarded after scoring. Only the marks are sent.

### 5. The learner's state

- Everything lives in the instance's own bucket under `learner/`, each object sealed with AES-GCM under `HKDF(STATE_KEY, "pigeon-learner-v1")`:
  - `forms/<form>.json`: the question IDs in order and the key;
  - `attempts/<ulid>.json`: form, page, date, marks and each question's result;
  - `progress.json`: for each question, times seen, right, wrong and last seen.
- The Durable Object keeps only an index (attempts, scores and dates) for Home, and rebuilds it from the bucket when it is lost.
- **Routes**, each an `Instance` operation in the functional core:
  - `GET /v0/app/learner/progress`
  - `POST /v0/app/learner/forms`
  - `GET /v0/app/learner/forms/<n>`
  - `POST /v0/app/learner/attempts`
- **Hints.** Each missed question shows the learner's answer, the right one, its explanation and its source section ("Learn to Drive Smart, Chapter 6"). The text is gentle and second-person.
- **Auto by default.** Once the learner's device is linked (§7), their two writes are pre-approved from the start: generating a new test (`POST /v0/app/learner/forms`) and saving an assessed one (`POST /v0/app/learner/attempts`).
  - Each is signed by the device alone. There is no passkey prompt and no owner approval, and no window to open or renew.
  - This narrows the iOS app ADR's §3 rule (every write takes a passkey) for these two operations only, whose data is the learner's own.
  - A passkey is still required for membership, role and instance operations.

### 6. Roles

- A new capability, `learner`, in `.config/roles.yaml`:
  - `user: [learner]`
  - `administrator` gains `learner` too, as the members ADR §2 requires for any new capability.
- `learner` allows exactly three operations: reading the instance's progress, generating a test, and saving an assessed one. On an instance without the `learner` feature it allows nothing.
- `user` gains nothing else. In particular it does not gain `auto`: the platform's Auto window approves changes, and a learner proposes none. A learner never changes the questions, the scoring or the instance.
- The learner is a **User** of their instance. The owner is an administrator of it, from stand-up.

### 7. Adding a member to a standalone

- An administrator invites from the app with `member.invite`, naming the private instance. The `/join/<token>` page and its passkey enrolment run on `platform.pigeon.dev`, the relying party, and create a `person` account there with no membership on the platform instance.
- After the match code, `member.admit` (owner tier) records the membership in the platform's private listing. The owner's app then posts the account's ID, credential ID and public key, and the role, to the stack's `POST /v0/app/members`. The stack verifies the owner's device signature and passkey proof against the keys it was handed at stand-up.
- The learner's app links its device to their stack with a native passkey assertion (`ASAuthorizationController`, associated domain `platform.pigeon.dev`). The stack verifies it against their handed-over public key.
- No credential crosses between the platform and the stack in either direction.

### 8. The app

- `InstancesSheet` lists `GET /v0/app/instances`. Choosing an instance moves the app's API base to that instance's `base_url`.
- An instance with `learner` opens on its Progress screen:
  - **Progress:** score chart and topics to brush up on;
  - **Scan:** the camera;
  - **Check marks:** confirm unclear marks;
  - **Score:** saved to progress;
  - **Hints:** study tips for missed questions.
- A User sees all five screens. An administrator sees Progress and Hints read-only, with no Scan.
- An account with a single membership opens straight into that instance.

### 9. Distribution

- The learner is added as an internal TestFlight tester by the owner in App Store Connect, outside the repository. The release ADR §8 is unchanged.

### 10. Steps

Steps 1 to 3 need nothing else; the rest wait on the members ADR's steps 5a to 6b and on `instance.standup` (the instances ADR §8).

1. The question bank, the form generator and the page geometry in `PigeonCore`, with tests: one form fits one page; a form's key round-trips; weighting favours missed topics.
2. The PDF renderer and the five screens, with progress kept on the device until step 7. The learner can practise from here.
3. The on-device reader, tested against rendered pages photographed at an angle.
4. `learner` in `.config/roles.yaml` (owner tier).
5. `shape: app` in the platform kind, `worker.yaml`'s `workers_dev`, and the `/v0/app/*`-only router.
6. `instances.private`, the merged `GET /v0/app/instances`, and `InstancesSheet` from it.
7. The `learner` routes and the sealed bucket store. Step 2's on-device progress moves into the instance.
8. `POST /v0/app/members` on a standalone, and the hand-over from the owner's app.
9. Owner steps:
   - create the learner's Cloudflare account and its `workers.dev` subdomain;
   - stand up the instance from the app;
   - invite the learner and admit them;
   - add them to TestFlight.

## Amendments

### 2026-11-15: the bank is an app resource, a form keeps its circles' positions, and steps 1 to 3 are briefs

- **The bank** lives at `src/pigeon-ios/Pigeon/Learner/questions.json`, not under `catalog/`, which holds provider catalogs (`catalog/README.md`). This amendment adds it to what the ADR governs.
  - It is one JSON array, one question per line: `id`, `topic`, `question`, four `choices`, `answer` (`A` to `D`), `explanation`, `source`, and optional `unverified` and `combined` flags.
  - It holds 267 questions in 11 topics. 19 are `unverified` and are never drawn; 17 are `combined`.
  - Questions on the GLP rules of 19 October 2026 follow the BC news release of that change. Older manuals and practice sites disagree with them.
- **A form keeps its layout.** Wrapped text sets where each circle falls, and text measurement differs between devices and font versions. So the form's record (§5) keeps every circle's centre in millimetres from the page's top-left, as laid out when it printed, and the reader samples there. Nothing is re-measured after printing.
- **Steps 1 to 3** are carried out as briefs: step 1 is `PigeonCore`'s learner model, form generator and page layout; step 2 is the PDF, the five screens and on-device progress; step 3 is the reader.
### 2026-11-15: Review of !482 against the iOS app ADR: they agree

- **!482 carries out the iOS app ADR's 2026-11-15 amendment, execution 3**
  ([`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md)).
- **What it adds to `PigeonCore`:**
  - `Tracker.swift` and `Ops.swift`, models for `/v0/app/tracker`,
    `/v0/app/ops` and `/v0/app/events`;
  - their pure helpers;
  - `PigeonCoreTests/TrackerTests.swift`.
- **It touches nothing this ADR decides:**
  - no question bank, form generator or page geometry;
  - no `learner` capability or attempt storage.
  - Its public names (`Tracker*`, `Ops*`, `Rail*`, `railStops`, `elapsed`,
    `minutesText`, `barHeights`) leave this ADR's names free.
- **The two agree, and neither changes.** This ADR's `PigeonCore` work adds
  files beside these and reuses none of them.

### 2026-11-15: a sample page, form 1023, that runs the whole flow and keeps nothing

The owner wants to try print, scan, check marks and score on someone else's
phone without that person sitting a real test.

- **Form 1023 is the sample.** Its code strip is all ten form bits set, page 0.
  A real form never takes it: new form numbers wrap modulo 1023, not 1024.
- **Its questions are plainly not the test.** 25 fixed questions in
  `PigeonCore`, topic `Sample`, each starting "Sample <n>:", such as "Sample 1:
  Which colour is the sky?". None comes from the bank or from ICBC.
- **Its circles never move.** The sample page is laid out without wrapping, so
  its circles are the same on every device, and the reader takes them from
  `PigeonCore` rather than from a stored form (the first amendment's rule is
  for real forms, whose text wraps).
- **Two printings.** A blank sample to fill by hand, and a filled-in one whose
  circles are already marked (21 right, 2 wrong, one double and one blank), so
  the scanner can be tried with nobody filling it in. Both print through
  AirPrint and can be saved as a PDF from the app. Each is headed "SAMPLE PAGE"
  and says it is not a real test.
- **It runs every step.** The camera finds its anchors, straightens it, reads
  its code strip and its circles; the check-marks screen, the score and the
  study tips follow, each labelled as a sample.
- **Nothing is kept.** A sample is scored in memory and discarded: no stored
  form or circles, no attempt, no progress by topic, no weighting of the next
  form, nothing in the score chart. `PigeonCore`'s `LearnerLedger` holds the
  rule, so it is tested; the app saves only what the ledger keeps.
- **Where it is.** "Try a sample page", a quiet link under the learner home's
  buttons, opens a screen to print either sheet, save its PDF, and scan it. A
  sample scanned from "Scan a page" is recognised by its code strip the same way.
- **Tests:** the filled-in sample, drawn as a camera sees it, reads back to its
  marks and scores 21; recording a sample leaves the ledger unchanged.

### 2026-11-15: the scan counts down 3-2-1 on a steady page, with a manual shutter

On build 14 the camera captured as soon as the page was in frame, before the
learner had steadied it. §4's "once they are steady" becomes:

- **A countdown.** Once the four anchors are found, a 3-2-1 countdown shows
  over the camera, and the frame at 0 is read.
- **It restarts.** If an anchor is lost, or any anchor moves more than 2% of
  the picture's shorter side between frames, the countdown cancels and starts
  again at 3 when the page is back and steady. A frame at 0 whose code strip
  cannot be read restarts it too.
- **A manual shutter.** A button reads the current frame at once, if its
  anchors are found.
- `PigeonCore`'s `ScanCountdown` holds the rule, so it is tested; the reader
  takes anchors already found, so a frame is searched for them once.
### 2026-11-15: The standalone joins, stands up and carries over on its own; steps 4 to 8 are split

Steps 4 to 8 waited on the members ADR's steps 5a to 6b and on
`instance.standup`. Neither is on `main`, and the learner needs only a
little of either. This amendment builds that little here.

**Dependencies, resolved.**

- **The members ADR's steps 5a to 6b stay with that ADR.** The learner never
  calls the platform, so the platform's gate (5a to 5d) and its pages'
  sign-in cookie (6a) are not needed. Invitation, enrolment and admission run
  on the standalone instead (below), over `core/member`'s existing `invite`,
  `enrol` and `admit`.
- **A standalone's roles are compiled in.** It reads the release's
  `.config/roles.yaml` through `include_str!` and refuses to start serving if
  `check_roles` fails. 5a's deploy report stays the platform's way.
- **`instance.standup` is built for `shape: app` only,** as a third task of
  the platform job, `standup <instance>`, on a Runner. The owner pastes a
  one-time token for the new account on the platform-run page and proves it
  with a passkey, as for `platform.apply`. `shape: full` stays unbuilt.

**§1, the app shape, made exact.**

- **The kind.** `shape: full | app`. With `app`, `domains` may be empty and
  `instance` is required. `database`, `queue`, `access` and `service_tokens`
  are not required.
- **The Worker template** is `catalog/pigeon/kinds/platform/app-worker.yaml`,
  not under `infra/`, so `pigeon cobot workers` never deploys it into the
  platform's account.
  - It has the same crate and features as the control Worker.
  - Its only Durable Object is `Instance`, and its one R2 binding is
    `LEARNER`.
  - Its vars are `INSTANCE_NAME`, `SHAPE: app` and `PASSKEY_RP_ID:
    platform.pigeon.dev`; its secrets are `STATE_KEY` and `HANDOVER`;
    `workers_dev` is true.
  - The control Worker's `worker.yaml` is unchanged.
- **The router.** With `SHAPE=app`, `fetch` answers `/v0/app/*` and nothing
  else. It returns 404 for any other path before it reads another binding or
  secret, and it needs no Access settings.
- **The hand-over.** `HANDOVER` holds public keys only: the owner's account
  ID, their passkeys' IDs and public keys, and their devices' IDs and public
  keys. On its first request, the `Instance` seeds `accounts`, `members` (the
  owner as administrator), `devices` and the passkey ring from it if they are
  empty, and never again.
- **The job** creates the bucket, renders the template, deploys it with
  wrangler, puts `STATE_KEY` (32 random bytes, made in the job) and
  `HANDOVER`, and checks `GET /v0/app/instances` there. It reports only the
  base URL and the bucket.
  - Its log names no instance, account or host: wrangler's output is not
    kept.
  - A bundle over the free plan's compressed Worker size fails the job with
    that size, before anything is created. Moving to Workers Paid is then the
    owner's decision, because it is a spend.

**§2, the private listing.**

- `standup` records the entry under `instances.private` when the job
  reports. The owner's devices' public keys are already on the stack, so the
  owner's app is linked there from the first minute.
- `GET /v0/app/instances` on a standalone answers itself alone.

**§5, the store, and the carry-over.**

- **Forms.** The app makes a form and lays out its circles (`PigeonCore`),
  and the instance stores both, as `forms/<n>.json`.
- **Attempts.** The instance recomputes an attempt's results from the stored
  key; the app's results are not trusted.
- **Sync.** `POST /v0/app/learner/sync` takes `{forms, attempts}` and is
  idempotent: an item already held with the same content is accepted again,
  and one held with different content is refused by name. It answers every
  form number and attempt ID the instance holds. `POST .../forms` and
  `POST .../attempts` are its one-item cases.
- **Sealing.** Each object is sealed with AES-GCM under
  `HKDF-SHA256(STATE_KEY, info "pigeon-learner-v1")`.
- **The phone's store stays the cache.** Its files and their format, the
  ones step 2 wrote, do not change.
  - An item is pending until a sync answer names it.
  - Nothing on the phone is deleted, so a failed or partial sync loses
    nothing, and is retried on the next open and after each write.
  - Once linked, the first sync carries everything practised before the link.
- **The carry-over's test** is in `PigeonCoreTests`. It seeds a store in step
  2's file format with forms, circles and scored attempts, then syncs against
  a fake instance. It checks three things:
  - every item arrives, with the same content;
  - a sync that fails, or answers only part, leaves the rest pending and
    every file in place;
  - once confirmed, the store reads back unchanged.

  A Rust test holds the instance's side: sync is idempotent, refuses a
  different content by name, round-trips through the seal, and recomputes
  results.

**§7, adding a member, replaced.** Every step runs on the standalone, and the
platform holds no account for the learner.

1. **Invite.** The owner's app, on the standalone, posts
   `POST /v0/app/members/invites` with the email and role `user`, proved by
   passkey against the standalone's challenge. It shows the answer as a
   `pigeon://join?base=<base URL>&token=<token>` link and its QR code, which
   the owner sends to the learner. The link never passes through the
   platform.
2. **Enrol.** The learner's app, opened by the link:
   - asks `POST /v0/app/join/challenge` with the token for a challenge;
   - creates a passkey natively for relying party `platform.pigeon.dev`,
     through its associated domain;
   - takes the authenticator data and the public key from the attestation
     with `PigeonCore`;
   - posts `POST /v0/app/join` with the token, the passkey, and its device
     key signed as a link is.

   The standalone checks the client data and the relying party's hash, then
   creates a `person` account, enrols it and holds the device. It answers the
   match code. The passkey-only ADR's "a `workers.dev` host can't register" is
   unchanged: the app registers, and the stack only verifies.
3. **Admit.** The owner's app shows the enrolled invitation's match code. The
   owner compares it with the learner's screen, then admits with a passkey
   (`POST /v0/app/members/admit`). The learner becomes a User, and the device
   is linked to their account.
4. **Linked.** The learner's app asks `GET /v0/app/join/<id>`, which answers
   only the state, until it reads `admitted`. It then keeps the standalone as
   its one instance, and syncs.

**Distribution and the link.** The app registers the `pigeon` URL scheme
(`project.yml`). This amendment adds `catalog/pigeon/kinds/platform/**`,
`src/pigeon-ios/project.yml`, `.config/policies/roles.yaml` and the new
backplane files listed below to what the ADR governs.

**Steps 4 to 8, as changes.** The planner carries each one: each is owner
tier, touches device or passkey proofs, or is the carry-over the owner made a
hard requirement, so no brief would leave its executor without a choice.

| # | Change | Paths |
|---|---|---|
| 4 | `learner` in `.config/roles.yaml`, its policy, and `CAPABILITIES` | `.config/roles.yaml`, `.config/policies/roles.yaml`, `core/member/` |
| 5a | `shape`, the app template, the app-only router, the hand-over seed | `catalog/pigeon/kinds/platform/**`, `worker/standalone.rs`, `worker/mod.rs` |
| 5b | `standup <instance>` in the platform job, `instance.standup`, and `instances.private` | `platform_job.rs`, `worker/platforms.rs`, `worker/standalone.rs` |
| 6 | `GET /v0/app/instances`, the app's per-instance API base, and `InstancesSheet` | `worker/standalone.rs`, `Pigeon/**`, `PigeonCore/**` |
| 7a | The learner routes, the sealed store and sync | `core/learner/**`, `worker/learner.rs` |
| 7b | The app's sync and carry-over, with its test | `Pigeon/Learner/**`, `PigeonCore/**`, `PigeonCoreTests/Learner*.swift` |
| 8a | Invites, join and admit on a standalone | `worker/members.rs`, `core/member/**` |
| 8b | The owner's invite and admit, and the learner's join | `Pigeon/**`, `PigeonCore/**`, `project.yml` |

- Step 9 (the owner's steps) becomes a guide sent to the owner once 8b is
  released.
- Each app change ships with `ios_release` once it merges.

### 2026-11-15: Reviewed !563 against !582, the architecture's coverage: they agree

[`2026-11-15-runtime-architecture-coverage-agents-deploys.md`](2026-11-15-runtime-architecture-coverage-agents-deploys.md)
derives every route, binding and hostname of the platform's Workers from the
tree. !563 adds the app shape's router to the control Worker's code, and its
template beside the platform kind.

- **The app shape's routes are covered as they are.** Each is a
  `"/v0/app/…"` literal in `worker/standalone.rs`, and later in
  `worker/learner.rs`, `worker/members.rs` and `worker/instances.rs`. The
  coverage test reads those under the `app` client, as §1 says. The `SHAPE`
  check comes before every other path, and it adds no path of its own.
- **No personal stack enters the diagram.**
  - `catalog/pigeon/kinds/platform/app-worker.yaml` isn't under `infra/`, so
    it is not one of the platform's Worker specs. Its bindings (`LEARNER`,
    `STATE_KEY`, `HANDOVER`) describe no deployed part of the platform.
  - A standalone's account, `workers.dev` host and label are runtime data in
    `instances.private` alone. No repository file names them, so §1's
    derivation never meets them, and the privacy test has nothing to find.

Neither ADR changes.
