## A change's plan approval asks the owner for its inputs, on the approval page and in the app

* Date: 2026-11-13
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: security
* Governs: `src/pigeon-cli/crates/backplane/src/approve.rs`, `src/pigeon-cli/crates/backplane/src/worker/approve.rs`, `src/pigeon-cli/crates/backplane/src/worker/app.rs`, `src/pigeon-cli/crates/backplane/src/core/app/approvals.rs`, `src/pigeon-ios/Pigeon/Sheets.swift`, `src/pigeon-ios/PigeonCore/Models.swift`

## Context

Some changes need values only the owner can get, such as the verification
codes a vendor's dashboard shows for a domain.
[`2026-11-13-runtime-zone-vendor-dns-placements.md`](2026-11-13-runtime-zone-vendor-dns-placements.md)'s
step 5 is the first: Proton's two verification codes for `platform.pigeon.dev`.
Today the agent asks for them in chat and the owner pastes them there. The owner
wants the approval itself to ask for them, in the app and on the approval page.

### What exists

- **Approvals.**
  [`2026-11-09-security-approvals-once-passkeys.md`](2026-11-09-security-approvals-once-passkeys.md)
  sets tiers and grants.
  - An agent's `change_authorize` runs at once under a mandate, an Auto window,
    a policy or the change's grant.
  - Otherwise it waits on a grant, redeemed on `/approve/<n>`, with a code or
    passkey for the `code` tier.
  - [`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md)
    §4 redeems the same grant from the app, through
    `GET`/`POST /v0/app/approvals/<id>`.
  - Both redeemers build the operation's request themselves and run it as the
    owner (`APPROVER`).
- **A value typed at approval.** The platform-plan page takes an account token
  (`approve::platform_page`, `platforms::decide`). That value is a secret, so
  it's sealed and never shown.
- **Nothing generic.** A change has no notion of values asked of the owner.

### The decisions

- **A change asks at its proposal.** `change_propose` takes `asks`: up to 8
  questions, each with a key, a label, optional help, a character set and a
  length range.
  They're fixed for the life of the change.
- **The plan approval answers them.** `change.authorize` on a change with asks
  succeeds only with an answer to every ask. The change records the answers in
  `answers`, and `change_status` shows them.
- **Only the owner answers.**
  - Answers enter only through the two redeemers: the approval page's form and
    the app's `POST`.
  - An agent's request that carries `answers` is refused.
  - A change with asks is never authorized under a mandate, an Auto window, a
    policy or a grant. It always waits for the owner.
- **Answers are public by design.**
  - An ask is only for a value that will be published anyway, such as a DNS
    verification code.
  - The agent commits the answers to the public repository.
  - The page and the app say so above the fields.
  - Every character set is within `[A-Za-z0-9._:=+/-]` and every length
    within 1 to 256, so no answer holds whitespace or quotes.
  - A set and a length range, not a regex, keep the Worker free of a regex
    engine, and are checked the same way in Rust, in the page's `pattern`
    attribute and in Swift.
  - A secret is never asked this way. It goes through the platform-plan form or
    `secrets set`.
- **The MCP card doesn't take answers.** `approval_wait` on a change with asks
  points to the page or the app.

Out of scope: asks at any other operation, typed or secret answers, and
changing asks after the proposal.

## Implementation plan.

### 1. Asks on a change

`change_propose`'s new `asks` argument is JSON text:

```json
[
  { "key": "pm_verification", "label": "pm-verification code",
    "help": "Proton Pass → Settings → Aliases → platform.pigeon.dev → Info",
    "chars": "lower", "min": 20, "max": 40 }
]
```

- **`key`** matches `^[a-z][a-z0-9_]{0,39}$` and is unique.
- **`label`** is 1 to 80 characters.
- **`help`** is optional, at most 300 characters.
- **`chars`** is one of these sets, defaulting to `token`:

  | `chars` | Characters |
  | --- | --- |
  | `hex` | `0-9a-f` |
  | `lower` | `a-z` |
  | `alnum` | `A-Za-z0-9` |
  | `token` | `A-Za-z0-9._:=+/-` |

- **`min` and `max`** default to 1 and 256, with `1 ≤ min ≤ max ≤ 256`.
- Anything else, more than 8 asks, or an unknown set or bad range is refused at
  proposal.

`Change` gains:

- `asks: Vec<Ask>` and `answers: BTreeMap<String, String>`;
- both `#[serde(default, skip_serializing_if = …)]`, so stored changes still
  read;
- both set in `Change::propose`.

### 2. Answering at the plan approval

- **The event.** `Event::ApprovePlan` gains `answers`, read from the request's
  `answers` object.
- **`approve_plan` refuses:**
  - a missing answer, naming the key;
  - an answer to a key that wasn't asked;
  - a value with a character outside its set, or a length outside its range,
    naming the key but never echoing the value.
- **When it passes,** `approve_plan` stores the answers beside `plan_approval`.
- **A change without asks** ignores an empty `answers` and refuses a non-empty
  one.

### 3. Only the owner answers

In `worker::ask`, before the mandate, window, policy and grant paths:

- **A request carrying `answers` is refused,** with "answers come only from the
  owner, on the approval page or in the app". Neither redeemer goes through
  `ask`.
- **A `change.authorize` on a change with asks goes straight to pending.** It
  gets the page link, the event and the push as today. The stage's `next` says
  "the owner answers its asks and authorizes the plan".

### 4. The approval page

`approve::page` renders asks only for `change.authorize`, and only when the
change has asks:

- a note: "Each answer is stored on the change in plain text and will be
  committed to the public repository. Never enter a secret.";
- for each ask, a labelled `<input name="ask_<key>" required>`, with the
  set and range as the `pattern` attribute (`[a-z]{20,40}`, for example) and
  the help beneath. All of it is HTML-
  escaped, with no script, under the page's CSP.

`worker::approve::post` reads `ask_<key>` for each of the change's asks (the
change comes from `/peek`) and adds `answers` to the request's args before
`run`. The core checks them (§2), so the page only collects them.

### 5. The app

- **Reading.** `approvals::detail` adds `asks: [{key, label, help, chars, min, max}]`
  for a `change.authorize` with asks, and `[]` otherwise.
- **Answering.** `app::approve` reads `answers` from the body, keeps only the
  change's ask keys, and adds them to the args.
- **The passkey** still proves the grant as today. The answers aren't part of
  the challenge, since the grant fixes the change and its asks.
- **iOS.**
  - `ApprovalDetail` gains `asks: [Ask]`, decoding a missing key as `[]`.
  - `ApprovalSheet` shows the public-values note and a text field per ask, with
    its help. Approve is disabled until every field is within its set and range.
  - `Store.approve` sends `answers`.
  - `PigeonCoreTests` decodes a detail with and without asks.

### 6. The MCP card

`elicit::card` on a change with asks answers "this approval asks for inputs;
answer it on the page or in the app", with the page's link. It doesn't redeem
the grant.

### Execution

1. **This ADR,** with the zone ADR's amendment saying that its step 5 asks for
   the codes this way.
2. **`feat(backplane)`: asks and answers in the core.** §1 to §3: `Ask`, the
   `asks` argument, `approve_plan`'s checks, the refusals in `ask`, the stage
   line, and their tests.
3. **`feat(backplane)`: the page and the app API.** §4 to §6, with tests that
   a form or app body with answers authorizes, a bad answer is refused naming
   only its key, and the card refuses.
4. **`feat(ios)`: asks in the approval sheet.** §5's Swift, checked by the iOS
   check on its Mac runner.
5. **The first use:** the zone ADR's step 5.

### Verification

- A proposal with a bad key, a long label, 9 asks, an unknown set or a bad
  range is refused.
- An authorize with a missing answer, an extra key, or a value outside its set
  or range is refused. The refusal never contains the value.
- An agent's `change_authorize` with `answers` is refused. One without
  `answers` on a change with asks is pending even in an Auto window.
- The page shows the note and one field per ask, escaped. Its post authorizes,
  and `change_status` shows `answers`.
- The app's detail lists the asks, and its approve with answers authorizes.
- The card on a change with asks points to the page and redeems nothing.
- A change without asks behaves exactly as before.

## Amendments

### 2026-11-13: Reconciled with the breaker in `core/change/mod.rs`

- **The conflict.** !363 (step 2) and
  [`2026-11-13-development-session-stages-subscriptions-breaker.md`](2026-11-13-development-session-stages-subscriptions-breaker.md)'s
  !361 each added a module at the same line of `core/change/mod.rs`:
  `asks` and `breaker`.
- **Both stand.** The two are independent, so the file declares both, and
  neither decision changes.
- **How it was carried.** !363 merges `main` rather than rebasing onto it,
  since its branch is only fast-forwarded.
