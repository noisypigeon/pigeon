-- A change's session and the globs its ADR will govern; see
-- .claude/decisions/2026-11-08-development-concurrent-agent-sessions.md §1, §2.
-- Expand only: an older Worker writes neither and reads past both.

ALTER TABLE changes ADD COLUMN session TEXT;
ALTER TABLE changes ADD COLUMN affects TEXT NOT NULL DEFAULT '[]';
