-- The journal of each object's entries, in shadow; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md §A.5. Expand only.
CREATE TABLE IF NOT EXISTS journal (object TEXT NOT NULL, seq INTEGER NOT NULL, epoch INTEGER NOT NULL, at INTEGER NOT NULL, kind TEXT NOT NULL, body TEXT NOT NULL, PRIMARY KEY (object, seq));
