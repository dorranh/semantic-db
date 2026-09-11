-- One issue can contribute to several labels. Do not sum these counts to
-- calculate total issues. DISTINCT protects issue counts across the join.
SELECT COALESCE(t.team, 'Unmapped') AS team,
       l.label,
       COUNT(DISTINCT i.issue_id) AS open_issues
FROM issues i
JOIN issue_labels l ON i.issue_id = l.issue_id
LEFT JOIN repository_teams t ON lower(i.repository) = lower(t.repository)
WHERE i.state = 'OPEN'
GROUP BY COALESCE(t.team, 'Unmapped'), l.label
ORDER BY team, l.label;
