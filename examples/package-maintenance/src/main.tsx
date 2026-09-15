import React, { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  api,
  count,
  type Package,
  type Member,
  type Detail,
  type Issue,
} from "./api";
import "./style.css";

function App() {
  const [packages, setPackages] = useState<Package[]>([]),
    [members, setMembers] = useState<Member[]>([]);
  const [selected, setSelected] = useState("requests"),
    [revision, setRevision] = useState(0);
  const [detail, setDetail] = useState<Detail | null>(null),
    [error, setError] = useState(""),
    [loading, setLoading] = useState(true);
  const [panel, setPanel] = useState<"work" | "model" | "ask">("work");
  const [askEnabled, setAskEnabled] = useState(false),
    [live, setLive] = useState(false);
  useEffect(() => {
    const controller = new AbortController();
    api<{ ask_enabled: boolean; data_mode: string }>("/health", {
      signal: controller.signal,
    })
      .then((h) => {
        setAskEnabled(h.ask_enabled);
        setLive(h.data_mode === "live");
      })
      .catch(() => {});
    return () => controller.abort();
  }, []);
  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setError("");
    setDetail(null);
    Promise.all([
      api<Package[]>("/packages", { signal: controller.signal }),
      api<Member[]>("/members", { signal: controller.signal }),
      api<Detail>(`/packages/${encodeURIComponent(selected)}`, {
        signal: controller.signal,
      }),
    ])
      .then(([p, m, d]) => {
        setPackages(p);
        setMembers(m);
        setDetail(d);
      })
      .catch((e) => {
        if (!controller.signal.aborted) setError(e.message);
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [selected, revision]);
  const refresh = () => setRevision((v) => v + 1);
  return (
    <div className="shell">
      <header className="masthead">
        <a className="brand" href="/" aria-label="Maintenance desk home">
          <span className="brand-mark">s:</span>
          <span>
            Semantic DB<span className="brand-caption">APPLICATION LAB</span>
          </span>
        </a>
        <span className="environment">
          <span className="dot" />
          {live ? "Live sources" : "Local fixtures"}
        </span>
      </header>
      <div className="intro">
        <div>
          <p className="eyebrow">PACKAGE MAINTENANCE</p>
          <h1>The maintenance desk.</h1>
          <p className="intro-copy">
            Adoption, open issues, and the people keeping things moving.
          </p>
        </div>
        <button
          className="secondary refresh"
          onClick={refresh}
          disabled={loading}
        >
          {loading ? "Refreshing…" : "↻ Refresh sources"}
        </button>
      </div>
      <nav className="tabs" aria-label="Workspace">
        <button
          className={panel === "work" ? "active" : ""}
          onClick={() => setPanel("work")}
        >
          Workspace
        </button>
        <button
          className={panel === "model" ? "active" : ""}
          onClick={() => setPanel("model")}
        >
          Behind the query
        </button>
        <button
          className={panel === "ask" ? "active" : ""}
          onClick={() => setPanel("ask")}
        >
          Ask the model <span className="small-badge">OPTIONAL</span>
        </button>
      </nav>
      {error ? (
        <div className="error" role="alert">
          <strong>Sources could not be refreshed.</strong> {error}
          <button onClick={refresh}>Try again</button>
        </div>
      ) : null}
      {panel === "model" ? (
        <ModelPanel />
      ) : panel === "ask" ? (
        <AskPanel enabled={askEnabled} />
      ) : (
        <div className="workspace">
          <aside className="package-rail">
            <div className="section-label">
              TRACKED PACKAGES <span>{packages.length}</span>
            </div>
            <div className="package-list">
              {packages.map((p) => (
                <button
                  key={p.package_name}
                  className={`package-item ${selected === p.package_name ? "selected" : ""}`}
                  onClick={() => setSelected(p.package_name)}
                  aria-pressed={selected === p.package_name}
                >
                  <span className="package-symbol">⬡</span>
                  <span>
                    <strong>{p.package_name}</strong>
                    <small>{p.team}</small>
                  </span>
                  <span className="package-arrow">↗</span>
                </button>
              ))}
            </div>
            <div className="rail-note">
              <strong>One workspace. Three sources.</strong>
              <p>
                Ownership from your app.
                <br />
                Issues from GitHub.
                <br />
                Downloads from PyPI.
              </p>
            </div>
          </aside>
          <main className="detail" aria-busy={loading}>
            {loading ? (
              <div className="empty">
                Loading the latest source observations…
              </div>
            ) : detail ? (
              <PackageDetail
                key={`${selected}:${revision}`}
                detail={detail}
                members={members}
                refresh={refresh}
              />
            ) : (
              <div className="empty">
                No complete result to display. Refresh the sources to try again.
              </div>
            )}
          </main>
        </div>
      )}
      <footer>
        <span>Built on Semantic DB</span>
        <span>
          Source observations are independent. App edits are saved to Postgres.
        </span>
      </footer>
    </div>
  );
}
function PackageDetail({
  detail,
  members,
  refresh,
}: {
  detail: Detail;
  members: Member[];
  refresh: () => void;
}) {
  const p = detail.package;
  const [team, setTeam] = useState(p.team),
    [notes, setNotes] = useState(p.notes),
    [saving, setSaving] = useState(false),
    [error, setError] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  async function save(event: React.FormEvent) {
    event.preventDefault();
    setSaving(true);
    setError("");
    try {
      await api(`/packages/${encodeURIComponent(p.package_name)}`, {
        method: "PATCH",
        body: JSON.stringify({ team, notes }),
      });
      refresh();
    } catch (e) {
      setError((e as Error).message);
      setSaving(false);
    }
  }
  return (
    <>
      <div className="package-heading">
        <div>
          <span className="eyebrow">PYTHON PACKAGE</span>
          <h2>{p.package_name}</h2>
          <a
            href={`https://github.com/${p.repository}`}
            target="_blank"
            rel="noreferrer"
          >
            {p.repository} ↗
          </a>
        </div>
        <span className="team-badge">{p.team}</span>
      </div>
      <div className="metrics">
        <div>
          <span>Downloads in window</span>
          <strong>{count(p.downloads)}</strong>
          <small>September 1–14, 2026</small>
        </div>
        <div>
          <span>Open issues</span>
          <strong>{count(p.open_issues)}</strong>
          <small>In the mapped repository</small>
        </div>
        <div>
          <span>Waiting for an owner</span>
          <strong>{count(p.unassigned_issues)}</strong>
          <small>Open and unassigned in this app</small>
        </div>
      </div>
      <section className="chart-panel">
        <div className="section-heading">
          <h3>Daily downloads</h3>
          <span className="source-tag">PYPI / CLICKHOUSE</span>
        </div>
        <Trend data={detail.trend} />
      </section>
      <section className="issue-section">
        <div className="section-heading">
          <h3>
            Issue workspace{" "}
            <span className="muted">{detail.issues.length}</span>
          </h3>
          <span className="source-tag">GITHUB + YOUR APP</span>
        </div>
        {detail.issues.length === 0 ? (
          <p className="empty">
            No issues observed in the configured repository scope.
          </p>
        ) : (
          <div className="issues">
            {detail.issues.map((i) => (
              <React.Fragment key={i.issue_id}>
                <div className="issue-row">
                  <span
                    className={`issue-state ${i.state === "OPEN" ? "open" : "closed"}`}
                    title={i.state}
                  >
                    {i.state === "OPEN" ? "○" : "✓"}
                  </span>
                  <div className="issue-title">
                    <a href={i.url} target="_blank" rel="noreferrer">
                      {i.title}
                    </a>
                    <small>
                      #{i.number} · {i.state.toLowerCase()} ·{" "}
                      <span className={`priority priority-${i.priority}`}>
                        {["", "High", "Normal", "Low"][i.priority]} priority
                      </span>
                    </small>
                  </div>
                  <button
                    className="assign-button"
                    onClick={() =>
                      setEditing(editing === i.issue_id ? null : i.issue_id)
                    }
                    aria-expanded={editing === i.issue_id}
                    aria-label={`Edit triage for issue ${i.number}`}
                  >
                    {i.assignee_name ?? "Assign owner"} <span>↗</span>
                  </button>
                </div>
                {editing === i.issue_id ? (
                  <IssueEditor issue={i} members={members} refresh={refresh} />
                ) : null}
              </React.Fragment>
            ))}
          </div>
        )}
      </section>
      <form className="ownership" onSubmit={save}>
        <div className="section-heading">
          <h3>Maintenance notes</h3>
          <span className="source-tag">YOUR APP / POSTGRES</span>
        </div>
        <div className="ownership-fields">
          <label>
            Owning team
            <input
              value={team}
              onChange={(e) => setTeam(e.target.value)}
              maxLength={120}
              required
            />
          </label>
          <label>
            Package notes
            <textarea
              value={notes}
              onChange={(e) => setNotes(e.target.value)}
              maxLength={4000}
              rows={2}
            />
          </label>
        </div>
        <div className="save-row">
          <p>These fields belong to your application.</p>
          <button type="submit" disabled={saving}>
            {saving ? "Saving…" : "Save package"}
          </button>
        </div>
        {error ? (
          <p className="error" role="alert">
            Save failed. {error}
          </p>
        ) : null}
      </form>
    </>
  );
}
function IssueEditor({
  issue,
  members,
  refresh,
}: {
  issue: Issue;
  members: Member[];
  refresh: () => void;
}) {
  const [assignee, setAssignee] = useState(issue.assignee_id ?? ""),
    [priority, setPriority] = useState(Number(issue.priority)),
    [notes, setNotes] = useState(issue.triage_notes),
    [error, setError] = useState(""),
    [saving, setSaving] = useState(false);
  async function save(e: React.FormEvent) {
    e.preventDefault();
    setSaving(true);
    setError("");
    try {
      await api(`/issues/${encodeURIComponent(issue.issue_id)}`, {
        method: "PATCH",
        body: JSON.stringify({
          assignee_id: assignee || null,
          priority,
          notes,
        }),
      });
      refresh();
    } catch (e) {
      setError((e as Error).message);
      setSaving(false);
    }
  }
  return (
    <form className="issue-editor" onSubmit={save}>
      <label>
        Assignee
        <select value={assignee} onChange={(e) => setAssignee(e.target.value)}>
          <option value="">Unassigned</option>
          {members.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}
            </option>
          ))}
        </select>
      </label>
      <label>
        Priority
        <select
          value={priority}
          onChange={(e) => setPriority(Number(e.target.value))}
        >
          <option value={1}>High</option>
          <option value={2}>Normal</option>
          <option value={3}>Low</option>
        </select>
      </label>
      <label className="editor-notes">
        Triage notes
        <textarea
          rows={2}
          value={notes}
          onChange={(e) => setNotes(e.target.value)}
          maxLength={4000}
        />
      </label>
      <button type="submit" disabled={saving}>
        {saving ? "Saving…" : "Save triage"}
      </button>
      {error ? (
        <p className="error" role="alert">
          Save failed. {error}
        </p>
      ) : null}
    </form>
  );
}
function Trend({ data }: { data: Detail["trend"] }) {
  if (!data.length)
    return (
      <p className="empty">
        No download observations for this package in the window.
      </p>
    );
  const max = data.reduce(
    (a, d) => (BigInt(d.downloads) > a ? BigInt(d.downloads) : a),
    1n,
  );
  const points = data
    .map(
      (d, i) =>
        `${20 + (i * 720) / Math.max(data.length - 1, 1)},${150 - Number((BigInt(d.downloads) * 120n) / max)}`,
    )
    .join(" ");
  return (
    <>
      <svg
        className="trend"
        viewBox="0 0 760 176"
        role="img"
        aria-label="Daily package downloads"
      >
        <title>
          {data
            .map((d) => `${d.download_date}: ${count(d.downloads)}`)
            .join("; ")}
        </title>
        {[30, 70, 110, 150].map((y) => (
          <line
            key={y}
            x1="20"
            x2="740"
            y1={y}
            y2={y}
            stroke="#dbe4f1"
            strokeDasharray="3 5"
          />
        ))}
        <polygon points={`20,150 ${points} 740,150`} fill="#e6edff" />
        <polyline
          points={points}
          fill="none"
          stroke="#345ad8"
          strokeWidth="3"
          strokeLinejoin="round"
        />
        {data.map((d, i) => (
          <circle
            key={d.download_date}
            cx={20 + (i * 720) / Math.max(data.length - 1, 1)}
            cy={150 - Number((BigInt(d.downloads) * 120n) / max)}
            r="3.5"
            fill="#345ad8"
          >
            <title>
              {d.download_date}: {count(d.downloads)}
            </title>
          </circle>
        ))}
      </svg>
      <div className="chart-labels">
        <span>{data[0].download_date}</span>
        <span>Daily peak {count(max.toString())}</span>
        <span>{data.at(-1)?.download_date}</span>
      </div>
    </>
  );
}
function ModelPanel() {
  const [model, setModel] = useState<{
      catalog: { name: string; description: string; view_sql?: string }[];
      queries: Record<string, string>;
    } | null>(null),
    [error, setError] = useState("");
  useEffect(() => {
    const c = new AbortController();
    api<NonNullable<typeof model>>("/model", { signal: c.signal })
      .then(setModel)
      .catch((e) => {
        if (!c.signal.aborted) setError(e.message);
      });
    return () => c.abort();
  }, []);
  return (
    <main className="standalone">
      <p className="eyebrow">BEHIND THE QUERY</p>
      <h2>Meaning, defined once.</h2>
      <p>
        The same authored views power this workspace and natural-language
        questions.
      </p>
      {error ? (
        <p role="alert">{error}</p>
      ) : !model ? (
        <p>Loading semantic model…</p>
      ) : (
        <>
          {model.catalog.map((r) => (
            <details key={r.name} open={r.name === "package_overview"}>
              <summary>{r.name}</summary>
              <p>{r.description}</p>
              {r.view_sql ? (
                <pre>{r.view_sql}</pre>
              ) : (
                <span className="source-tag">BASE RELATION</span>
              )}
            </details>
          ))}
          <details>
            <summary>Application query statements</summary>
            <pre>
              {Object.entries(model.queries)
                .map(([name, sql]) => `-- ${name}\n${sql}`)
                .join("\n\n")}
            </pre>
          </details>
        </>
      )}
    </main>
  );
}
interface AskResult {
  outcome: {
    status: string;
    question?: string;
    reason?: string;
    query?: {
      sql: string;
      evidence: {
        phrase: string;
        catalog_reference: string;
        interpretation: string;
      }[];
    };
  };
  rows?: Record<string, unknown>[];
}
function AskPanel({ enabled }: { enabled: boolean }) {
  const [question, setQuestion] = useState(
      "Show packages with more than 1 open issues",
    ),
    [result, setResult] = useState<AskResult | null>(null),
    [error, setError] = useState(""),
    [loading, setLoading] = useState(false);
  async function ask(e: React.FormEvent) {
    e.preventDefault();
    setLoading(true);
    setError("");
    setResult(null);
    try {
      setResult(
        await api<AskResult>("/ask", {
          method: "POST",
          body: JSON.stringify({ question }),
        }),
      );
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }
  return (
    <main className="standalone">
      <p className="eyebrow">ASK THE SEMANTIC MODEL</p>
      <h2>A question with a traceable answer.</h2>
      <p>
        Ask about package ownership, download observations, or issue triage.
        Every answer uses an authored view.
      </p>
      {!enabled ? (
        <div className="setup-note">
          <strong>Ask is not configured.</strong>
          <p>
            Set OPENAI_API_KEY and OPENAI_MODEL in the example’s .env, then
            restart. The workspace works without a model.
          </p>
        </div>
      ) : (
        <form className="ask-form" onSubmit={ask}>
          <label>
            Your question
            <textarea
              value={question}
              onChange={(e) => setQuestion(e.target.value)}
              maxLength={8000}
              required
              rows={3}
            />
          </label>
          <button disabled={loading}>
            {loading ? "Compiling and querying…" : "Ask question"}
          </button>
        </form>
      )}
      {error ? (
        <p className="error" role="alert">
          {error}
        </p>
      ) : null}
      {result ? (
        <section aria-live="polite">
          {result.outcome.status === "grounded" ? (
            <>
              <h3>Grounded query</h3>
              <pre>{result.outcome.query?.sql}</pre>
              <ul>
                {result.outcome.query?.evidence.map((e, i) => (
                  <li key={i}>
                    <strong>{e.catalog_reference}</strong>: {e.interpretation}
                  </li>
                ))}
              </ul>
              <div className="result-table">
                {result.rows?.length ? (
                  <table>
                    <thead>
                      <tr>
                        {Object.keys(result.rows[0]).map((k) => (
                          <th key={k}>{k}</th>
                        ))}
                      </tr>
                    </thead>
                    <tbody>
                      {result.rows.map((r, i) => (
                        <tr key={i}>
                          {Object.entries(r).map(([k, v]) => (
                            <td key={k}>{v === null ? "—" : String(v)}</td>
                          ))}
                        </tr>
                      ))}
                    </tbody>
                  </table>
                ) : (
                  <p>No matching rows.</p>
                )}
              </div>
            </>
          ) : (
            <p>{result.outcome.question ?? result.outcome.reason}</p>
          )}
        </section>
      ) : null}
    </main>
  );
}
createRoot(document.getElementById("root")!).render(<App />);
