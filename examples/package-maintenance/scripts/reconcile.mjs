import { mkdir, open, readFile, unlink } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import pg from "pg";
process.chdir(fileURLToPath(new URL("..", import.meta.url)));
try { process.loadEnvFile(".env"); } catch (error) { if (error.code !== "ENOENT") throw error; }
await mkdir(".run", { recursive: true });
const path = ".run/reconcile.lock";
let lock;
try { lock = await open(path, "wx", 0o600); }
catch (error) {
  if (error.code === "EEXIST") throw new Error("A reconciliation run owns .run/reconcile.lock. If its recorded process has stopped, remove the stale lock before running again.");
  throw error;
}
const client = new pg.Client({ connectionString: process.env.SEMANTIC_DATABASE_URL });
let dispatched = false;
let committed = false;
try {
  await lock.writeFile(`${process.pid}\n`);
  await client.connect();
  const sql = await readFile("writes/reconcile_issues.sql", "utf8");
  dispatched = true;
  const result = await client.query(sql);
  committed = true;
  console.log(JSON.stringify({ outcome: "Committed", command: result.command, affected_rows: result.rowCount }));
  console.table((await client.query("SELECT * FROM synced_issues ORDER BY issue_id")).rows);
} catch (error) {
  const unknown = dispatched && (error.code === "08007" || !/^[0-9A-Z]{5}$/.test(error.code ?? ""));
  console.error(committed
    ? "Committed; reading saved rows failed. The acknowledged write remains committed."
    : unknown
      ? "OutcomeUnknown: inspect saved rows before retrying."
      : `Reconciliation failed (${error.code ?? error.name}).`);
  process.exitCode = 1;
} finally {
  try { await client.end(); } finally {
    try { await lock.close(); } finally { await unlink(path); }
  }
}
