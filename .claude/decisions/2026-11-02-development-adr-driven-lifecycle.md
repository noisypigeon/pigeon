## ADR-driven development lifecycle

* Date: 2026-11-02
* Model: Claude Opus 5.5
* Author: Willow Graysen (@noisypigeon).
* Category: development

## Context

Since mid-September the work behind pigeon has lived in two repositories:

- `noisypigeon/runway`, the infrastructure and the reusable library under `tools/`;
- `noisypigeon/pigeon-cli`, the Rust CLI.

Both were the place to get my bearings on the goal and iterate on content, and both carry that history: renamed tracks, reversed decisions, records that amend records, and a decision corpus split across Discussions, files and archives. `noisypigeon/pigeon` starts fresh. It consumes nothing from runway or pigeon-cli for now. Anything it later takes from them (runway's `tools/` library, pigeon-cli's code, either repo's conventions) is adopted on purpose, by its own record, rather than inherited.

The one thing kept from that experience is the habit that worked best: a precedent is set by writing it down first. In this repository a precedent is an Architecture Decision Record (ADR), committed under `.claude/decisions/`. Every decision about how pigeon is built, released, run or secured is an ADR before it is code. Each ADR carries its own full implementation plan, so the record is detailed enough to execute without further confirmation.

This is the first ADR, so it cites none. It decides the development lifecycle itself, and the mechanisms that make the lifecycle checkable rather than merely documented.

**Bootstrap exception.** The repository had no commits and no `main` branch when this ADR was written, so there was nothing for a pull request to merge into. This ADR alone is therefore committed directly to `main`. Every change after it, this ADR's own implementation included, follows the lifecycle below.

Deliberately out of scope here, each left to a later ADR:

- `main` branch rules (require a PR, require the `lifecycle` checks, allow squash merges only). They are applied by hand in the repository's GitHub settings, because the session that implements this ADR cannot set repository rules.
- Release mechanics, versioning and changelogs.
- Runtime and language choices, and any application code.
- Adopting runway's `tools/` library (reusable workflows, plugins, pinned scripts).
- Linting that a merged ADR is never edited. It stays a convention until a later ADR enforces it.

## Implementation plan.

### 1. Where ADRs live and how they are named

ADRs are Markdown files in `.claude/decisions/`, named:

```
YYYY-MM-DD-<category>-<brief-description>.md
```

- `YYYY-MM-DD` is the date the ADR is written.
- `<category>` is one of `development`, `release`, `runtime`, `security` or `other`. Adding a category takes a new ADR that also changes the lint's list.
- `<brief-description>` is 3 to 5 lowercase words of `[a-z0-9]`, separated by single dashes.

The filename regex the lint applies is:

```
^[0-9]{4}-[0-9]{2}-[0-9]{2}-(development|release|runtime|security|other)-[a-z0-9]+(-[a-z0-9]+){2,4}\.md$
```

`.claude/decisions/README.md` is the only other file allowed in the directory, and the lint ignores it.

### 2. The template

Every ADR has exactly this shape:

```
## <Title>

* Date: YYYY-MM-DD
* Model: <the Claude model that wrote it, by name, e.g. Claude Opus 5.5>
* Author: Willow Graysen (@noisypigeon).
* Category: <category>

## Context

The context of what is being built, related ADRs, etc.

## Implementation plan.

The detailed implementation plan.
```

Rules the lint enforces:

- The first line is an H2 holding the ADR's title. It is not the literal word `Title`.
- The four bullets follow, in this order, each with a non-empty value:
  - `Date` equals the filename's date;
  - `Category` equals the filename's category;
  - `Author` is exactly `Willow Graysen (@noisypigeon).`
- The only H2 headings are the title, `## Context` and `## Implementation plan.`, in that order. Structure inside a section uses `###` and below.
- No H1 and no frontmatter.

### 3. The lifecycle

1. **Write the ADR**, including the full implementation plan to execute it.
2. **Push it to a feature branch.** The session's designated branch is used where one exists.
3. **Open a PR and merge it.** An ADR-only PR's title is `docs(adr): <summary>`.
4. **Carry on autonomously.** Once the ADR is on `main`, the same session implements it without pausing for confirmation. It restarts its feature branch from the new `main`, since a merged PR's branch is never reused for new work.
5. **Land the implementation as its own PR.** It cites the ADR it implements and is merged once its checks are green.

Further rules:

- Every PR is squash-merged. Its title becomes the commit on `main`.
- A merged ADR is not edited to change what it decided. A later ADR supersedes it and cites it by filename. Fixing a typo, or a broken link, is not a change of decision. This is a convention for now (see Context).
- The repository is public. Nothing pushed may contain secrets, credentials, key IDs or private bucket names: not commits, branch names, PR text or CI logs.

### 4. PR titles and bodies

- **Titles** follow Conventional Commits:

  ```
  ^(feat|fix|docs|chore|ci|refactor|test|build|perf|revert)(\([a-z0-9][a-z0-9-]*\))?!?: .+
  ```

- **Bodies** must cite at least one ADR by its path, `.claude/decisions/<file>.md`, and every cited path must exist in the PR's head. The PR template puts it under `## Decision`. An ADR's own PR cites itself.

### 5. Files the implementation adds

**`CLAUDE.md`.** The short entry point for any session in this repository. It states:

- that decisions are ADRs in `.claude/decisions/`, with a pointer to its README;
- the lifecycle in §3 and the PR conventions in §4;
- the public-repository rule;
- the local commands in §6.

It does not restate the template; it links to it.

**`.claude/decisions/README.md`.** The naming rules, the category list and a copyable template, matching §1 and §2.

**`.github/pull_request_template.md`.** Three sections:

- `## Decision`: the ADR path(s) the PR adds or implements;
- `## Summary`;
- `## Verification`: what was run, and its result.

**`scripts/check-decisions.sh`.** Bash and coreutils/grep only, no other dependencies.

- It takes an optional directory argument, defaulting to `.claude/decisions` resolved from the script's own location.
- It checks every file in that directory except `README.md` against §1 and §2.
- Each problem is printed as `file:line: reason`. It checks every file before exiting, and exits non-zero if any check failed.
- It fails if the directory holds no ADRs at all.

**`scripts/check-pr.sh`.** Same constraints.

- It reads the title and body from the `PR_TITLE` and `PR_BODY` environment variables. The workflow sets them from the event payload through `env:`, never by interpolating `${{ }}` into the script, so a title or body cannot inject shell.
- It checks the title against §4 and extracts every `.claude/decisions/*.md` path from the body. It fails if there are none, or if any cited path does not exist in the checkout.
- It takes the repository root as an optional argument, defaulting to the script's parent directory.

**`scripts/test-checks.sh`.** Builds fixtures in a temporary directory and asserts each script's exit status. It covers:

- a valid ADR, which passes;
- an unknown category;
- a 2-word slug and a 6-word slug;
- a `Date` bullet that disagrees with the filename;
- a `Category` bullet that disagrees with the filename;
- a missing or altered `Author` line;
- an empty `Model`;
- a missing `## Context`;
- the two sections in the wrong order;
- an extra H2;
- a literal `## Title`;
- an empty directory;
- a valid PR title and body, which passes;
- a non-conventional title;
- a body citing no ADR;
- a body citing an ADR that does not exist.

**`.github/workflows/lifecycle.yml`.**

- Triggers: `pull_request` (`opened`, `edited`, `synchronize`, `reopened`) and `push` to `main`.
- `permissions: contents: read`. Every third-party action is pinned to a full commit SHA, with its version in a trailing comment.
- Job `decisions`, on every trigger: checkout, then `scripts/check-decisions.sh`, then `scripts/test-checks.sh`.
- Job `pr`, on `pull_request` only: checkout of the PR head, then `scripts/check-pr.sh`, with `PR_TITLE`/`PR_BODY` passed through `env:`. Because `edited` is a trigger, fixing a title or body re-runs it.

### 6. Local commands

- `bash scripts/check-decisions.sh`
- `PR_TITLE='…' PR_BODY='…' bash scripts/check-pr.sh`
- `bash scripts/test-checks.sh`

### 7. Executing this plan

1. Commit this ADR directly to `main`, which is the bootstrap exception, and push it.
2. Restart the session's feature branch from `origin/main`.
3. Add the files in §5, then run the commands in §6 until all three are clean.
4. Commit with the title `ci: enforce the ADR-driven lifecycle` and push the feature branch.
5. Open a PR from the template, citing this ADR under `## Decision`.
6. Wait for both `lifecycle` jobs to go green, fixing anything red on the same branch.
7. Squash-merge it.
8. Confirm the `push` run of `lifecycle` on `main` is green.
9. Report the branch rules from Context's out-of-scope list, so they can be applied by hand.

### 8. Verification

- All three local commands pass against the real repository.
- `scripts/test-checks.sh` proves every rule in §1, §2 and §4 rejects a bad input. A check that passes everything is a failure of this ADR.
- The implementation PR's `decisions` and `pr` jobs are green on GitHub, and the post-merge `push` run on `main` is green.
