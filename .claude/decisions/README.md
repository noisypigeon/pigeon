# Decisions

Every decision about how pigeon is built, released, run or secured is an
Architecture Decision Record (ADR) here, written before the code it decides.
The rules are set by
[`2026-11-02-development-adr-driven-lifecycle.md`](2026-11-02-development-adr-driven-lifecycle.md)
and checked by `scripts/check-decisions.sh`.

## Naming

```
YYYY-MM-DD-<category>-<brief-description>.md
```

- `YYYY-MM-DD` is the date the ADR is written.
- `<category>` is one of `development`, `release`, `runtime`, `security` or
  `other`. Adding a category takes an ADR.
- `<brief-description>` is 3 to 5 lowercase words of `[a-z0-9]`, joined by
  dashes.

## Template

Copy this, replacing `<Title>` with the ADR's title:

```
## <Title>

* Date: YYYY-MM-DD
* Model: <the Claude model that wrote it, by name>
* Author: Willow Graysen (@noisypigeon).
* Category: <category>

## Context

The context of what is being built, related ADRs, etc.

## Implementation plan.

The detailed implementation plan.
```

- `Date` and `Category` must match the filename.
- The only H2s are the title, `## Context` and `## Implementation plan.`, in
  that order. Use `###` inside them.
- A merged ADR is not edited to change what it decided. A later ADR
  supersedes it and cites it by filename.
