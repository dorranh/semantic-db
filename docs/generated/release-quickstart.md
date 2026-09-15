# From download to a meaningful answer

Download the archive matching your OS and architecture from the project's
GitHub release assets. Verify its SHA-256 against the adjacent `.sha256` file,
extract it, and add the directory containing both executables to `PATH`.
On Unix use `shasum -a 256 ARCHIVE`; on Windows use
`Get-FileHash ARCHIVE -Algorithm SHA256`. Confirm `semantic-db --version` and
`semantic-server --version` report the same release.

## Start without credentials

```sh
semantic-db init my-project
cd my-project
semantic-db --config semantic-db.yaml --validate --connect
semantic-db --config semantic-db.yaml --query 'SELECT * FROM active_items ORDER BY id'
semantic-db --config semantic-db.yaml
```

The sample has three items and an `active_items` view containing the two active
items. No source service, model API key, or Rust compiler is required.

## Ask using your definitions

Copy `.env.example` to `.env`, then set `OPENAI_API_KEY` and `OPENAI_MODEL`.
Set `OPENAI_BASE_URL` for another OpenAI-compatible API. Run from the project
directory so both binaries find `.env`; process environment takes precedence.

```sh
semantic-db --config semantic-db.yaml --ask-views 'List active items' --dry-run
semantic-db --config semantic-db.yaml --ask-views 'List active items'
```

The first command shows generated SQL and binding evidence without executing
query rows; it still contacts the model. The second executes the query. In the
REPL use `.plan-views List active items` and `.ask-views List active items`.
Here “active” has a reusable executable definition in the authored view.

Compilation may return clarification or unsupported instead of SQL. Clarifications
are not a conversation session: resubmit a complete question with the missing
definition. Unmet requests must not execute a fallback query. General `.ask`
proposes SQL over catalog relations and has different guarantees; use the
authored-view mode for this onboarding path.

## Connect your data with skills

The archive's `skills` directory contains five installable skill folders. Copy
the selected folders into your agent's skills directory (for Codex,
`~/.codex/skills/`), or invoke their `SKILL.md` files directly if your agent
supports that workflow. Start with `semantic-db-bootstrap`; it coordinates
source discovery, modeling, validation and representative questions.

The core choices are PostgreSQL, ClickHouse and local files. Unsupported APIs
require a custom source build. The HTTP/GraphQL skills help implement that
developer path; GitHub is a worked GraphQL example, not a bundled release source.
Use [the connector guide](file-connectors.md) for manual setup without an agent.

## Use the same project in an application

```sh
semantic-server --config semantic-db.yaml
```

The PostgreSQL listener is `127.0.0.1:5544`; HTTP is `127.0.0.1:5545`. Use a
backend PostgreSQL driver, such as Node `pg`, to query `active_items`. Browser
code calls your application backend; it does not connect directly to PostgreSQL
or receive source/model credentials.

For Ask, the application backend sends:

```http
POST /compile
Content-Type: application/json

{"question":"List active items"}
```

The compiler uses authored views when their definitions fit, or generates SQL
over other catalog relations. Inspect the returned SQL and grounding evidence.
Execute successful SQL using the
PostgreSQL driver, and display clarification/unsupported outcomes to the user.
The source repository's package-maintenance application provides a full working
example of this flow. `/health` reports whether Ask is configured; `/catalog`
exposes the semantic catalog. Without provider setup, SQL continues to work.

These listeners target local/trusted deployments. Consult [delivery boundaries](delivery.md)
and the capability matrix before choosing database types or a dashboard client.
