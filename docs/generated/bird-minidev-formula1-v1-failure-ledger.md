# BIRD Formula1 Ask baseline failure ledger

This records the complete original 66-task Ask baseline, not a release pass or a merged followup. All failures remain acceptance work. Provider failures are separate from semantic/execution observations. No expected outcomes are waived.

## Source and counts

- Report: `.semantic-eval/bird-luna-v12/run-210719-18db58ba059a4381/report.json`
- SHA-256: `4aa0af473cd81ed390649dc071419c14b75f1766842905a93579c116dcf41d71`
- Recorded artifact digest: `9c2b85d1a3abbb899a31e26a758a60ccc3b66afbd83ef7be2cc9fb5fa4695e62`
- Configured model: `gpt-6-luna`
- Attempts: 66; passes: 8; provider failures: 17; other failed attempts: 41.
- Outcome totals: result 16, unsupported 26, needs_clarification 2, execution_error 5, provider_failure 17. Eight of the result attempts failed comparison.
- Setup, cleanup and artifact errors are null. This report predates terminal/finalized metadata; selection/full_coverage is not proof of a passing release.
- Subsequent dataset portability changes and harness budget changes do not alter this retained baseline. A new full run is needed for new-revision evidence.

## Semantic and execution failures

Diagnostics are exact observed explanations; they do not establish whether the dataset, interpreter, compiler or engine is responsible. Comparator differences retain the exact report evidence.

| Case ID | Outcome | Diagnostic / differences |
|---|---|---|
| `bird.formula1.1001` | `unsupported` | Answering this requires joining qualifying data to the race and circuit records to apply the year and circuit conditions, and to driver records to return the racer's name. Ordinary joins are unsupported. |
| `bird.formula1.1002` | `unsupported` | The request requires finding each driver's earliest race participation and returning the race name associated with that earliest date, then selecting the youngest driver. The catalog’s typed query operations cannot select the race row corresponding to a per-driver minimum date. |
| `bird.formula1.1003` | `unsupported` | The query profile can group accident counts by driver and select the highest-count group, but grouping also returns the driver dimension. It has no operation to return only that group's accident count, as requested. |
| `bird.formula1.1014` | `unsupported` | Finding lap records for circuits in Italy requires joining lapTimes to races and then to circuits; ordinary joins are unsupported. |
| `bird.formula1.846` | `unsupported` | Identifying the five drivers with the slowest Q1 times requires interpreting the qualifying time strings as durations. The available operations cannot parse those strings or compare them chronologically, so sorting the text values would not reliably identify the eliminated drivers. |
| `bird.formula1.847` | `unsupported` | Finding the best qualifying time requires parsing the q2 time string to compare lap durations; that computed expression is unsupported. |
| `bird.formula1.850` | `result` | ["row count: expected 3, actual 76"] |
| `bird.formula1.854` | `result` | ["column count: expected 2, actual 3"] |
| `bird.formula1.857` | `result` | ["row count: expected 1, actual 9"] |
| `bird.formula1.865` | `unsupported` | The catalog defines “oldest” as the minimum birth year, but the available operations cannot extract the year from the Date32 date-of-birth field. |
| `bird.formula1.866` | `unsupported` | The requested lap-time match requires a prefix/pattern comparison on the text time field, which is not a supported predicate operation. |
| `bird.formula1.869` | `result` | ["column count: expected 1, actual 2"] |
| `bird.formula1.877` | `result` | ["column count: expected 2, actual 4"] |
| `bird.formula1.879` | `unsupported` | The fastest-lap-speed field is Utf8, and the catalog provides no authored numeric parsing or conversion for selecting the fastest speed. |
| `bird.formula1.880` | `unsupported` | Fastest-lap speeds are stored as Utf8, and the requested percentage requires parsing and calculating from those values. Those operations are unsupported. |
| `bird.formula1.884` | `unsupported` | Finding the earliest recorded year and month requires deriving year and month from the minimum race date and filtering races to that calendar month. The available operations do not support this computed expression or comparison. |
| `bird.formula1.892` | `needs_clarification` | If multiple drivers share the maximum points scored, should I return all of them or just one? |
| `bird.formula1.894` | `execution_error` | Execution error: Semantic runtime: query decoded byte budget exhausted |
| `bird.formula1.895` | `execution_error` | Execution error: Semantic runtime: query decoded byte budget exhausted |
| `bird.formula1.896` | `needs_clarification` | If there are no Hamilton race-result rows since 2010, should the percentage be null or zero? |
| `bird.formula1.897` | `execution_error` | Execution error: Semantic runtime: query decoded byte budget exhausted |
| `bird.formula1.898` | `unsupported` | The requested age requires a computed expression, which is unsupported by the typed row-query operations. |
| `bird.formula1.902` | `result` | ["row count: expected 15, actual 5"] |
| `bird.formula1.906` | `unsupported` | Answering both parts requires identifying Hamilton’s earliest race using its race date and then retrieving the points from the corresponding result. The catalog supports neither ordering the result rows by a looked-up race date nor the required ordinary join between races and results. |
| `bird.formula1.909` | `unsupported` | The requested percentage requires multiplying the race-count ratio by 100, and that scaling expression is unsupported. |
| `bird.formula1.930` | `execution_error` | Execution error: Semantic runtime: query decoded byte budget exhausted |
| `bird.formula1.931` | `unsupported` | The fastestLapSpeed field is Utf8 text, so finding the maximum numeric lap speed requires parsing or converting text into a numeric value, which is unsupported. |
| `bird.formula1.933` | `execution_error` | Execution error: Semantic runtime: query decoded byte budget exhausted |
| `bird.formula1.940` | `unsupported` | The selected catalog relation supports column projection only; the requested distinct driver count requires an unsupported aggregate. |
| `bird.formula1.944` | `unsupported` | Calculating the percentage requires parsing and combining the race-result time strings, which this query protocol does not support. |
| `bird.formula1.950` | `result` | ["row count: expected 6, actual 11"] |
| `bird.formula1.951` | `unsupported` | The request requires counting constructors after applying a per-constructor condition that exactly two race records have zero points. The available operations cannot count groups that satisfy an aggregate condition. |
| `bird.formula1.955` | `unsupported` | The request requires parsing the results.time text into seconds, which is an unsupported computed expression, and obtaining each result’s year from races requires an ordinary join. |
| `bird.formula1.960` | `unsupported` | The fastestLapSpeed field is Utf8 text, but averaging requires a numeric field. Converting or parsing that text into numbers is an unsupported computed expression. |
| `bird.formula1.962` | `unsupported` | The requested percentage requires multiplying the ratio by 100, and this query protocol has no supported operation for that calculation. |
| `bird.formula1.963` | `unsupported` | The lap-time threshold requires parsing the text duration or converting 02:00.00 to the authored milliseconds unit. Neither operation is supported by the supplied catalog rules. |
| `bird.formula1.967` | `unsupported` | The request requires counting Netherlandic drivers after selecting the three youngest by birth date. The available operations do not support aggregating over rows filtered by a row-level ranking. |
| `bird.formula1.971` | `unsupported` | The catalog defines “oldest” as the minimum birth year, but the available query operations cannot extract a year from the Date32 birth-date field. |
| `bird.formula1.978` | `unsupported` | The request requires counting race occurrences by circuit and returning each circuit’s location and coordinates; the supplied catalog exposes only a column-projection capability, not an executable aggregate capability. |
| `bird.formula1.988` | `unsupported` | The request requires joining pitstops to drivers and computing average pit-stop duration from the Utf8 duration field, which needs unsupported parsing or conversion. |
| `bird.formula1.994` | `result` | ["column 0 type: expected float64, actual utf8", "column 2 type: expected utf8, actual float64"] |

## Provider failures

These are transport/service observations and are not counted as product semantic behavior.

| Case ID | Exact diagnostic | Provider diagnostics |
|---|---|---|
| `bird.formula1.1011` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
| `bird.formula1.861` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
| `bird.formula1.868` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
| `bird.formula1.872` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
| `bird.formula1.875` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
| `bird.formula1.881` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
| `bird.formula1.901` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
| `bird.formula1.904` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
| `bird.formula1.910` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
| `bird.formula1.912` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
| `bird.formula1.928` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
| `bird.formula1.937` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
| `bird.formula1.948` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
| `bird.formula1.954` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
| `bird.formula1.959` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
| `bird.formula1.977` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
| `bird.formula1.981` | provider_failure: Model provider failed; no automatic transport retry | ["model provider returned HTTP 429; check credentials, model, quota, and base URL"] |
