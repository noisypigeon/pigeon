## Tiered approvals: automatic, confirmed and code-proven, with task mandates and a page for mobile

* Date: 2026-11-08
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: security
* Governs: `.config/approvals.yaml`, `src/pigeon-cli/crates/backplane/src/approv*.rs`, `src/pigeon-cli/crates/backplane/src/worker/approve.rs`, `src/pigeon-cli/crates/backplane/src/mandates.rs`, `src/pigeon-cli/crates/backplane/src/worker/mandates.rs`, `src/pigeon-cli/crates/backplane/src/grants.rs`

## Context

[`2026-11-06-development-control-plane-approvals-images-state.md`](2026-11-06-development-control-plane-approvals-images-state.md)
puts the owner behind every change.
- **Its amendments make each approval a card in Claude** that asks for the
  owner's authenticator code (TOTP), bound to one grant of one operation and
  head.
- **The card covers `change.authorize`, `change.approve`, `change.merge` and
  `change.close`.** A change needs at least three cards: authorize, approve
  and merge.
- **On 2026-11-07 the owner answered about forty cards** for changes 30 to 42.
  - Most were routine: a follow-up within an ADR already approved, an ADR
    amendment recording a result, a spike to read and close.
  - Several timed out while the owner was away.

The owner asked for three things:
1. Routine approvals made automatically.
2. A plain approval, with no code, where the agent wants the owner's word on
   something that isn't dangerous.
3. A way to approve risky operations, with the code, from a phone.

The aim: the owner hands the agent a task to finish end to end, and hears from
it only when something unexpected happens or something destructive needs
approval.

What the design rests on:

- **Cards don't reach the phone.**
  - Claude Code's documentation ("Answer an elicitation request in a cloud
    session") says a cloud session's elicitation request shows as a card at
    claude.ai/code or in the Desktop app's Code tab; "if you see no card in
    either place, you can't answer the request there".
  - This session saw the same:
    - on the phone, the Claude app showed the tool's permission prompt
      ("Yes, every time" or "Yes, this once"), and no card, in either form or
      URL mode;
    - on the desktop, the same calls showed their cards.
  - A request made from the phone therefore waits out its timeout.
- **A form card can be answered by a hook.** Claude Code's `Elicitation` hook
  can answer a form without showing it. A hook comes from settings the agent
  can edit, so an accepted card proves nothing about who accepted it. That is
  why the code card exists, and why a plain Accept card can only be used
  where forging it does no harm.
- **A page drawn by the Worker, behind Access, proves a person.**
  - Only a person's Access identity can open it; Access refuses the agent's,
    the deployer's and the builds service tokens.
  - It works in any browser, a phone's included.
  - It shows what the Worker holds, not what a client draws, which the
    approvals ADR rated stronger than the card. It was removed only for
    convenience on the desktop.
- **The bound job already runs `main`'s code.** Since `!34` and `!38`, jobs
  run `main`'s newest `pigeon`. The bound job reads the change's files and
  scope, so it can classify a change by rules taken from `main` and not from
  the change's own branch.
- **Today's levels:** the agent's service token may propose, push, rerun
  checks and retry deploys. Everything that changes `main`, closes a change or
  applies infrastructure needs the owner.

## Implementation plan.

### 1. Three tiers, chosen by the control plane

Every operation that needs the owner falls into one of three tiers. The
control plane decides which, never the agent.

| Tier | Who approves | How |
|---|---|---|
| **auto** | the control plane, under a rule or a mandate | no prompt; recorded as `policy:<rule>` or `mandate:<id>` |
| **confirm** | the owner, as a person | one tap: an Accept card on the desktop, or the page's button |
| **code** | the owner, proven | the authenticator code: the code card on the desktop, or the page's field |

- **A tier may be raised, never lowered.** The agent may ask for a higher
  tier than its rule gives, for instance to have the owner look at
  something. Nothing makes a code-tier operation confirm or auto.
- **Every approval names its tier and its approver** in `change_status`.

### 2. The rules: `.config/policies/approvals.yaml`

One policy file, read from `main` by the bound job, classifies a change by its
files and the operation.

- **Guarded paths: always code.**
  - The approval machinery: `approvals.yaml` itself, backplane's `access.rs`,
    `grants.rs`, `totp.rs`, the classifier, and the Worker's approval page
    and cards.
  - Every policy in `.config/policies/`.
  - The two `secrets.config.yaml` files.
  - `.mcp.json` and `.claude/settings*.json`.

  A mandate never covers these.
- **Deploying paths: code without a mandate.** Their merge deploys something
  live:
  - a Worker's inputs (`cobot::scope::workers`);
  - `infra/**`;
  - the Dockerfile and `pigeon-start`;
  - D1 migrations.

  A mandate covers them only if it says it may deploy.
- **Everything else: confirm without a mandate.** Documentation, tooling,
  tests, and code that deploys nothing.
- **Standing auto rules,** with no mandate needed:
  - **Spikes:** a change touching only `tooling/spikes/**` is authorized and
    closed automatically. It is never merged, so its merge stays confirm.
  - **ADR amendments:** a change whose only files are existing ADRs that it
    appends to is authorized, approved and merged automatically. "Appends
    to" means added lines after `## Amendments`, as `decisions.yaml`
    already requires for a merged ADR. A new ADR file is a new decision, so
    it stays confirm.
  - **Rerunning checks and retrying deploys stay the agent's own,** as now.
- **Operations outside changes:**
  - `infra.apply`, and any destructive `sites.publish` or deploy override,
    stay code;
  - granting or widening a mandate is code;
  - ending a mandate or pausing auto is open to anyone, the agent included,
    because it only takes power away;
  - resuming after a pause is code.
- **The bound job reports the tier** for each head, with the rule that chose
  it. The Worker trusts that report, because the bound job runs `main`'s
  merged code. A push that moves the head classifies it again.

### 3. Mandates: a task approved once

- **What a mandate is.** The owner approves a task once, with the code, and
  the changes that carry it out go through automatically. A mandate holds:
  - the ADR the task executes, which must be on `main`, and a summary;
  - its **scope**, as globs; guarded paths are always cut out of it;
  - whether it **may deploy**;
  - a **budget**: at most 10 merges, default 5;
  - an **expiry**: at most 72 hours, default 24.
- **The agent proposes one** with a new tool, `mandate_propose`. The owner
  approves it at the code tier, on the card or the page, which shows its
  scope, budget, expiry and deploy flag.
- **A change runs under a mandate** when its proposal names the mandate, its
  plan cites the mandate's ADR, and its scope fits inside the mandate's.
  Then:
  - it is authorized automatically;
  - once its head is `checked`, with no violations, it is approved and
    merged automatically, using one unit of the budget;
  - its deploy runs as any other.
- **A mandate pauses itself, and the owner is told, when:**
  - a deploy fails or rolls back;
  - a check fails twice on the same head;
  - a change needs scope outside the mandate;
  - its budget is used up, or it expires.

  The owner resumes it, at the code tier, or lets it end.
- **The agent may end its own mandate early,** for instance when the task is
  done.
- **`mandate_status`** shows each mandate: its scope, budget used, expiry,
  and the changes merged under it.
- **The residual risk.** A mandate that may deploy trusts the agent's code
  within its scope, because the guarded paths stop mistakes, not a
  determined agent.
  - Its limits are its scope, its budget, its expiry, the pauses, and the
    record of every merge.
  - The owner grants deploy rights for tasks that need them, and can read
    every merged change afterwards.

### 4. The approval page, for the phone and any browser

The page removed on 2026-11-07 comes back, for both confirm and code, and
becomes the way to approve away from the desktop.

- **A request no longer blocks.**
  - A confirm-tier or code-tier operation called by the agent answers at once
    with `pending`: a grant, one-time and bound to the operation and head,
    valid for 24 hours; and a link, `https://<host>/approve/<grant>`.
  - The tool result carries the link, and the agent passes it on: in its
    reply, and as a push notification to the owner's phone.
- **The desktop keeps its cards** through a new tool, `approval_wait`, which
  holds the request for up to 10 minutes. On a client that shows forms it
  shows the card:
  - an Accept card for confirm;
  - the code card for code.

  The grant can be answered on the card or the page, whichever comes first.
- **The page, `GET /approve/<grant>`:**
  - It shows the operation, change, title, plan, scope, head, checks,
    violations, tier and the rule that chose it.
  - It is server-rendered, with no script, every value escaped, and a
    `default-src 'none'` content-security policy, as before.
  - A confirm request shows **Approve** and **Decline**. A code request adds
    one field for the 6-digit code.
- **`POST /approve/<grant>`** must be same-origin (`Sec-Fetch-Site`) and come
  from a person's Access identity. It then:
  - redeems the grant once;
  - for code, checks the code against the same TOTP guard as the card:
    each step used once, and five wrong codes in an hour lock it;
  - runs the operation as that person.
- **`GET /approvals`** is an inbox of every pending grant, with links. It is
  the page to bookmark on the phone.
- **Access on the phone.** The owner signs in to Access in the phone's
  browser once. The session lasts as long as the Access application allows,
  which the owner may lengthen for the phone. The page needs nothing else.
- **Why not in the chat.**
  - A code typed into the chat would pass through the agent, which could
    spend it on another operation.
  - A "yes" in the chat can't be told apart from the agent's own words.

  So neither is an approval.

### 5. Told only when needed

- **The agent sends a push notification** when:
  - a request is pending at the confirm or code tier, with its link;
  - a mandate pauses itself, with why;
  - a task under a mandate is finished, with the changes merged.

  Otherwise it works on without asking.
- **A daily digest** in `deploy_status` lists what merged automatically, by
  rule and mandate, so the owner can read it afterwards.

### Execution

Each step is its own change, and each touches guarded paths, so each is
approved at the code tier:

1. This ADR, with a `CLAUDE.md` pointer.
2. **The classifier, in shadow:**
   - `approvals.yaml` and backplane's pure classifier (files, operation,
     mandate to tier and rule);
   - the bound job reports the tier, and `change_status` shows it;
   - approvals still work as today, so the tiers can be checked against real
     changes first.
3. **The page and pending requests:**
   - the grant's 24 hours;
   - `/approve/<grant>` and `/approvals`, with the code field;
   - `approval_wait` and its cards.

   This is the mobile path for every tier, still with the code everywhere.
4. **The confirm tier goes live:** the Accept card and the page's button.
5. **Mandates:**
   - `mandate_propose`, `mandate_status` and `mandate_end`;
   - a D1 table for mandates, budgets and pauses;
   - automatic authorize, approve and merge within a mandate.
6. **The standing auto rules** (spikes and ADR amendments), and the digest.

### Verification

- **In shadow,** every change from step 2 shows a tier in `change_status`
  that the owner agrees with.
- **From a phone:**
  - a code-tier request is approved on `/approve/<grant>` with the code;
  - a wrong code is refused, and five lock the hour;
  - the agent's service token gets 403 on the page.
- **Under a mandate,** a two-change task with deploys merges and deploys
  with no prompt, and the owner hears from it only once it's finished.
- **A mandate pauses itself** when a deploy is made to fail, or a change
  reaches outside its scope, and the push notification says why.
- **A change to `approvals.yaml` under a mandate** still asks for the code.

## Amendments

### 2026-11-08: The rules live in `.config/approvals.yaml`; the classifier lands in shadow

- **Not under `.config/policies/`.** Sentinel loads every YAML file there as
  one of its policies, so the rules are `.config/approvals.yaml`, which is
  itself guarded.
- **Backplane starts no process.** `.config/policies/backplane.yaml` forbids
  `std::process::Command` in backplane, so the bound job reads the base's
  rules and diffs through cobot (`ci::base_file`, `ci::appends_only`).
- **A plan's tier comes from its scope.**
  - A scope glob reaches a guarded or deploying pattern when one's literal
    prefix starts the other.
  - A pattern with no literal prefix, such as `**/secrets.config.yaml`,
    counts only where it matches the scope entry itself. The merge's tier
    checks the files a head really touches, so a guarded file always needs
    the code at merge.
- **Shadow first.**
  - The bound job reports `tiers` (authorize, merge and close, each with its
    rule) for every head, and `change_status` shows them.
  - Nothing is approved by them yet.

### 2026-11-08: The page and pending approvals land, with the code everywhere

- **An owner's operation called by the agent no longer waits.**
  - It mints a page grant, lasting 24 hours (`grants::PAGE_TTL`; a card's
    stays 15 minutes), and adds it to the inbox the `Instance` Durable
    Object keeps.
  - It answers at once, not done: `pending` holds the operation, the change,
    the link and when it expires.
  - The agent passes the link on, in its reply and as a push notification.
- **`approval_wait`,** a new MCP tool for the agent (`approval.wait`), shows
  today's code card for one operation on a client that shows forms.
  - Its grant is the card's own. Whichever of the card or the page is used
    first wins; the other is then refused, because the change has moved on.
- **The page is the removed one, restored, with:**
  - one field for the authenticator code;
  - the head's tiers;
  - a link to `/approvals`, the inbox, newest first.
- **One code check for card and page.** `Instance`'s `/code/verify` runs the
  card's `Guard` for both: each step used once, and five wrong codes in an
  hour lock both.
  - The page checks the code before it redeems, so a wrong code doesn't
    spend the link.
- **Only a person's Access identity** opens the page, or posts to it,
  same-origin. The operation runs as that person.
- **The confirm tier is not live yet.** Until step 4, the page asks for the
  code at every tier.
- **The page's files join the guarded list:** `approve.rs` and
  `worker/approve.rs`.

### 2026-11-08: The confirm tier goes live for merges and closes

- **A grant records its tier when minted** (`Grant.tier`, from
  `approvals::tier_of` on the change's current head).
  - Approving and merging take the head's merge tier.
  - Closing takes its close tier.
  - Auto counts as confirm until automatic approval lands.
  - A head with no tiers needs the code; closing, which takes nothing from
    `main`, needs a tap.
- **Authorizing stays at the code tier.** A plan is authorized before any
  push, so the bound job hasn't classified it, and the Worker doesn't read the
  rules. Mandates (step 5) need it classified.
- **At the confirm tier:**
  - the page shows one button and no code field;
  - `approval_wait` shows an Accept card with no fields;
  - neither checks a code.

  The page still needs a person's Access identity. The card is the one an
  `Elicitation` hook could forge, which §1 accepts for confirm because it can
  only approve what isn't dangerous.
- **A new push clears the old head's tiers,** so a grant never takes a tier
  from another head.
- **`change_status` shows the bound job's log tail** as `bound_log`. It says
  which rules the job read and the tiers it gave, without the dashboard.

### 2026-11-08: Mandates land in two parts; first, granting them

- **Step 5 splits.**
  - **5a: mandates themselves.** The agent proposes one, the owner grants it
    with the code, either may see it, anyone may end it.
  - **5b: approving inside one.** Automatic authorize, approve and merge,
    and the pauses.
- **The `Instance` Durable Object keeps them** (`backplane::mandates`), with
  the grants that open their page. Not D1: they are few, and one instance
  holds them.
- **`mandate.propose`** takes an ADR under `.claude/decisions/`, a summary,
  scope globs, a deploy flag, a budget (1 to 10 merges, default 5) and hours
  (1 to 72, default 24).
  - It answers with a page link, `/mandate/<id>?grant=…`, lasting 24 hours,
    also listed in `/approvals`.
  - The page shows the mandate and takes the code, with the same guard as
    every approval.
  - A grant starts the hours; it can't be granted again once ended.
- **`mandate.status`** lists every mandate. **`mandate.end`** ends one, open
  to the agent because ending only takes power away.
- **Covering a scope.** A change's glob lies inside a mandate's when it is
  that glob's path or, for a wildcard, a narrower `**` beneath it; `**` alone
  never fits a narrower mandate.
- **5b leans on the bound job.** A guarded file makes a head's merge tier
  `guarded`, and a mandate never lowers that. So the Worker needs no copy of
  the rules to keep guarded paths out of a mandate.
- **Guarded** as approval machinery: `mandates.rs` and `worker/mandates.rs`.

### 2026-11-08: Approving inside a mandate (5b)

- **`change.propose` takes an optional `mandate`,** kept on the change.
- **Before it opens a pending approval,** the Worker asks the `Instance` for
  the mandate's decision (`mandates::decide`):
  - **allow:** the mandate is active, unexpired and within budget; the plan
    cites its ADR; its scope covers the change's; and, to approve or merge,
    the head's merge tier is auto or confirm, or code only for `deploys`
    with a mandate that may deploy. The operation runs as `mandate:<id>`,
    and a merge spends one unit of the budget;
  - **ask:** anything else, such as a guarded head, a head with no tiers yet
    or a plan that cites another ADR. The owner gets the usual link;
  - **pause:** a change whose scope reaches outside the mandate. The mandate
    pauses, saying why, and the owner gets the usual link.
- **A failed deploy pauses every active mandate.**
- **Not yet:** pausing after two failed checks on one head, resuming, and
  telling the owner of a pause; `mandate_status` shows the pause and why.

### 2026-11-08: Mandate operations reach the control plane; a used link says so

- **The Worker refused `mandate.*`.** It sent them on as Container
  operations, so mandates could be neither proposed, listed nor ended over
  MCP. They are now control-plane operations, as `change.*` are, and a
  test holds it.
- **A used approval link answers that it was used:** "this link was
  already used: `<operation>` on change `<n>` is done". It no longer says
  "not valid".
  - A redeemed grant is kept, marked `used`, until it expires, which is
    when minting prunes it.
  - It still can't be used twice.

### 2026-11-14: A mandate or policy never approves a brief's head awaiting review, or escalated

Per
[`2026-11-13-development-model-tiered-agent-briefs.md`](2026-11-13-development-model-tiered-agent-briefs.md)
§5 and §6:

- **A change carrying out a brief** is approved under a mandate or by policy
  only once its brief's planner has accepted that head (`brief_review` with
  `accept`). It is never approved by either while it is escalated to the
  planner.
- **The gate is in the lifecycle,** not in `mandates.rs` or `approvals.rs`.
  `Change::approve` refuses any approver named `mandate:`, `window:` or
  `policy:` for such a head. A mandate's `allow` approves nothing, and the
  head stays `in-review` or `escalated`.
- **Few such changes reach a mandate.** An executor may not name one on
  `change.propose`. A planner's change may name both a brief and a mandate,
  and the gate holds for it too.
- **Not yet:** refusing an executor `mandate.propose`. `mandates.rs` is
  guarded, so that is a change at the owner's code.
