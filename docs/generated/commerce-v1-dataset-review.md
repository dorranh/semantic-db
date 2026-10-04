# commerce-v1 independent dataset review

Reviewer checkpoint: first ten paired cases, 2026-10-04. This is an intermediate
review, not acceptance or release approval.

Reviewed SHA-256 revisions after initial corrections:

| Artifact | SHA-256 |
| --- | --- |
| cases.json | 00a7036aac5981657af2e4c4f0e9c4c122abca676fef2d7bc81eed405f9a2a20 |
| manifest.json | 15092ec72e5dd45750df7e0a8bf691203aa59669076d9fd9e46eb86c2efa154d |
| model.ossie.yaml | cdeaf762b6012b580f5fd41385461dde00601e53183d270d4f5972905ac54395 |
| data/canonical.json | 5abdf036e7d38a4ed5e75cecf9dff0ccd903193cc58323fc0d007c6b704f0adc |

The reviewer independently reconstructed the initial ten answers using Python
iteration, joins by dictionary lookup, predicate evaluation, and integer sums
over canonical.json. This did not execute the authoring script, its SQLite
reference SQL, Semantic DB, or product outputs. All ten gold row lists matched.
Completed gross is 11900 CHF cents; qualifying refunds total 1600; net is 10300.
Ada's historical regions are DE for order101, CH for102 and107, under half-open
validity. UTF-8/apostrophe names, empty notes versus null, unmatched product999,
and billing customer99 are represented correctly. CSV null matching was checked
with the configured pattern against the actual parsed token.

Initial findings and disposition:

| Finding | Disposition |
| --- | --- |
| distinguishing_rows strings disagree with the generic vector contract | Rechecked fixed arrays |
| Completed-only shipping case has no missing-recipient witness | Rechecked revised all-orders case, including order105 null |
| Manifest does not bind schema/canonical files and has no fixture checks | Rechecked both paths and nine row-count fixtures; schema/key/digest enforcement still requires harness inspection |
| Ada historical case has no missing-history witness | Analyst plans Chen104 in the full historical bank; pending |
| Initial manifest requires100 pairs while only10 exist | Intentional unfinished-suite target; it cannot validate as a complete suite yet |
| Active subscriptions, net sales, weighted unit price, history, fiscal and rounding policy live chiefly in model instructions | Require exact units/grains/edge definitions and supported metrics/concepts; complex prompt-authored rules are acceptable desired semantics when no structured importer profile exists, with that product gap documented |
| Fiscal quarters assigned solely by comparison with April2024 | Dec31-2023 and Oct27-2024 should both be Q3 for an April-start fiscal year; correction requested |

Compose and seed were inspected statically. Both services pin postgres16.6-bookworm;
host publication is dynamic and loopback-only; the one-shot psql bootstrap mounts
bundle-relative seed and canonical CSV exports, uses ON_ERROR_STOP, and resets
the dedicated schema transactionally. The reviewer has not executed Docker,
database COPY, provider loading, Cargo, SQL acceptance, Ask acceptance, or public
interfaces. No schema-validation success is claimed from this review: Python
jsonschema was unavailable; the analyst separately reported core schema checks.

Harness review is ongoing. Early concrete findings sent to the implementer:
nonfinite Arrow floats must not serialize to JSON null; semantic comparison must
handle exact temporal-unit normalization without conflating wall timestamps and
instants; offline expected validation should enforce integer widths, declared
temporal resolution, and timezone validity. Release completeness needs a mandatory
companion contract, and host calendar/scope must not silently disappear.

## Full-bank checkpoint

Reviewed case revision
`65fc02e262ccdbf04c19fdaae5a4666b5e3bb5d2794c25bf14cd2c9fb59f4444`
and canonical revision
`44c4481f6280b0dfe14ba67d3a9d9a410f6402ad71e8257bed236489a2b9cb50`.
An isolated copied bundle was used to avoid mixed revisions during author edits.
Subsequent case/model edits require another checkpoint.

The bank has exactly100 result pairs and15 companions: five clarification,
four rejected, three unsupported, three execution-error outcomes. All planned
primary counts match. Inspection of actual relation references confirms53 pairs
combine CSV and PostgreSQL relations; tags agree with these references.

A fresh SQLite database was reconstructed directly from canonical JSON without
running the authoring helper. Reference SQL was executed with only the ANSI DATE
literal syntax adapted. Decimal cases were computed separately with Python
Decimal and ROUND_HALF_UP, rather than relying on SQLite affinity. All100
expected row lists matched. This independently checks fixture/result agreement;
it does not itself prove that every SQL expression matches its question.
Questions, SQL, and model rules were also read together.

Separate Python iteration checked buyer net sales, product net sales, line refund
aggregation and weighted numerator/denominator. Exact monetary checks include:

| Measure | Independent answer |
| --- | --- |
| Completed order average | 11900/8 =1487.50 cents |
| Tea weighted completed unit price | 3400/3 =1133.333333 cents, rounded to six places |
| Mug weighted completed unit price | 6000/4 =1500.000000 cents |
| Book weighted completed unit price | 2000/1 =2000.000000 cents |
| Order105 weighted price | null for denominator zero |
| 100 EUR at0.950050 CHF/unit | 95.005 rounds to95.01 CHF |
| -100 EUR at0.950050 CHF/unit | -95.005 rounds to-95.01 CHF |

Canonical CSV cells, null tokens, row counts, and declared nonempty key uniqueness
were checked separately. Diagnostic lookup relations deliberately declare no
keys. Fiscal-quarter calculations now agree with the April start, and an authored
May period distinguishes zero-filled missing sales. Chen104 distinguishes missing
history from current-region substitution. Monetary float expectations were
replaced by exact decimal results; only the dimensionless refund fraction uses
tolerance. Error companion expectations now identify cast/zero/overflow causes.

Pending artifact finding at this checkpoint: despite a reported correction,
both copied model revision3135dac... and live revisionb58849... still contained
the unsupported `governed_rules_v1` object under `ai_context`. The importer accepts
only instructions, synonyms and examples. The analyst was asked to fix the
generator and preserve these detailed desired definitions in supported authored
text. This is an artifact import defect; broader structured governed rule support
is a separate product gap and does not justify removing the desired semantics.

Pending harness findings include complete physical schema/canonical enforcement,
provider injection and scripted typed Ask tests, runtime error classification,
preserved compilation accounting after execution failures, and mandatory public
interface execution. Unique report paths and an outer setup deadline appeared in
revisions after the initial findings; runtime verification belongs to the parent.

## Final dataset revision disposition

Business definition, question/SQL equivalence and gold arithmetic review is
complete for the revisions below. A later actual loader run exposed an identity
extension placement defect, described below; artifact integration disposition is
therefore pending its correction and recheck. This does not certify product
execution, rich structured profile support, or release acceptance.

| Artifact | SHA-256 |
| --- | --- |
| cases.json | fe1c4ef5b23c850fad2b655b1626468397569340c828266c2399f418ff8b7226 |
| model.ossie.yaml | 8626ab9b023c5fac42ad61811a40f89bfc2b5e94a276f13d3874f9affadfca92 |
| manifest.json | ee9b8c73c582217b768ff87cad8d2fd338f1566507d544a19d66d93a1100e20e |
| data/canonical.json | 44c4481f6280b0dfe14ba67d3a9d9a410f6402ad71e8257bed236489a2b9cb50 |
| schemas.json | 1eb6b08028ae6027315bf92ac8a374e188f41265ed7c1052453f1cd21fe975c7 |
| Expected file tree, harness framing | 47a7b80fa57685a85dd14a63ed72418b9feff1c92d56eb02e511bd7a370dc247 |
| Entire bundle, harness framing | a93087408df79c6c6b04d09242f0e4dca94047e411fc68af70df9d38b7ac3609 |

Harness framing means sorted relative file paths, path bytes followed by NUL,
little-endian u64 content length, then exact content bytes. The analyst's
independent bundle digest uses different framing and must not be confused with
this value.

The final recheck confirmed the unsupported AI property is actually removed
from disk and generator output: ai_context has only instructions. Detailed
governed rules remain in that supported text. Seven metrics and five business
concepts use current executable contracts; complex history/fiscal/ratio policies
remain explicit desired semantics without fabricated executable support. Recurring
monthly cents now carry CHF_cent_per_month consistently with their metric.

Only five cases changed after the complete monetary recomputation: one Boolean
projection was added and independently reconstructed as [6,-1,false], and four
additional cases now explicitly assert physical types. A later unordered UNION
ALL wording/order-policy change was rechecked as a bag: customer IDs1,3,5,4,99
occur4,3,2,1,1 times respectively. Monetary case reasoning now correctly identifies
the independent Decimal method. Gold values otherwise remain unchanged from the
fully recomputed100-case revision. All15 companion requests were reviewed against
authored ambiguity, scope, missing data, corrupted lookup and arithmetic policies.

The selected fixture provides deliberate fanout, role, null, current/as-of,
half-open boundary, positive/negative rounding, spring/autumn DST, fractional
temporal, empty schema, rank ties, zero-denominator and bag multiplicity witnesses.
It does not establish model generalization outside these authored questions.

The parent's subsequent real Docker run bootstrapped successfully but model
loading rejected entity_identity extensions attached to datasets. The current
executable importer handles identity extensions at semantic_model.custom_extensions
with an explicit dataset member. The reviewer had checked the identity contract
contents but missed their executable scope. This is an artifact placement defect,
not a gold defect or a reason to delete identity declarations. The analyst is
moving the existing contracts and fixing generation; another model/bundle digest
checkpoint is required. No model import success is claimed for the revisions in
the preceding table.

Identity placement correction was independently rechecked at model revision
`5209f629bf95b4bb311710e2ab09c0a7ae7f0da3ec9288bd717a8fc3d691307a`:
all nine identities now live in model custom_extensions, retain their exact
dataset/key mappings, and no dataset custom_extensions remain. Diagnostic lookup
relations remain unkeyed. Case/gold revisions are unchanged. The corrected
bundle's harness-framed digest is
`b464390e41f3ca5af118cb46bd95d453c4b5bc0492857c31056bf0d3dae4b338`.
Static placement findings are resolved; actual loading remains the parent's
verification responsibility.

Schema/provenance refinement rechecked at schema digest
`bbbe5ec7e57cbd71495f59d252acd28aec8c292b8ba2bef7bd3c9723acce4636`
and manifest digest
`a978be75a3e66149ff77bde43dc6fd5f80a5d1fcfec6c44adc17b5e72c2eba07`.
`nullable` remains a canonical data-domain invariant. The new `physical_nullable`
is true for every CSV column because the provider does not attest CSV NOT NULL;
PostgreSQL values follow the explicit DDL. Declared canonical keys remain data
checks, not invented provider uniqueness evidence. Public case IDs select an
empty-string parameter, a cross-source billing join and fixed-clock previous-month
interpretation. This bundle's harness-framed digest is
`565bc5656bca6db98162e77301bc367113eee2b5a650f09bf5eb936170b2f35d`.

Later harness source now verifies canonical records, chosen physical types and
column names, compares declared physical nullability, and checks actual imported
project connectors against authored source placement. Temporal precision and
UTC-label validation were added. ModelProvider injection and scripted row/graph
paths exist; actual test outcomes and live/provider/Docker execution evidence
are owned by the parent. Public interface capture and selected-case execution
were still being revised when this source review checkpoint was recorded.

Final allocation counterexample correction was independently rechecked at case
revision `3e32a92b85b11da6a8b53f1a70169f12c46a0badbc14fe825be1057549a6b4cc`
and model revision
`ac0966eb2b8a44fd23947550af0ca55b16a69a8d5547f7278cc890c4f148ac0c`.
The previous gross allocation equaled the raw line values and therefore did not
distinguish a plausible wrong projection. The revised question explicitly allocates
order101's net2000 cents after all500 refund cents, in proportion to original
line amounts1000/2500 and1500/2500. Independent Fraction arithmetic gives exact
integer allocations800 and1200; gold, SQL and authored allocation rules agree.
No canonical data changed. The latest harness-framed bundle digest is
`e0b60f6905392d1f322c5925364a7865c9c80a547e8538406d8e7adf2a50657d`.
This is the latest reviewed artifact revision; all earlier digests are historical
checkpoints. No substantive business/oracle finding remains open after this
correction. Product execution and release acceptance remain separate verification.

## Harness source review disposition

The reviewer read the generic manifest/load/compare/run/lifecycle/public-interface
code and focused tests without executing Cargo. Pure normalization remains separate
from effects, and the public provider injection path allows deterministic row,
graph, clarification and transport tests without exposing the oracle to the model.
Bag matching preserves multiplicity and uses complete bipartite matching when
float tolerance is nontransitive. Decimal normalization remains exact; temporal
units normalize losslessly while local timestamps and UTC instants stay distinct.
Nonfinite floats are rejected. Chosen physical widths, nullability metadata and
source connector placement now have explicit fixture verification.

Compile records survive accepted artifact execution errors. Provider failures,
timeouts, cancellation and output-budget limits cannot count as successful semantic
outcomes. Unimplemented reference SQL authorization returns a separate
harness_unsupported incomplete outcome for scoped cases. Reports preserve full
requested attempt cohorts after setup failure and use unique output directories.
Lifecycle startup/bootstrap/cleanup use structured Compose arguments, with bounded
waits and captured logs; cleanup fallback remains best effort on abrupt process
termination. A release run requires full SQL/Ask automatic context, at least three
repetitions, fresh managed setup and required public checks. Missing credentials
remain nonpasses, not a waived baseline.

The revised public adapter executes all three authored case IDs through CLI SQL,
CLI typed Ask and HTTP typed compilation followed by server PostgreSQL execution.
It checks retained clock/timezone, forwards selected scalar/date/timestamp bound
values, and retains actual result schemas/rows, compilation evidence and sanitized
CLI/server logs. The generic public scope adapter explicitly fails rather than
silently discarding host scope. The selected live smoke requests do not force a
graph artifact; deterministic graph tests cover the in-process path. No public
runtime success is inferred from static code inspection.

Artifact digests are checked before setup and after execution so later file edits
cannot inherit an earlier revision's successful report. One edge remained under
repair at this checkpoint: default output inside the dataset working directory
would modify the hashed bundle itself. The implementation must place default run
outputs outside that bundle, or reject explicit overlap before starting effects.

Source checkpoint SHA-256 values (later formatting or fixes supersede these):

| Source | SHA-256 |
| --- | --- |
| compare.rs | 97059d482ca20914c950e34d0f066409f704dc68389ed0cdc1e2138c881e6afb |
| contract.rs | 47e61858f47d33a0a05763742cb1c36e061753d89cb56c091e449e2786a77551 |
| runner.rs | 96793561508436a17888e64c8811a7ffc741def4003989b053792f1e6669c8b8 |
| lifecycle.rs | ec7fe7c448469462c08523ff1a55d5d681051301ac2008963de50a2a57bdc770 |
| public.rs | 918af04a545f6da6585f27ef190a64a5739b4a45a5948ee07e0012873b8650b5 |

Actual Cargo, Docker, copied-bundle, SQL acceptance, live Ask and public smoke
outcomes must be taken from the parent's retained reports. This review neither
ran those commands nor certifies a zero-failure release.

The output-location edge was subsequently rechecked in source: default
`.semantic-eval` requested from within a dataset relocates to the dataset parent's
output directory, while explicit output paths inside the bundle are rejected
before directory creation or setup. A focused explicit-overlap regression is
present. The reviewer did not run that test; the parent owns its execution.

Final read-only CI/lifecycle check found no new CI failure-concealment issue:
SQL and Ask run independently, the job is informational, summaries/artifacts run
after failures, and a final step preserves unsuccessful step outcomes. One generic
filter completeness bug was reported: SQL-only selection of a companion with no
reference SQL could skip every selected case and return success for an empty
attempt cohort. The implementation must reject that explicit unavailable cohort
or report it incomplete before it can claim a passing development run. Full
release SQL+Ask coverage is not bypassed by this specific edge.

The local PostgreSQL endpoint template subsequently added explicit
`?sslmode=disable`, matching the dedicated non-TLS fixture and the connector's
configuration requirement. Rechecked manifest digest:
`a799b16b9ef7e4bdebbba399fbc60ce6663682d20f9107e2e6402aad47188f52`.
Latest harness-framed bundle digest:
`447feb964579ef4e49fd843474f81433a53d1bc07654e0ec722b014a4cbfe126`.
Case/model/canonical/gold revisions are unchanged from the net-allocation review.

The zero-attempt SQL selection correction was rechecked in source: SQL-only
execution rejects an empty executable cohort and an explicit selection containing
any case without reference SQL, before output creation or setup. A focused
regression is present. This resolves the final filter completeness finding.

Actual loading then exposed a CSV identity-nullability gap. The revised project
explicitly declares `non_nullable_columns` for the physical CSV fields whose
authored data-domain contracts are nonnull. Static alignment was checked for all
six CSV sources, including diagnostic lookups: no nullable note/valid_to field is
misdeclared, and physical_nullable is false exactly for constrained fields.
Project digest:
`b60442a09c657809c17678e45e0e8b790ea5be39e939e18106e134698b4b9724`;
schema digest:
`d95c4068252857a304419ce8658a826192a5bd771da190787e722295977f7889`;
latest harness-framed bundle digest:
`b7f30c2e568dd1640a33f55bb0c0734fb709badf0898846de6fa5e01d01e367a`.
Case/model/gold revisions are unchanged.

The proposed source implementation validates unique/nonempty physical names,
rejects missing columns and non-CSV use, and passes nonnull schema fields to the
actual Arrow CSV reader. Independent dependency-source inspection confirmed that
Arrow RecordBatch validation rejects materialized nulls in such fields. Because
projected reads and optimized counts can avoid materializing a key, the parent
requested an opt-in streamed preflight of all constrained columns before publishing
the provider. This is necessary to claim existing-file enforcement at load rather
than only when a particular field is read. It does not establish CSV uniqueness;
canonical key uniqueness remains an independently checked data invariant.

The requested preflight was then independently rechecked in source: when the
option is present, the guided DataFrame projects every constrained physical
column and consumes execute_stream batches completely before returning its
provider. It does not use a COUNT aggregate that an optimizer could elide. Tests
place nulls in each constrained component beyond the inference sample and require
load failure before an Engine exists. Static strictness findings are resolved;
the parent owns test execution. This establishes an observed invariant at load,
not a snapshot guarantee against subsequent file mutation.

The first real decoder tests subsequently revealed a dependency behavior missed
by static schema inspection: DataFusion's CSV path applied null_regex during
inference but did not forward it to the actual Arrow decoder. This invalidated the
earlier inference that merely passing the guided schema through DataFusion proved
the selected null-token contract. No source test success was claimed by the reviewer.

The corrected source adapter was independently read after formatting. It uses
Arrow CSV ReaderBuilder directly with the configured null regex, header,
delimiter, quote and escape options; the same compression conversion and sorted
file enumeration as the existing JSON adapter; a one-slot batch channel; and
complete-schema decoding before StreamingTable projection/count. Nonnullable
preflight fully consumes the adapter before publishing a provider. Dropping the
receiver releases blocking_send and stops decoding at the next batch boundary.
Memory is bounded by queued batch count, not by an invented maximum cell size.

The reviewer found and rechecked a bypass correction: regex-only inference must
use the new decoder even without physical overrides or nonnullable columns.
Regressions cover that path, null versus empty and quoted tokens, gzip file and
directory reads, headerless/custom dialect options, malformed regex, and nulls
beyond the inference sample in each constrained component. The parent reported
19 source file tests passing; the reviewer did not execute them.

Final reviewed source digests:

| Source | SHA-256 |
| --- | --- |
| semantic-sources/src/files.rs | 83432ab8de2a09c5222c8e85e698728ec920e8e712bf397a00d3b82d56321a2b |
| semantic-sources/src/files/csv.rs | b1dc66aeb37dfa6229fe0d4c894a2006ad2db7a8a4dbb95307440b6a5e47ed81 |
| semantic-sources/src/files/json.rs | f9f3163d8cf673dfcf98eb435837cd436f3c5b35c2f2a051b700088f5e6445a0 |

No further material static adapter finding remains open at this revision. Real
commerce loading and acceptance still require the parent's execution evidence.

The parent later reported full real SQL execution of103 attempts: setup and
cleanup succeeded,98 passed, four ranking outputs initially failed because the
harness lacked UInt64 output conversion, and explicit arithmetic overflow returned
a wrapped result. The reviewer checked the generic unsigned output correction:
Arrow values are preserved directly as decimal strings with actual unsigned
schemas, integer normalization uses i128, and declared signed/unsigned bounds are
validated independently. Equal mathematical signed/unsigned values compare under
the default semantic contract; asserted physical types remain exact. u64::MAX is
retained without a signed cast or float conversion. This output support does not
claim new unsigned source coverage in commerce's initial type matrix.

The overflow result remains a genuine product failure against the unchanged
execution-error oracle. A complete run may report complete=true with failing
cases; it is not a zero-failure acceptance or release success.

Final provider diagnostic preservation was read independently: each sequential
interface attempt clears an Arc/Mutex error list, provider wrappers capture safe
ProviderError Display messages on failure, and that attempt drains its list into
CaseReport.provider_errors. The wrapper adds no retries. HTTP status survives
typed diagnostic abstraction, while provider non-success handling deliberately
omits response bodies; credential configuration has no Debug representation.
Fixed messages plus status codes are recorded, rather than arbitrary server text.
Default envelope forwarding does not double-capture an inner complete failure.
No cross-case attribution or credential/body disclosure path was found in this
serial runner. A persisted HTTP429 regression is present.

The parent reports final full SQL execution102/103 passing, with only the explicit
overflow requirement failing, and92 focused tests plus warning-free Clippy. Those
are parent execution reports, not commands run by the reviewer. Full live Ask
diagnostic execution was still in progress at this checkpoint. The dataset and
gold revisions have not changed; no passing release or live generalization claim
is established by this review.

Final diagnostic/comparator source checkpoint:

| Source | SHA-256 |
| --- | --- |
| semantic-eval/src/runner.rs | 94d86bc399096ede450655013e87e125d9b294b45988ec36ab12cb37d22f2234 |
| semantic-eval/src/compare.rs | a2fc2a3d7ccbb2f0e47538398daaca45db8f3de1b8462a3fffa7cef142a8eac8 |
| semantic-interpreter/src/provider.rs | bd529f1ee90a37da77d04309683592b24479abe12718f672a0d6eaf4c7ff8b03 |
