-- Each attempt of a macOS job on a provider, and where a release was archived; see
-- .claude/decisions/2026-11-12-development-macos-builds-provider-failover.md §2.
-- Expand only: a new table and new columns an older Worker never reads.

CREATE TABLE IF NOT EXISTS macos_jobs (
  kind        TEXT NOT NULL CHECK (kind IN ('check', 'archive')),
  number      INTEGER NOT NULL,
  sha         TEXT NOT NULL,
  attempt     INTEGER NOT NULL,
  provider    TEXT NOT NULL,
  run         TEXT,
  url         TEXT,
  outcome     TEXT NOT NULL,
  queued_at   INTEGER NOT NULL,
  started_at  INTEGER,
  finished_at INTEGER,
  minutes     INTEGER,
  PRIMARY KEY (kind, number, sha, attempt)
);
CREATE INDEX IF NOT EXISTS macos_jobs_queued ON macos_jobs (queued_at);
CREATE INDEX IF NOT EXISTS macos_jobs_finished ON macos_jobs (finished_at);

ALTER TABLE ios_releases ADD COLUMN archive_provider TEXT;
ALTER TABLE ios_releases ADD COLUMN archive_build TEXT;
