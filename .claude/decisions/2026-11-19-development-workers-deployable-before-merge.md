## Every Worker spec is proven deployable before merge, on the build that will deploy it and on its own

* Date: 2026-11-19
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development
* Governs: `src/pigeon-agent/src/workers/mod.rs`

## Context

The owner, 2026-11-19: "How can we make these tests and iterations go
faster?" Approved for this ADR: catch undeployable changes before merge.

### What failed on 19 November

Four merges passed every check and failed only at deploy. Each one cost a
revert, a restore and extra owner approvals:

- **!1317:** the binder.
- **!1325 and !1337:** pigeon-green's queue binding. `$deployment.queue_name`
  reached wrangler raw.
- **!1353:** a new Route form, `{ host, zone }`, was used in
  `infra/pigeon/workers/control/worker.yaml` in the same change that added
  its parser. The deploy rendered the spec with `main`'s previous stored
  build, which rejected it with ``unknown field `host` ``.

### Why the checks miss these

- **The deploy runs on the build before its own.** The deploy job ships
  Workers with `main`'s `latest_build` and stores its own commit's build
  only after its Workers step (`src/pigeon-agent/src/ship.rs`). The
  blue-green ADR's 2026-11-18 amendment, "A deploy ships Workers on the
  build before its own", records this and the two-step rule that follows
  from it, but nothing enforces the rule.
  - Every spec struct is `deny_unknown_fields`
    (`src/pigeon-agent/src/workers/mod.rs`, `Spec`, `Route`, `Bindings`, …).
    So a field the stored build doesn't know fails the deploy's parse.
- **No check validates a Worker spec on a spec-only change.** A change to
  `infra/**/workers/**` selects `infra.validate`, the infra suite and
  `policy.check`.
  - `infra.validate` (`catalog.rs` → `infra::validate_in`) never calls
    `workers::validate`.
  - `this_repositorys_specs_validate` lives in the `workers::` suite
    module, which `infra-tests` doesn't select.
- **Rendering can leave a binding unresolved, and nothing looks.**
  - `bind` rewrites an unknown `$deployment.<key>` to `$output.<unit>.<key>`
    whenever a layer has no declared outputs. So a typo binds silently.
  - `render_with` leaves an unresolved var `$output.` literal by design, and
    never resolves `$output.` in `name`, `custom_domain`, `forwards` or a
    route's `zone`.
  - So a literal `$deployment.` or `$output.` can reach wrangler.
- **No check runs wrangler.** `wrangler deploy --dry-run` is used only by
  `standup_job.rs`, for size. GitHub's `ci.yml` has no node, wrangler or
  worker-build. A Checker Container has all three.

### What a Checker already holds

- **Main's build.** Every job Container starts through `pigeon-start` on
  `latest_build`: it fetches `builds/<sha>/pigeon`, checks it against
  `PIGEON_BUILD_SHA256`, and runs it from `$TMPDIR/pigeon-build`. The bound
  job's log says `pigeon: /tmp/pigeon-build`. The binary that will deploy a
  change is already on the Checker, verified, with no fetch and no
  credential.
- **The head's build.** A check whose argv is a pigeon operation runs
  `cargo run -q --locked -p pigeon-harness -- run <op>`
  (`deploy::scope::Layout::operation`), so it compiles the head.
- **Wrangler and worker-build**, from the Dockerfile's `ci` stage.

## Implementation plan.

### 1. The rule

**A spec field never ships in the same change as its parser.**

- **Change 1 adds the field.**
  - The parser, renderer and validator learn the new field.
  - Tests use fixtures, not this repository's specs.
  - It merges and deploys, and that deploy stores a build that knows the
    field.
- **Change 2 uses the field** in an `infra/**/workers/**` spec, once
  `deploy.status`'s `latest_build` is at or after change 1's merge.
- **The same holds for anything the deploy's build must understand:** a new
  spec struct, Route form, binding kind, `$deployment` binder behaviour, or
  Worker. The ADR that adds one plans it as two changes.
- **The check in §2 enforces it.** The main-build pass fails change 2 if it
  is proposed too early. The owner never has to remember the rule.

### 2. The check: `pigeon workers deployable`

A `pigeon workers` subcommand, beside `validate`. It is in every build that
has `--features agent`, the stored build included.

**One run, two passes, both over every Worker spec in the tree.** There are
few specs, and rendering is cheap. Checking every spec means a parser or
binder change is checked against every spec it reaches.

**The head pass** runs in the head's build, the process the check started:

1. `discover`, then for each spec:
   - each stack of its kind, as `validate` does;
   - also the stack the deploy picks, with `DEPLOYMENT=platform/blue`
     through `select_bound`, so its colour fallback is exercised.
2. `validate`, as today.
3. **Render with stub outputs.**
   - The outputs are built from the bound spec's own references
     (`references`). Each one is stubbed only when its unit declares it:
     - a `config.yaml` unit's kind schema lists it under `x-pigeon.outputs`;
     - or it is a site's `bucket`, as `site_buckets` fills.
   - Each stub has the shape the reference asks for:
     - with no index, `stub-<name>`, or 32 zero hex digits when the name
       ends in `_id`;
     - with `[i]`, a list of `i + 1` stubs;
     - with `[name]`, a list holding `name`.
   - A reference that no unit declares is a problem.
   - Images are `registry.cloudflare.com/stub-<class>:stub`.
   - No state, no passphrase and no credential is read, so the pass runs the
     same locally and on a Checker.
4. **Unresolved bindings fail.**
   - Any string in the rendered `wrangler.jsonc` that contains `$deployment.`
     or `$output.` fails, with the spec, the stack, the JSON path and the
     literal.
   - An `$output.<unit>.<key>` whose key no layer's schema declares fails
     too, even where `render_with` would leave it as a var. This closes the
     silent-typo path in `bind`.
5. **Dry-run.** `wrangler deploy -c <rendered> --dry-run --outdir <tmp>` on a
   copy of the rendered config:
   - `build.command` is removed;
   - `main` is a stub module that exports an empty class for each Durable
     Object and Container class the spec declares.
   - So wrangler checks the config (bindings, migrations, Containers,
     routes' shape) without compiling the wasm. The wasm builds already run
     in the `clippy … (wasm)` checks.
   - Wrangler on `PATH` is required whenever `PIGEON_BUILD_SHA256` is set (on
     a Checker). Locally, a missing wrangler skips this step and says so.
   - The dry-run doesn't check routes against the zone. The validator's
     `exact_host` still does.

**The main pass** runs in main's build, the binary the deploy will use:

- **Which binary.** On a Checker, it is `$TMPDIR/pigeon-build`, run only if
  its sha256 equals `PIGEON_BUILD_SHA256`. Locally, it is the
  session-installed `~/.cache/pigeon/bin/pigeon`, which `pigeon session
  install` verified against `deploy.status`'s `latest_build`.
  - `--main PATH` names another binary.
  - On a Checker, a missing or mismatched binary fails the check. It never
    skips.
- **What it runs.**
  - If main's build has the subcommand (`<main> workers deployable --help`
    exits 0), it runs
    `<main> workers deployable --root <head> --pass head`. That repeats
    steps 1 to 5 with the deploy's own parser, binder and renderer.
  - Until then (the bootstrap), it runs `<main> workers validate
    --deployment <stack>` for each spec and stack. That parse is what failed
    !1353.
  - The report says which form ran.
- **The result.** A failure says: "main's build (`<sha>`) can't deploy this
  spec; a spec field ships one change after its parser (§1)".

The check passes only if both passes pass. Its output names specs, stacks
and JSON paths. Stub values carry no real outputs, so no ID, bucket or
private zone name can appear.

### 3. Selection

`deploy::scope::jobs_of` adds the check `workers deployable`, with argv
`layout.pigeon(["workers", "deployable", "--root", <root>])`, when a
change touches:

- `infra/**/workers/**`;
- `src/pigeon-agent/src/workers/**`;
- `config.yaml` or `catalog/*/kinds/**`, which feed the binder;
- `Dockerfile`, which a spec's Containers name.

The check runs on a Checker. Its argv is `cargo run … workers deployable`,
which `actions::on_github` doesn't send to GitHub.

### 4. Later: the candidate on the standby colour

Once `pigeon-green` serves, a deploy goes to the standby first
([`2026-11-18-runtime-independent-blue-green-workers.md`](2026-11-18-runtime-independent-blue-green-workers.md)
A.4 and step 9).

- **The owner of that step is the blue-green planner,** agreed on
  2026-11-19. It is gated on step 9 and is not part of this ADR's execution.
- **What it does.** The standby deploy calls `pigeon workers deployable` on the
  rendered config before `wrangler deploy`.
- **A later pre-merge step,** deploying a change's candidate to the standby
  before it merges, needs the lease and drain work that step owns. It is
  proposed there, not here.

### Execution

1. This ADR.
2. **Brief b1 (sonnet): `src/pigeon-agent/src/workers/deployable.rs`.**
   - Stub outputs, the head pass, the unresolved-binding scan and the
     dry-run.
   - The `pigeon workers deployable` subcommand.
   - The main pass with its bootstrap fallback.
   - Tests in `src/pigeon-agent/tests/suite/workers/`:
     - a `$deployment.typo` fails;
     - a raw `$deployment.queue_name` in a queue binding fails;
     - an unknown field fails the main pass on a fake old binary;
     - this repository's specs pass.
   - The only hooks: `mod deployable;` in `workers/mod.rs`, and the CLI entry
     in `pigeon-harness`. Nothing in `jobs.rs`.
3. **Brief b2 (sonnet), after b1 merges and deploys:** §3's selection in
   `deploy::scope`, with its scope tests. Sent to the blue-green planner
   first, since its A2 edits `checks_scope.rs`.
4. **The first change that runs it** is b2's own. Its main pass is the
   bootstrap form until b1's build is `latest_build`.
5. §4 belongs to the blue-green planner.
