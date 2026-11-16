-- Each insights source's newest collection: when, the newest hour filled, and what failed; see
-- .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md, 2026-11-15 amendment.
-- Expand only: a new table an older Worker never reads.

CREATE TABLE IF NOT EXISTS insights_sources (
  source TEXT PRIMARY KEY,
  at     INTEGER NOT NULL,
  hour   INTEGER,
  failed TEXT
);
