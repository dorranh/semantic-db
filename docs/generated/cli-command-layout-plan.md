# CLI command layout plan

Status: proposal only. The command split described here is not implemented.

## Problem

`sdb repl` currently opens an interactive editor, executes batch SQL or Ask,
validates and inspects a project, dispatches writes, and maintains caches. Its
help must describe all of those jobs, and options for one job can be confusing
beside options for another. Argument conflicts improve safety but do not make
the command's purpose obvious.

## Proposed command surface

| Job | Proposed command | Replaces |
| --- | --- | --- |
| Interactive SQL and dot commands | `sdb repl` | `sdb repl` with no batch action |
| One SQL read | `sdb query 'SELECT …'`, `sdb query --file query.sql`, or `sdb query -` for stdin | `sdb repl --query`, `--file`, or piped input |
| Natural-language read | `sdb ask 'request'` or `sdb ask --views-only 'request'` | `sdb repl --ask` and `--ask-views` |
| Explicit mutation | `sdb write 'UPDATE …'` | `sdb repl --write` |
| Offline or connected check | `sdb validate [--connect]` | `sdb repl --validate [--connect]` |
| Model and project inventory | `sdb inspect` | `sdb repl --inspect` |
| Cache maintenance | `sdb cache status`, `sdb cache refresh NAME`, `sdb cache invalidate KEY` | `sdb repl --cache-*` |
| Server and scaffold | `sdb server`, `sdb init` | No change |

`--project-config PATH` should be a shared project option where relevant. Without
it, commands should use `semantic-db.yaml` in the current directory when present.
`--no-project` should remain available for an intentionally empty REPL or query.
Each command should announce an automatically discovered project on stderr and
reject actions that require a project when none is available.

## Option ownership

- `repl`: history and color options, plus read policy for queries entered in the
  session. It should not accept batch action flags.
- `query`: read consistency, cache policy, timeout, read report, and read
  explanation. SQL text, a file, and stdin are mutually exclusive inputs.
- `ask`: provider-backed compilation options, view-only selection, read policy,
  and a compile-only mode. The help must say that compile-only still contacts
  the model.
- `write`: write explanation and timeout. A write cannot be mixed with read
  explanation or report options.
- `validate` and `inspect`: project checks only. `--connect` belongs to
  `validate` and must remain opt-in.
- `cache`: one operation per invocation. Status should not construct source
  providers; refresh may need them.

Keep option names consistent across commands that share a behavior. Use typed
parsers so invalid cache ages and timeouts fail before loading a project. `--help`
for each command should fit on a normal terminal screen where possible.

## Execution semantics to preserve

- Offline validation and inspection must not load credentials or query rows.
- Connected validation checks physical schemas but does not promise row access.
- Query, file, stdin, Ask, and interactive reads should apply the same read
  consistency and cache policy.
- An authored view is loaded from project configuration; interactive `.view` and
  batch `--view` are not part of the new command surface.
- `write` executes only explicitly authored mutation SQL. Explain mode must not
  apply source or destination changes.
- Exit status 2 is for argument and usage errors; execution or connection
  failures remain nonzero with actionable context.

## Migration and verification

There is no published CLI to preserve. Implement the split in one change rather
than keeping the old `repl` batch flags as aliases. Update `sdb init` output,
repository scripts, tests, and generated documentation together. Documentation
outside `docs/generated` requires separate approval under `AGENTS.md`.

Tests should cover each new command's help, valid inputs, project discovery,
missing-project errors, incompatible options, offline behavior without secrets,
cache operations, read policy parity across input forms, write explain safety,
and terminal REPL behavior. Verify the package-maintenance scripts and custom
connector binary, which call the shared CLI frontend.

## Decisions before implementation

1. Choose the exact query syntax: positional SQL versus `--sql`, and whether `-`
   is the only stdin marker.
2. Decide whether Ask and write use dedicated commands as proposed or become
   `sdb query ask` and `sdb query write` subcommands.
3. Decide whether to keep the generic `--dry-run` spelling or use action-specific
   `--plan`, `--compile-only`, and `--explain` options.
4. Decide whether `sdb server` should also discover the current directory's
   project file; it currently requires `--project-config` explicitly.
