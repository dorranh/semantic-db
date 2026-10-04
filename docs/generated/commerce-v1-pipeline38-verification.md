# Commerce v1.0.3 pipeline 38 verification receipt

Frozen artifact `da95e740c30ebcc8325417ef8682f633809a0b3f6e861a5345f3435c6aff4c7d` received fresh full SQL 103/103 PASS. Complete, finalized and full_coverage are true; setup, cleanup and artifact errors are null. Report `.semantic-eval/commerce-month-order-sql/run-403703-18db64b0f0704590/report.json`, SHA-256 `d2f0e871b1d5d32789691e53b83b2870f7cf531cf9bfd0a72311d27747180fbf`.

The focused pipeline 38 SQL+Ask run passed 13/14 attempts across seven paired cases: all seven SQL attempts and six Ask attempts. Complete/finalized are true, full_coverage=false. This is filtered evidence, not a full Ask score. Report `.semantic-eval/commerce-nested-calendar/run-403371-18db64b067083e59/report.json`, SHA-256 `32e90e40956099f1730965e35cc64f0a02c0038a0dc3bfb8b10359d0ab762fd2`.

| Case | Ask outcome |
| --- | --- |
| calendar.monthly | PASS |
| calendar.missing_month | PASS |
| absence.no_subscriptions | PASS |
| absence.unsold_products | PASS |
| absence.completed_buyers | PASS |
| existence.completed_buyers | PASS |
| concept.zero_active | FAIL: expected one column, actual two; extra client_id remains. |

The calendar results verify the genuine authored lookup/group/composition route against unchanged golds. Product absence verifies the minimal inverse relationship with nested qualification in this focused run. The zero-active projection failure remains a real acceptance failure; no question/gold/tolerance change is used to waive it. Historical full-suite failures remain recorded with their original artifact digests. No dataset change accompanies this receipt.
