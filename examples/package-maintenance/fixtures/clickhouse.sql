CREATE TABLE IF NOT EXISTS pypi.pypi_downloads_per_day (
  date Date, project String, count Int64
) ENGINE = SummingMergeTree ORDER BY (project, date);
INSERT INTO pypi.pypi_downloads_per_day
SELECT toDate('2026-09-01') + number, 'requests', toInt64(1200000 + number * 37000) FROM numbers(14);
INSERT INTO pypi.pypi_downloads_per_day VALUES ('2026-09-01', 'requests', 17);
INSERT INTO pypi.pypi_downloads_per_day
SELECT toDate('2026-09-01') + number, 'urllib3', toInt64(1600000 + number * 21000) FROM numbers(14);
INSERT INTO pypi.pypi_downloads_per_day
SELECT toDate('2026-09-01') + number, 'httpx', toInt64(320000 + number * 19000) FROM numbers(14);
INSERT INTO pypi.pypi_downloads_per_day
SELECT toDate('2026-09-01') + number, 'requests-toolbelt', toInt64(95000 + number * 3000) FROM numbers(14);
-- Outside the explicit demo window; must not appear in totals.
INSERT INTO pypi.pypi_downloads_per_day VALUES ('2020-01-01', 'requests', 999999999);
