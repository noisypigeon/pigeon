## The app's Tracker and Approvals become one tab, Now: two cards and the merge queue

* Date: 2026-11-15
* Model: Claude
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `src/pigeon-ios/Pigeon/RootView.swift`, `src/pigeon-ios/Pigeon/TrackerView.swift`, `src/pigeon-ios/PigeonCore/Tracker.swift`, `src/pigeon-ios/PigeonCoreTests/TrackerTests.swift`, `src/pigeon-cli/crates/backplane/src/core/app/tracker.rs`

## Context

The app has three tabs
([`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md),
its 2026-11-15 amendment "Three tabs", !510):

- **Tracker**: each session's changes on a rail, its cards made glanceable
  and silent sessions hidden by the amendment "Glanceable tracker cards"
  (!548, !549). !586 makes a filter that matches nothing fall back to all.
- **Approvals**: what waits on the owner. !547 replaces its `Accept edits`
  switch with Auto's state and Pause/Resume, and lists the owner's items
  under "For you", per
  [`2026-11-15-security-standing-auto-owner-flags.md`](2026-11-15-security-standing-auto-owner-flags.md)
  §4 and §5 (!546 is the control plane's side).
- **Insights**.

The owner reads the first two tabs together: who is working, what needs
them, and what merges next. Today they flip between tabs for that, and
neither tab shows the merge queue in order: the tracker groups changes by
session, so a session's two queued changes sit together however far apart
they are in the queue.

The owner approved a mockup on 2026-11-15
(https://claude.ai/artifact/4Pxaxv3d1pzA6Y9Fvd6N2H): one tab, two cards, the
queue, and a sheet behind each.

## Implementation plan.

### 1. The tab

The tab bar becomes **Now** and **Insights**. Now is the default, and its
badge is the Needs me count.

From the top:

1. **The top bar,** unchanged.
2. **Auto's state,** as !547 draws it: on, held or paused, with why, the
   merges in the last 6 hours, and Pause or Resume, each by passkey. With no
   window, the paused card with Resume.
3. **Two cards side by side:**
   - **Active sessions**: their number, and a chip per model class with its
     count: Opus, Sonnet, Haiku, Fable, then Other. A class at 0 isn't
     shown.
   - **Needs me**: the number of items waiting on the owner (§3), filled in
     the accent, with a line by kind: `1 approval · 1 decision · 1 change at
     needs you`.
4. **The merge queue, in order.** The change merging now comes first, marked
   `Merging`, then one cell per queued change with its position. Each cell
   shows its position, number, model class, session, the title's subject
   on one line, and its wait (`after !578`, `keeps its place`). Two changes
   of one session at different places are two cells. Past 10 cells, a
   `Show n more` row shows the rest.

**Tapping:**

- **Active sessions** opens their sheet: the counts; the tracker's filter
  chips (needs you, stalled, on a provider) with !586's fall-back; `◌ n
  silent` and the hidden line, from !549. Each session is a card with its
  class badge and role, a title, what it's doing or needs (`Merging !578
  now`, `Fixing: …`, `Waiting on checks: 7 of 8`, `Needs you: …`), its
  changes as chips, and **Open in Claude**, its label. A change chip opens
  the change's sheet.
- **Needs me** opens its sheet: every item from §3, oldest first, each
  answered in place: an approval's **Approve with passkey** (the existing
  approval card), a decision's options and free text, an action's Done,
  and a change at `needs: owner` with its `next` and a link to its
  approval. The line under the list says that answered items leave it.
- **A queue cell** opens the change's sheet, the one !549 built, gaining:
  its stage and position; who it needs and `next`; `waits_on`, as a link to
  that change's sheet; its checks, each with its status, and the GitHub run;
  its session's class, role and **Open session in Claude**. When it needs
  the owner, an **Approve with passkey** button opens its listed approval.

The session cards leave the tab itself. The tracker's live reading is
unchanged.

**The deep link:** `pigeon://approval/<id>` selects Now and opens that
approval's card; `pigeon://home` selects Now.

### 2. Active sessions and their classes

- **Active** is a session the tracker lists: one holding a live ticket or an
  open change, or a change merged within the hour. Silent sessions count:
  the card answers who is working at all, and the sheet hides the silent
  ones as the tracker does.
- **A session's class** is the one its latest ticket declared, live or
  lapsed (`Tickets::class`), as `change_list`'s `model` already is. Today
  the tracker reads it from a live ticket only, so a session whose ticket
  lapsed showed no class. A session never started has none, and counts as
  Other.
- **Fable joins the classes.** `sessions::Class` gains `fable`, the most
  capable, after `opus`: a planner, covering every brief, with Opus's
  limits in the per-class breaker. A brief still names `haiku` or `sonnet`.
  `models.json` gains its ID.
- **`Class::parse` maps a model's name or ID to its class,** case-blind:
  any of `haiku`, `sonnet`, `opus` or `fable` inside it (`claude-opus-5-5`,
  `Opus 5.5`) names that class. Anything else is still refused, so a ticket
  never holds an unknown class.
- **The app decodes the class as a string**, and draws any it doesn't know
  as Other, so an older app reads a newer control plane.

### 3. Needs me

One list of what waits on the owner, from two reads the app already makes:

| Kind | From |
|---|---|
| **Approval** | `GET /v0/app/home`'s `approvals` |
| **Decision** and **action** | `GET /v0/app/home`'s `items` (!546) |
| **Change at needs you** | the tracker's changes with `needs: owner` and no listed approval |

- A change's approval and its `needs: owner` stage are one item, keyed by
  its number: the approvals-always-listed ADR lists an approval whenever a
  change needs the owner, so the third row only catches a change whose
  listing is late.
- The count is the number of items. The tab's badge is the same number.

### 4. The data

The tab reads three routes, all the owner's, behind the device signature:

- **`GET /v0/app/home`**, unchanged: Auto, approvals, items.
- **`GET /v0/app/tracker`**, followed live by `/v0/app/events` as today.
  It gains:
  - a session's class from its latest ticket (§2);
  - **`queue`**: the merge queue in order, each
    `{number, title, session, model, stage, position, needs, waits_on,
    next, since}`. `position` is 0 for the change merging, else its place
    from `queued n/m`. The queue includes changes with no session, which
    the session cards leave out.
- **`GET /v0/app/changes/<n>`**, new: one change's sheet on demand: its
  stage, its jobs (`name`, `status`, `attempts`), its GitHub runs, and
  the id of its listed approval, if any. It is read when the sheet opens
  and on the events that name that change, never polled.

No new endpoint lists sessions. The tracker already is that read, and one
read keeps the cards, the sheet and the queue in agreement.

### 5. What moves, and what stays

- `ApprovalsView` goes. Auto's card and the owner's items, from !547, move
  into the Now tab and the Needs me sheet. The approval card
  (`ApprovalSheet`) is unchanged.
- `TrackerView`'s session cards and chips move into the Active sessions
  sheet. Their `PigeonCore` logic (`visible`, `isSilent`, `RowStatus`,
  `effectiveFilter`) is reused unchanged.
- The pure parts are new in `PigeonCore`, each tested: the class tally,
  the queue's order, Needs me's merge and count, and a session's "doing"
  line.

### Execution

Each step is a change by this planner. Steps 2 and 3 may run at once.

1. **`docs(adr)`:** this ADR.
2. **`feat(backplane)`:** §2's `fable` and `Class::parse` (`sessions.rs`,
   the matches in `core/brief/mod.rs` and `core/change/breaker.rs`,
   `tooling/session/models.json`); §4's tracker class, `queue`, and
   `GET /v0/app/changes/<n>` (`core/app/tracker.rs`, `worker/app.rs`,
   `worker/devices.rs`), with tests.
3. **`feat(ios)`:** §1, §3 and §5 in `RootView.swift`, `TrackerView.swift`,
   `HomeView.swift`, `Sheets.swift`, `Store.swift`, and `PigeonCore`'s
   `Tracker.swift` and `Models.swift`, with tests. It builds on !547 and
   !586 with `after` while either is open, and reads `queue` and the change
   route only once step 2 has deployed, falling back to the tracker's rows
   until then.

Then `ios.release`, and the TestFlight build goes to the owner.

### Verification

- The tab shows the live session count and its classes, and a class at 0
  isn't shown.
- With a session holding two queued changes, the queue shows two cells at
  their positions, and the merging change first.
- Needs me counts an approval and its change's `needs: owner` stage once.
- Pause and Resume work from the Now tab.
- A push opens the Now tab with its approval's card.

## Amendments

### 2026-11-15: A change's sheet is `change.status`, and its route a shim

The single-entry-point planner moves the app onto backplane operations
(`wire::CATALOG`) served through `/mcp`, so the Now tab adds no read of its
own:

- **`change.status` gains `position`, `model` and `approval`:** the change's
  place in the merge queue (0 while it merges), its session's class from its
  latest ticket, and the app's id of its listed approval, if any. It already
  answers the stage, jobs and GitHub runs, so it is the sheet.
- **`GET /v0/app/changes/<n>` is a thin shim:** device-signed, it runs
  `change.status` as the owner through `run()` and answers its result
  unchanged. It goes when the app reads operations itself.
- The tracker's `queue` and a lapsed ticket's class are fields of the
  existing tracker read, which the single-entry-point ADR names
  `tracker.show`; its result keeps this shape when it becomes that
  operation.
