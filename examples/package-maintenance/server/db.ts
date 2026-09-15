import pg from "pg";
// Preserve database integer/decimal precision and timestamp text at the API boundary.
pg.types.setTypeParser(20, (value) => value);
pg.types.setTypeParser(1700, (value) => value);
pg.types.setTypeParser(1082, (value) => value);
pg.types.setTypeParser(1114, (value) => value);
pg.types.setTypeParser(1184, (value) => value);
export const semantic = new pg.Pool({
  connectionString: process.env.SEMANTIC_DATABASE_URL,
  max: 6,
  connectionTimeoutMillis: 5000,
  query_timeout: 35000,
});
semantic.on("error", () => console.error("Semantic DB idle connection failed"));
export const httpUrl = process.env.SEMANTIC_HTTP_URL ?? "http://127.0.0.1:5545";
export const queries = {
  packages: "SELECT * FROM package_overview ORDER BY package_name",
  package: "SELECT * FROM package_overview WHERE package_name = $1",
  issues:
    "SELECT i.* FROM issue_workspace i JOIN packages p ON lower(i.repository) = lower(p.repository) WHERE p.name = $1 ORDER BY i.priority, i.number",
  trend:
    "SELECT download_date, downloads FROM downloads_daily WHERE package_name = $1 ORDER BY download_date",
  members: "SELECT id, name FROM members ORDER BY name",
};
