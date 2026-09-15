SELECT p.name AS package_name, p.repository, p.team, p.notes,
       d.downloads, COALESCE(i.open_issues, 0) AS open_issues,
       COALESCE(i.unassigned_issues, 0) AS unassigned_issues
FROM packages p
LEFT JOIN (SELECT package_name, SUM(downloads) AS downloads FROM downloads_daily GROUP BY package_name) d
  ON p.name = d.package_name
LEFT JOIN (
  SELECT repository, COUNT(*) AS open_issues,
         SUM(CASE WHEN assignee_id IS NULL THEN 1 ELSE 0 END) AS unassigned_issues
  FROM issue_workspace WHERE state = 'OPEN' GROUP BY repository
) i ON lower(p.repository) = lower(i.repository)
