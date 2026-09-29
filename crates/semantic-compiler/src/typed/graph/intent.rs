//! Exact graph-ledger coverage. This proves traceability of a proposal, not that
//! natural-language extraction found every requirement or chose its meaning.
use super::*;

pub async fn compile_graph_intent(
    engine: &Engine,
    intent: GraphIntentQuery,
    mut options: CompileOptions,
) -> TypedCompilation {
    options.graph_request_evidence = Some(intent.evidence);
    compile_graph(engine, intent.query, options).await
}

/// None means that the operation has no legacy source_text field; its validated
/// nonempty spans are retained in the evidence envelope instead.
pub(super) fn requirements<'a>(
    query: &'a GraphQuery,
    options: &CompileOptions,
) -> Result<BTreeMap<GraphRequirementRef, Option<&'a str>>, CompileDiagnostic> {
    let mut required = BTreeMap::new();
    let mut add = |target, source| {
        options.check()?;
        if required.insert(target, source).is_some() {
            return Err(diagnostic(
                "request_coverage",
                "Graph requirement identities must be unique",
            ));
        }
        if required.len() > options.max_nodes {
            return Err(diagnostic(
                "work_limit",
                "Graph requirement budget exhausted",
            ));
        }
        Ok(())
    };
    for node in &query.nodes {
        add(
            GraphRequirementRef::Node {
                node: node.id.clone(),
            },
            Some(node.source_text.as_str()),
        )?;
        match &node.operation {
            GraphOperation::Rows { query } => {
                for requirement in &query.requirements {
                    add(
                        GraphRequirementRef::Leaf {
                            node: node.id.clone(),
                            requirement: requirement.id.clone(),
                        },
                        Some(requirement.source_text.as_str()),
                    )?;
                }
            }
            GraphOperation::Set { columns, .. } => {
                for column in columns {
                    add(
                        GraphRequirementRef::Output {
                            node: node.id.clone(),
                            slot: column.id.clone(),
                        },
                        None,
                    )?;
                }
            }
            GraphOperation::Compose { keys, outputs, .. } => {
                for slot in keys
                    .iter()
                    .map(|key| &key.id)
                    .chain(outputs.iter().map(|output| &output.id))
                {
                    add(
                        GraphRequirementRef::Output {
                            node: node.id.clone(),
                            slot: slot.clone(),
                        },
                        None,
                    )?;
                }
            }
        }
    }
    for index in 0..query.ordering.len() {
        add(GraphRequirementRef::Order { index }, None)?;
    }
    if query.limit.is_some() {
        add(GraphRequirementRef::Limit, None)?;
    }
    Ok(required)
}

pub(super) fn validate(
    query: &GraphQuery,
    evidence: &GraphRequestEvidence,
    options: &CompileOptions,
) -> Result<(), CompileDiagnostic> {
    bounded_json(evidence, options.max_input_bytes)
        .map_err(|_| diagnostic("input_limit", "Graph evidence exceeds its byte budget"))?;
    super::super::intent::validate_header(
        evidence.version,
        &evidence.request_id,
        &evidence.original_request,
        &evidence.unresolved_alternatives,
    )?;
    let mut required = requirements(query, options)?;
    if evidence.requirements.len() != required.len() {
        return Err(diagnostic(
            "request_coverage",
            "Every graph requirement needs exactly one evidence entry",
        ));
    }
    let mut spans = 0usize;
    for item in &evidence.requirements {
        options.check()?;
        let source = required.remove(&item.target).ok_or_else(|| {
            diagnostic(
                "request_coverage",
                "Graph evidence contains a duplicate or unavailable requirement",
            )
        })?;
        spans = spans.saturating_add(item.source_spans.len());
        if spans > options.max_nodes {
            return Err(diagnostic(
                "work_limit",
                "Graph request span budget exhausted",
            ));
        }
        let text = super::super::intent::span_text(&evidence.original_request, &item.source_spans)?;
        if source.is_some_and(|source| text != source) {
            return Err(diagnostic(
                "request_span",
                "Graph source text must exactly match its joined request spans",
            ));
        }
    }
    Ok(())
}
