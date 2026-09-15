CREATE TABLE packages (name TEXT PRIMARY KEY, repository TEXT NOT NULL, team TEXT NOT NULL, notes TEXT NOT NULL DEFAULT '');
CREATE TABLE members (id TEXT PRIMARY KEY, name TEXT NOT NULL);
CREATE TABLE issue_triage (
  issue_id TEXT PRIMARY KEY,
  assignee_id TEXT REFERENCES members(id),
  priority INTEGER NOT NULL DEFAULT 2 CHECK (priority BETWEEN 1 AND 3),
  notes TEXT NOT NULL DEFAULT ''
);
