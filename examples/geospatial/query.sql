SELECT well_id, well_name, total_depth_m
FROM wells
WHERE basin = 'North Basin'
  AND status = 'active'
  AND total_depth_m >= 2500
ORDER BY well_id;
