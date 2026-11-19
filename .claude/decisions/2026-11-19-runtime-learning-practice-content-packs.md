## Learning: a practice-sheet capability parameterised by a content pack, with ICBC 7L as its first pack

* Date: 2026-11-19
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `src/pigeon-ios/PigeonCore/Learner*.swift`, `src/pigeon-ios/Pigeon/Learner/**`, `src/pigeon-core/src/learner/**`

## Context

[`2026-11-15-runtime-learner-practice-standalone-app.md`](2026-11-15-runtime-learner-practice-standalone-app.md)
built one practice app for one test: BC's Class 7L knowledge test. Its 2026-11-18
amendment made it the `icbc` capability.

Everything about that test is fixed in code:

- the bank is one app resource, `Pigeon/Learner/questions.json`, with 267
  questions;
- `makeForm` draws a fifth from the topic `Signs`, a tenth `combined`, and
  weights topics missed 25% or more;
- the page says "BC Class 7L Practice Test", and the screens say "Class 7L
  practice";
- the instance's state is under `learner/`, sealed with
  `HKDF(STATE_KEY, "pigeon-learner-v1")`.

The page geometry, the code strip, the reader, scoring, sync and the sealed
store aren't specific to the test.

The owner asked to generalise this into a reusable capability, parameterised
by its content, with ICBC as its first user. June's instance will use the
ICBC-7L variant once their onboarding resumes. That onboarding stays
parked: this ADR builds and tests the capability, and onboards no one.

[`2026-11-19-runtime-capability-interface-manifest-trait.md`](2026-11-19-runtime-capability-interface-manifest-trait.md)
§1 gives a capability `built_on`, so a variant can configure an engine.

## Implementation plan.

### 1. Names

- **`learning`** is the engine: forms, pages, scoring, progress, hints and
  the sealed store. It isn't grantable, since it means nothing without a
  pack.
- **A pack** is content plus the rules for drawing a form from it.
- **`icbc`** is the variant `built_on: learning` with config `{pack:
  icbc-7l}`. It keeps its name, so the learner's existing grant (the
  migration's) and the users ADR's vocabulary stand. The Members screen
  labels it "ICBC 7L".
- A new test is a new pack and a new variant capability, each with its own
  grant. No engine code changes.

### 2. A pack

`src/pigeon-ios/Pigeon/Learning/Packs/<pack>/`, bundled with the app:

```yaml
# pack.yaml
id: icbc-7l                   # [a-z0-9-]
version: 1.0.0
title: BC Class 7L Practice Test     # printed on the page
short: Class 7L practice             # the screens' eyebrow
locale: en-CA
choices: 4                    # 2 to 4
per_page: 25
form:
  quotas:                     # each fraction of a form, drawn first
    - { topic: Signs, fraction: 0.2 }
    - { flag: combined, fraction: 0.1 }
  weight_missed: { threshold: 0.25, picks: 2 }
  exclude_flags: [unverified]
sources: { label: "Learn to Drive Smart" }
```

- **`questions.json`** sits beside it, in today's shape, unchanged:
  `Pigeon/Learner/questions.json` moves to `Packs/icbc-7l/questions.json`
  byte for byte.
- **The validator** (`pigeon dev capabilities`, the capability ADR's §5)
  checks every pack in Rust:
  - `pack.yaml` against `catalog/capabilities/learning/pack.schema.json`;
  - unique question ids, `choices` entries each, an `answer` within them,
    and a topic for every question;
  - every quota's topic or flag exists, and the quotas sum to at most 1;
  - at least `per_page` questions are drawable.
- A pack holds no personal data. It is public content.

### 3. The engine, parameterised

- **Swift (`PigeonCore`):**
  - `Learner*.swift` becomes `Learning*.swift`, taking a `Pack`;
  - `makeForm(pack:bank:…)` reads the quotas and weighting from the pack;
  - `LearnerPDF` takes its title from the pack;
  - the screens take their eyebrow from it.
- **Geometry, the code strip, the reader and scoring don't change.** With
  `icbc-7l`, a form has the same number, questions and key as today. A
  golden test pins this for seeds 0 to 1022.
- **The routes** stay `/v0/app/learner/*` for `icbc`, so old app builds keep
  working. The engine registers `/v0/app/learning/<pack>/*` for every other
  pack.
- **Storage:**
  - `icbc` keeps `learner/` and `pigeon-learner-v1`, so no sealed object is
    rewritten;
  - another pack uses `learning/<pack>/`, sealed with
    `HKDF(STATE_KEY, "pigeon-learning-v1:<pack>")`.
  - The manifest's `storage.r2_prefixes` lists both.
- **Rust (`core/learner` → `core/capability/learning`):** the pack id
  travels in each form and attempt, and the index is keyed by pack. A record
  without one reads as `icbc-7l`.
- **Hooks:**
  - `Enable` checks the pack exists in this build;
  - `Upgrade { from }` re-indexes after a pack's major version changes;
  - `OnRevoke` stops the device's two pre-approved writes at once;
  - `Before(learning.form)` refuses a pack that the capability's config
    doesn't name.

### 4. ICBC 7L on June's instance

- When onboarding resumes, June's standalone enables `icbc` (`{pack:
  icbc-7l}`), and June holds an `icbc` grant. Nothing about that is done
  now.
- The parked steps (users ADR step 8, the iOS Members screen B3) are
  unchanged.

### 5. Tests

- **Golden forms:** for `icbc-7l`, the parameterised `makeForm` equals
  today's for every form number. The same holds for the page's title and
  every circle's position.
- **A second pack fixture,** `example-pack`, with 2-choice questions and a
  different quota, renders, scans and scores in the simulator tests. This
  proves the engine isn't 7L-specific. The fixture is a test resource and
  never ships.
- **Storage:** an `icbc` attempt still lands under `learner/`, and an
  `example-pack` one under `learning/example-pack/`. Neither can read the
  other's key.
- **The capability:** `icbc`'s manifest validates; `learning` isn't
  grantable; a Standard member with `icbc` is refused `example-pack`.

### 6. Steps

| Step | What | Who | After |
| --- | --- | --- | --- |
| 1 | This ADR | planner | the capability ADR |
| 2 | `learning` and `icbc` manifests, `pack.schema.json`, the pack validator | brief | capability step 2 |
| 3 | `PigeonCore`: `Pack`, parameterised `makeForm`, PDF title and eyebrows; the bank moves; golden tests | brief (sonnet, on a macOS Bench) | 2 |
| 4 | Rust: `core/learner` → `capability/learning`, pack ids in records, routes per pack | brief; `worker/learner.rs` reviewed by the planner (users ADR path) | capability step 6 |
| 5 | The `example-pack` fixture and the second-pack tests | brief | 3, 4 |

### 7. Rollback

- Steps 3 and 4 keep `icbc`'s routes, prefix and key derivation. Reverting
  either one leaves stored data readable.
- The golden test fails any change that alters a 7L form.
