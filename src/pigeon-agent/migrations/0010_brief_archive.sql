-- Each finished brief, whole, moved off the Instance; see .claude/decisions/2026-11-13-development-model-tiered-agent-briefs.md, its 2026-11-20 archive amendment. Expand only.
CREATE TABLE IF NOT EXISTS brief_archive (number INTEGER PRIMARY KEY, body TEXT NOT NULL, archived_at INTEGER NOT NULL);
