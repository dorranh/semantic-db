# Pipeline 39 row-wise rate SQL verification receipt

Immutable runtime built from source commit 09ce774 uses compiler pipeline 39, execution profile 14 and context renderer 3. Fresh full SQL passed 169/169: commerce 103/103 and BIRD 66/66. Both reports are complete, finalized and full_coverage=true; setup, cleanup and artifact errors are null.

| Dataset | Frozen artifact digest | Report and SHA-256 |
| --- | --- | --- |
| Commerce v1.0.5 | 3e512e3301b2a7739dba833011159bff88e2d492b80f07da185f835f4871876a | `.semantic-eval/commerce-rate-rowwise-sql/run-505936-18db697d25261734/report.json` — `2c6b0d1655e8b54bf2ed4841e61fad88d86171d409e6c82b79ecaa4bbb6f85b7` |
| BIRD-derived v1.0.3 | 51e7b79cf0b89866f8c0bf965d38781a916ab819147d587af035a0bd3143f2bc | `.semantic-eval/bird-rate-rowwise-sql/run-503619-18db695ed655fb41/report.json` — `b49810476d0d6829250548db1938ef725f84b3d4ef99e4f7873d71a9ea7ab880` |

Analyst independently verified report hashes, artifact identities and result counts. Parent also validated the commerce authored FX profiles offline; reviewer approved their exact model/generator/version delta and regeneration.

Immutable binary hashes read from disk: `/tmp/semantic-eval-rate-rowwise-runtime/semantic-eval` SHA-256 `bdb300c22cd637e59dc08acf1867e85331a471fd34f49395dda7ca7b7281e6c4`; `/tmp/semantic-eval-rate-rowwise-runtime/sdb` SHA-256 `49c4c3b9bded7e75167df8e4284f74712dca9fb41797fba742d3e07c849918ca`.

This is SQL regression evidence, not an Ask accuracy score or proof that exact-date missing/nonunique FX guards exist. The new profile supports row-wise conversion; stronger execution-stage negative oracle migration remains separate and unimplemented. BIRD full Ask is running and checkpoints are not full scores. Both artifacts remain frozen; no production, data, question, SQL, gold or tolerance edit accompanies this receipt.
