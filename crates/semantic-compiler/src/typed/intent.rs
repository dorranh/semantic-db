//! Request-ledger integrity. Span checks establish traceability, never proof that
//! an interpreter understood or extracted every clause in the user's request.
use super::{CompileDiagnostic, CompileOptions, TypedCompilation, diagnostic};
use semantic_plan::typed::{IntentQuery, RequestEvidence, RowQuery};

pub async fn compile_intent(
    engine: &semantic_engine::Engine,
    intent: IntentQuery,
    mut options: CompileOptions,
) -> TypedCompilation {
    options.request_evidence = Some(intent.evidence);
    super::compile_semantic(engine, intent.query, options).await
}
pub(super) fn validate(
    query: &RowQuery,
    evidence: &RequestEvidence,
    options: &CompileOptions,
) -> Result<(), CompileDiagnostic> {
    super::bounded_json(evidence, options.max_input_bytes)
        .map_err(|_| diagnostic("input_limit", "Request evidence exceeds its byte budget"))?;
    if evidence.version != 1
        || evidence.request_id.trim().is_empty()
        || evidence.request_id.len() > 256
        || evidence.original_request.trim().is_empty()
    {
        return Err(diagnostic(
            "request_evidence",
            "Request evidence needs version 1, a bounded identity and original text",
        ));
    }
    if !evidence.unresolved_alternatives.is_empty() {
        return Err(diagnostic(
            "unresolved_alternatives",
            "Competing interpretations must be resolved before binding",
        ));
    }
    if evidence.requirement_spans.len() != query.requirements.len() {
        return Err(diagnostic(
            "request_coverage",
            "Every mandatory requirement must have source spans and no orphan span mappings",
        ));
    }
    let mut total = 0usize;
    for requirement in &query.requirements {
        options.check()?;
        let spans = evidence
            .requirement_spans
            .get(&requirement.id)
            .ok_or_else(|| {
                diagnostic(
                    "request_coverage",
                    "A mandatory requirement has no source spans",
                )
            })?;
        total = total.saturating_add(spans.len());
        if total > options.max_nodes {
            return Err(diagnostic("work_limit", "Request span budget exhausted"));
        }
        if spans.is_empty() {
            return Err(diagnostic(
                "request_coverage",
                "Requirement source spans cannot be empty",
            ));
        }
        let mut previous_end = 0;
        let mut text = Vec::new();
        for span in spans {
            if span.start >= span.end || span.start < previous_end {
                return Err(diagnostic(
                    "request_span",
                    "Source spans must be nonempty, ordered and nonoverlapping within a requirement",
                ));
            }
            let value = evidence
                .original_request
                .get(span.start..span.end)
                .ok_or_else(|| {
                    diagnostic(
                        "request_span",
                        "Source spans must lie on UTF-8 boundaries within the original request",
                    )
                })?;
            if value.trim().is_empty() {
                return Err(diagnostic(
                    "request_span",
                    "Whitespace alone cannot substantiate a requirement",
                ));
            }
            previous_end = span.end;
            text.push(value);
        }
        if text.join(" ") != requirement.source_text {
            return Err(diagnostic(
                "request_span",
                "Requirement source text must exactly equal its source spans joined by a space",
            ));
        }
    }
    Ok(())
}
