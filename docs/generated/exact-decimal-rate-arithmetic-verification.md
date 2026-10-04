# Exact Decimal rate arithmetic: first-slice verification receipt

The first source slice implements the checked Decimal multiplication/quantization engine UDF, catalog exact-rate contract and typed static SemanticDataCondition. The execution profile is LocalOnly version 13. Compiler wiring, Ossie importing, exact-date lookup execution and public runtime adapters are not yet implemented. This receipt is arithmetic/catalog evidence, not live FX acceptance.

Parent ran the broad catalog/runtime/engine check: 140 passing tests across 41 nonzero targets, with 45 successful test-result records and zero failures. Log `/tmp/semantic-eval-fx-arithmetic-catalog-tests.log`. Analyst independently counted those records from the log without running Cargo. Focused arithmetic checks passed three tests across two nonzero targets: `/tmp/semantic-eval-fx-arithmetic-focused.log`.

The first compile attempt had a 44-versus-45 list mismatch; a later existing count test had another 44-versus-45 mismatch. Both were corrected. Retained logs preserve those earlier failures; the successful reruns above do not imply the first attempts passed. Reviewer independently verified corrected bypass validation and catalog checks.

Parent confirmed terminal Clippy session 77783 exited 0 for `cargo clippy -p semantic-catalog -p semantic-runtime -p semantic-engine --all-targets --locked -- -D warnings`, logged in `/tmp/semantic-eval-fx-arithmetic-clippy.log`. The build used dev/test debug info 0, jobs 2 and incremental compilation disabled. Formatting check `cargo fmt --all --check` also exited 0. Core source and this receipt are ready for the separate core commit; compiler/wire integration follows as the next slice. Generic prompt changes and draft proposals are separate.

No live Decimal FX SQL/Ask proof exists for this new profile. No dataset artifact, expected metadata, numeric gold, SQL, tolerance or source data changed. The stronger execution-stage missing_exact_rate/non_unique_rate companions remain a planned migration pending genuine lookup/runtime support, independent review and parent authorization. Existing row-wise and future one-match capabilities must not be conflated.
