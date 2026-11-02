# CLAUDE.md

`noisypigeon/pigeon` is a fresh start. It consumes nothing from
`noisypigeon/runway` or `noisypigeon/pigeon-cli`; anything taken from them is
adopted on purpose, by an ADR.

## Decisions come first

Every decision is an ADR in `.claude/decisions/`, written before the code.
Naming, categories and the template are in
[`.claude/decisions/README.md`](.claude/decisions/README.md). The lifecycle is
decided by
[`2026-11-02-development-adr-driven-lifecycle.md`](.claude/decisions/2026-11-02-development-adr-driven-lifecycle.md).
Read the ADRs relevant to a change before making it. If a change would
contradict one, write a new ADR that supersedes it rather than drifting.

## Lifecycle

1. Write the ADR, including the full implementation plan to execute it.
2. Push it to a feature branch, open a PR (`docs(adr): <summary>`) and merge it.
3. Carry on into the implementation autonomously, with no pause for
   confirmation. Restart the feature branch from the new `main`; never reuse a
   merged PR's branch history.
4. Land the implementation as its own PR, citing the ADR, and merge it once the
   `lifecycle` checks are green.

- Every PR is squash-merged; its title becomes the commit on `main`.
- PR titles are Conventional Commits:
  `type(scope)?!?: summary`, type one of
  `feat|fix|docs|chore|ci|refactor|test|build|perf|revert`.
- PR bodies follow `.github/pull_request_template.md` and cite the ADR by path
  (`.claude/decisions/<file>.md`) under `## Decision`.

## Public repository

Everything pushed is public and permanent: commits, branch names, PR text and
CI logs. Never push secrets, credentials, key IDs or private bucket names.

## Commands

- `bash scripts/check-decisions.sh` lints every ADR.
- `PR_TITLE='…' PR_BODY='…' bash scripts/check-pr.sh` checks a PR's title and
  body.
- `bash scripts/test-checks.sh` proves both checks reject bad input.
