//! Explicit retained compiler decisions. Raw IR is opt-in sensitive capture;
//! normal compilation records continue to carry only digests and rule IDs.
use super::*;
use semantic_catalog::{Authority, FactResolution, SlotMeaning, SourceRef};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopedFactRef {
    pub id: String,
    pub scope: String,
    pub authority: Authority,
    pub source_refs: Vec<SourceRef>,
}
impl std::fmt::Debug for ScopedFactRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedFactRef").finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageArtifact {
    pub stage: String,
    pub before_digest: Option<String>,
    pub after_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_ir: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_ir: Option<serde_json::Value>,
}
impl std::fmt::Debug for StageArtifact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StageArtifact")
            .field("has_before", &self.before_digest.is_some())
            .field("ir_retained", &self.after_ir.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleDecision {
    pub requirement_id: String,
    pub rule: String,
    pub input_artifact_digest: String,
    pub output_artifact_digest: String,
    pub definition_refs: Vec<semantic_catalog::ObjectRef>,
    pub scoped_facts: Vec<ScopedFactRef>,
    pub preconditions: Vec<String>,
    pub alternatives: Vec<String>,
    pub analyses_reused: Vec<String>,
    pub analyses_invalidated: Vec<String>,
}
impl std::fmt::Debug for RuleDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuleDecision")
            .field("definition_count", &self.definition_refs.len())
            .field("scoped_fact_count", &self.scoped_facts.len())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionTrace {
    pub version: u32,
    pub stages: Vec<StageArtifact>,
    pub decisions: Vec<RuleDecision>,
}
impl std::fmt::Debug for DecisionTrace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecisionTrace")
            .field("version", &self.version)
            .field("stage_count", &self.stages.len())
            .field("decision_count", &self.decisions.len())
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub enum StageReplayOutcome {
    Matched {
        stages_checked: usize,
        decisions_checked: usize,
    },
    Diverged {
        stage: String,
        expected_digest: String,
        actual_digest: String,
    },
}
impl std::fmt::Debug for StageReplayOutcome {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Matched {
                stages_checked,
                decisions_checked,
            } => formatter
                .debug_struct("Matched")
                .field("stages_checked", stages_checked)
                .field("decisions_checked", decisions_checked)
                .finish(),
            Self::Diverged { .. } => formatter.write_str("Diverged { .. }"),
        }
    }
}

impl DecisionTrace {
    pub(super) fn from_query(query: &CompiledQuery, include_ir: bool) -> Self {
        let values = [
            (
                "proposal",
                serde_json::to_value(&query.intent).expect("proposal serializes"),
            ),
            (
                "bound",
                serde_json::to_value(&query.bound).expect("bound query serializes"),
            ),
            (
                "relational",
                serde_json::to_value(&query.relational).expect("relational plan serializes"),
            ),
            (
                "emitted_sql",
                serde_json::to_value(&query.sql).expect("SQL artifact serializes"),
            ),
        ];
        let mut stages = Vec::with_capacity(values.len());
        for (index, (stage, after)) in values.iter().enumerate() {
            let before = index.checked_sub(1).map(|previous| &values[previous].1);
            stages.push(StageArtifact {
                stage: (*stage).into(),
                before_digest: before.map(semantic_catalog::canonical_digest),
                after_digest: semantic_catalog::canonical_digest(after),
                before_ir: include_ir.then(|| before.cloned()).flatten(),
                after_ir: include_ir.then(|| after.clone()),
            });
        }
        let bound_digest = stages[1].after_digest.clone();
        let relational_digest = stages[2].after_digest.clone();
        let decisions = query
            .bound
            .requirements
            .iter()
            .map(|requirement| {
                let operation = serde_json::to_value(&requirement.operation)
                    .expect("bound operation serializes");
                let kind = operation
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("operation");
                let scoped_facts = query
                    .bound
                    .output_meanings
                    .get(&requirement.id)
                    .map(scoped_facts)
                    .unwrap_or_default();
                let definition_refs = query
                    .bound
                    .definitions
                    .iter()
                    .filter(|definition| {
                        scoped_facts.iter().any(|fact| {
                            fact.id.strip_suffix("/unit") == Some(definition.id.as_str())
                                || fact.id.strip_suffix("/source_grain")
                                    == Some(definition.id.as_str())
                        })
                    })
                    .cloned()
                    .collect();
                RuleDecision {
                    requirement_id: requirement.id.clone(),
                    rule: format!("typed.{kind}.v1"),
                    input_artifact_digest: bound_digest.clone(),
                    output_artifact_digest: relational_digest.clone(),
                    definition_refs,
                    scoped_facts,
                    preconditions: vec![
                        "pinned_catalog_snapshot".into(),
                        "exact_checked_types".into(),
                    ],
                    alternatives: vec![],
                    analyses_reused: vec![],
                    analyses_invalidated: vec![],
                }
            })
            .collect();
        Self {
            version: 1,
            stages,
            decisions,
        }
    }

    pub(super) fn compare(&self, actual: &Self) -> StageReplayOutcome {
        for (expected, actual) in self.stages.iter().zip(&actual.stages) {
            if expected.stage != actual.stage || expected.after_digest != actual.after_digest {
                return StageReplayOutcome::Diverged {
                    stage: expected.stage.clone(),
                    expected_digest: expected.after_digest.clone(),
                    actual_digest: actual.after_digest.clone(),
                };
            }
            if expected.before_digest != actual.before_digest {
                return StageReplayOutcome::Diverged {
                    stage: expected.stage.clone(),
                    expected_digest: expected.before_digest.clone().unwrap_or_default(),
                    actual_digest: actual.before_digest.clone().unwrap_or_default(),
                };
            }
        }
        if self.stages.len() != actual.stages.len() {
            return StageReplayOutcome::Diverged {
                stage: "stage_count".into(),
                expected_digest: self.stages.len().to_string(),
                actual_digest: actual.stages.len().to_string(),
            };
        }
        let expected = semantic_catalog::canonical_digest(
            &serde_json::to_value(&self.decisions).expect("decisions serialize"),
        );
        let found = semantic_catalog::canonical_digest(
            &serde_json::to_value(&actual.decisions).expect("decisions serialize"),
        );
        if expected != found {
            StageReplayOutcome::Diverged {
                stage: "decisions".into(),
                expected_digest: expected,
                actual_digest: found,
            }
        } else {
            StageReplayOutcome::Matched {
                stages_checked: self.stages.len(),
                decisions_checked: self.decisions.len(),
            }
        }
    }

    pub(super) fn retained_ir_is_consistent(&self) -> bool {
        self.stages.iter().all(|stage| {
            stage.before_ir.as_ref().is_none_or(|value| {
                stage.before_digest.as_ref() == Some(&semantic_catalog::canonical_digest(value))
            }) && stage
                .after_ir
                .as_ref()
                .is_none_or(|value| stage.after_digest == semantic_catalog::canonical_digest(value))
        })
    }
}

fn scoped_facts(meaning: &SlotMeaning) -> Vec<ScopedFactRef> {
    fn collect<T>(resolution: &FactResolution<T>, out: &mut Vec<ScopedFactRef>) {
        let facts = match resolution {
            FactResolution::Unknown => return,
            FactResolution::Known { contributors, .. } => contributors,
            FactResolution::Conflicting { alternatives } => alternatives,
        };
        out.extend(facts.iter().map(|fact| ScopedFactRef {
            id: fact.id.clone(),
            scope: fact.scope.clone(),
            authority: fact.authority.clone(),
            source_refs: fact.origins.clone(),
        }));
    }
    let mut facts = Vec::new();
    collect(&meaning.unit, &mut facts);
    collect(&meaning.source_grain, &mut facts);
    collect(&meaning.entity, &mut facts);
    facts.sort_by(|a, b| (&a.scope, &a.id).cmp(&(&b.scope, &b.id)));
    facts.dedup_by(|a, b| a.scope == b.scope && a.id == b.id);
    facts
}
