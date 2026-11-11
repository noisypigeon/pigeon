-- Each iOS release to TestFlight; see
-- .claude/decisions/2026-11-11-release-ios-app-buildkite-testflight.md §4, §5.
-- Expand only: a new table an older Worker never reads.

CREATE TABLE IF NOT EXISTS ios_releases (
  build INTEGER PRIMARY KEY,
  sha TEXT NOT NULL,
  buildkite_build INTEGER,
  state TEXT NOT NULL DEFAULT 'queued',
  error TEXT,
  redeemed_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS ios_releases_buildkite ON ios_releases (buildkite_build, sha);
