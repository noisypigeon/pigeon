-- The app's insights: cost and errors per hour, by source and kind; see
-- .claude/decisions/2026-11-12-runtime-ios-app-approvals-insights.md §7.
-- Expand only: a new table an older Worker never reads.

CREATE TABLE IF NOT EXISTS insights (
  hour   INTEGER NOT NULL,
  metric TEXT NOT NULL CHECK (metric IN ('cost', 'errors')),
  source TEXT NOT NULL,
  kind   TEXT NOT NULL,
  value  REAL NOT NULL,
  PRIMARY KEY (hour, metric, source, kind)
);
