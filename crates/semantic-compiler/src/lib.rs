//! A first catalog-aware LLM compiler. The model proposes a grounding outcome;
//! deterministic validation checks its shape, evidence references, and SQL plan.
//! Domain correctness still needs curated metadata and semantic evaluation.

pub mod provider;
pub mod views;

use std::collections::BTreeSet;

use semantic_catalog::{Catalog, Relation, RelationKind};
use semantic_engine::Engine;
pub use semantic_plan::GroundingOutcome;
use semantic_plan::{ViewSelection, ViewSelectionOutcome};
use serde::Serialize;
use thiserror::Error;

use provider::{Message, ModelProvider, ProviderError, Role};

#[derive(Debug, Error)]
pub enum CompilerError {
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error("natural-language request must be nonempty")]
    EmptyRequest,
    #[error("model output failed validation after {attempts} attempt(s): {diagnostic}")]
    Validation { attempts: usize, diagnostic: String },
}

#[derive(Debug, Serialize)]
pub struct Compilation {
    pub outcome: GroundingOutcome,
    pub attempts: usize,
    /// Present only for the authored-view mode, after deterministic lowering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_selection: Option<ViewSelection>,
}

pub struct Compiler<P> {
    provider: P,
    max_repairs: usize,
}

impl<P: ModelProvider> Compiler<P> {
    pub fn new(provider: P) -> Self {
        Self {
            provider,
            max_repairs: 1,
        }
    }

    /// Limit repairs to at most three additional model calls. Provider/transport
    /// failures are never retried automatically.
    pub fn with_max_repairs(mut self, max_repairs: usize) -> Self {
        self.max_repairs = max_repairs.min(3);
        self
    }

    /// Plan valid proposals without executing rows. Unresolved outcomes return
    /// immediately and are never repaired into a guessed interpretation.
    pub async fn compile(
        &self,
        engine: &Engine,
        request: &str,
    ) -> Result<Compilation, CompilerError> {
        self.compile_internal(engine, request, false).await
    }

    /// Select an authored view and lower a bounded, typed query over it. No
    /// free-form model SQL is accepted and no fallback to `compile` occurs.
    /// The selected view's definition is preserved, but choosing the applicable
    /// view and interpreting all natural-language clauses remain model judgments.
    pub async fn compile_views(
        &self,
        engine: &Engine,
        request: &str,
    ) -> Result<Compilation, CompilerError> {
        self.compile_internal(engine, request, true).await
    }

    async fn compile_internal(
        &self,
        engine: &Engine,
        request: &str,
        views_only: bool,
    ) -> Result<Compilation, CompilerError> {
        if request.trim().is_empty() {
            return Err(CompilerError::EmptyRequest);
        }
        let context: Vec<_> = engine
            .catalog()
            .relations()
            .filter(|relation| !views_only || matches!(relation.kind, RelationKind::View { .. }))
            .map(relation_context)
            .collect();
        if context.is_empty() {
            return Ok(Compilation {
                outcome: GroundingOutcome::Unsupported {
                    reason: if views_only {
                        "No authored views are registered. Register a view defining the requested business meaning first."
                    } else {
                        "No relations are registered. Load a catalog or register a relation first."
                    }.into(),
                },
                attempts: 0,
                view_selection: None,
            });
        }
        let mut messages = vec![
            Message {
                role: Role::System,
                content: if views_only {
                    include_str!("views_prompt.txt")
                } else {
                    include_str!("prompt.txt")
                }
                .into(),
            },
            Message {
                role: Role::User,
                content: serde_json::json!({
                    "request": request,
                    "catalog": context,
                })
                .to_string(),
            },
        ];
        for attempt in 1..=self.max_repairs + 1 {
            let text = self.provider.complete(&messages).await?;
            let validation = decode_proposal(engine, request, &text, views_only).await;
            match validation {
                Ok((outcome, view_selection)) => {
                    return Ok(Compilation {
                        outcome,
                        attempts: attempt,
                        view_selection,
                    });
                }
                Err(diagnostic) => {
                    if attempt > self.max_repairs {
                        return Err(CompilerError::Validation {
                            attempts: attempt,
                            diagnostic,
                        });
                    }
                    messages.push(Message {
                        role: Role::Assistant,
                        content: text,
                    });
                    messages.push(Message { role: Role::User, content: serde_json::json!({
                        "validation_error": diagnostic,
                        "instruction": "Repair the output while preserving the original request."
                    }).to_string() });
                }
            }
        }
        unreachable!("at least one compilation attempt")
    }
}

/// Projection deliberately omits physical source paths, owners, and row samples.
/// View SQL is included because it may supply an explicit concept definition.
pub fn catalog_context(catalog: &Catalog) -> serde_json::Value {
    serde_json::Value::Array(catalog.relations().map(relation_context).collect())
}

fn relation_context(relation: &Relation) -> serde_json::Value {
    let definition = match &relation.kind {
        RelationKind::Base { .. } => None,
        RelationKind::View { sql, .. } => Some(sql),
    };
    serde_json::json!({
        "name": relation.name,
        "description": relation.description,
        "grain": relation.grain,
        "view_sql": definition,
        "semantics": relation.semantics,
        "columns": relation.schema.fields().iter().map(|field| serde_json::json!({
            "name": field.name(), "type": field.data_type().to_string(), "nullable": field.is_nullable()
        })).collect::<Vec<_>>()
    })
}

async fn decode_proposal(
    engine: &Engine,
    request: &str,
    text: &str,
    views_only: bool,
) -> Result<(GroundingOutcome, Option<ViewSelection>), String> {
    let shape_error =
        || "Return a JSON object matching exactly one documented outcome shape.".to_owned();
    let (outcome, selection) = if views_only {
        match serde_json::from_str::<ViewSelectionOutcome>(text).map_err(|_| shape_error())? {
            ViewSelectionOutcome::Selected { selection } => {
                let query = views::lower_view_selection(engine, request, &selection)
                    .await
                    .map_err(|error| error.to_string())?;
                // Lowering has already checked the binding and planned this SQL.
                return Ok((GroundingOutcome::Grounded { query }, Some(selection)));
            }
            ViewSelectionOutcome::NeedsClarification { phrases, question } => (
                GroundingOutcome::NeedsClarification { phrases, question },
                None,
            ),
            ViewSelectionOutcome::Unsupported { reason } => {
                (GroundingOutcome::Unsupported { reason }, None)
            }
        }
    } else {
        (
            serde_json::from_str::<GroundingOutcome>(text).map_err(|_| shape_error())?,
            None,
        )
    };
    validate_outcome(engine, &outcome).await?;
    Ok((outcome, selection))
}

async fn validate_outcome(engine: &Engine, outcome: &GroundingOutcome) -> Result<(), String> {
    match outcome {
        GroundingOutcome::Grounded { query } => {
            if query.sql.trim().is_empty() || query.evidence.is_empty() {
                return Err("Grounded output requires SQL and nonempty grounding evidence.".into());
            }
            let mut references = BTreeSet::new();
            for relation in engine.catalog().relations() {
                references.insert(relation.name.clone());
                for field in relation.schema.fields() {
                    references.insert(format!("{}.{}", relation.name, field.name()));
                }
            }
            for evidence in &query.evidence {
                if evidence.phrase.trim().is_empty()
                    || evidence.interpretation.trim().is_empty()
                    || !references.contains(&evidence.catalog_reference)
                {
                    return Err("Evidence requires nonempty phrases/interpretations and existing relation or relation.column references.".into());
                }
            }
            engine
                .plan_generated_sql(&query.sql)
                .await
                .map_err(|error| format!("SQL planning failed: {error}"))?;
        }
        GroundingOutcome::NeedsClarification { phrases, question } => {
            if phrases.is_empty()
                || phrases.iter().any(|phrase| phrase.trim().is_empty())
                || question.trim().is_empty()
            {
                return Err("Clarification requires nonempty phrases and a question.".into());
            }
        }
        GroundingOutcome::Unsupported { reason } => {
            if reason.trim().is_empty() {
                return Err("Unsupported output requires a reason.".into());
            }
        }
    }
    Ok(())
}
