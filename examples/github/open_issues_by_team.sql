-- The mapping is authored locally. A missing mapping remains visible.
SELECT COALESCE(t.team, 'Unmapped') AS team,
       i.repository,
       COUNT(*) AS open_issues
FROM issues i
LEFT JOIN repository_teams t ON lower(i.repository) = lower(t.repository)
WHERE i.state = 'OPEN'
GROUP BY COALESCE(t.team, 'Unmapped'), i.repository
ORDER BY team, i.repository;
