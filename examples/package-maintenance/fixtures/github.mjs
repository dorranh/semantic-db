import http from "node:http";
const base = [
  [
    "psf/requests",
    "I_requests_1",
    101,
    "Proxy authentication drops on redirect",
    "OPEN",
  ],
  [
    "psf/requests",
    "I_requests_2",
    102,
    "Document the connection pool lifecycle",
    "OPEN",
  ],
  [
    "psf/requests",
    "I_requests_3",
    103,
    "Preserve Unicode in response headers",
    "CLOSED",
  ],
  [
    "urllib3/urllib3",
    "I_urllib3_1",
    201,
    "Improve retries for interrupted streams",
    "OPEN",
  ],
  [
    "urllib3/urllib3",
    "I_urllib3_2",
    202,
    "Clarify certificate configuration",
    "OPEN",
  ],
  [
    "encode/httpx",
    "I_httpx_1",
    301,
    "Async client cancellation leaves pending work",
    "OPEN",
  ],
];
let control = { delay_ms: 0, fail_after_first_page: false, closed_issue: null };
export function createFixtureServer() {
  return http.createServer(async (req, res) => {
    res.setHeader("Content-Type", "application/json");
    if (req.method === "GET" && req.url === "/health")
      return res.end(JSON.stringify({ ready: true, control }));
    const chunks = [];
    let size = 0;
    for await (const chunk of req) {
      size += chunk.length;
      if (size > 16384) {
        res.writeHead(413);
        return res.end("{}");
      }
      chunks.push(chunk);
    }
    let body;
    try {
      body = JSON.parse(Buffer.concat(chunks).toString() || "{}");
    } catch {
      res.writeHead(400);
      return res.end("{}");
    }
    if (req.method === "POST" && req.url === "/control") {
      if (body.reset)
        control = {
          delay_ms: 0,
          fail_after_first_page: false,
          closed_issue: null,
        };
      else
        control = {
          delay_ms: Math.max(
            0,
            Math.min(Number(body.delay_ms ?? control.delay_ms) || 0, 30000),
          ),
          fail_after_first_page:
            body.fail_after_first_page ?? control.fail_after_first_page,
          closed_issue: body.closed_issue ?? control.closed_issue,
        };
      return res.end(JSON.stringify(control));
    }
    if (req.url !== "/graphql" || req.method !== "POST") {
      res.writeHead(404);
      return res.end("{}");
    }
    const mode = { ...control };
    const v = body.variables ?? {};
    await new Promise((resolve) => setTimeout(resolve, mode.delay_ms));
    if (mode.fail_after_first_page && v.after)
      return res.end(
        JSON.stringify({
          data: null,
          errors: [{ message: "Injected late-page failure" }],
        }),
      );
    const repo = `${v.owner}/${v.name}`;
    const all = base
      .filter((row) => row[0] === repo)
      .map(([repository, id, number, title, state]) => ({
        id,
        number,
        title,
        state: mode.closed_issue === id ? "CLOSED" : state,
        author: number === 102 ? null : { login: "fixture-maintainer" },
        createdAt: "2026-09-01T09:00:00Z",
        updatedAt: "2026-09-14T12:00:00Z",
        closedAt:
          state === "CLOSED" || mode.closed_issue === id
            ? "2026-09-14T12:00:00Z"
            : null,
        url: `https://github.com/${repository}/issues/${number}`,
      }))
      .filter((row) => !v.states || v.states.includes(row.state));
    const offset = Number(v.after ?? 0),
      end = offset + Math.min(Number(v.first) || 2, 2);
    res.end(
      JSON.stringify({
        data: {
          repository: {
            nameWithOwner: repo,
            issues: {
              nodes: all.slice(offset, end),
              pageInfo: {
                hasNextPage: end < all.length,
                endCursor: String(end),
              },
            },
          },
        },
      }),
    );
  });
}
if (import.meta.url === `file://${process.argv[1]}`)
  createFixtureServer().listen(4010, "127.0.0.1", () =>
    console.log("GitHub fixture: 127.0.0.1:4010"),
  );
