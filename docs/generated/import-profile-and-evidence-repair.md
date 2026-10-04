# Imported representation profiles and graph evidence repair

Ossie imports now publish executable representation profiles
`ossie/source-bound-dataset` and `ossie/direct-projection-view`, revision 2. These
identify imported dataset and view representations. Supported query operations
come from the closed typed protocol and its physical and authored-contract
checks. The old `column-projection` label caused false Unsupported responses for
ordinary aggregation. Nonexecutable capability gates remain enforced.

Tests execute Filter + Group + Count on an imported source and Count on an
imported direct-projection view, verify the profiles exposed in model context,
and reject aggregation on a descriptive-only relation.

Graph evidence feedback now reports expected/provided counts and at most eight
missing structural locations, plus bounded duplicate/unavailable evidence entry
indices. These paths contain fixed schema keys and array ordinals, never model
IDs, aliases, request text or literals. Coverage still requires exactly one entry
per requirement and valid request spans. Privacy tests inspect both serialized
diagnostics and Debug output, including equal-count duplicate/missing errors.
The prompt explicitly requires separate evidence for every copied passthrough
output using its new ID. Compiler pipeline revision is 37.

The change follows a genuine pipeline-36 focused failure report:
`.semantic-eval/commerce-roles-project/run-379974-18db6145bfa126db/report.json`,
SHA256 `d44e063cc5ca74ec1182fb56ccca37d1cbfaf107185c5a39ddcca6ed79df8acd`.
Both questions failed strict request coverage because the model omitted copied
comparison-node outputs and repeated the omission during repair. This report
remains a separate failed observation.

Parent verification passed 318 tests across 72 targets in semantic-ossie,
semantic-compiler and semantic-interpreter, all-target Clippy with warnings
denied, formatting and semantic-eval/CLI builds. Logs:
`/tmp/semantic-eval-profile-coverage-{tests,clippy,build}.log`. One initial
compile-time formatting-message borrowing error was corrected before the
successful verification; its log is retained separately.
