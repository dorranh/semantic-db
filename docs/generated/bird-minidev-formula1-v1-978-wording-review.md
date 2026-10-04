# Case 978: visible counting-grain clarification proposal

Published in BIRD-derived v1.0.3 after independent reviewer approval and parent authorization. Original source annotations remain immutable. The preserved PostgreSQL and SQLite question is: “How many times the circuits were held in Austria? Please give their location and coordinates.” Both original evidence strings identify Austria as country='Austria' and coordinates as lat/lng. Both original SQL variants select DISTINCT location,lat,lng from circuits; neither counts race events or even returns a count. The reviewed derived task preserves that geographic-venue population and adds the requested total count alongside each venue.

The current catalog defines geographic venue identity as (location,lat,lng), allowing multiple historical circuit IDs. That defines an entity key, but it does not clearly resolve the question's phrase “how many times ... held” into a venue count rather than race occurrences. The effective NL therefore remains genuinely ambiguous even though the reviewed SQL and gold have an explicit venue interpretation. No observed model answer is needed to establish that ambiguity.

Published effective question:

> How many distinct circuit venues are located in Austria, treating a venue as a distinct (location, latitude, longitude) combination? Return the total venue count followed by each venue's location, latitude and longitude, repeating the total count on each row.

This exposes the already reviewed task grain and result shape without disclosing any expected value or choosing easier product semantics. It does not count circuit IDs or race events and does not drop any requested field. SQL, gold, types, tolerances and source data remain unchanged. The original question/evidence/SQL variants stay immutable in source/selected_original.json and pinned upstream annotation files. If approved, record this as a visible effective-NL erratum for the BIRD-derived corrected variant, not as an official benchmark question.

Independent reviewer approval was obtained before the cases/generator revision. The review should verify venue grain against original DISTINCT SQL, preserve the existing count-plus-location shape, and ensure no expected count or source-result rows enter the wording. Parent authorizes artifact release separately. Other result-order/projection failures (994/897) are general compiler/proposal fixes and do not justify wording changes.

The generator reproduces this sole effective-question change and emits source/question-errata.json; source/provenance.json now pins that record hash. All 66 SQL strings, golds, tolerances and source data remain unchanged. Artifact digest `51e7b79cf0b89866f8c0bf965d38781a916ab819147d587af035a0bd3143f2bc`; detailed before/after hashes in bird-minidev-formula1-v1-978-revision.json and 978-baseline-hashes.json. Bundle refrozen for actual-patch review and parent validation.

## Focused p37 runtime observation

The effective venue-count clarification yielded the correct total venue count of two, but returned the count after the location/coordinate columns instead of first as explicitly requested. This remains a genuine result-shape failure for the generic final projection work; it is not a reason to alter SQL, gold or tolerance. Reversed venue row order is permitted because case comparison.ordered=false. No row-sort erratum is introduced.
