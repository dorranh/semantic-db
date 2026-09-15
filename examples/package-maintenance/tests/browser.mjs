// Run after npm start. Uses a dedicated agent-browser session; no API credentials.
import { execFileSync } from "node:child_process";
import { mkdirSync } from "node:fs";
import assert from "node:assert/strict";
const session = `maintenance-e2e-${process.pid}`;
const run = (...args) =>
  execFileSync("agent-browser", ["--session", session, ...args], {
    encoding: "utf8",
    timeout: 35000,
  });
const original = await fetch(
  "http://127.0.0.1:3001/api/packages/requests",
).then((r) => r.json());
mkdirSync(".run", { recursive: true });
try {
  run("open", "http://127.0.0.1:5173");
  run("wait", "--text", "Save package");
  run("snapshot", "-i");
  assert.match(run("get", "text", "body"), /20,167,017/);
  run("find", "label", "Owning team", "fill", "Browser check team");
  run("find", "role", "button", "click", "--name", "Save package");
  run("wait", "--text", "Browser check team");
  run("snapshot", "-i");
  assert.equal(
    (
      await fetch("http://127.0.0.1:3001/api/packages/requests").then((r) =>
        r.json(),
      )
    ).package.team,
    "Browser check team",
  );
  run("find", "role", "button", "click", "--name", "Edit triage for issue 101");
  run("snapshot", "-i");
  run("find", "label", "Triage notes", "fill", "Browser checked this note");
  run("find", "role", "button", "click", "--name", "Save triage");
  run("wait", "--text", "Save package");
  // Wait for successful refresh, not simply the still-visible save form.
  run(
    "wait",
    "--fn",
    "document.querySelector('main[aria-busy]')?.getAttribute('aria-busy') === 'false' && !document.querySelector('.issue-editor')",
  );
  const updated = await fetch(
    "http://127.0.0.1:3001/api/packages/requests",
  ).then((r) => r.json());
  assert.equal(
    updated.issues.find((i) => i.issue_id === "I_requests_1").triage_notes,
    "Browser checked this note",
  );
  run("find", "role", "button", "click", "--name", "Behind the query");
  run("wait", "--text", "package_overview");
  run("snapshot", "-i");
  assert.match(run("get", "text", "body"), /SUM\(downloads\)/);
  run("find", "role", "button", "click", "--name", "Ask the model OPTIONAL");
  const health = await fetch("http://127.0.0.1:3001/api/health").then((r) => r.json());
  run("wait", "--text", health.ask_enabled ? "Your question" : "Ask is not configured.");
  run("find", "role", "button", "click", "--name", "Workspace");
  run("wait", "--text", "Save package");
  run("screenshot", "--full", ".run/desktop.png");
  run("set", "viewport", "390", "844");
  run("screenshot", "--full", ".run/mobile.png");
  assert.equal(
    run(
      "eval",
      "document.documentElement.scrollWidth <= window.innerWidth",
    ).trim(),
    "true",
  );
  console.log(
    "Browser check passed: package save, triage save, model inspection, optional Ask, mobile layout.",
  );
} finally {
  const issue = original.issues.find((i) => i.issue_id === "I_requests_1");
  await fetch("http://127.0.0.1:3001/api/packages/requests", {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      team: original.package.team,
      notes: original.package.notes,
    }),
  });
  await fetch("http://127.0.0.1:3001/api/issues/I_requests_1", {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      assignee_id: issue.assignee_id,
      priority: Number(issue.priority),
      notes: issue.triage_notes,
    }),
  });
  run("close");
}
