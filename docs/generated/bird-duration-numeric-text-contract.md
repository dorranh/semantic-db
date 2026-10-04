# Read-only duration and numeric-text fidelity audit

Frozen BIRD artifact unchanged. Source observations are recorded in bird-duration-numeric-text-source-audit.json, independently read from preserved formula_1.sqlite. This proposal supplies no model answer hints or computed physical columns.

## Actual representations and bounded profiles

lapTimes.time has 400524 nonnull values: 400481 two-component minute:second strings and 43 three-component hour:minute:second strings, all three decimal digits. Independently Decimal-parsing every row agrees exactly with its milliseconds field, zero disagreements. pitStops.duration has 5815 nonnull three-decimal values, 5701 seconds-only and 114 minute:second strings; all 5815 independently agree with milliseconds. These two actual rowwise equivalences can support an authored guarded equivalent-representation contract; no blanket equivalence is claimed for results or qualifying fields.

qualifying q1 has 109 nulls and no empty strings; one nonnull value uses two fractional digits. q2 has 3577 nulls and 23 empty strings; q3 has 4935 nulls and 28 empty strings. Those empty values are actual stored strings, not CSV null tokens. A q2 anomaly at race 891/driver 3 is 1:48:552, which fails ordinary subordinate-seconds range. Do not silently reinterpret its second colon as a decimal point or pretend every qualifying timing is valid. The specific 846/847 populations do not include that anomaly; generic parsing must define failure on malformed nonmissing values under the selected read population, rather than eagerly failing on irrelevant rows.

results.fastestLapTime has 4994 nonnull minute:second values with three decimals; 18185 nulls. results.fastestLapSpeed has the same counts, nonnull ordinary decimal text with three fractional digits. Propose a guarded finite numeric decimal-to-measure profile for that actual speed field, with authored measured speed unit verified against upstream table descriptions; no money semantics or float tolerance for currency. Do not assume every Utf8 field is numeric.

results.time mixes champion elapsed race durations and trailing-driver offsets: 948 three-component, 1678 two-component and 3163 one-component nonnull strings; 4839 start with plus. Some offsets contain suffixes (+8.959 sec, +6.361s), and +1:05.0421 has four fractional digits. A generic digit-count statistic is not a valid grammar validator. Separate elapsed duration, signed offset and optional explicitly authored suffix grammars. Never compare elapsed winners and offsets as if they were the same physical quantity, strip a plus to infer whole elapsed time, or impose milliseconds truncation on finer fractions without a reviewed policy.

Minimal elapsed parse profile accepts unsigned S[.fraction], M:SS[.fraction], H:MM:SS[.fraction], exact decimal fraction with a bounded declared maximum scale; seconds and subordinate minutes in [0,60), initial magnitude nonnegative, no whitespace/suffix/sign unless separately declared. Nulls propagate; empty text requires a field-specific missing policy (qualifying empty timing as unavailable observation is plausible but must be explicitly reviewed). Malformed values fail with a typed diagnostic, not zero, and are guarded only under the query's selected population. Output can be exact integer milliseconds only when precision is losslessly representable; otherwise use exact Decimal seconds at declared scale. A separate duration-to-threshold unit conversion preserves exact equality and strict inequality.

## Existing case obligations

| Case | Reference semantics and boundaries |
| --- | --- |
| 846 | Race 20, q1 descending lexical reference and five rows; verify numeric equivalence on selected valid timing strings before changing implementation strategy. Do not infer an added missing q2/q3 elimination rule. |
| 847 | Race 19 q2 nonnull minimum, driver identity tie-break. Excludes unknown timings. Its lexical reference is equivalent only on the actual selected formatting population, not all qualifying text. |
| 879 | Numeric maximum fastestLapSpeed then nationality; ties remain original reference selection unless independently adjudicated. No lexical speed sort. |
| 880 | Paul di Resta race 853 versus 854 speed difference, denominator 853 and percentage factor 100, preserving supplied evidence. This is speed arithmetic, not elapsed finishing-time parsing. |
| 955 | Champion positionOrder 1 per race, years before 1975, mean elapsed seconds per year. Preserve current reviewed reference's champion parsing/zero branch meaning; audit selected champion rows separately before proposing a universal parser replacement. |
| 960 | Mean numeric fastestLapSpeed for the named 2009 event, measurement Float64 contract. |
| 963 | Distinct French driver identity with observed lap duration strictly less than 02:00.00 = 120 seconds = 120000 milliseconds. The independently proven lapTimes equivalence may authorize existing milliseconds, preserving strict threshold and driver grain. |
| 988 | German drivers born inclusive 1980–1985, mean observed pit-stop duration per driver, top three with driverId tie-break. Seconds-only/minute formats parse uniformly; no birthday-adjusted age substitution. |
| 1011 | Each driver's minimum observed lap duration, shortest twenty driver identities, driverId tie-break; project names only. Multiple laps do not create duplicate identity rows. |
| 1014 | Per Italian circuit minimum observed nonnull fastest-lap duration, then date/raceId/resultId deterministic ties; retain original timing text and circuit identity. Circuits without observations do not invent records. |

Population and tie obligations are separate from parse grammar. A parse profile does not grant multihop grouping, arbitrary ranking, date extraction, full-name concatenation or final projection automatically. Keep numeric measurement tolerances unchanged and independent oracles pinned to original/corrected references; compare exact coefficient arithmetic before any Float64 presentation. Generic tests cover variable fractions, hours, malformed range/components, empty/null policy, offsets/suffix rejection, overflow and policy-visible scope. No benchmark question/gold/source configuration/model changes are proposed here.

## Proposed local booked aliases once supported

The current concept importer still lacks aliases. After a bounded generic optional alias contract exists, proposed orders.completed aliases are booked order, booked orders, booked purchase, booked purchases, booked sale and booked sales. Accompany them with the explicit local commerce definition: those terms select status completed, including signed credits. They do not redefine bare gross sales, subtract refunds, sum subscription measures or establish a universal accounting meaning. Publish only after actual importer support, independent review and parent author authorization; no aliases are currently authored.
