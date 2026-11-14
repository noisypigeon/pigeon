## Non-platform profiles: enrolled from the app, managed in another Cloudflare account or standalone on their own stack

* Date: 2026-11-13
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `catalog/pigeon/kinds/**`, `catalog/cloudflare/capabilities.yaml`, `src/pigeon-cli/crates/backplane/src/instance.rs`, `src/pigeon-cli/crates/backplane/src/worker/app.rs`, `src/pigeon-ios/Pigeon/**`, `src/pigeon-ios/PigeonCore/**`

## Context

Today the platform serves one instance, itself.

- **The Worker is single-instance.** `INSTANCE_NAME` is `pigeon`, and the `Instance` Durable Object, changes, events and approvals all belong to it.
- **The platform's account is the only one the platform can reach.**
  - [`2026-11-12-development-grants-applies-rescue-yield.md`](2026-11-12-development-grants-applies-rescue-yield.md) §1 designs one `granter` for the pigeon account. Its ledger, `.config/grants.yaml`, is being built in !349.
  - The noisypigeon account (`noisypigeon.com` and the owner's other zones) is still applied by hand, from pigeon with `--instance` and a pasted token, per [`2026-11-05-development-pigeon-its-own-instance.md`](2026-11-05-development-pigeon-its-own-instance.md).
- **The app shows Platform alone.** [`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md) §8 makes a profile an instance, and leaves other profiles to a later ADR. The app already has:
  - a device session, bound to a Secure Enclave key (§2);
  - the owner's passkey, with relying party `platform.pigeon.dev`, for every write (§3);
  - approval cards that redeem the existing grants and tiers (§4).
- **The pieces this design builds on are already decided:**
  - an instance is one `config.yaml` of kind units ([`2026-11-05-runtime-instance-config-kind-units.md`](2026-11-05-runtime-instance-config-kind-units.md));
  - grants, minting and R2 state per instance ([`2026-11-04-development-platform-instance-separation.md`](2026-11-04-development-platform-instance-separation.md) §4, the control-plane ADR §8, and the grants ADR §1 to §3);
  - each instance's Artifacts namespace lives in its own account ([`2026-11-05-development-git-hosting-cloudflare-artifacts.md`](2026-11-05-development-git-hosting-cloudflare-artifacts.md));
  - a `pigeon/platform` stack is a parameter, and `platform.standup` builds one from nothing ([`2026-11-08-runtime-blue-green-self-standup.md`](2026-11-08-runtime-blue-green-self-standup.md) §3 and §5);
  - the control plane already proposes changes of its own, such as `reconcile-<n>`.

The owner asked for **non-platform profiles**, in two shapes, **enrolled from the app**:

1. **Managed.** The owner authorizes the platform to manage another Cloudflare account and its zones. The platform builds features; the profile's account runs the jobs that use them.
   - Use cases: the noisypigeon blog and public websites; data operations reaching into other clouds.
2. **Standalone.** No Cloudflare zone. The profile would otherwise live in the pigeon account's instance, but it must have its own account and keep its own state there.
   - Use case: aggregating social media, personal metrics and the like.

This ADR designs the mechanism, as building blocks. **It builds no use case.** Each use case is a later ADR. The first owner step is to enroll the second Cloudflare account from the app (§10, step 6).

**Out of scope:**

- any feature (job kind) itself;
- moving noisypigeon's units onto the platform;
- moving a profile between modes;
- OAuth flows for third-party APIs;
- push notifications from a standalone stack.

## Implementation plan.

### 1. Terms and modes

- **Profile**: an instance other than pigeon. It has a name (a slug), a mode, and an accent for the app and approval pages.
- **Platform**: pigeon itself. It is never a profile entry and never changes mode.
- **Granter**: one standing Cloudflare account token in one account, holding *Account API Tokens Write*, from which tokens are minted per use (grants ADR §1).
- **Ceiling**: the most a profile's minted tokens may ever hold: capabilities, and the zones they may name.
- **Feature**: a job kind the platform defines and releases, which a profile turns on.
- **Runner**: the compute in a managed profile's account that runs its features.
- **Enrolment**: adding a profile, from its ledger entry to its first passing `profile.check`.

| | Managed | Standalone |
| --- | --- | --- |
| Control plane: changes, approvals, events, `Instance` | pigeon's stack, keyed by profile | its own `pigeon/platform` stack, in its account |
| OpenTofu state | the pigeon account, a bucket and key per profile | its own account, its own bucket and key |
| Feature jobs run in | its account, on its runner (§6) | its account, on its own stack's `Runner` |
| What pigeon holds for its account | a granter, bounded by a ceiling | nothing after stand-up; only public keys were handed over |
| Its repository | its Artifacts namespace, through pigeon's git front door | its Artifacts namespace, through its own front door |
| Zones | any it declares | none needed |
| Approved in the app with | the owner's passkey, on pigeon | the same passkey and device key, on its own stack (§8) |

### 2. The ledger: `.config/profiles.yaml`

A guarded file on `main`, in the shape of `.config/grants.yaml`. Each entry declares one profile by name. It holds no account ID, zone ID, bucket name or hostname. Enrolment writes it (§8); nobody needs to edit it by hand.

```yaml
# Instances other than pigeon; see .claude/decisions/2026-11-13-runtime-non-platform-profiles.md.
profiles:
  noisypigeon:
    mode: managed
    accent: blue
    accounts:
      cloudflare:
        zones: [noisypigeon.com]
        ceiling: [zone.read, dns.write, transform.write, redirects.write, r2.write, artifacts.write]
    runner: false
  example:                    # illustrative; a standalone takes no accounts
    mode: standalone
    accent: yellow
```

- **`mode`** is `managed` or `standalone`.
- **`accounts`** is only for `managed`. One entry per provider; Cloudflare first.
  - `zones` are names, which are public anyway.
  - `ceiling` names capabilities from `catalog/<provider>/capabilities.yaml`.
- **`runner: true`** asks for a runner (§6). A runner adds `workers.write` and `containers.write` to the ceiling's checks.
- **One check, in the functional core,** validates an entry. The app's form, the generated change, the MCP tool and a new sentinel policy, `.config/policies/profiles.yaml`, all call it:
  - names are slugs, and never `pigeon`;
  - modes and accents come from fixed lists;
  - every ceiling capability exists in its catalog;
  - a standalone has no `accounts`;
  - no value looks like an account or zone ID (32 lowercase hex characters).
- **Tier: `owner`.** Adding a profile, widening its ceiling, changing its zones or removing it is `owner` tier, every time, so no Auto window or mandate covers it. Narrowing a ceiling is `code`.
- **Unknown fields are refused,** as in !349's ledger, so a typo can't widen a profile.
- **D1 holds what the repository can't:** a migration adds `profiles` (name, mode, base URL, status, created) and `profile_accounts` (profile, provider, account ID, granter row, verified).
  - `status` is one of `proposed`, `awaiting-grant`, `awaiting-standup`, `standing-up`, `ready`, `failed` and `removed`.
  - An account ID is shown to the owner by its last four characters, and never in a response to an agent.

### 3. Granters for a managed profile

- **One granter per profile and provider.** For Cloudflare, it is an account-owned token in the profile's account, holding *Account API Tokens Write* plus the ceiling's permissions, limited to the ceiling's zones.
- **Entered once, with one passkey proof,** from the app (§8) or the page `/grants/profiles/<profile>/<provider>`. The value is pasted by the owner, never given to an agent. The Worker checks the token before storing it:
  - it is active;
  - it sees exactly one account, whose ID is recorded in `profile_accounts`;
  - every ceiling zone exists in that account.
- **Stored in D1 `grants`, encrypted under `GRANT_KEY`,** as the control-plane ADR §8 decides. A Worker binds Secrets Store secrets by name at deploy, so a profile's granter can't be one. pigeon's own `granter` stays in Secrets Store (grants ADR §1).
- **Minting chooses the granter from the run's profile,** never from the request. Before Cloudflare is called:
  - the requested capabilities must be within the profile's ceiling;
  - each zone scope must be among its zones;
  - otherwise the mint is refused with an event.

  Cloudflare is not relied on to bound a minted token by its granter; the ceiling is enforced here, and `profile.check` audits it.
- **Every minted token** is account-owned in the profile's account, scoped to the run's zones, and expires. Audit rows name the profile.
- **The grants ledger gains `profile:`** (default `pigeon`). A grant with `profile: noisypigeon` mints from that profile's granter, and may route only to that profile's jobs or runner. This lands as an amendment of the grants ADR, after !349.
- **Revoking a profile's granter** stops every run of that profile at once, and nothing else.

### 4. Held grants

Some credentials can't be minted: a social network's API key, or another cloud whose catalog has no minting module yet.

- **A ledger entry may `hold`** a value instead of minting, generating or deriving one:

  ```yaml
  grants:
    example-feed:
      profile: example
      hold: { provider: example-api, expires: 90d }
      route: { job: example.collect }
  ```

- **The owner pastes it once,** in the app or on its `/grants/<request>` page, with one passkey proof. It is stored encrypted under `GRANT_KEY`, by the stack that runs the profile, and routed only to `job`.
- **No agent reads it,** and there is no `grant.get`. `grant.list` shows its name, route and age.
- **The digest names a held grant 14 days before `expires`.** `grant.rotate` on one asks the owner to paste a new value, as an approval card in the app.

### 5. A managed profile in pigeon's control plane

Every piece that today assumes `pigeon` takes the profile instead.

- **`Instance`:** one Durable Object per profile, `id_from_name(<profile>)`, with its own run queue, unit locks and slot count.
- **Changes:** `change_propose` takes `profile` (default `pigeon`).
  - The change's repository is the profile's, in its own Artifacts namespace, reached as `/git/<profile>/<repo>.git` through pigeon's front door, with a token minted from the profile's granter (`artifacts.write`).
  - The merge queue is per repository, as it is today.
  - A profile's change is judged only by the profile's own `.config/policies/`, per the own-instance ADR's amendment §3. pigeon's rules never judge it.
- **State:** a bucket per profile in the pigeon account, found by prefix, as the grants ADR §2 already plans. Its key is derived from `STATE_KEY` with HKDF-SHA-256 over the profile's name, so a new profile needs no deploy.
- **Approvals, events, digest and push** carry the profile. Every card and page shows its name and accent first, so it is always clear which instance an approval is for.
- **Mandates and Auto windows name one profile.** A window started on pigeon never covers a profile's change, and the reverse.
- **Kinds:** a managed profile may not declare a `pigeon/platform` unit.

### 6. Features and the runner

The platform builds features; a managed profile's account runs them.

- **A feature is a job kind** that the platform defines and releases with backplane. It declares:
  - its settings' schema;
  - the capabilities it needs;
  - any held grants it needs.

  A profile turns one on with a unit in its `config.yaml`. This ADR defines no feature; the first is a use case's ADR.
- **A runner is a new kind, `pigeon/runner`,** under `catalog/pigeon/kinds/`. It is one unit in the profile's `config.yaml`, for a profile whose ledger entry has `runner: true`. It holds, in the profile's account:
  - a Worker with one Container class running the released `pigeon-backplane` image, as `pigeon backplane jobs`;
  - a Durable Object that holds each job's pending I/O, as the platform's `Runner` does;
  - an R2 bucket for the jobs' outputs and logs;
  - an Access application on its `workers.dev` hostname, with one service token for pigeon's dispatch.
- **Deployed by pigeon's deploy job,** with tokens minted from the profile's granter (`workers.write`, `containers.write`, `access.write`). The release image is pushed into the profile account's registry by digest.
- **Dispatch:** pigeon's control plane calls the runner over HTTPS, with:
  - the Access service token;
  - a request signed under the profile's `RUNNER_TOKEN`, with a timestamp and a nonce, and a signed response, as the Container link is (separation ADR, spike report §2).
  - `RUNNER_TOKEN` is a `generate` grant, routed to the runner's Worker secret, with pigeon's copy encrypted under `GRANT_KEY`.
- **A job's credentials** are minted from the profile's grants only, sealed to the job, and revoked at its end. A runner never holds a pigeon-account token.
- **Results:** a summary and an event come back to pigeon. Logs stay in the profile's bucket.
- **Plans and applies of the profile's units** stay on pigeon's `Runner`, as the grants ADR §3 decides. Only features run on the runner.
- **A runner needs Workers Paid** on the profile's account, because it runs a Container.

### 7. Standalone profiles

A standalone profile is a second copy of the platform's stack, in its own account, running the platform's released code.

- **Its own repository and `config.yaml`,** in its own Artifacts namespace, declare:
  - one `pigeon/platform` unit, `platform/main`, with `instance` set to its name;
  - its own units, content and policies.

  It never forks backplane. Its code is the platform's release.
- **No zone.** The `pigeon/platform` kind lets `domains` be empty. The stack serves on its `workers.dev` hostname, behind Access for `workers.dev`, with `/v0/app/*` in its `public_paths` as pigeon's are.
- **Its own everything:** Worker, Durable Objects, `Runner` and `Checker`, D1, state bucket, queue, Access application and service tokens, Secrets Store `granter` and `STATE_KEY`.
- **Stood up once by pigeon, with `profile.standup <name>`.** This is `platform.standup` (blue-green §5) aimed at another account:
  - It takes a stand-up credential for that account, entered as §8 describes. It is sealed to the job and never stored.
  - The job creates the state bucket in the profile's account, and generates `STATE_KEY` and `SESSION_KEY` straight into the new stack's secrets. pigeon never sees or keeps them.
  - It deploys the pinned release's bundle, and copies the release image into the account's registry by digest.
  - **It hands over public keys only:** the owner's passkey credentials (ID and public key) and the enrolling device's public key. So the app is linked to the new stack, and the same passkey approves there, from the first minute.
  - It reports the base URL into D1 `profiles`.
  - `pigeon backplane standup --local --instance DIR` stays the break-glass path.
- **Afterwards, pigeon holds no credential for it, and it holds none for pigeon.** pigeon keeps only its name, mode, base URL and accent, so the app can list it.
- **Releases:** its `config.yaml` pins `release: backplane/vX.Y.Z`. Advancing is an ordinary change in its own repository, and its own `MainRun` deploys it. Its own home screen says when it is behind the platform's latest release.
- **`profile.teardown <name>`** takes a stand-up credential again, at `owner` tier. It refuses while the stack's state bucket holds state, unless the owner confirms its deletion.

### 8. Enrolling a profile from the app

**Profiles** in the app gains **Add profile**. Every step below is the app calling a route under `/v0/app/`, signed with the device key; each route is an `Instance` operation in the functional core, so the page, the MCP tool `profile_enroll` and the app share one implementation. An agent may propose an enrolment; only the owner's passkey completes one.

**Step 1: describe it.** A form, checked as it is typed by `PigeonCore` with the same rules as §2:

- name, accent and mode;
- for managed: the zones, and a ceiling chosen from presets the platform serves (`sites`: zone, DNS, rules, R2 and Artifacts; `sites + runner`), each expandable to its capabilities;
- for standalone: nothing more.

**Step 2: the ledger change.** `POST /v0/app/profiles` proposes a generated change, `profile-<name>`, whose only file is `.config/profiles.yaml`.

- A `Checker` job writes the commit, as reconciliations are written. The bound job and sentinel judge it as any change.
- It is `owner` tier, so the app shows it at once as an approval card with the entry's diff. Face ID approves and merges it (approval queues the merge).
- On merge the profile is `awaiting-grant` (managed) or `awaiting-standup` (standalone).

**Step 3: the credential.** The app shows a recipe, served by the platform from the entry, never typed by an agent:

- for managed, the granter: *Account API Tokens Write* plus the ceiling's permissions, and the zones to limit it to;
- for standalone, the stand-up credential's permissions (blue-green §5).

Each permission has a copy button, and **Open Cloudflare** opens the dashboard's API Tokens page in Safari. The owner creates the token there and copies it.

**Step 4: hand it over.** Back in the app, **Paste token** reads the clipboard once, on that tap.

- `POST /v0/app/challenge` binds the passkey challenge to the operation (`profile.grant` or `profile.standup`), the profile, and the token's SHA-256. The token travels only in the body of the call that redeems that assertion.
- The app never stores the value. It overwrites the clipboard once the call answers.
- Managed: `PUT /v0/app/profiles/<p>/grants/cloudflare` checks and stores the granter (§3), then runs `profile.check`. The app shows the zones found against the ceiling.
- Standalone: `POST /v0/app/profiles/<p>/standup` starts `profile.standup`. The app follows its phases from the event stream, and reminds the owner to revoke the stand-up token at the end.

**Step 5: ready.** The profile appears in the switcher, with its accent.

- **Managed:** the app talks to pigeon, naming the profile on each call.
- **Standalone:** the app talks to the stack's base URL with the same device key, which the stand-up handed over. The stack accepts assertions for relying party `platform.pigeon.dev` with its own challenges and the passkey public keys it was given. No app release is needed per profile, and the stack never calls pigeon.
- The standalone's own granter, if it needs one, is entered the same way, on the standalone, from its profile in the app.

**Removing a profile** is the same shape in reverse: a generated `owner`-tier change removing the entry; then, with a passkey, the granter is revoked (managed), or a stand-up token is pasted for `profile.teardown` (standalone).

**A failure leaves a profile where it was.** A refused token stores nothing, and the profile stays `awaiting-grant`. A failed stand-up resumes from its failed phase (blue-green §5). The app shows the profile as `failed` with why, and offers **Retry**.

**Routes added to the iOS ADR's §4 table:**

| Route | What it does |
| --- | --- |
| `GET /v0/app/profiles` | the platform and every profile: name, mode, status, base URL, accent |
| `GET /v0/app/profiles/presets` | the ceiling presets, and each credential recipe |
| `POST /v0/app/profiles` | proposes the enrolment change |
| `PUT /v0/app/profiles/<p>/grants/<provider>` | enters a granter, with an assertion |
| `POST /v0/app/profiles/<p>/standup` | starts `profile.standup`, with an assertion |
| `DELETE /v0/app/profiles/<p>` | proposes the removal change |

### 9. Operations, and what a profile never gets

| Operation | Does | Tier |
| --- | --- | --- |
| `profile.list`, `profile.show` | the ledger, joined with D1 status; never an account ID | auto |
| `profile.enroll`, `profile.remove` | propose the ledger change (§8, step 2) | auto to propose; the change is `owner` |
| `profile.check <p>` | for managed: mints a read-only token, lists the zones it sees against the ceiling, and checks the granter's expiry; for standalone: the stack's `/healthz` and release | auto |
| `profile.grant <p> <provider>` | enters a granter | `owner` |
| `profile.standup <p>`, `profile.teardown <p>` | §7 | `owner` |
| `runner.ping <p>` | a no-op job on the profile's runner, proving dispatch, signing and minting | auto |

Events: `profile.proposed`, `profile.added`, `profile.granted`, `profile.checked`, `profile.stood_up`, `profile.failed`, `profile.removed` and `runner.dispatched`.

A profile never gets:

- a pigeon-account token, or any other profile's;
- a capability or zone beyond its ceiling;
- pigeon's policies, mandates, Auto windows or slots;
- for a standalone, any credential held by pigeon after its stand-up.

### 10. Execution

Each step is one change, behind the one before. Steps 3 onwards need the grants ADR's step 1 (!349 and what follows it). Step 7 also needs that ADR's steps 2 and 4.

1. **This ADR.**
2. **The grants ADR's steps 1, 2 and 4,** as already planned.
3. **`feat(backplane)`: profiles.** `.config/profiles.yaml` with no profiles, the core check and its policy, the D1 migration, `profile.list`, `profile.show`, and `GET /v0/app/profiles`.
4. **`feat(backplane)`: enrolment.** `profile.enroll` and `profile.remove` as generated changes, managed granters (§3), `profile.grant`, `profile.check`, the presets, and §8's routes for managed profiles. The grants ledger's `profile:` lands as an amendment of the grants ADR.
5. **`feat(ios)`: Add profile,** for managed profiles: the form, the approval card for the ledger change, the recipe, the paste and the check.
6. **The owner enrolls the second Cloudflare account, from the app:**
   1. Add profile: `noisypigeon`, managed, its zones, the `sites` preset; approve the change with Face ID;
   2. follow the recipe in Cloudflare's dashboard, copy the token, paste it, and prove with Face ID;
   3. the check shows each zone, within the ceiling.

   Nothing moves into the account yet. The token is made at this step, not earlier, so it never sits unused.
7. **`feat(backplane)`: the control plane keyed by profile (§5).**
8. **`feat(catalog)` and `feat(backplane)`: `pigeon/runner`,** its deploy and dispatch, and `runner.ping`. The owner turns on Workers Paid in the profile's account first.
9. **`feat(backplane)`: held grants (§4),** with their paste in the app.
10. **`feat(catalog)`, `feat(backplane)` and `feat(ios)`: standalone (§7, §8).** `domains` may be empty, `profile.standup` and `profile.teardown`, the hand-over of public keys, release pins, and the app talking to a standalone's base URL. A rehearsal enrolls, stands up and tears down a scratch profile in a scratch account, from the app.

Then each use case is its own ADR: noisypigeon's sites onto its managed profile, data operations as features, and the aggregator as a standalone profile.

### 11. Verification

- **Ledger:** the core check refuses an ID-shaped value, an unknown capability, `accounts` on a standalone, a profile named `pigeon`, and an unknown field. The app's form, the generated change and the policy agree on every case, from one set of test vectors.
- **Enrolment:** an enrolment change can't merge under an Auto window or a mandate. An assertion bound to one token's digest doesn't redeem with another token, or for another profile.
- **Granters:** a token that sees two accounts, or is missing a ceiling zone, is refused, and nothing is stored.
- **Ceilings:** a mint asking beyond the ceiling, or for a zone outside it, is refused before Cloudflare is called.
- **Isolation:** a run of profile A never reads B's granter, state key or bucket. A test runs two profiles with fake granters and checks every mint's account.
- **Policies:** a profile's change is judged by its own policies and none of pigeon's.
- **Runner:** an unsigned, stale or replayed dispatch is refused. `runner.ping` passes, and its minted token is revoked afterwards.
- **Standalone:** after the rehearsal's stand-up:
  - D1 holds no credential or state key for the scratch profile;
  - the stack serves on `workers.dev` with no zone;
  - the app approves on it with the same passkey and device, with no new app build;
  - teardown removes every resource it created.
- **The app:** `PigeonCore` tests the form's checks and each enrolment state's transitions; the clipboard is overwritten after a paste.
- `mise run ci` passes at every step.

## Amendments

### 2026-11-14: A profile is now an instance

[`2026-11-14-runtime-accounts-profiles-instances.md`](2026-11-14-runtime-accounts-profiles-instances.md)
frees "profile" for the signed-in person. From now on, every "profile" in
this ADR means an **instance**, and its names follow that ADR's §5 table:

- the ledger is `.config/instances.yaml`, keyed `instances:`, and its policy
  is `.config/policies/instances.yaml`;
- D1's tables are `instances` and `instance_accounts`;
- the operations are `instance.list`, `instance.enroll`, `instance.remove`,
  `instance.check`, `instance.grant`, `instance.standup` and
  `instance.teardown`, and the events are `instance.*`;
- `profile.show <p>` is `instance.show` with `name`;
- the routes are `/v0/app/instances/…` and `/grants/instances/<p>/<provider>`;
- the grants ledger's field and `change_propose`'s argument are `instance`;
- the app's **Add profile** is **Add instance**.

Step 3 builds under these names from the start, so nothing is renamed twice.
