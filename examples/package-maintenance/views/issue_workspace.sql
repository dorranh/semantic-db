SELECT i.issue_id, i.repository, i.number, i.title, i.state, i.url,
       t.assignee_id, m.name AS assignee_name,
       CAST(COALESCE(t.priority, 2) AS INT) AS priority, COALESCE(t.notes, '') AS triage_notes
FROM issues i
LEFT JOIN issue_triage t ON i.issue_id = t.issue_id
LEFT JOIN members m ON t.assignee_id = m.id
