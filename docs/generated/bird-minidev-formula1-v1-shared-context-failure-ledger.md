# BIRD full shared-context run: terminal evidence and complete failure ledger

The frozen v1.0.2 artifact `1a88e121488ce099e5ea97cc9efcdba90edf7d2b56c58dfadc3b1569ca550c01` passed SQL 66/66 and Ask 18/66: 84/132 attempts passed. All 132 planned attempts finished. The report is finalized and has full coverage, but `complete=false` because one Ask attempt had an HTTP 429 provider failure. The other 47 Ask failures are nonprovider failures; they remain acceptance failures. Setup, cleanup and artifact errors are all null. This does not establish full semantic acceptance.

Full report: `.semantic-eval/bird-luna-shared-context/run-258343-18db5d74c20816e2/report.json`. SHA-256: `e8f2edebaea82d21ad53e260e79f90f3f40fbd09de3b485a34385e60361191fa`.

The separate transport retry for 884 returned unsupported, 0/1, with no provider error and `complete=true`, `finalized=true`. Its filtered scope cannot be merged into a replacement full report or presented as full semantic coverage. Retry report: `.semantic-eval/bird-luna-shared-context-transport-retry/run-336909-18db5eebbfbf56a5/report.json`. SHA-256: `df00992dbcea73ebe7658ea826b46b858f2a366fd06a39ea3a2dabb6967db886`.

Per-call provider-reported input usage has median 37944 and maximum 38468 over 67 calls in the full report. Compared with the earlier approximately 66700-token BIRD requests, this confirms reduced input size for this run. It does not demonstrate a guaranteed accuracy benefit or causation. Earlier baseline reports remain historical evidence; this ledger does not overwrite them.

## Priorities supported by observed failures

- Duration parsing and guarded numeric text conversion: 846, 879, 880, 955, 960, 963, 988, 1011. Keep exact milliseconds and distinguish elapsed durations from offsets; no lexical numeric sorting.
- Exact ratio scaling and general typed expressions: 909, 954, 962; 944 also needs duration subtraction. 881/896 ask zero-population questions, which need an explicit faithful task contract rather than a capability waiver.
- Grouped selection, postaggregate projection and related-field ranking: 869, 897, 951, 981, 994, 1002, 1014. Distinguish unavailable expression support from overly narrow advertised catalog capabilities such as “column projection only.”
- Gregorian date/year expressions and typed pattern predicates: 898, 971, 988, 861, 866; 963 needs authored duration-unit equivalence for its numeric threshold.
- Relationship direction, occurrence scope and filtered lookups: 972, 1001, 906. Genuine FK authoring is separate from compiler support for lookup filtering, multihop grouping or ordering.
- Bounded per-task upstream evidence/context: 892, 948, 967, 978 and other clarification outcomes need source-policy fidelity review. Preserve immutable upstream variants and reviewed adjudications. Do not expose reference SQL or expected answers to the model, and do not assume every clarification is automatically illegitimate.

The following ledger copies every failed case's full diagnostic, provider errors and comparison differences without truncation. Actual result rows are included where present. These are observed report values, not independently adjudicated causes. Null diagnostics are retained rather than replaced with invented explanations.

## bird.formula1.962 — unsupported

```json
{
  "id": "bird.formula1.962",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The requested percentage requires dividing the qualifying driver count by the comparison count and scaling the result by 100. The available ratio operation supports division but not multiplication or another computed expression.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.898 — unsupported

```json
{
  "id": "bird.formula1.898",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The request asks for the driver's age, which requires a computed expression not supported by this row-query protocol.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.854 — result

```json
{
  "id": "bird.formula1.854",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "row count: expected 1, actual 11"
  ],
  "actual": {
    "columns": [
      {
        "name": "lat",
        "type": "float64",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      },
      {
        "name": "lng",
        "type": "float64",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        -34.9272,
        138.617
      ],
      [
        -34.9272,
        138.617
      ],
      [
        -34.9272,
        138.617
      ],
      [
        -34.9272,
        138.617
      ],
      [
        -34.9272,
        138.617
      ],
      [
        -34.9272,
        138.617
      ],
      [
        -34.9272,
        138.617
      ],
      [
        -34.9272,
        138.617
      ],
      [
        -34.9272,
        138.617
      ],
      [
        -34.9272,
        138.617
      ],
      [
        -34.9272,
        138.617
      ]
    ]
  }
}
```

## bird.formula1.955 — unsupported

```json
{
  "id": "bird.formula1.955",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Champion finish times are stored as text in varying formats, and converting them into numeric seconds requires parsing components. No authored conversion or supported computed-expression operation is available for that calculation.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.877 — result

```json
{
  "id": "bird.formula1.877",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "column count: expected 2, actual 4"
  ],
  "actual": {
    "columns": [
      {
        "name": "driver_id",
        "type": "int64",
        "tolerance": null,
        "nullable": false,
        "physical_nullable": null
      },
      {
        "name": "dob",
        "type": "date32",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      },
      {
        "name": "forename",
        "type": "utf8",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      },
      {
        "name": "surname",
        "type": "utf8",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "815",
        "1990-01-26",
        "Sergio",
        "Pérez"
      ]
    ]
  }
}
```

## bird.formula1.880 — unsupported

```json
{
  "id": "bird.formula1.880",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The fastest-lap-speed field is Utf8, and the requested percentage calculation requires interpreting its text values numerically. No authored conversion or supported operation provides that parsing and calculation.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.869 — unsupported

```json
{
  "id": "bird.formula1.869",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The request requires selecting constructor result row(s) with the maximum points and looking up the constructor website. The validated ranking proposal is rejected because the window engine does not accept the Float64 points field as a window key, and the available operations cannot express the maximum-row selection while preserving possible ties without that ranking.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.972 — unsupported

```json
{
  "id": "bird.formula1.972",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The request combines a driver birth-date condition and driver fields with fastest-lap data in results. The authored results-to-drivers relationship supports row lookup for displaying related fields, but not filtering by the looked-up birth date; ordinary joins are unsupported.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.950 — result

```json
{
  "id": "bird.formula1.950",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "row count: expected 6, actual 11"
  ],
  "actual": {
    "columns": [
      {
        "name": "constructor_name",
        "type": "utf8",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "Ligier"
      ],
      [
        "Brabham"
      ],
      [
        "Fondmetal"
      ],
      [
        "McLaren"
      ],
      [
        "Larrousse"
      ],
      [
        "Dallara"
      ],
      [
        "Jordan"
      ],
      [
        "Tyrrell"
      ],
      [
        "Team Lotus"
      ],
      [
        "Minardi"
      ],
      [
        "March"
      ]
    ]
  }
}
```

## bird.formula1.1002 — unsupported

```json
{
  "id": "bird.formula1.1002",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The request requires identifying the driver with the latest birth date and their earliest race across related tables, then combining first and last names. The available row-query operations cannot rank by fields reached through lookups or construct a concatenated full name.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.988 — unsupported

```json
{
  "id": "bird.formula1.988",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The birth-year condition requires extracting the year from drivers.dob, but computed expressions are unsupported and the catalog provides no authored concept or date bounds for this condition.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.951 — unsupported

```json
{
  "id": "bird.formula1.951",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Answering this requires grouping constructor results by constructor to identify those with exactly two race records and zero points, then counting qualifying constructors while applying nationality from the related constructors table. The catalog query operations do not support that multi-stage aggregation and relation composition.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.879 — unsupported

```json
{
  "id": "bird.formula1.879",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Finding the maximum lap speed requires numeric comparison, but fastestlapspeed is authored as Utf8 text and no checked text-to-number conversion is available.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.906 — unsupported

```json
{
  "id": "bird.formula1.906",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Determining Lewis Hamilton’s first race requires sorting his results by the related race date, then round and race ID. Row ordering is limited to fields on the selected relation, so this related-field ordering is unsupported.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.1011 — unsupported

```json
{
  "id": "bird.formula1.1011",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The authored interpretation identifies lap time as the text field `time`, whose formats vary. Correctly ranking durations requires parsing those strings, an unsupported computed expression; sorting the text directly would not reliably identify the shortest times.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.850 — result

```json
{
  "id": "bird.formula1.850",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "row count: expected 3, actual 76"
  ],
  "actual": {
    "columns": [
      {
        "name": "race_name",
        "type": "utf8",
        "tolerance": null,
        "nullable": false,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "European Grand Prix"
      ],
      [
        "European Grand Prix"
      ],
      [
        "European Grand Prix"
      ],
      [
        "European Grand Prix"
      ],
      [
        "European Grand Prix"
      ],
      [
        "European Grand Prix"
      ],
      [
        "European Grand Prix"
      ],
      [
        "European Grand Prix"
      ],
      [
        "European Grand Prix"
      ],
      [
        "Luxembourg Grand Prix"
      ],
      [
        "Luxembourg Grand Prix"
      ],
      [
        "European Grand Prix"
      ],
      [
        "European Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "European Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ],
      [
        "German Grand Prix"
      ]
    ]
  }
}
```

## bird.formula1.909 — unsupported

```json
{
  "id": "bird.formula1.909",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The requested percentage requires multiplying the ratio of German European Grand Prix races to all European Grand Prix races by 100. The supported ratio operation returns the quotient without that multiplication, and no authored metric defines this percentage.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.944 — unsupported

```json
{
  "id": "bird.formula1.944",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The percentage requires subtracting the champion’s time from the last finisher’s time before division. The available typed operations cannot express that subtraction, and the time strings have no authored parsing operation.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.928 — result

```json
{
  "id": "bird.formula1.928",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "column count: expected 3, actual 1"
  ],
  "actual": {
    "columns": [
      {
        "name": "reference_name",
        "type": "utf8",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "hamilton"
      ]
    ]
  }
}
```

## bird.formula1.994 — unsupported

```json
{
  "id": "bird.formula1.994",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The catalog exposes only column projection for these relations. Finding the constructor with the highest total points requires aggregation and ranking, which are unsupported.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.865 — result

```json
{
  "id": "bird.formula1.865",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "column count: expected 2, actual 5"
  ],
  "actual": {
    "columns": [
      {
        "name": "driver_id",
        "type": "int64",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      },
      {
        "name": "forename",
        "type": "utf8",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      },
      {
        "name": "surname",
        "type": "utf8",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      },
      {
        "name": "date_of_birth",
        "type": "date32",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      },
      {
        "name": "oldest_rank",
        "type": "uint64",
        "tolerance": null,
        "nullable": false,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "306",
        "Jean-Pierre",
        "Beltoise",
        "1937-04-26",
        "1"
      ]
    ]
  }
}
```

## bird.formula1.960 — unsupported

```json
{
  "id": "bird.formula1.960",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The catalog defines fastestlapspeed as Utf8 text, and no authored conversion or supported expression makes it numeric for averaging.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.897 — unsupported

```json
{
  "id": "bird.formula1.897",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The requested ranking and maximum point score require grouped aggregates, but the relevant catalog relations declare only the executable column-projection capability.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.857 — result

```json
{
  "id": "bird.formula1.857",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "row count: expected 1, actual 9"
  ],
  "actual": {
    "columns": [
      {
        "name": "lat",
        "type": "float64",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      },
      {
        "name": "lng",
        "type": "float64",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        24.4672,
        54.6031
      ],
      [
        24.4672,
        54.6031
      ],
      [
        24.4672,
        54.6031
      ],
      [
        24.4672,
        54.6031
      ],
      [
        24.4672,
        54.6031
      ],
      [
        24.4672,
        54.6031
      ],
      [
        24.4672,
        54.6031
      ],
      [
        24.4672,
        54.6031
      ],
      [
        24.4672,
        54.6031
      ]
    ]
  }
}
```

## bird.formula1.981 — unsupported

```json
{
  "id": "bird.formula1.981",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The request requires selecting the driver with the latest known date of birth, then finding that driver's earliest qualifying race and returning its race details. The available typed operations cannot perform those linked row-level selections across qualifying, drivers, and races without unsupported join and computation steps.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.896 — needs_clarification

```json
{
  "id": "bird.formula1.896",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "If there are no Hamilton race results since 2010, should the percentage be null or zero?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

## bird.formula1.945 — unsupported

```json
{
  "id": "bird.formula1.945",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The circuits relation supports column projection only; counting matching circuits requires an unsupported aggregate operation.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.967 — needs_clarification

```json
{
  "id": "bird.formula1.967",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "For each of the three youngest drivers, do you want the abbreviated driver code (the `code` field) or the driver’s race number (the `number` field)? ",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

## bird.formula1.861 — unsupported

```json
{
  "id": "bird.formula1.861",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The Q3 time is a text prefix/pattern match, but the supported typed predicates do not provide prefix or LIKE matching.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.892 — needs_clarification

```json
{
  "id": "bird.formula1.892",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "Do you mean the driver with the highest points in a single race, or the driver with the greatest total points across races?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

## bird.formula1.866 — unsupported

```json
{
  "id": "bird.formula1.866",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The lap-time phrase requires matching a time-string pattern, but the supported row-query predicates cannot express that pattern, and the catalog provides no authored conversion to a typed duration.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.868 — result

```json
{
  "id": "bird.formula1.868",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "row count: expected 1, actual 19"
  ],
  "actual": {
    "columns": [
      {
        "name": "latitude",
        "type": "float64",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      },
      {
        "name": "longitude",
        "type": "float64",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ],
      [
        2.76083,
        101.738
      ]
    ]
  }
}
```

## bird.formula1.948 — needs_clarification

```json
{
  "id": "bird.formula1.948",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "Should I find the maximum points recorded in constructor results, or the maximum points in constructor standings?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

## bird.formula1.895 — result

```json
{
  "id": "bird.formula1.895",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "column 0 type: expected float64, actual decimal128(38,18)"
  ],
  "actual": {
    "columns": [
      {
        "name": "average_lap_time",
        "type": "decimal128(38,18)",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "109398.548387096774193548"
      ]
    ]
  }
}
```

## bird.formula1.971 — unsupported

```json
{
  "id": "bird.formula1.971",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The authored meaning of “oldest” is MIN(year(dob)), which requires extracting a year from the date of birth; computed expressions are unsupported.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.846 — unsupported

```json
{
  "id": "bird.formula1.846",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The five drivers eliminated in Q1 are identified by their slowest Q1 times, but q1 is stored as text. Correctly ranking those times requires parsing the timing strings, which is an unsupported computed expression.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.902 — result

```json
{
  "id": "bird.formula1.902",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "row count: expected 15, actual 5"
  ],
  "actual": {
    "columns": [
      {
        "name": "race",
        "type": "utf8",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "French Grand Prix"
      ],
      [
        "Italian Grand Prix"
      ],
      [
        "Canadian Grand Prix"
      ],
      [
        "Brazilian Grand Prix"
      ],
      [
        "Japanese Grand Prix"
      ]
    ]
  }
}
```

## bird.formula1.1014 — unsupported

```json
{
  "id": "bird.formula1.1014",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "Listing per-circuit lap records requires finding the minimum observed lap duration per circuit across the lap-times-to-races-to-circuits relationships. The available operations do not support that multi-hop grouped aggregation.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.1003 — result

```json
{
  "id": "bird.formula1.1003",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "column count: expected 1, actual 2"
  ],
  "actual": {
    "columns": [
      {
        "name": "driverid",
        "type": "int64",
        "tolerance": null,
        "nullable": false,
        "physical_nullable": null
      },
      {
        "name": "accidents",
        "type": "int64",
        "tolerance": null,
        "nullable": false,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "31",
        "2"
      ]
    ]
  }
}
```

## bird.formula1.978 — needs_clarification

```json
{
  "id": "bird.formula1.978",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "Do you mean the number of circuit venues located in Austria, or the number of race events held at Austrian circuits, counted by location?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

## bird.formula1.915 — needs_clarification

```json
{
  "id": "bird.formula1.915",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "If multiple drivers share the earliest date of birth, should I return all their countries or choose one driver?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

## bird.formula1.954 — unsupported

```json
{
  "id": "bird.formula1.954",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The requested percentage requires multiplying the ratio of finished driver entries to all driver entries by 100. The available checked ratio operation does not support scaling its result.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.930 — result

```json
{
  "id": "bird.formula1.930",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "row count: expected 37, actual 1"
  ],
  "actual": {
    "columns": [
      {
        "name": "race_name",
        "type": "utf8",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      }
    ],
    "rows": [
      [
        "Chinese Grand Prix"
      ]
    ]
  }
}
```

## bird.formula1.872 — result

```json
{
  "id": "bird.formula1.872",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "result",
  "diagnostic": null,
  "provider_errors": [],
  "differences": [
    "row count: expected 1, actual 0"
  ],
  "actual": {
    "columns": [
      {
        "name": "abbreviated_code",
        "type": "utf8",
        "tolerance": null,
        "nullable": true,
        "physical_nullable": null
      }
    ],
    "rows": []
  }
}
```

## bird.formula1.884 — provider_failure

```json
{
  "id": "bird.formula1.884",
  "interface": "ask",
  "passed": false,
  "incomplete": true,
  "outcome": "provider_failure",
  "diagnostic": "provider_failure: Model provider failed; no automatic transport retry",
  "provider_errors": [
    "model provider returned HTTP 429; check credentials, model, quota, and base URL"
  ],
  "differences": [
    "expected result, actual provider_failure"
  ],
  "actual": null
}
```

## bird.formula1.881 — needs_clarification

```json
{
  "id": "bird.formula1.881",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "needs_clarification",
  "diagnostic": "If there are no driver results for that date, should the completion rate be null or zero?",
  "provider_errors": [],
  "differences": [
    "expected result, actual needs_clarification"
  ],
  "actual": null
}
```

## bird.formula1.1001 — unsupported

```json
{
  "id": "bird.formula1.1001",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The request requires joining qualifying results to races and circuits to identify the event, and to drivers to return the racer's name. The catalog supports only separately scoped existence or absence requirements and authored row lookups, not the ordinary multi-relation joins needed here.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## bird.formula1.963 — unsupported

```json
{
  "id": "bird.formula1.963",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The lap-time threshold is stated as a duration, while the numeric lap-time field is in milliseconds. The catalog provides no authored conversion rule for this comparison, and the text time field cannot be safely compared as a duration.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```

## Separate 884 transport retry

```json
{
  "id": "bird.formula1.884",
  "interface": "ask",
  "passed": false,
  "incomplete": false,
  "outcome": "unsupported",
  "diagnostic": "The catalog identifies the intended earliest year/month as the year and month of the minimum race date, but the available query operations cannot derive that minimum and use its month to filter race rows and return their names.",
  "provider_errors": [],
  "differences": [
    "expected result, actual unsupported"
  ],
  "actual": null
}
```
