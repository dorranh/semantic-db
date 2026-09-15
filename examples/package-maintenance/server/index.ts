import express, { type ErrorRequestHandler } from "express";
import { semantic, httpUrl, queries } from "./db";
import { db, updatePackage, updateTriage, InputError } from "./mutations";
const app = express();
app.disable("x-powered-by");
app.use(express.json({ limit: "16kb" }));
// Vite proxies same-origin requests. No permissive CORS on the local application API.
app.use((req, res, next) => {
  const origin = req.get("origin");
  if (
    origin &&
    ![
      "http://127.0.0.1:5173",
      "http://localhost:5173",
      "http://127.0.0.1:3001",
    ].includes(origin)
  )
    return res.status(403).json({ error: "Unrecognized origin" });
  next();
});
app.get("/api/health", async (_req, res) => {
  const response = await fetch(`${httpUrl}/health`, {
    signal: AbortSignal.timeout(3000),
  });
  res
    .status(response.status)
    .json({
      ...(await response.json()),
      data_mode: process.env.SEMANTIC_LIVE ? "live" : "fixture",
    });
});
app.get("/api/packages", async (_req, res) =>
  res.json((await semantic.query(queries.packages)).rows),
);
app.get("/api/members", async (_req, res) =>
  res.json((await semantic.query(queries.members)).rows),
);
app.get("/api/packages/:name", async (req, res) => {
  const values = [req.params.name];
  const [summary, issues, trend] = await Promise.all([
    semantic.query(queries.package, values),
    semantic.query(queries.issues, values),
    semantic.query(queries.trend, values),
  ]);
  if (!summary.rows.length)
    return res.status(404).json({ error: "Package not found" });
  res.json({
    package: summary.rows[0],
    issues: issues.rows,
    trend: trend.rows,
  });
});
app.patch("/api/packages/:name", async (req, res) => {
  await updatePackage(req.params.name, req.body);
  res.json({ saved: true });
});
app.patch("/api/issues/:id", async (req, res) => {
  await updateTriage(req.params.id, req.body);
  res.json({ saved: true });
});
app.get("/api/model", async (_req, res) => {
  const response = await fetch(`${httpUrl}/catalog`, {
    signal: AbortSignal.timeout(3000),
  });
  if (!response.ok) throw new Error("Model unavailable");
  res.json({ catalog: await response.json(), queries });
});
app.post("/api/ask", async (req, res) => {
  const response = await fetch(`${httpUrl}/compile`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ question: req.body.question }),
    signal: AbortSignal.timeout(90000),
  });
  const result = await response.json();
  if (!response.ok) return res.status(response.status).json(result);
  if (result.outcome.status === "grounded")
    result.rows = (await semantic.query(result.outcome.query.sql)).rows;
  res.json(result);
});
const errors: ErrorRequestHandler = (error, _req, res, _next) => {
  if (error instanceof InputError)
    return res.status(400).json({ error: error.message });
  if (error?.code === "P2025")
    return res.status(404).json({ error: "Record not found" });
  if (error?.code === "P2003")
    return res.status(400).json({ error: "Choose an existing assignee" });
  if (error?.type === "entity.parse.failed")
    return res.status(400).json({ error: "Invalid JSON body" });
  console.error("Request failed:", error?.code ?? error?.name ?? "unknown");
  res
    .status(502)
    .json({
      error:
        "The request could not complete. Check the source services and retry. No complete query result is available.",
    });
};
app.use(errors);
const server = app.listen(3001, "127.0.0.1", () =>
  console.log("Application API: 127.0.0.1:3001"),
);
async function close() {
  server.close();
  await Promise.all([semantic.end(), db.$disconnect()]);
  process.exit(0);
}
process.on("SIGTERM", close);
process.on("SIGINT", close);
