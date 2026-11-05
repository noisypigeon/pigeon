-- D1 schema v0; see .claude/decisions/2026-11-05-runtime-control-plane-worker.md §4.

CREATE TABLE instances (
  id      TEXT PRIMARY KEY,
  name    TEXT NOT NULL UNIQUE,
  created INTEGER NOT NULL,
  status  TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'suspended'))
);

CREATE TABLE desired_state (
  instance TEXT NOT NULL REFERENCES instances (id),
  unit     TEXT NOT NULL,
  version  INTEGER NOT NULL CHECK (version > 0),
  config   TEXT NOT NULL CHECK (json_valid(config)),
  author   TEXT NOT NULL,
  created  INTEGER NOT NULL,
  PRIMARY KEY (instance, unit, version)
);

CREATE TABLE plans (
  id        TEXT PRIMARY KEY,
  instance  TEXT NOT NULL REFERENCES instances (id),
  version   INTEGER NOT NULL,
  digest    TEXT NOT NULL,
  summary   TEXT NOT NULL CHECK (json_valid(summary)),
  requester TEXT NOT NULL,
  created   INTEGER NOT NULL,
  expires   INTEGER NOT NULL CHECK (expires > created)
);

CREATE TABLE approvals (
  plan     TEXT NOT NULL REFERENCES plans (id),
  approver TEXT NOT NULL,
  created  INTEGER NOT NULL,
  PRIMARY KEY (plan, approver)
);

CREATE TABLE runs (
  id       TEXT PRIMARY KEY,
  instance TEXT NOT NULL REFERENCES instances (id),
  kind     TEXT NOT NULL CHECK (kind IN ('plan', 'apply')),
  plan     TEXT REFERENCES plans (id),
  status   TEXT NOT NULL DEFAULT 'queued'
           CHECK (status IN ('queued', 'running', 'succeeded', 'failed')),
  attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
  started  INTEGER,
  finished INTEGER,
  CHECK ((kind = 'apply') = (plan IS NOT NULL))
);

CREATE TABLE grants (
  instance   TEXT NOT NULL REFERENCES instances (id),
  provider   TEXT NOT NULL,
  ciphertext BLOB NOT NULL,
  created    INTEGER NOT NULL,
  revoked    INTEGER,
  PRIMARY KEY (instance, provider)
);

CREATE TABLE audit (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  at        INTEGER NOT NULL,
  actor     TEXT NOT NULL,
  operation TEXT NOT NULL,
  instance  TEXT REFERENCES instances (id),
  target    TEXT,
  outcome   TEXT NOT NULL
);

CREATE INDEX runs_by_instance ON runs (instance, status);
CREATE INDEX plans_by_instance ON plans (instance, expires);
CREATE INDEX audit_by_instance ON audit (instance, at);
