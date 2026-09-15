CREATE TABLE synced_issues (
  issue_id TEXT PRIMARY KEY,
  repository TEXT NOT NULL,
  number BIGINT NOT NULL,
  title TEXT NOT NULL,
  state TEXT NOT NULL,
  url TEXT NOT NULL
);
