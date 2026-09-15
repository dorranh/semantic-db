import { test } from "node:test";
import assert from "node:assert/strict";
import pg from "pg";
import { readFile } from "node:fs/promises";
const base = "http://127.0.0.1:3001/api";
async function json(path: string, options: RequestInit = {}) {
  const r = await fetch(base + path, {
    ...options,
    headers: { "Content-Type": "application/json" },
  });
  assert.equal(r.status, 200, await r.clone().text());
  return r.json();
}
async function control(body: object) {
  const r = await fetch("http://127.0.0.1:4010/control", {
    method: "POST",
    body: JSON.stringify(body),
  });
  assert.ok(r.ok);
}
test("three-source reads, Semantic DB writes, reconciliation, and pg interoperability", async () => {
  await control({ reset: true });
  const initial = await json("/packages/requests");
  const snapshot = new pg.Client({
    connectionString:
      "postgresql://maintenance:maintenance@127.0.0.1:5543/maintenance",
  });
  await snapshot.connect();
  const originalTriage = (
    await snapshot.query("SELECT * FROM issue_triage WHERE issue_id=$1", [
      "I_requests_2",
    ])
  ).rows[0];
  const originalSynced = (await snapshot.query("SELECT * FROM synced_issues")).rows;
  await snapshot.end();
  const expected = String(14 * 1200000 + 37000 * ((13 * 14) / 2) + 17);
  assert.equal(initial.package.downloads, expected);
  assert.equal(initial.package.open_issues, "2");
  assert.equal(initial.trend.length, 14);
  assert.equal(initial.issues.length, 3);
  assert.ok(initial.issues.some((i: any) => i.state === "CLOSED"));
  assert.ok(initial.issues.every((i: any) => i.repository === "psf/requests"));
  const unknown = await fetch(base + "/packages/not-a-package");
  assert.equal(unknown.status, 404);
  const chHeaders = {
    Authorization:
      "Basic " + Buffer.from("maintenance:maintenance").toString("base64"),
  };
  const ch = (sql: string) =>
    fetch("http://127.0.0.1:8124", {
      method: "POST",
      headers: chHeaders,
      body: sql,
    });
  assert.ok((await ch("SYSTEM FLUSH LOGS")).ok);
  const logged = await ch(
    "SELECT query FROM system.query_log WHERE type='QueryFinish' AND query LIKE '%pypi_downloads_per_day%' AND query NOT LIKE '%system.query_log%' AND query LIKE '%FORMAT ArrowStream%' ORDER BY event_time_microseconds DESC LIMIT 1 FORMAT JSONEachRow",
  );
  assert.ok(logged.ok);
  const remote = JSON.parse((await logged.text()).trim()).query;
  assert.match(remote, /WHERE/);
  assert.match(remote, /2026-09-01/);
  assert.match(remote, /2026-09-15/);
  assert.match(remote, /project.*IN/);
  const shared = await json("/packages/requests-toolbelt");
  assert.equal(shared.package.open_issues, "2");
  const missing = await json("/packages/no-downloads-demo");
  assert.equal(missing.package.downloads, null);
  try {
    await json("/packages/requests", {
      method: "PATCH",
      body: JSON.stringify({
        team: "Integration team",
        notes: "Stored in Postgres",
      }),
    });
    assert.equal(
      (await json("/packages/requests")).package.team,
      "Integration team",
    );
    await json("/issues/I_requests_2", {
      method: "PATCH",
      body: JSON.stringify({
        assignee_id: "sam",
        priority: 1,
        notes: "A human-owned note",
      }),
    });
    const updated = await json("/packages/requests");
    assert.equal(
      updated.issues.find((i: any) => i.issue_id === "I_requests_2")
        .assignee_name,
      "Sam Rivera",
    );
    await control({ closed_issue: "I_requests_2" });
    assert.equal((await json("/packages/requests")).package.open_issues, "1");
    await control({ reset: true });
    await control({ fail_after_first_page: true });
    const failed = await fetch(base + "/packages/requests");
    assert.equal(failed.status, 502);
    await control({ reset: true });
    assert.equal(
      (await json("/packages/requests")).package.downloads,
      expected,
    );
    const client = new pg.Client({
      connectionString: "postgresql://local@127.0.0.1:5544/semantic",
    });
    await client.connect();
    try {
      const reconcile = await readFile(new URL("../../writes/reconcile_issues.sql", import.meta.url), "utf8");
      await client.query(reconcile);
      const stored = (await client.query("SELECT * FROM synced_issues ORDER BY issue_id")).rows;
      assert.ok(stored.length > 0);
      assert.equal((await client.query(reconcile)).rowCount, 0);
      assert.deepEqual((await client.query("SELECT * FROM synced_issues ORDER BY issue_id")).rows, stored);
      await control({ closed_issue: "I_requests_2" });
      await client.query(reconcile);
      const changed = (await client.query("SELECT * FROM synced_issues ORDER BY issue_id")).rows;
      assert.equal(changed.find((r: any) => r.issue_id === "I_requests_2").state, "CLOSED");
      assert.equal((await client.query("SELECT notes FROM issue_triage WHERE issue_id = $1", ["I_requests_2"])).rows[0].notes, "A human-owned note");
      await control({ fail_after_first_page: true });
      await assert.rejects(client.query(reconcile));
      assert.deepEqual((await client.query("SELECT * FROM synced_issues ORDER BY issue_id")).rows, changed);
      await control({ reset: true });
      const text =
        "SELECT " +
        Array.from(
          { length: 10 },
          (_, i) => `$${i + 1}::bigint AS n${i + 1}`,
        ).join(",");
      const result = await client.query({
        name: "ten-params",
        text,
        values: Array.from({ length: 10 }, (_, i) => String(i + 1)),
      });
      assert.equal(result.rows[0].n10, "10");
      const again = await client.query({
        name: "ten-params",
        text,
        values: Array.from({ length: 10 }, (_, i) => String(i + 2)),
      });
      assert.equal(again.rows[0].n10, "11");
      assert.equal(
        (await client.query("SELECT $1::bigint AS exact", ["9007199254740993"]))
          .rows[0].exact,
        "9007199254740993",
      );
      assert.equal(
        (await client.query("SELECT $1::text AS value", [null])).rows[0].value,
        null,
      );
      for (const sql of [
        "BEGIN",
        "COMMIT",
        "DELETE FROM issues",
        "SELECT 1; SELECT 2",
      ]) {
        await assert.rejects(client.query(sql));
        assert.equal((await client.query("SELECT 1 AS ok")).rows[0].ok, "1");
      }
      const all = await Promise.all(
        Array.from({ length: 3 }, async () => {
          const c = new pg.Client({
            connectionString: "postgresql://local@127.0.0.1:5544/semantic",
          });
          await c.connect();
          try {
            return (await c.query("SELECT count(*) AS n FROM packages")).rows[0]
              .n;
          } finally {
            await c.end();
          }
        }),
      );
      assert.deepEqual(all, ["5", "5", "5"]);
    } finally {
      await client.end();
    }
  } finally {
    await control({ reset: true });
    await json("/packages/requests", {
      method: "PATCH",
      body: JSON.stringify({
        team: initial.package.team,
        notes: initial.package.notes,
      }),
    });
    const direct = new pg.Client({
      connectionString:
        "postgresql://maintenance:maintenance@127.0.0.1:5543/maintenance",
    });
    await direct.connect();
    try {
      await direct.query("DELETE FROM synced_issues");
      for (const row of originalSynced) await direct.query(
        "INSERT INTO synced_issues(issue_id,repository,number,title,state,url) VALUES($1,$2,$3,$4,$5,$6)",
        [row.issue_id,row.repository,row.number,row.title,row.state,row.url],
      );
      if (originalTriage) {
        await direct.query(
          "UPDATE issue_triage SET assignee_id=$2, priority=$3, notes=$4 WHERE issue_id=$1",
          [
            originalTriage.issue_id,
            originalTriage.assignee_id,
            originalTriage.priority,
            originalTriage.notes,
          ],
        );
      } else {
        await direct.query("DELETE FROM issue_triage WHERE issue_id=$1", [
          "I_requests_2",
        ]);
      }
    } finally {
      await direct.end();
    }
  }
});
