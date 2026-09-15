import { spawn, spawnSync, execFileSync } from "node:child_process";
import {
  existsSync,
  copyFileSync,
  mkdirSync,
  writeFileSync,
  readFileSync,
  unlinkSync,
} from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { createHash } from "node:crypto";
const directory = fileURLToPath(new URL("..", import.meta.url));
process.chdir(directory);
if (!existsSync(".env")) copyFileSync(".env.example", ".env");
process.loadEnvFile(".env");
const mode = process.argv[2];
const root = path.resolve(directory, "../..");
const processTitle =
  "maintenance-" +
  createHash("sha256").update(directory).digest("hex").slice(0, 12);
function run(command, args, cwd = directory) {
  const result = spawnSync(command, args, {
    cwd,
    stdio: "inherit",
    env: process.env,
  });
  if (result.status !== 0)
    throw new Error(
      `${command} failed (${result.status ?? result.error?.message})`,
    );
}
function stop() {
  if (!existsSync(".run/pid")) return;
  const pid = Number(readFileSync(".run/pid", "utf8"));
  try {
    const command = execFileSync("ps", ["-p", String(pid), "-o", "args="], {
      encoding: "utf8",
    });
    if (command.trim() === processTitle) process.kill(pid, "SIGTERM");
  } catch {
    /* Already stopped. Never signal an unverified process. */
  }
}
async function wait(url) {
  for (let attempt = 0; attempt < 120; attempt++) {
    try {
      if ((await fetch(url, { signal: AbortSignal.timeout(1000) })).ok) return;
    } catch {}
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`Service did not become ready: ${url}`);
}
if (mode === "setup" || mode === "reset") {
  if (mode === "reset") {
    stop();
    run("docker", ["compose", "down", "--volumes"]);
  }
  run("docker", ["compose", "up", "-d", "--wait"]);
  run("npm", ["run", "generate"]);
  run("npm", ["run", "migrate"]);
  run("npm", ["run", "seed"]);
} else if (mode === "stop") {
  stop();
  run("docker", ["compose", "stop"]);
} else if (mode === "start") {
  mkdirSync(".run", { recursive: true });
  if (existsSync(".run/pid")) {
    try {
      process.kill(Number(readFileSync(".run/pid", "utf8")), 0);
      throw new Error("Example is already running; use npm run stop first.");
    } catch (e) {
      if (e.code !== "ESRCH") throw e;
    }
  }
  run("cargo", ["build", "-p", "semantic-server", "--features", "github", "--locked"], root);
  process.title = processTitle;
  writeFileSync(".run/pid", String(process.pid));
  const children = [];
  let stopping = false;
  function cleanup(code = 0) {
    if (stopping) return;
    stopping = true;
    for (const child of children) child.kill("SIGTERM");
    try {
      unlinkSync(".run/pid");
    } catch {}
    setTimeout(() => process.exit(code), 500);
  }
  function launch(command, args) {
    const child = spawn(command, args, {
      cwd: directory,
      stdio: "inherit",
      env: process.env,
    });
    children.push(child);
    child.on("error", () => cleanup(1));
    child.on("exit", (code) => {
      if (!stopping) cleanup(code || 1);
    });
  }
  process.on("SIGINT", () => cleanup());
  process.on("SIGTERM", () => cleanup());
  try {
    if (!process.env.SEMANTIC_LIVE) {
      launch(process.execPath, ["fixtures/github.mjs"]);
      await wait("http://127.0.0.1:4010/health");
    }
    launch(path.join(root, "target/debug/semantic-server"), [
      "--config",
      process.env.SEMANTIC_LIVE ? "semantic-db.live.yaml" : "semantic-db.yaml",
      "--query-timeout-seconds",
      process.env.SEMANTIC_LIVE ? "120" : "30",
    ]);
    await wait("http://127.0.0.1:5545/health");
    launch(process.execPath, ["--import", "tsx", "server/index.ts"]);
    await wait("http://127.0.0.1:3001/api/health");
    launch(process.execPath, [
      "node_modules/vite/bin/vite.js",
      "--host",
      "127.0.0.1",
    ]);
    console.log("Maintenance desk: http://127.0.0.1:5173");
  } catch (e) {
    console.error(e.message);
    cleanup(1);
  }
} else throw new Error("Use setup, start, stop, or reset");
