## Capabilities: one manifest and one trait, defined once and enabled per instance

* Date: 2026-11-19
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `src/pigeon-core/src/wire.rs`, `src/pigeon-core/src/member/mod.rs`, `src/**/worker/learner.rs`

## Context

[`2026-11-18-security-users-levels-capability-grants.md`](2026-11-18-security-users-levels-capability-grants.md)
splits the platform into base features, which a level decides, and
capabilities, which the Administrator grants per user. Its owner's answers
name six capabilities: `subdomain`, `domain`, `blog`, `icbc`, `sites` and
`instances`. Only `icbc` is grantable.

The owner asked for every capability to have one standard interface, so that
a new capability is declared, checked and wired the same way, and custom
behaviour has a fixed place to go.

### What exists, read from `main` (73090e3)

- **Names:** `core/member::CAPABILITIES` (six) and `GRANTABLE` (`icbc`) are
  constants. `Members::grant` refuses a scope on `icbc` by name.
- **Requirements:** `wire::Operation::requires()` infers each operation's
  requirement from side lists. Its one capability case is
  `instances.standup`, matched by name.
- **ICBC** is not a catalog operation. It is HTTP routes on the standalone's
  `/v0/app/learner/` door (`worker/learner.rs`). They check
  `members.holds(account, "icbc", None)` in code, and are switched on by the
  `FEATURES` var.
- **Validators:** `wire/registry_tests.rs` checks that each `Capability`
  requirement names a known capability and one of the operation's arguments.
  `architecture.describe` covers routes and bindings, but has no notion of a
  capability.

So a capability today is a name in a constant plus code scattered across the
worker. Nothing says which routes, storage, secrets or config belong to it.

## Implementation plan.

### 1. The manifest

Each capability is a directory, `catalog/capabilities/<name>/`, holding
`capability.yaml`, checked against `catalog/capabilities/schema.json`:

```yaml
name: blog                      # [a-z][a-z0-9-]*, unique
version: 1.0.0                  # semver; a breaking change bumps major
summary: A blog site from the blog template, served under a subdomain or domain.
built_on: null                  # another capability whose engine this one configures
scope:
  kind: site                    # none | dns_label | domain | site | instance
  arg: blog                     # the argument operations name their scope with
  needs: [subdomain|domain]     # grants the holder must also hold to use it (alternatives with |)
grantable: true                 # false until its operations mean something
operations: [blog.create, blog.post.put, blog.post.delete, blog.publish, blog.show]
routes:                         # what it serves; every path is under its door
  - { door: app, prefix: /v0/app/blog/ }
storage:
  object_keys: [blog.index]     # Instance Durable Object keys
  r2_prefixes: [blog/]          # in the instance's bucket
  d1_tables: []                 # platform shape only
migrations: migrations/         # numbered SQL or object migrations, may be empty
secrets: []                     # Secrets Store names only; never a value
config:                         # per-instance config, JSON Schema
  type: object
  properties: {}
hooks: [enable, disable, upgrade, on_grant, on_revoke]
members:                        # the Members screen's grant entry
  label: Blog
  scope_label: Blog site
  help: Write and publish posts on one blog.
tests: [src/pigeon-core/src/capability/blog/tests.rs]
```

- **Names, not values.** A secret is a Secrets Store name the instance binds.
  A domain, zone or label is never in a manifest. Private values live in the
  instance's `Instance` object, per
  [`2026-11-15-runtime-zoned-standalone-account-imports.md`](2026-11-15-runtime-zoned-standalone-account-imports.md) §1.
- **`built_on`** says that a capability configures another's engine with its
  own config, such as `icbc` on `learning` (the learning ADR). The engine
  supplies the operations, routes and hooks. The variant supplies name,
  grant, config and tests. A variant's grant is its own: holding `icbc` is
  not holding `learning`.
- **`operations`** are catalog operations. A capability reached only by app
  routes still lists its routes, so the door and the check are known.

### 2. The trait

`pigeon_core::capability`, in the functional core:

```rust
pub trait Capability: Sync {
    /// The parsed manifest, embedded at build time.
    fn manifest(&self) -> &'static Manifest;
    /// Whether `scope` is a well-formed scope for a grant of it.
    fn check_scope(&self, scope: Option<&str>) -> Result<(), Refusal>;
    /// Whether a grant scoped `granted` covers the argument `asked`.
    fn covers(&self, granted: Option<&str>, asked: Option<&str>) -> bool;
    /// The instance's config for it, checked against the manifest's schema.
    fn config(&self, raw: &serde_json::Value) -> Result<Config, Refusal>;
    /// What enabling, disabling or upgrading it on an instance does, as a plan the shell applies.
    fn lifecycle(&self, hook: Lifecycle, ctx: &InstanceCtx) -> Result<Plan, Refusal>;
    /// Custom behaviour around a grant, a revocation or one of its operations; the default does nothing.
    fn behaviour(&self, hook: Behaviour, ctx: &CallCtx) -> Result<Outcome, Refusal> { Ok(Outcome::Pass) }
}
```

- `Lifecycle` is `Enable`, `Disable` and `Upgrade { from }`. `Behaviour` is
  `OnGrant`, `OnRevoke`, `Before(op)` and `After(op)`.
- **The shell applies, the core decides.** A hook returns a `Plan`: storage
  to create or drop, objects to migrate, a job to dispatch. The Worker
  applies it through the `Instance` object, so each step is journaled and
  fenced by the lease, as blue-green's §A.3 requires.
- **Disable keeps data.** Disabling removes routes and refuses operations.
  Dropping storage is its own owner-tier act.
- **`REGISTRY: &[&dyn Capability]`** lists every capability. `CAPABILITIES`
  and `GRANTABLE` become functions over it, so a name is declared once.

### 3. One requirement, read from the registry

- `Operation::requires()` asks the registry first. An operation one
  capability lists requires `Capability { name, scope_arg }`, taken from
  its manifest. The `instances.standup` special case moves into the
  `instances` manifest.
- An operation listed by two capabilities is refused by the validator.
  A `built_on` variant names its engine's operations through `variant`: the
  engine's operations take an argument naming the variant (`pack`, in the
  learning ADR), and the check asks for the variant's grant.
- `core/member::check` is unchanged. `Members::grant` asks the capability's
  `check_scope` in place of its hard-coded `icbc` rule.
- **App routes** call `member::check` with the route's capability, so
  `worker/learner.rs`'s hand-written `holds` goes away.

### 4. Enabled per instance

- **Defined once:** the manifest and the trait ship in the build every
  instance runs.
- **Enabled per instance:** an instance's `capabilities` map, `{name:
  {version, config}}`:
  - the platform: `.config/instance.yaml`;
  - a standalone: its `Instance` object's `capabilities` key, set by the
    owner with `capability.enable` and `capability.disable` (owner tier,
    passkey), and seeded at stand-up.
- The `FEATURES` var is read as `capabilities` for one release, then removed:
  `learner` maps to `icbc`.
- **A disabled capability** answers 404 on its routes and `not enabled` on its
  operations, before any grant is checked. The Administrator holding every
  capability doesn't enable one.
- **Upgrades:** an instance records the version it runs. A deploy whose
  manifest version is newer runs `Upgrade { from }` once per instance, in
  the `Instance` object, after replay.

### 5. The validator

`pigeon dev capabilities` (Rust, in `pigeon dev ci`), and
`.config/policies/capabilities.yaml` for `policy.check`. It fails when:

- a manifest doesn't match the schema, or its directory name isn't its
  `name`;
- `REGISTRY` and `catalog/capabilities/` differ;
- an operation it lists isn't in `CATALOG`, or another capability lists it,
  or its scope `arg` isn't one of the operation's arguments;
- a route's prefix isn't under its door, or overlaps another capability's;
- storage keys, prefixes or tables overlap another capability's or a base
  feature's;
- a secret isn't a valid Secrets Store name, or a value sits beside it;
- a migration is out of order;
- a test path doesn't exist;
- `grantable: true` while `operations` and `routes` are both empty;
- a `built_on` names a missing capability, or forms a cycle;
- `members.label` is missing.

`capability_tests.rs` checks the trait against each manifest: `check_scope`
refuses a malformed scope, `covers` is false for a foreign scope, and
`lifecycle(Disable)` drops nothing.

### 6. The architecture

- `architecture.describe` gains a part per capability enabled on the
  instance, linked to its routes, storage and secrets, with the manifest as
  evidence.
- `tests/suite/instance/architecture.rs` fails on a capability route,
  storage prefix or secret that the description lacks, as it already does for
  routes and bindings.

### 7. The Members screen

`members.list` returns, for each grantable capability, its `members`
entry and scope kind. The iOS Members screen and the web Members root (B2)
draw their grant form from that, never from a hard-coded list.

### 8. Steps

| Step | What | Who |
| --- | --- | --- |
| 1 | This ADR | planner |
| 2 | `catalog/capabilities/schema.json`, the six manifests as they are today (`icbc` grantable, the others not), `Manifest`, the trait, `REGISTRY`, and `CAPABILITIES`/`GRANTABLE` from it | brief |
| 3 | `requires()` from the registry; `Members::grant` through `check_scope` (`core/member`, a gate path) | planner |
| 4 | `pigeon dev capabilities`, the policy, `capability_tests.rs` | brief |
| 5 | Per-instance `capabilities`: instance schema, the `Instance` key, `capability.enable`/`disable`, `FEATURES` read as `icbc` | brief, its owner-tier operations reviewed by the planner |
| 6 | `worker/learner.rs` through `member::check`; the architecture part and test | brief |
| 7 | `members.list`'s grant entries | brief |

Each capability's own ADR adds its operations and turns `grantable` on.

### 9. Rollback

Each step is its own change. Before step 3 nothing reads the registry, and
step 3 reverts to the inferred `requires()` without touching stored grants,
whose shape doesn't change.

Each step's change adds the paths it creates to what this ADR governs, by an
amendment in the same change: `catalog/capabilities/**`, `src/**/capability/**`,
`.config/policies/capabilities.yaml` and `src/**/wire/capability_tests.rs`.

## Amendments

### 2026-11-19: Steps 2 and 3 land; the constants stay as a pinned mirror

- **Step 2 (!1350)** added the six manifests under `catalog/capabilities/`
  and `pigeon_core::capability`, holding `Manifest`, the trait, `Declared`
  and `REGISTRY`. This amendment adds `catalog/capabilities/**` and
  `src/**/capability/**` to what the ADR governs.
- **Step 3** reads the registry where it decides:
  - `Operation::requires()` asks `capability::owner_of`. The hard-coded
    `instances.standup` case goes, because the `instances` manifest lists
    that operation.
  - `Members::grant` refuses a scope through the capability's
    `check_scope`, in place of the rule written out for `icbc`.
- **The constants stay, as a mirror.** `member::CAPABILITIES` and
  `GRANTABLE` remain the compiled lists the grant offer and the roles
  checks read. A test pins them to the registry, so the two can't drift.
  They become functions over the registry when a capability first turns
  grantable (the Subdomain, Domain and Blog ADR's step 7), in that
  owner-tier change.

### 2026-11-19: The capability paths are gate paths

Found in review of !1352. Once `requires()` reads the registry, a manifest's
`operations` decide what a Standard member holding that capability may call.

- **The risk.** A code-tier change under Auto could list an approval, owner,
  credential or platform operation in a manifest, and so widen Standard
  members' reach without the owner's passkey.
- **The fix.** `catalog/capabilities/**` and `src/pigeon-core/src/capability/**`
  join both `guarded` and `gate` in `.config/approvals.yaml`. Every change to
  them takes the owner's passkey, Auto or not.
- **Two tests follow, apart:**
  - no manifest lists an operation that is Administrator-only, asks for a
    grant or a page, applies outside the control plane, or belongs to the
    `change`, `mandate`, `lane`, `grant`, `member`, `service`, `secrets`,
    `platform`, `owner`, `approval`, `deploy` or `infra` areas;
  - no operation is owned by more than one capability.
