//! Request-ledger integrity. Span checks establish traceability, never proof that
//! an interpreter understood or extracted every clause in the user's request.
use super::{CompileDiagnostic, CompileOptions, TypedCompilation, diagnostic};
use semantic_plan::typed::{IntentQuery, RequestEvidence, RequestSpan, RowQuery};

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
    validate_header(
        evidence.version,
        &evidence.request_id,
        &evidence.original_request,
        &evidence.unresolved_alternatives,
    )?;
    let mut requirements = Vec::new();
    semantic_plan::typed::visit_requirements(&query.requirements, &mut |requirement, depth, _| {
        options.check()?;
        if depth > options.max_depth || requirements.len() >= options.max_nodes {
            return Err(diagnostic(
                "work_limit",
                "Related evidence budget exhausted",
            ));
        }
        requirements.push(requirement);
        Ok(())
    })?;
    if evidence.requirement_spans.len() != requirements.len() {
        return Err(diagnostic(
            "request_coverage",
            "Every mandatory requirement must have source spans and no orphan span mappings",
        ));
    }
    let mut total = 0usize;
    for (position, requirement) in requirements.into_iter().enumerate() {
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
        let exact_text = span_text(&evidence.original_request, spans)?;
        if exact_text != requirement.source_text {
            return Err(diagnostic(
                "request_span",
                &format!(
                    "Requirement at index {position} source_text must equal the request substrings at UTF-8 byte ranges {spans:?}, joined by a space. Repair source_text or the spans without changing the requirement"
                ),
            ));
        }
    }
    Ok(())
}

pub(super) fn validate_header(
    version: u32,
    request_id: &str,
    original_request: &str,
    alternatives: &[String],
) -> Result<(), CompileDiagnostic> {
    if version != 1
        || request_id.trim().is_empty()
        || request_id.len() > 256
        || original_request.trim().is_empty()
    {
        return Err(diagnostic(
            "request_evidence",
            "Request evidence needs version 1, a bounded identity and original text",
        ));
    }
    if !alternatives.is_empty() {
        return Err(diagnostic(
            "unresolved_alternatives",
            "Competing interpretations must be resolved before binding",
        ));
    }
    Ok(())
}

pub(super) fn span_text(
    original_request: &str,
    spans: &[RequestSpan],
) -> Result<String, CompileDiagnostic> {
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
        let value = original_request.get(span.start..span.end).ok_or_else(|| {
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
    Ok(text.join(" "))
}
