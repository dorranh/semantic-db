# Read-only audit of globally broadcast BIRD evidence

Both artifacts remain frozen. Original selected_original.json and upstream annotation/provenance files are immutable. No model, question, gold, SQL or source change accompanies this audit. The current author generator collects each selected task's evidence into one model instructions string; this removes the original question ID/relation/grain scope. Model error on a clear request is not reclassified as question ambiguity.

## Exact source origins and incompatible broad readings

| Source question ID | Original evidence | Original relation/grain meaning |
| --- | --- | --- |
| 892 | “the most points scored refers to max(points); Full name of the driver refers to drivers.forename and drivers.surname;” | Maximum observed driverStandings points snapshot, then driver name; not summing event contributions. |
| 897 | “Full name of the driver refers to drivers.forename and drivers.surname; the most winning refers to MAX(COUNT(wins)); average point scores refers to MAX(points);” | DriverStandings rows with wins>=1; rank by COUNT(wins), report maximum snapshot points per driver. Preserve this unusual explicitly supplied evidence rather than replacing it with conventional total wins. |
| 948 | “maximum points = MAX(points); British is a nationality” | Maximum constructorStandings points snapshot among British constructors. |
| 994 | “Monaco Grand Priz refers to the race; race in year between 1980 and 2010” | Source SQL aggregates constructorResults event points by constructor with SUM across the Monaco date range, then ranks those totals. Its question asks the multi-event contribution, not largest single event. |
| 901 | “in September 2005 refers to MONTH(date) = 9 and YEAR(date) = 2005” | Task-specific date scope for races, unrelated to points aggregation. Must not become a universal race filter. |

Broadcasting “most points = MAX(points)” without scope can encourage applying another task's snapshot instruction to 994. That conflicts with 994's original reference task and explicit multi-race domain grain. This is a contextual-authoring hazard, not proof that the broadcast caused a particular model output or that 994 NL needs an erratum. No universal most→SUM rule is correct either: maximum snapshots and cumulative contributions are different. Summing driver/constructor standings across race snapshots generally repeats cumulative balances.

Other examples have the same structural hazard: all task-specific race names, years, nationalities, COUNT(wins) and duration thresholds appear globally. Generic definitions such as forename/surname full-name fields or lat/lng coordinate fields can be authored as genuine schema meanings. Specific predicates and arithmetic interpretations belong to their originating task, with provenance, not unconditional model defaults.

## Current supported context boundary

Current semantic-eval Context exposes reference_time, timezone, allowed_relations and Gregorian calendar. Case metadata has question, SQL, expected, requirements and oracle notes, but no supported per-case official-evidence input field. Requirements/oracle notes are evaluation metadata and must not be smuggled into model prompts; reference SQL and expected answers must never be supplied. Existing interpreter grounding evidence is model-output justification, not a substitute for a bounded caller-supplied task-knowledge API. Therefore genuine per-case evidence scoping requires a reviewed generic context contract and consistent SQL/Ask/public forwarding, not an invented artifact field.

## Proposed faithful author/context repair

Retain immutable upstream evidence and original questions. Add a generic bounded provenance-pinned task-knowledge input, selected only from that case's original canonical evidence or explicitly reviewed variant evidence. Distinguish task evidence from request text so strict request-span validation cannot pretend added evidence was user wording. Validate size, schema, provenance and allowed scope; do not include SQL, gold, row results or answer-oriented oracle reasoning. Original PG/SQLite evidence differences remain explicitly adjudicated, not silently mixed.

Separately author truthful relation/measure grain: constructorResults points are race-event contributions suitable for additive aggregation over selected events, while standings points are season-to-date snapshots at race checkpoints. A metric needs correct dimensions, snapshot or additive state and empty/unit contracts; bare field annotations must not invent executable authority. Confirm source rows/table descriptions before changing inaccurate per-race standings text. This generic grain distinction can help faithful proposal selection without specifying any case answer or a universal arithmetic default.

Independent reviewer should verify source attribution above, context API absence, clear 994 meaning and proposed grain contracts. Parent decides the next bounded API/author patch after active runs end. Existing incorrect MAX result remains a real failure against unchanged reference SQL/gold. No dataset edits or Cargo were performed.
