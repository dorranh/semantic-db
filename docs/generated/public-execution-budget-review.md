# Public execution budget forwarding

This static design review follows the evaluator's truthful rejection of public checks under custom execution limits. It is not implementation or runtime evidence.

## Minimal public controls

Expose optional `--query-max-requests`, `--query-max-decoded-bytes` and `--query-max-remote-bytes` on SQL, typed Ask and server startup, alongside existing `--query-timeout-seconds`. Omitted limits preserve configured engine values. Validate positive values before project/source loading or provider calls, consistently with evaluator capacity bounds.

Use a small pure helper to clone current `engine.query_options()`, apply supplied overrides, validate, and install the resulting options. Avoid importing Clap into engine/runtime libraries or adding a new public abstraction solely for these three flags.

Current CLI `set_timeout` reconstructs options from defaults, which discards configured byte/request/cache policy. Server startup similarly installs default-based options. Preserve existing options in both. Apply the helper before read policies capture query options. Reuse the same effective options for direct typed execution, generated SQL, prepared parameters, cache/materialization paths and the PostgreSQL frontend.

HTTP typed Ask performs compilation; its resulting SQL and parameters execute through the PostgreSQL frontend, which already clones the server engine's query options. Server startup therefore controls the actual HTTP-plus-PG execution contract. The evaluator's current HTTP server launch also omits its requested query timeout; forward timeout with the three capacity limits.

## Harness integration

Forward the report's resolved effective limits to every CLI SQL/Ask command and server launch. Do not forward only manifest values: direct run overrides must retain precedence. Remove the custom-budget rejection guard after all paths support the controls. Preserve expected public-check identities, clock/role scope, typed parameters and retained execution evidence.

Keep output collection limits distinct from execution limits. These controls bound cumulative remote request/byte/decode admission, not final result size or model token usage.

## Verification that matters

- A nondefault request/byte budget survives subsequent timeout configuration; unspecified limits and cache policy remain intact.
- Invalid zero values fail before source loading, model calls or subprocess fixture work.
- The same real query fails under a restrictive limit and returns the unchanged typed gold under sufficient limits through CLI SQL, scripted typed Ask, and HTTP compilation followed by PG execution.
- Use a query requiring actual multiple fetches to verify request limits, not merely command-line parsing; use source data large enough to exercise decode/remote limits.
- All public paths retain and expose the same resolved execution limits as in-process SQL/Ask. Nondefault timeout reaches the launched server.
- Process cleanup, cancellation and partial-report honesty remain unchanged when a budget fails.


## Implemented controls and verified runtime evidence

The frozen implementation adds all three optional controls to SQL, Ask and server startup. It validates before project/provider loading, clones existing query options, preserves cache/materialization policy and untouched limits, and forwards the report's exact effective timeout and three limits to public subprocesses. Typed CLI and HTTP compilation inherit the engine timeout. The former custom-budget refusal guard has been removed after forwarding was implemented.

Parent verification reports 98 tests across 24 targets passed (`public-budgets-tests.log`), with all-targets Clippy for three packages and the CLI build passing (`public-budgets-clippy.log`, `public-budgets-build.log`). These commands were run by the parent, not this reviewer.

The retained controlled runtime probe `.semantic-eval/bird-cli-budget-probe/summary.json` reports all six checks passed for **bird.formula1.894**, whose real lap-time source has 400,524 rows. CLI SQL, scripted typed Ask, and HTTP compilation followed by PostgreSQL execution each fail under `max_requests=1` and match the unchanged gold under `max_requests=1024`. CLI failures retain the exact budget diagnostic. The HTTP/PG low-cap path returns the sanitized `source query failed; no complete result` after successful compilation; the matched high-cap execution provides the controlled comparison. This verifies public request-budget forwarding with real multiple remote fetches. It is controlled inference/execution evidence, not live Luna acceptance proof or a separate byte-limit runtime test.

The first two failed probe attempts remain preserved as attempts 1 and 2; the successful third attempt has retained runtime logs. Parent-owned Compose cleanup finished with exit 0, recorded in `/tmp/semantic-eval-budget-probe-cleanup.log`.
