# CLAUDE.md

## Comments are terse; ADRs hold the reasoning

A comment in code or config says what something is, in a line or two. When the
why matters, cite the ADR that holds it. Rationale, history and alternatives
belong in the ADR, never in the file.

## Pointers

- Decisions are ADRs in `.claude/decisions/`, written before the code. Their
  naming, template and amendments are in
  [`.claude/decisions/README.md`](.claude/decisions/README.md).
- The development lifecycle (ADR PR, then autonomous implementation PR, squash
  merges, PR conventions) is
  [`2026-11-02-development-adr-driven-lifecycle.md`](.claude/decisions/2026-11-02-development-adr-driven-lifecycle.md).
- Every PR adds one line to [`CHANGELOG.md`](CHANGELOG.md), under today's UTC
  date, linking itself: `- <change> ([#N](https://github.com/noisypigeon/pigeon/pull/N))`.
- The repo is public: never push secrets, credentials, key IDs or private
  bucket names, in commits, branch names, PR text or CI logs.

## Commands

- `bash .github/scripts/check-decisions.sh`
- `bash .github/scripts/check-changelog.sh`
- `PR_TITLE='…' PR_BODY='…' PR_NUMBER=N bash .github/scripts/check-pr.sh`
- `bash .github/scripts/test-checks.sh`
