## The owner pages take the iOS app's look: shared tokens, one stylesheet, the avatar inline

* Date: 2026-11-16
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: runtime
* Governs: `src/pigeon-core/src/look.rs`, `src/pigeon-core/src/look/**`, `src/pigeon-ios/tests/suite/design.rs`

## Context

The owner asked for the web UI to match the design the iOS app is
establishing. The app's look is settled in
[`2026-11-12-runtime-ios-app-approvals-insights.md`](2026-11-12-runtime-ios-app-approvals-insights.md)
§1 and lives in `src/pigeon-ios/Pigeon/Theme.swift`: a dot grid, hard offset
shadows, SF Pro and SF Mono, light and dark, a pink accent. `LinkView.swift`
("Link this iPhone") sets the buttons and the round avatar.

Every page the Worker serves to the owner (`/approvals`, `/approve/<n>`,
`/mandate/<id>`, `/grants/<id>`, `/ci-window/<id>`, `/platform/<id>`,
`/app/link`, `/devices`, `/passkeys`, `/passkeys/enroll`, and each outcome)
renders through one shell in `src/pigeon-core/src/approve.rs`, with its style
inline under a CSP of `default-src 'none'; style-src 'unsafe-inline'`
([`2026-11-08-security-tiered-approvals-task-mandates.md`](2026-11-08-security-tiered-approvals-task-mandates.md)
§4). That file is guarded, so each restyle there would need the owner's
proof.

The owner decided (owner item #15) that the site templates
(`src/pigeon-site-templates`) follow once the owner pages have.

## Implementation plan.

### 1. Tokens, mirrored from the app

- **`look/tokens.css`** holds the tokens as CSS custom properties:
  - every colour of `Theme.swift`'s `Theme`, named in kebab case
    (`onAccent` is `--on-accent`), dark in `:root` and light under
    `prefers-color-scheme: light`;
  - radii: card 14, primary button 12, secondary 10; the shadow's 3 px
    offset; the page's 24 px padding and 18 px gap; the type scale: display
    32 bold, body 17, small 15, mono 14 and its 12 px eyebrow.
- **Dark first.** The pages are dark unless the device asks for light, as
  the owner approves mostly on a phone at night.
- **`Theme.swift` stays the source.** Swift can't read a CSS file, and a
  generator would add a build step to both sides. The web mirrors it, and
  `src/pigeon-ios/tests/suite/design.rs` fails on Linux when a colour, the
  card radius or the shadow offset differs.

### 2. One stylesheet, over plain elements

- **`look/style.css`** styles the pages' existing elements, with no class in
  their markup, so the guarded files never change for a restyle:
  - the body is the dot grid: 2.2 px dots every 16 px, as a CSS gradient;
  - a definition list, a list item and a `pre` are cards: `--card`, radius
    14, a hard `--shadow` offset;
  - a submit button and the passkey button are primary: pink, dark text,
    radius 12, at least 52 px tall, full width at phone width, the hard
    shadow;
  - other buttons (`Notify this device`, `Refuse`, and the remove buttons
    in a list) are secondary: `--chip`, light text, radius 10, 44 px;
  - the passkey status, which `passkey.js` fills with its errors, is pink;
  - links are pink and underlined; inputs are dark fields with a 2 px
    outline in `--line`, the accent when focused.
- **Phone width first:** one column, 24 px padding, at most 34 rem wide.
- **System fonts only:** `-apple-system, system-ui` for SF Pro and
  `ui-monospace, SFMono-Regular, Menlo` for SF Mono. No font is fetched.
- **Contrast:** every text pair is at least 4.5:1 on both schemes, checked
  by `look`'s tests.

### 3. `look::page` and the avatar

- **`look::page(title, head, body)`** is the shell: the doctype, viewport,
  `color-scheme` and `theme-color`, both stylesheets inline, and the avatar
  above `body`, in a `<main>`.
- **The avatar is inline**, a 144 px WebP of the app's `Avatar.png` (about
  4 KB) as a `data:` URI, shown at 72 px in a round frame with a 2 px
  `--ink` ring. The CSPs then need only `img-src data:`: no route, no
  request, and no image from anywhere else. A `data:` image runs nothing
  and fetches nothing.

### 4. Behaviour stays the same

- **A pinned skeleton.** `look`'s tests render every page from fixed inputs
  and keep each `<form>`, `<input>`, `<button>`, `<script>`, `<a>` and
  `<label>` tag, in order, with its attributes. They must equal
  `look/skeleton.txt`, pinned before the wire-up, so no form, hidden grant
  field, proof field, purpose or script changes with the look.
- **No script, header or route changes**, except the CSPs' `img-src data:`.

### Execution

1. This ADR, `look/` (tokens, stylesheet, avatar, `look::page`, the pinned
   skeleton and the contrast test) and `design.rs`. No guarded file.
2. In `approve.rs` (guarded): `shell_with` calls `look::page`; `CSP`,
   `PROOF_CSP` and `INBOX_CSP` gain `img-src data:`; `MANIFEST`'s colours
   become `--background`. The skeleton passes unchanged.
3. Before and after screenshots at 390 px, in both schemes, to the owner.
4. The site templates take the same tokens and rules, in their own change
   under this ADR's amendment.

### Verification

- `look`'s tests: the skeleton, contrast on both schemes, one avatar, and
  no `url(` or `@import` in either stylesheet.
- `design.rs`: tokens equal `Theme.swift`'s.
- The pages at 390 px, dark and light, match the app's "Link this iPhone".

## Amendments

### 2026-11-16: Step 4: the site templates take the look

The owner chose the site templates too (owner item #15). Each carries the
tokens, not a second palette:

- **A byte-equal copy.** `look/tokens.css` is copied to each template's
  `static/assets/css/tokens.css`, since a site is built from its template's
  files alone. `src/pigeon-site-templates/tests/suite/look.rs` fails when a
  copy differs.
- **The blog gets a `pigeon` colour theme,** in `assets/css/pigeon.css`,
  linked after `main.css` and `tokens.css` and scoped to
  `.color-theme-pigeon`. It maps the blog's colour variables to the tokens
  and adds the dot grid, the round avatar with its `--ink` ring, cards with
  the hard shadow for asides, quotes and code, and the button radius and
  shadow. It becomes `theme.toml`'s default `look.color`, and `gameboy`
  stays selectable. A site pins its template's version, so a live site
  changes only when its own config moves to the new version.
- **The architecture page** takes its neutrals, text and links from the
  tokens (`--link` is `--accent`), the dot grid, the display type for its
  heading, and the hard shadow on its cards. The diagram's category colours
  stay, as they encode kinds, not the brand.
- **Fonts are unchanged:** each template's `fonts` feature and family stay
  the site's choice, and nothing new is fetched.

### 2026-11-16: The avatar at the app's 40 pt header size

The owner asked for the logo the size it is in the app. §3's 72 px is
`LinkView`'s first-run size; the app's screens show the avatar at 40 pt in
their header (`RootView.swift`), with a 2 pt `--card` ring and a 2 pt hard
`--shadow`. The pages take that: a 40 px `<img>` with the same ring and
shadow. The blog's header keeps its own avatar size.

### 2026-11-16: Reviewed against the always-listed approvals' link reuse

Review !824 asked whether this ADR agrees with
[`2026-11-15-security-owner-approvals-always-listed.md`](2026-11-15-security-owner-approvals-always-listed.md)'s
2026-11-16 amendment (!801), which governs `worker/approve.rs` and the grant
paths this ADR's pages post through. They agree, and neither changes the
other:

- !801 decides which `?grant=` token a page is offered: `ChangeRun` hands
  out an unused page grant again for the same head, operation and tier.
- This ADR changes only how a page looks: `look::page` wraps the same body,
  and the CSPs gain `img-src data:`. The pinned skeleton keeps every form,
  its hidden `grant` and `operation` fields, the proof fields and
  `passkey.js` as they were, so the token !801 hands out is posted and
  verified unchanged.
- Neither touches the other's files: this ADR's changes leave
  `worker/approve.rs`, `worker/listed.rs` and `approval/**` alone.

### 2026-11-16: Reviewed against the faster merge path

Review !831 asked whether this ADR agrees with
[`2026-11-16-development-merge-path-measured-faster.md`](2026-11-16-development-merge-path-measured-faster.md)
(!818). They agree, and neither changes the other. That ADR decides how a
merge takes its turn: the release bot's merge turn, overlapping head jobs,
and GitHub polling. It governs `worker/main_run.rs`, `worker/merges.rs`,
`worker/github.rs`, `cobot/ci.rs` and `release/**`, none of which this ADR
touches. This ADR changes only how the owner pages and site templates look,
and its changes merge through that path like any other.

### 2026-11-16: Reviewed against the owner's answers on merges

Review !838 asked whether this ADR agrees with !820's 2026-11-16 amendments
to
[`2026-11-16-development-merge-path-measured-faster.md`](2026-11-16-development-merge-path-measured-faster.md)
and
[`2026-11-09-security-approvals-once-passkeys.md`](2026-11-09-security-approvals-once-passkeys.md).
They agree, and neither changes the other:

- !820 decides that one approval covers its head's merge, and that checks
  carry across a clean restack.
- This ADR changes only how the approval page looks. The page offers the same
  operation and posts the same `grant`, `operation`, proof and `review`
  fields, as its pinned skeleton checks. `MERGES_NOTE` and `COVERS` keep
  their wording. So an approval made on the restyled page carries its merge
  exactly as !820 sets out.
