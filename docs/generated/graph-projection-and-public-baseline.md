# Graph projection and public-interface baseline

The compiler now supports a pure graph Project operation, selecting and renaming
existing slots without changing row multiplicity. It preserves physical field
metadata, nullability, lineage and authored meanings. Every projection node and
output requires request evidence. Pipeline revision is 36.

A shared grain check now requires each original grouping key exactly once before
retaining its proof under new output IDs. This closes a defect where projecting
one key twice could conceal another missing key and incorrectly authorize fact
composition. Project and all five existing passthrough binders use this check;
true global aggregates retain their empty grouping-key proof.

Parent verification passed 294 tests across 72 targets in semantic-plan,
semantic-compiler, semantic-interpreter and semantic-eval, followed by all-target
Clippy with warnings denied, formatting, and semantic-eval/CLI builds. Logs are
`/tmp/semantic-eval-graph-project-{tests,clippy,build}.log`.

Before Project, a private debug-capture run on unchanged commerce artifacts
returned two real Ask failures. One explicitly identified the missing final
projection. The other compared payer and recipient IDs but retained a Boolean
helper column and included orphan payer order 109. Its captured SQL scanned only
orders. Project fixes the output-shape gap; matched customer lookups are also
needed to implement the requested known identities. The original report is
`.semantic-eval/commerce-roles-debug/run-367883-18db60bd26bf2ef9/report.json`, SHA256
`dea3fd768631e0d1dc27ef7b9d150018547bb797f3f4d78c80b91cc7f1b49b5d`.

Separate frozen pipeline-35 live Luna public-interface baselines passed commerce
103/103 SQL and 9/9 public checks. BIRD passed 66/66 SQL and 8/9 public checks.
Public checks cover SQL CLI, typed Ask CLI, and HTTP compilation followed by
PostgreSQL-wire execution for each artifact's three declared representative
cases. Execution admission budgets were forwarded through the normal public
interfaces. These are representative interface checks, not full Ask acceptance.

Commerce report:
`.semantic-eval/commerce-public-budgets-live/run-370737-18db60dc064ff356/report.json`,
SHA256 `c89a04fb27b0d6dfc3aa1dfc50b261b6f1537d4c8b7369999426e0b1e1d5ef35`.
It is complete, finalized, and covers all SQL cases and declared public checks.

BIRD report:
`.semantic-eval/bird-public-budgets-live/run-374415-18db60f8e074a966/report.json`,
SHA256 `b5449ef25b9855df0eea9a646429606c6182cf342056fd5784dbe9fd059a8fdd`.
It is finalized but incomplete because one public check failed. The circuit
count question 945 returned Unsupported in CLI Ask, claiming the relation allowed
only projection; the same question passed HTTP and SQL. The Ossie executable
import profile uses the misleading label `ossie/column-projection`. That label
describes import handling and is not the compiler's supported-operation list.
This is a genuine remaining interpretation failure; the successful HTTP attempt
does not replace it.
