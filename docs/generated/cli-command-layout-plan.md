# CLI command layout plan

Status: implemented. This records the command layout and migration choices.

## Problem

Previously, `sdb repl` opened an interactive editor, executed batch SQL or Ask,
validated and inspected a project, dispatched writes, and maintained caches.
Its help described all of those jobs, and options for one job appeared beside
options for another. Argument conflicts improved safety but did not make the
command's purpose obvious.

## Command surface

| Job | Command | Previous CLI |
| --- | --- | --- |
| Interactive SQL and dot commands | `sdb repl` | `sdb repl` with no batch action |
| One authored SQL statement | `sdb sql 'SELECT …'`, `sdb sql 'UPDATE …'`, `sdb sql --file statement.sql`, or `sdb sql -` for stdin | `sdb repl --query`, `--file`, piped input, `--write`, and `--explain-write` |
| Natural-language read | `sdb ask 'request'` | `sdb repl --ask` and `--ask-views`; the separate view-only path is removed |
| Offline or connected check | `sdb validate [--connect]` | `sdb repl --validate [--connect]` |
| Model and project inventory | `sdb inspect` | `sdb repl --inspect` |
| Cache maintenance | `sdb cache status`, `sdb cache refresh NAME`, `sdb cache invalidate KEY` | `sdb repl --cache-*` |
| Server and scaffold | `sdb server`, `sdb init` | Server gains project discovery |

`--project-config PATH` is a shared project option where relevant. Without
it, commands use `semantic-db.yaml` in the current directory when present.
`--no-project` remains available for an intentionally empty REPL or SQL
read. Mutations require a project with an explicit write binding.
Each command announces an automatically discovered project on stderr and
rejects actions that require a project when none is available.
`sdb server` also discovers `semantic-db.yaml` in the current directory;
`--project-config PATH` remains an explicit override.

For `sdb sql`, a single `-` is the stdin operand, as in
`cat statement.sql | sdb sql -`. A double dash (`--`) ends option parsing; it
is useful before a positional SQL string that begins with a dash, but does not
mean stdin. Normal positional SQL needs neither marker. There is no redundant
`--sql` spelling.

## Option ownership

- `repl`: history and color options, plus read policy for queries entered in the
  session. It should not accept batch action flags.
- `sql`: one authored SQL statement from positional text, a file, or stdin;
  these inputs are mutually exclusive. Timeout applies to reads and writes.
  Read consistency, cache policy, and read reports apply only to reads.
  `--plan` prints a read's raw logical plan. `--explain` reports read
  dependencies and consistency checks or a write plan, based on the statement
  type. Both skip row and mutation execution. A mutation with a read-only
  option is a usage error, not an ignored option.
- `ask`: general provider-backed compilation over registered base relations and
  authored views, read policy, and `--compile-only` to show generated SQL and
  evidence without executing rows. The help must say that compile-only still
  contacts the model. No view-only switch is exposed.
- `validate` and `inspect`: project checks only. `--connect` belongs to
  `validate` and must remain opt-in.
- `cache`: one operation per invocation. Status should not construct source
  providers; refresh may need them.

Keep option names consistent across commands that share a behavior. Use typed
parsers so invalid cache ages and timeouts fail before loading a project. `--help`
for each command should fit on a normal terminal screen where possible.

## Execution semantics

- Offline validation and inspection must not load credentials or query rows.
- Connected validation checks physical schemas but does not promise row access.
- Query, file, stdin, Ask, and interactive reads should apply the same read
  consistency and cache policy.
- `sql` must parse exactly one statement and dispatch by statement type:
  `SELECT` and other supported reads through the read engine; `INSERT`,
  `UPDATE`, `DELETE`, and `MERGE` through the write dispatcher. Unsupported or
  malformed SQL must fail. An unrecognized statement must never be treated as
  a write. The existing token-based `is_write_statement` helper is a dispatch
  hint, not a substitute for full validation by the chosen engine path.
- An authored view is loaded from project configuration; interactive `.view` and
  batch `--view` are not part of the new command surface.
- Mutations execute only explicitly supplied SQL against a registered write
  binding. Ask remains read-only. Explaining a mutation must not apply source
  or destination changes. `EXPLAIN` before a mutation must follow the write
  explanation path, while a read explanation stays on the read path.
- General Ask may query authored views when they fit the request. Removing the
  dedicated view-only compiler does not remove project-defined SQL views.
- Exit status 2 is for argument and usage errors; execution or connection
  failures remain nonzero with actionable context.

## Migration and verification

The split replaces the old `repl` batch flags without aliases. The dedicated
view-only Ask path is removed from the CLI, REPL, compiler API, prompt, typed
selection structures, tests, examples, and generated docs. Project-authored
views and ordinary SQL over them remain. HTTP `/compile` continues to use
general Ask. Repository docs outside `docs/generated` were updated with the
approval required by `AGENTS.md`.

Tests cover each new command's help, valid inputs, project discovery,
missing-project errors, incompatible options, offline behavior without secrets,
cache operations, read policy parity across input forms, read/write dispatch
with comments and `EXPLAIN`, malformed and multiple statements, rejection of
read-only options on mutations, write explain safety, and terminal REPL
behavior. The package-maintenance scripts and custom connector binary use the
shared CLI frontend.

## Preview behavior

The CLI uses distinct names:

| Invocation | Output | Execution |
| --- | --- | --- |
| `sdb sql --plan 'SELECT …'` | Raw logical plan | No rows |
| `sdb sql --explain 'SELECT …'` | Read dependencies and consistency checks | No rows |
| `sdb sql --explain 'UPDATE …'` | Write plan | No source or destination changes |
| `sdb ask --compile-only 'request'` | Generated SQL and grounding evidence | Model call, no query rows |

`--plan` accepts reads only. It does not use read consistency or cache policy,
so those options conflict with `--plan`. A read `--explain` may use those policies
because they affect its explanation. `--plan` and `--explain` conflict, as do
`--read-report` and either preview mode. For writes, reject all read-only
options. The overloaded `--dry-run`, `--explain-read`, and `--explain-write`
spellings are removed.
