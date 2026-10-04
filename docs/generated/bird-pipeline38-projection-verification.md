# BIRD pipeline 38 projection verification receipt

Frozen BIRD-derived v1.0.3 artifact `51e7b79cf0b89866f8c0bf965d38781a916ab819147d587af035a0bd3143f2bc` passed fresh full SQL 66/66. Report `.semantic-eval/bird-nested-sql/run-403576-18db64b0ca1a3f7b/report.json`, SHA-256 `adddcc4c8ab874ff1324bdabfea15ced83077219c29503105965457826298841`.

The terminal focused pipeline 38 projection run passed 5/6 attempts: all three SQL cases and Ask 897/978 passed. Ask 897 now removes auxiliary columns; Ask 978 now projects the requested venue-count-first shape. Ask 994 has the requested column layout but still uses MAX(points) instead of the required SUM over its date range, producing 43/Red Bull/Austrian instead of 218.5/McLaren/British. That is a genuine aggregation failure, not an annotation or row-order erratum.

Focused report `.semantic-eval/bird-final-project-live/run-409502-18db6523c268be4f/report.json`, SHA-256 `bb1d51bdf1dc75934217fc79cee1c701fc149b6145ee6bc42c3bda098b4f281b`. Complete/finalized are true, full_coverage=false. This filtered run does not establish a full Ask score or complete semantic acceptance. Historical baseline failures remain preserved with their original artifacts. No dataset, question, SQL, gold, tolerance or source-data change accompanies this receipt; both live bundles remain frozen while the separate commerce full Ask run executes.
