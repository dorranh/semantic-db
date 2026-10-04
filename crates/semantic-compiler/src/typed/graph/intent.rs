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
            GraphOperation::Calculate {
                passthrough,
                ratios,
                ..
            } => {
                for slot in passthrough
                    .iter()
                    .map(|item| &item.id)
                    .chain(ratios.iter().map(|item| &item.id))
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
            GraphOperation::Project { columns, .. } => {
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
            GraphOperation::Conditional {
                passthrough,
                outputs,
                ..
            } => {
                for slot in passthrough
                    .iter()
                    .map(|item| &item.id)
                    .chain(outputs.iter().map(|item| &item.id))
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
            GraphOperation::Cast {
                passthrough, casts, ..
            } => {
                for slot in passthrough
                    .iter()
                    .map(|item| &item.id)
                    .chain(casts.iter().map(|item| &item.id))
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
            GraphOperation::NullTest {
                passthrough, tests, ..
            } => {
                for slot in passthrough
                    .iter()
                    .map(|item| &item.id)
                    .chain(tests.iter().map(|item| &item.id))
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
            GraphOperation::CompareSlots {
                passthrough,
                comparisons,
                ..
            } => {
                for slot in passthrough
                    .iter()
                    .map(|item| &item.id)
                    .chain(comparisons.iter().map(|item| &item.id))
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
            GraphOperation::Filter { .. } => {}
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
    if evidence.requirements.len() > options.max_nodes {
        return Err(diagnostic(
            "work_limit",
            "Graph evidence entry budget exhausted",
        ));
    }
    let mut seen = BTreeSet::new();
    let mut invalid_entries = Vec::new();
    for (position, item) in evidence.requirements.iter().enumerate() {
        options.check()?;
        if !required.contains_key(&item.target) || !seen.insert(item.target.clone()) {
            invalid_entries.push(position);
        }
    }
    let missing = required
        .keys()
        .filter(|target| !seen.contains(*target))
        .collect::<Vec<_>>();
    if !missing.is_empty()
        || !invalid_entries.is_empty()
        || evidence.requirements.len() != required.len()
    {
        let locations = requirement_locations(query, options)?;
        let missing_locations = missing
            .iter()
            .take(8)
            .filter_map(|target| locations.get(*target))
            .cloned()
            .collect::<Vec<_>>();
        return Err(diagnostic(
            "request_coverage",
            &format!(
                "Graph evidence requires exactly one entry per requirement; expected {}, provided {}; missing {} at {}; duplicate or unavailable {} at evidence/requirements indices {:?}",
                required.len(),
                evidence.requirements.len(),
                missing.len(),
                missing_locations.join(", "),
                invalid_entries.len(),
                invalid_entries.iter().take(8).collect::<Vec<_>>()
            ),
        ));
    }
    let mut spans = 0usize;
    for (position, item) in evidence.requirements.iter().enumerate() {
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
                &format!(
                    "Graph evidence at index {position} source_text must equal the request substrings at UTF-8 byte ranges {:?}, joined by a space. Repair source_text or the spans without changing the requirement",
                    item.source_spans
                ),
            ));
        }
    }
    Ok(())
}

/// Structural repair locations never expose model identities or request content.
fn requirement_locations(
    query: &GraphQuery,
    options: &CompileOptions,
) -> Result<BTreeMap<GraphRequirementRef, String>, CompileDiagnostic> {
    let mut locations = BTreeMap::new();
    for (node_index, node) in query.nodes.iter().enumerate() {
        options.check()?;
        locations.insert(
            GraphRequirementRef::Node {
                node: node.id.clone(),
            },
            format!("query/nodes/{node_index}"),
        );
        let mut output = |section: &str, index: usize, id: &String| {
            locations.insert(
                GraphRequirementRef::Output {
                    node: node.id.clone(),
                    slot: id.clone(),
                },
                format!("query/nodes/{node_index}/operation/{section}/{index}"),
            );
        };
        match &node.operation {
            GraphOperation::Rows { query } => {
                for (index, requirement) in query.requirements.iter().enumerate() {
                    locations.insert(
                        GraphRequirementRef::Leaf {
                            node: node.id.clone(),
                            requirement: requirement.id.clone(),
                        },
                        format!("query/nodes/{node_index}/operation/query/requirements/{index}"),
                    );
                }
            }
            GraphOperation::Set { columns, .. } => {
                for (index, column) in columns.iter().enumerate() {
                    output("columns", index, &column.id);
                }
            }
            GraphOperation::Project { columns, .. } => {
                for (index, column) in columns.iter().enumerate() {
                    output("columns", index, &column.id);
                }
            }
            GraphOperation::Compose { keys, outputs, .. } => {
                for (index, column) in keys.iter().enumerate() {
                    output("keys", index, &column.id);
                }
                for (index, column) in outputs.iter().enumerate() {
                    output("outputs", index, &column.id);
                }
            }
            GraphOperation::Calculate {
                passthrough,
                ratios,
                ..
            } => {
                for (index, column) in passthrough.iter().enumerate() {
                    output("passthrough", index, &column.id);
                }
                for (index, column) in ratios.iter().enumerate() {
                    output("ratios", index, &column.id);
                }
            }
            GraphOperation::Conditional {
                passthrough,
                outputs,
                ..
            } => {
                for (index, column) in passthrough.iter().enumerate() {
                    output("passthrough", index, &column.id);
                }
                for (index, column) in outputs.iter().enumerate() {
                    output("outputs", index, &column.id);
                }
            }
            GraphOperation::Cast {
                passthrough, casts, ..
            } => {
                for (index, column) in passthrough.iter().enumerate() {
                    output("passthrough", index, &column.id);
                }
                for (index, column) in casts.iter().enumerate() {
                    output("casts", index, &column.id);
                }
            }
            GraphOperation::NullTest {
                passthrough, tests, ..
            } => {
                for (index, column) in passthrough.iter().enumerate() {
                    output("passthrough", index, &column.id);
                }
                for (index, column) in tests.iter().enumerate() {
                    output("tests", index, &column.id);
                }
            }
            GraphOperation::CompareSlots {
                passthrough,
                comparisons,
                ..
            } => {
                for (index, column) in passthrough.iter().enumerate() {
                    output("passthrough", index, &column.id);
                }
                for (index, column) in comparisons.iter().enumerate() {
                    output("comparisons", index, &column.id);
                }
            }
            GraphOperation::Filter { .. } => {}
        }
    }
    for index in 0..query.ordering.len() {
        locations.insert(
            GraphRequirementRef::Order { index },
            format!("query/ordering/{index}"),
        );
    }
    if query.limit.is_some() {
        locations.insert(GraphRequirementRef::Limit, "query/limit".into());
    }
    Ok(locations)
}

#[cfg(test)]
mod coverage_feedback_tests {
    use super::*;
    #[test]
    fn structural_feedback_locates_missing_copied_output_without_private_content() {
        let private = "private-model-secret";
        let query: GraphQuery = serde_json::from_value(serde_json::json!({
            "version":1,"nodes":[{"id":private,"source_text":"private request secret", "operation":{"kind":"rows","query":{"version":1,"input":{"relation":"private relation","instance":"private instance"},"requirements":[{"id":"private-leaf","source_text":"private request secret","operation":{"kind":"project","field":{"instance":"private instance","field":"private field"},"alias":"private alias"}}],"unresolved":[]}}},
            {"id":"private-derived","source_text":"private request secret","operation":{"kind":"compare_slots","input":private,"passthrough":[{"id":"private-copy","slot":"private-leaf","alias":"private alias"}],"comparisons":[{"id":"private-boolean","left":"private-leaf","right":"private-leaf","operator":"eq","alias":"private bool"}]}}],
            "root":"private-derived","ordering":[],"limit":null,"unresolved":[]
        })).unwrap();
        let options = CompileOptions::default();
        let required = requirements(&query, &options).unwrap();
        let omitted = GraphRequirementRef::Output {
            node: "private-derived".into(),
            slot: "private-copy".into(),
        };
        let evidence = GraphRequestEvidence {
            version: 1,
            request_id: "private-request-id".into(),
            original_request: "private request secret".into(),
            requirements: required
                .keys()
                .filter(|target| **target != omitted)
                .map(|target| GraphRequirementEvidence {
                    target: target.clone(),
                    source_spans: vec![RequestSpan { start: 0, end: 22 }],
                })
                .collect(),
            unresolved_alternatives: vec![],
        };
        let diagnostic = validate(&query, &evidence, &options).unwrap_err();
        assert_eq!(diagnostic.code, "request_coverage");
        assert!(
            diagnostic
                .message
                .contains("query/nodes/1/operation/passthrough/0")
        );
        for rendered in [
            serde_json::to_string(&diagnostic).unwrap(),
            format!("{diagnostic:?}"),
        ] {
            for secret in [
                private,
                "private-copy",
                "private-derived",
                "private request secret",
                "private alias",
                "private-request-id",
            ] {
                assert!(!rendered.contains(secret), "{rendered}");
            }
        }
        let mut duplicate = evidence.clone();
        duplicate
            .requirements
            .push(duplicate.requirements[0].clone());
        let diagnostic = validate(&query, &duplicate, &options).unwrap_err();
        assert_eq!(diagnostic.code, "request_coverage");
        assert!(diagnostic.message.contains("duplicate or unavailable 1"));
        assert!(diagnostic.message.contains("passthrough/0"));
        let mut extra = evidence;
        extra.requirements.push(GraphRequirementEvidence {
            target: GraphRequirementRef::Node {
                node: "private-unavailable".into(),
            },
            source_spans: vec![RequestSpan { start: 0, end: 22 }],
        });
        let diagnostic = validate(&query, &extra, &options).unwrap_err();
        assert_eq!(diagnostic.code, "request_coverage");
        assert!(!diagnostic.message.contains("private-unavailable"));
    }
}
