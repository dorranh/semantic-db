# Interactive SQL REPL

Start an interactive session with a project or a CSV source:

```sh
cargo run -p semantic-cli -- --config examples/geospatial/semantic-db.yaml
cargo run -p semantic-cli -- --csv wells=examples/geospatial/wells.csv
```

The REPL uses normal terminal scrollback. SQL input is highlighted; result cells
retain their original formatting. Successful executions show elapsed time, and
errors report time before failure. Natural-language timings include compilation
and any execution.

## Editing and completion

- End SQL with `;` and press Enter to execute. Trailing comments are allowed.
  Semicolons inside strings and comments do not terminate a statement.
- Without a terminating semicolon, Enter adds a line. Arrow keys let you edit
  earlier lines of the same statement. A history entry contains the whole statement.
- Tab inserts a unique completion or common prefix; another Tab lists alternatives.
- Up/Down browse history; Ctrl-R searches it. With colors enabled, subdued hints
  suggest matching history entries using Rustyline's standard editing keys.
- Ctrl-C clears the input buffer. Ctrl-D exits when the buffer is empty and uses
  standard editor deletion behavior otherwise. `.quit` and `.exit` also exit.
- Dot commands occupy one line; use `.help` for the command list.

Autocomplete covers dot commands, `.schema` relation names, SQL keywords, and
catalog tables, views, and columns. It also works inside `.view NAME=SQL`.
Natural-language arguments to `.ask`, `.plan`, `.ask-views`, and `.plan-views`
are not treated as SQL.

Completion considers the entire editing buffer, including text after the cursor.
For example, with `FROM wells w` already entered, `w.` in the SELECT list offers
the columns of `wells`. Joins, aliases, CTEs, nested subqueries, and correlated
outer references are supported. Ambiguous column suggestions are qualified.
Quoted identifiers preserve case; unquoted prefixes match without case sensitivity.

CTE and derived-table columns come from explicit column lists, named projections,
direct column references, and resolvable stars. Expressions without a declared
name are not guessed. Completion is best effort while SQL is unfinished:
unresolved scopes fall back to keywords and known relation names. It does not
plan SQL, read rows, or contact providers. New `.view` definitions become available
immediately after registration.

## History and colors

History persists by default, shared across projects, retaining the latest 1,000
entries. It includes SQL, dot commands, natural-language requests, and failed
submissions. Blank input and consecutive duplicates are excluded. Accepted entries
are saved before execution; simultaneous sessions append under file locks.

The startup message shows the history path. The default is `semantic-db/history`
under the platform state directory (Linux: `$XDG_STATE_HOME`, otherwise
`~/.local/state`). Where a state directory is unavailable, the local application
data directory is used (macOS: `~/Library/Application Support`; Windows:
LocalAppData). History storage failures warn once and retain session history.

| Option | Behavior |
| --- | --- |
| `--history-file PATH` | Override the persistent history file; a relative path uses the working directory. |
| `--no-history` | Keep session navigation/search, without reading or writing a history file. Conflicts with `--history-file`. |
| `--no-color` | Disable REPL styling. |

A nonempty `NO_COLOR` or `TERM=dumb` also disables styling. Styling is disabled
when output is redirected. Batch queries, files, and piped input keep their existing
plain output and do not initialize REPL history.

SQL still accepts one statement per submission, and execution still collects
results in memory. Use `LIMIT` for exploratory queries. This change does not add
a pager, a full-screen UI, progress indicators, or cancellation during execution.

## Verification

`cargo test -p semantic-cli --locked` covers the editor helper, completion scopes,
statement framing, history persistence, batch compatibility, and terminal behavior.
The terminal smoke tests use Python 3's standard-library PTY support on Unix;
they exercise completion, multiline editing/recall, reverse search, resizing,
interrupt/EOF handling, view refresh, restart persistence, and color controls.

They can also run directly after `cargo build -p semantic-cli --locked`:

```sh
python3 apps/semantic-cli/tests/repl_pty.py
```
