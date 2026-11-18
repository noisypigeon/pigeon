-- The lease naming the acting color, and the fence an act's batch starts with; see .claude/decisions/2026-11-18-runtime-independent-blue-green-workers.md §A.3. Expand only.
CREATE TABLE IF NOT EXISTS lease (id INTEGER PRIMARY KEY CHECK (id = 1), color TEXT NOT NULL, epoch INTEGER NOT NULL CHECK (epoch > 0), state TEXT NOT NULL CHECK (state IN ('serving', 'draining', 'promoting')));
CREATE TABLE IF NOT EXISTS fence (id INTEGER PRIMARY KEY CHECK (id = 1), ok INTEGER NOT NULL);
INSERT OR IGNORE INTO lease (id, color, epoch, state) VALUES (1, 'pigeon-control', 1, 'serving');
