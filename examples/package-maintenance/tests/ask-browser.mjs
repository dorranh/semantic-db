// Standalone Ask check: scripted model, real Semantic DB and application, local CSVs.
// Requires a built semantic-server, npm dependencies and agent-browser; port 3001 must be free.
import assert from "node:assert/strict";
import { execFile, spawn } from "node:child_process";
import { once } from "node:events";
import { createServer } from "node:http";
import { mkdtemp, readFile, writeFile, rm, copyFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, resolve, join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { createServer as createViteServer } from "vite";
import react from "@vitejs/plugin-react";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const binary = process.env.SEMANTIC_SERVER_BIN ?? resolve(root, "../../target/debug/semantic-server");
const session = `maintenance-ask-${process.pid}`;
const browser = async (...args) => (await promisify(execFile)("agent-browser", ["--session", session, ...args], { timeout: 35000 })).stdout;
const directory = await mkdtemp(join(tmpdir(), "maintenance-ask-"));
const children = [];
let vite;
let model;
const calls = [];
const cases = [
  {
    question: "Show package ownership for requests",
    sql: "SELECT package_name, team FROM package_overview WHERE package_name = 'requests'",
    relation: "package_overview", interpretation: "Apply the authored package ownership mapping for requests.",
    cells: ["requests", "Platform"],
  },
  {
    question: "Count packages by team",
    sql: "SELECT team, COUNT(*) AS packages FROM package_overview GROUP BY team ORDER BY team",
    relation: "package_overview", interpretation: "Count one row per tracked package, grouped by its owning team.",
    cells: ["Platform", "1"],
  },
  {
    question: "Sum requests downloads from 2026-08-31 inclusive to 2026-09-01 exclusive",
    sql: "SELECT SUM(count) AS downloads FROM pypi_downloads WHERE project = 'requests' AND date >= DATE '2026-08-31' AND date < DATE '2026-09-01'",
    relation: "pypi_downloads", interpretation: "Sum contributions in the explicit date range, outside the authored demo window.",
    cells: ["11"],
  },
  {
    question: "Show stale packages",
    outcome: { status: "needs_clarification", phrases: ["stale"], question: "What age counts as stale?" },
    text: "What age counts as stale?",
  },
  {
    question: "Predict downloads tomorrow",
    outcome: { status: "unsupported", reason: "The catalog contains observations, not forecasts." },
    text: "The catalog contains observations, not forecasts.",
  },
];
async function listen(server, port = 0) {
  server.listen(port, "127.0.0.1");
  await once(server, "listening");
  return server.address().port;
}
async function freePort() {
  const probe = createServer();
  const port = await listen(probe);
  await new Promise((done) => probe.close(done));
  return port;
}
function start(command, args, options) {
  const child = spawn(command, args, { stdio: ["ignore", "pipe", "pipe"], ...options });
  child.output = "";
  child.stdout.on("data", (chunk) => { child.output += chunk; });
  child.stderr.on("data", (chunk) => { child.output += chunk; });
  child.on("error", (error) => { child.output += error.message; });
  children.push(child);
  return child;
}
async function ready(url, child) {
  for (let attempt = 0; attempt < 200; attempt++) {
    assert.equal(child.exitCode, null, child.output);
    try {
      if ((await fetch(url, { signal: AbortSignal.timeout(500) })).ok) return;
    } catch {}
    await new Promise((done) => setTimeout(done, 100));
  }
  assert.fail(`Service did not start: ${child.output}`);
}
try {
  // Fail before starting any process if the normal application is already running.
  const probe = createServer();
  await listen(probe, 3001);
  await new Promise((done) => probe.close(done));
  const fixtures = {
    "app.packages": "name,repository,team,notes\nrequests,psf/requests,Platform,Fixture\n",
    "app.members": "id,name\nsam,Sam Rivera\n",
    "app.issue_triage": "issue_id,assignee_id,priority,notes\nI_1,sam,2,Fixture\n",
    "github.issues": "issue_id,repository,number,title,state,url\nI_1,psf/requests,1,Fixture issue,OPEN,https://github.com/psf/requests/issues/1\n",
    "pypi.daily": "date,project,count\n2026-08-31,requests,11\n2026-09-01,requests,20\n2026-09-02,requests,30\n2026-09-15,requests,40\n",
  };
  const config = JSON.parse(await readFile(join(root, "semantic-db.yaml"), "utf8"));
  config.connections = { local: { connector: "csv" } };
  delete config.app_tables;
  for (const [source, csv] of Object.entries(fixtures)) {
    await writeFile(join(directory, `${source}.csv`), csv);
    config.sources[source] = { connection: "local", path: `${source}.csv` };
  }
  for (const [name, view] of Object.entries(config.views)) {
    await copyFile(join(root, view.sql_file), join(directory, `${name}.sql`));
    view.sql_file = `${name}.sql`;
  }
  await copyFile(join(root, "model.ossie.yaml"), join(directory, "model.ossie.yaml"));
  await writeFile(join(directory, "semantic-db.json"), JSON.stringify(config));
  model = createServer(async (req, res) => {
    try {
      assert.equal(req.url, "/chat/completions");
      let body = "";
      for await (const chunk of req) body += chunk;
      const input = JSON.parse(JSON.parse(body).messages[1].content);
      assert(input.catalog.some((r) => r.name === "pypi_downloads" && r.view_sql === null));
      assert(input.catalog.some((r) => r.name === "package_overview" && r.view_sql));
      const scenario = cases.find((c) => c.question === input.request);
      assert(scenario, `Unexpected question: ${input.request}`);
      calls.push(input.request);
      const outcome = scenario.outcome ?? { status: "grounded", query: {
        sql: scenario.sql,
        evidence: [{ phrase: scenario.question, catalog_reference: scenario.relation, interpretation: scenario.interpretation }],
      } };
      res.setHeader("Content-Type", "application/json");
      res.end(JSON.stringify({ choices: [{ message: { content: JSON.stringify(outcome) }, finish_reason: "stop" }] }));
    } catch (error) {
      res.writeHead(500).end(String(error));
    }
  });
  const modelPort = await listen(model);
  const pgPort = await freePort();
  let httpPort = await freePort();
  while (httpPort === pgPort) httpPort = await freePort();
  const semantic = start(binary, ["--config", "semantic-db.json", "--port", String(pgPort), "--http-port", String(httpPort)], {
    cwd: directory,
    env: { ...process.env, OPENAI_API_KEY: "fixture", OPENAI_MODEL: "fixture", OPENAI_BASE_URL: `http://127.0.0.1:${modelPort}` },
  });
  await ready(`http://127.0.0.1:${httpPort}/health`, semantic);
  const application = start(process.execPath, ["--import", "tsx", "server/index.ts"], {
    cwd: root,
    env: { ...process.env, SEMANTIC_LIVE: "", SEMANTIC_DATABASE_URL: `postgresql://local@127.0.0.1:${pgPort}/semantic`, SEMANTIC_HTTP_URL: `http://127.0.0.1:${httpPort}` },
  });
  await ready("http://127.0.0.1:3001/api/health", application);
  vite = await createViteServer({
    configFile: false, root, plugins: [react()],
    server: { host: "127.0.0.1", port: 0, proxy: { "/api": { target: "http://127.0.0.1:3001", headers: { origin: "http://127.0.0.1:3001" } } } },
  });
  await vite.listen();
  await browser("open", `http://127.0.0.1:${vite.httpServer.address().port}`);
  await browser("wait", "--text", "Save package");
  await browser("snapshot", "-i");
  await browser("find", "role", "button", "click", "--name", "Ask the model OPTIONAL");
  await browser("wait", "--text", "Your question");
  for (const scenario of cases) {
    await browser("snapshot", "-i");
    await browser("find", "label", "Your question", "fill", scenario.question);
    await browser("find", "role", "button", "click", "--name", "Ask question");
    await browser("wait", "--text", scenario.interpretation ?? scenario.text);
    if (scenario.sql) {
      assert.equal((await browser("get", "text", "section[aria-live] pre")).trim(), scenario.sql);
      const cells = JSON.parse(await browser("eval", "JSON.stringify(Array.from(document.querySelectorAll('section[aria-live] td'), e => e.textContent))"));
      assert.deepEqual(JSON.parse(cells), scenario.cells);
    } else {
      assert.equal((await browser("get", "count", "section[aria-live] table")).trim(), "0");
      assert.equal((await browser("get", "count", "section[aria-live] pre")).trim(), "0");
    }
  }
  assert.deepEqual(calls, cases.map((c) => c.question));
  console.log("Ask browser check passed: view, aggregate, base data outside view dates, clarification, unsupported; real SQL execution with scripted model responses.");
} finally {
  await browser("close").catch(() => {});
  await vite?.close();
  for (const child of children.reverse()) {
    if (child.exitCode === null && child.signalCode === null) {
      const exited = once(child, "exit");
      child.kill("SIGTERM");
      await exited;
    }
  }
  if (model?.listening) await new Promise((done) => model.close(done));
  await rm(directory, { recursive: true, force: true });
}
