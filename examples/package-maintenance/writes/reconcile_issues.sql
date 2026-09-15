REQUIRE IDEMPOTENT
MERGE INTO synced_issues AS target
USING (SELECT issue_id, repository, number, title, state, url FROM issues) AS source
ON target.issue_id = source.issue_id
WHEN MATCHED THEN UPDATE SET repository = source.repository,
  number = source.number, title = source.title, state = source.state, url = source.url
WHEN NOT MATCHED THEN INSERT (issue_id, repository, number, title, state, url)
  VALUES (source.issue_id, source.repository, source.number, source.title, source.state, source.url);
