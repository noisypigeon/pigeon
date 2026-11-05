-- Changes and their jobs; see .claude/decisions/2026-11-05-development-changes-through-pigeon-mcp.md §1.
-- The `Change` Durable Object owns each change and mirrors it here for listing and audit.

CREATE TABLE changes (
  instance      TEXT NOT NULL REFERENCES instances (id),
  repo          TEXT NOT NULL,
  number        INTEGER NOT NULL CHECK (number > 0),
  slug          TEXT NOT NULL,
  title         TEXT NOT NULL,
  plan          TEXT NOT NULL,
  scope         TEXT NOT NULL CHECK (json_valid(scope)),
  proposer      TEXT NOT NULL,
  state         TEXT NOT NULL CHECK (state IN ('proposed', 'planned', 'pushed', 'bounded',
                  'refused', 'checked', 'failed', 'approved', 'merged', 'closed')),
  head          TEXT CHECK (head IS NULL OR length(head) = 40),
  plan_approver TEXT,
  approver      TEXT,
  approved_sha  TEXT,
  violations    TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(violations)),
  created       INTEGER NOT NULL,
  updated       INTEGER NOT NULL CHECK (updated >= created),
  PRIMARY KEY (instance, repo, number),
  CHECK ((state = 'proposed') = (plan_approver IS NULL)),
  CHECK (state NOT IN ('pushed', 'bounded', 'refused', 'checked', 'failed', 'approved', 'merged')
         OR head IS NOT NULL),
  CHECK ((state IN ('approved', 'merged')) = (approver IS NOT NULL AND approved_sha = head)),
  CHECK (plan_approver IS NULL OR plan_approver <> proposer),
  CHECK (approver IS NULL OR approver <> proposer)
);

CREATE TABLE jobs (
  instance TEXT NOT NULL,
  repo     TEXT NOT NULL,
  change   INTEGER NOT NULL,
  sha      TEXT NOT NULL CHECK (length(sha) = 40),
  name     TEXT NOT NULL,
  status   TEXT NOT NULL CHECK (status IN ('queued', 'running', 'passed', 'failed', 'cancelled')),
  attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  log      TEXT,
  updated  INTEGER NOT NULL,
  PRIMARY KEY (instance, repo, change, sha, name),
  FOREIGN KEY (instance, repo, change) REFERENCES changes (instance, repo, number)
);

CREATE INDEX changes_by_state ON changes (instance, state, updated);
