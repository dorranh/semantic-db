SELECT w.basin, SUM(s.depth_m) AS drilled_m, COUNT(*) AS samples
FROM samples s
JOIN wells w ON s.well_id = w.well_id
GROUP BY w.basin
ORDER BY w.basin;
