SELECT project AS package_name, date AS download_date, SUM(count) AS downloads
FROM pypi_downloads
WHERE date >= DATE '2026-09-01' AND date < DATE '2026-09-15'
  AND project IN ('requests', 'requests-toolbelt', 'urllib3', 'httpx', 'no-downloads-demo')
GROUP BY project, date
