## Accounts, profiles and instances: who signs in, who they are, and what they act on

* Date: 2026-11-14
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `src/pigeon-ios/Pigeon/HomeView.swift`, `src/pigeon-ios/Pigeon/Sheets.swift`, `src/pigeon-ios/PigeonCore/Models.swift`, `src/pigeon-cli/crates/backplane/src/core/app/devices.rs`, `src/pigeon-cli/crates/backplane/src/worker/devices.rs`

## Context

The app uses one word, **profile**, for something that is an instance.

- [`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md) §8 makes a profile an instance: a name, base URL, accent and icon. The app's `ProfilesSheet` lists Platform alone, beside Settings.
- [`2026-11-13-runtime-non-platform-profiles.md`](2026-11-13-runtime-non-platform-profiles.md) builds on that meaning: `.config/profiles.yaml`, D1 `profiles`, `profile.*` operations and `/v0/app/profiles`. Its step 3 is not on `main` yet. Today the word exists in code only in the app's Swift.
- **The person who signs in has no record of their own.** There is one owner, implied:
  - their passkeys are the `passkeys` keyring in the `Instance`'s storage;
  - their linked devices are its `devices` list (iOS ADR §2);
  - approvals run as `APPROVER`;
  - the app's appearance lives only on the phone, in `@AppStorage`.

The owner asked for two things:

1. **Rename what the app calls a profile to "instance",** everywhere it means one: the UI, the app API, operations, the ledger, D1 and the ADR vocabulary from now on.
2. **Free "profile" for the person:** their name, avatar, passkeys and devices, and preferences such as appearance. It is separate from the **account**, the identity that authenticates.

**Sessions this design meets:**

- The non-platform-profiles step-3 session builds the ledger, D1, `profile.list`/`show` and `/v0/app/profiles`. It builds them under the new names instead (§5), so the rename lands once.
- The "instance users and roles" session invites accounts to an instance as Administrator, User or Service Account. It builds on §1 and §2.
- The passkey-only session (!417) makes passkeys an account's only factor, with a keyring per account. §1 and §2 assume both.

**Out of scope:** inviting other people, roles and service-account credentials (the roles ADR); how a passkey is registered or recovered (the passkey-only ADR); and any instance other than Platform.

## Implementation plan.

### 1. Terms

| Term | What it is | Where it lives | Scope |
| --- | --- | --- | --- |
| **Instance** | one `config.yaml` of kind units, its repository and its control plane. Platform (`pigeon`) is one. | its repository; the ledger for the rest (§5) | itself |
| **Account** | the identity that authenticates: an ID, a kind, its passkeys and its linked devices | the platform's `Instance` storage | the whole platform |
| **Profile** | how a person account appears and what it prefers: name, avatar, appearance | beside its account | the whole platform |
| **Membership** | an account's role on one instance | the roles ADR | one instance |

- **An account is the only thing that signs in.** A person account proves itself with a passkey, for relying party `platform.pigeon.dev`, and with nothing else. There is no password, code or email factor.
  - Cloudflare Access's email is a network gate in front of pages. It is never an account's identity.
  - A linked device's signature says which account is calling. It authorizes no write on its own, as iOS ADR §3 decides.
- **An account has a kind:**
  - `person`: has passkeys, devices and exactly one profile;
  - `service`: has no passkey, device or profile. It authenticates with credentials the roles ADR defines.

  Only `person` accounts exist after this ADR. `service` is reserved for the roles ADR.
- **A profile belongs to one person account, and is never per instance.** The same name, avatar and appearance show whichever instance is selected.
- **An account acts on an instance only through a membership.** Until the roles ADR lands, the owner's account is Platform's only member, with the role the roles ADR calls Administrator. `APPROVER` stays its approvals' name until then.
- **The app shows** (§4):
  - the selected **instance** on Home's left;
  - the signed-in **profile**'s avatar on Home's right;
  - passkeys and devices under the profile, because they are the person's, though they belong to the account.

### 2. Records

In `core/account/` (pure, tested), stored in the platform's `Instance` under the key `accounts`:

```rust
pub struct Account { pub id: String, pub kind: Kind, pub created: u64 }
pub enum Kind { Person, Service }          // serde: "person", "service"
pub struct Profile {
    pub account: String,
    pub name: String,                      // 1 to 64 chars, cleaned as a device name is
    pub appearance: Appearance,            // "system" (default), "light", "dark"
    pub avatar: Option<String>,            // the stored JPEG's SHA-256, hex
    pub updated: u64,
}
pub struct Accounts { pub accounts: Vec<Account>, pub profiles: Vec<Profile> }
```

- **An account's ID** is `acct_` and 26 lowercase Crockford base32 characters, from 16 random bytes. It never looks like a Cloudflare ID.
- **The avatar's bytes** are stored under `avatar:<account>`:
  - JPEG only, checked by its leading `FF D8 FF`;
  - at most 64 KiB;
  - the app crops it to a square and scales it to 256 px before upload.
- **This ADR moves no passkey.** The `passkeys` key is the owner account's keyring until the passkey-only ADR (!417) gives each account its own. That keyring is keyed by this account's ID, which is also the WebAuthn user handle.
- **A `Device` gains `account`,** the ID of the account it is linked to.
- **The migration** is `Accounts::ensure_owner(self, random, now) -> (Self, Account)`. The `Instance` runs it on first use of any account route. It is idempotent:
  - with no person account, it creates one;
  - with none, it also creates its profile: name `Owner`, appearance `system`, no avatar;
  - it stamps each device whose `account` is empty with that account's ID.

  Nothing else moves. A rollback leaves `accounts` and `avatar:*` unread, and the `account` field ignored.

### 3. The app's API

Every route is behind the device signature, as the iOS ADR's §4 routes are. The account is the signing device's `account`.

| Route | What it does | Proof |
| --- | --- | --- |
| `GET /v0/app/profile` | `{account: {id, kind, created}, profile: {name, appearance, avatar}, passkeys: [{id, name, created, used}], devices: [{id, name, linked, seen, current}]}` | device |
| `PATCH /v0/app/profile` | sets `name` and/or `appearance`; answers the profile | device |
| `GET /v0/app/profile/avatar` | the JPEG, or 404 | device |
| `PUT /v0/app/profile/avatar` | `{jpeg: <base64>}`; answers the profile | device |
| `DELETE /v0/app/profile/avatar` | removes it; answers the profile | device |
| `DELETE /v0/app/devices/<id>` | unlinks one of the account's devices | passkey, purpose `unlink:<id>` |
| `DELETE /v0/app/passkeys/<id>` | removes one of the account's passkeys, never below the minimum | passkey, purpose `passkey:remove:<id>`, as !417 names it |

- **`POST /v0/app/challenge` gains two `op`s:** `unlink` and `passkey:remove`, each with `id`. They mint for the purposes above through the `Instance`'s `/passkeys/challenge`, as `auto` does. No guarded file changes.
- **A profile edit takes only the device signature.** Name, avatar and appearance change how the person appears, not what anyone may do. This amends iOS ADR §3's "every write asks for one" for these three routes only.
- **Removing a passkey or unlinking a device** changes how the account signs in, so it takes a passkey.
  - Going below the minimum is refused with `409`, so the account can't lock itself out. The minimum is one, or the passkey-only ADR's two after its cutover.
  - Unlinking the calling device signs the app out.
- **Adding a passkey** stays on `/passkeys`, which the passkey-only ADR owns. A passkey row carries `synced` once that ADR records it.
- **No MCP operation reads a profile.** An agent has no use for a person's avatar. Agents see accounts only through the roles ADR's memberships.

### 4. The app

- **Home's header:**
  - left: an **instance chip**, with the selected instance's accent dot, its name (`Platform`) and a chevron. It opens **Instances**.
  - right: the profile's **avatar**, or the app icon when there is none. It opens **Profile**.
- **Instances** (`InstancesSheet`, today's `ProfilesSheet` renamed) lists the instances with a checkmark on the selected one. Today that is Platform alone.
  - Its Settings row goes. Appearance was its only content, and it moves to Profile.
  - Rows come from `/v0/app/instances` once that exists (§5). **Add instance** is the non-platform ADR's step 5.
- **Profile** (`ProfileSheet`) has four sections:
  - **Avatar and name:** tap the avatar to pick a photo (`PhotosPicker`), or remove it; tap the name to edit it.
  - **Appearance:** System, Light or Dark, saved to the profile.
  - **Passkeys:** each one's name, when it was created and when it was last used. Remove asks for Face ID, and is disabled at the minimum.
  - **Devices:** each one's name and when it was last seen, with this phone marked **This iPhone**. Unlink asks for Face ID; unlinking this phone signs out.
- **`PigeonCore` gains:**
  - `Account`, `Profile`, `PasskeyRow`, `DeviceRow` and `Appearance`;
  - `Profile.validName(_:)`, mirroring the backplane's cleaning;
  - `Appearance.resolve(local:remote:)`, the app-side migration. The profile's appearance wins once it differs from `system`. Otherwise the phone's stored choice is uploaded once and then cleared.

### 5. The rename: "profile" that means an instance becomes "instance"

From this ADR on, every name below uses the right-hand side. Merged ADRs keep their text and gain an amendment saying so (step 1).

| Was | Is |
| --- | --- |
| profile (meaning an instance), "non-platform profile" | instance, "non-platform instance" |
| `.config/profiles.yaml`, key `profiles:` | `.config/instances.yaml`, key `instances:` |
| `.config/policies/profiles.yaml` | `.config/policies/instances.yaml` |
| D1 `profiles`, `profile_accounts` | D1 `instances`, `instance_accounts` |
| `profile.list` | `instance.list` |
| `profile.show <p>` | `instance.show` with `name`; without one, it answers the serving instance as today |
| `profile.enroll`, `remove`, `check`, `grant`, `standup`, `teardown` | `instance.enroll`, `remove`, `check`, `grant`, `standup`, `teardown` |
| events `profile.*` | events `instance.*` |
| MCP tool `profile_enroll` | `instance_enroll` |
| `/v0/app/profiles`, `/presets`, `/<p>/grants/<provider>`, `/<p>/standup` | `/v0/app/instances`, and the same sub-paths |
| page `/grants/profiles/<p>/<provider>` | `/grants/instances/<p>/<provider>` |
| grants ledger field `profile:` | `instance:` |
| `change_propose`'s `profile` | `instance` |
| app: `ProfilesSheet`, "Profiles", "Switch profile", `Sheet.profiles` | `InstancesSheet`, "Instances", "Switch instance", `Sheet.instances` |

- **Not renamed:** Apple's provisioning profiles (`signing.rs`, `apple.rs`, the release ADR) and Cargo's `[profile.*]`. They are a different thing.
- **Who renames what:**
  - The step-3 session builds the ledger, D1, `instance.list`, `instance.show`'s `name` and `/v0/app/instances` under the new names, so they never exist under the old ones.
  - This ADR's step 2 renames what `main` already holds: the Swift.
  - If step 3 merges under the old names first, step 2 gains a second haiku brief to rename its paths.

### 6. Execution

Each step is one change. Steps 2 and 3 run in parallel; step 4 waits for both.

1. **This ADR,** with an amendment to each of the non-platform-profiles ADR and the iOS ADR recording §5's vocabulary. Planner.
2. **`refactor(ios)`: Profiles becomes Instances.** A `haiku` brief: `Sheets.swift` and `HomeView.swift`, literal renames per §5's last row. No behaviour changes.
3. **`feat(backplane)`: accounts and profiles.** A `sonnet` brief:
   - `core/account/` with §2's records, `ensure_owner` and the name, avatar and last-passkey checks, with their tests;
   - `Device.account` in `core/app/devices.rs`;
   - the `Instance`'s `/accounts/*` handlers;
   - `worker/account.rs` with §3's routes, wired from `worker/devices.rs`'s `signed`;
   - the two challenge `op`s in `worker/app.rs`.
4. **`feat(ios)`: the profile.** A `sonnet` brief: §4's `PigeonCore` types and tests, `ProfileSheet`, the header's chip and avatar, and the appearance migration.
5. **Amendments:** this ADR gains each landed step's paths in `Governs`, and the **Model variance** amendment. Planner.

### 7. Verification

- **Migration:** `ensure_owner` on an empty store makes one person account and its profile, and stamps devices without an account. Run twice, it changes nothing.
- **Profile:**
  - a name that is empty, or only control characters, is refused;
  - an avatar that isn't JPEG, or is over 64 KiB, is refused;
  - an unknown appearance is refused.
- **Credentials:**
  - removing a passkey below the minimum answers 409;
  - removing or unlinking without a passkey, or with one for another purpose or ID, answers 403;
  - unlinking another account's device answers 404.
- **Rename:** `grep -rn 'Profile' src/pigeon-ios/Pigeon/HomeView.swift src/pigeon-ios/Pigeon/Sheets.swift` finds only the person's profile.
- **The app:** `PigeonCore` tests `validName` and `Appearance.resolve`. A TestFlight build shows the instance chip and the avatar, edits the name, and removes a second passkey with Face ID.
- `mise run ci` passes at every step.
