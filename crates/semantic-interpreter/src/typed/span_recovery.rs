//! Bounded recovery of model offsets; the compiler remains the evidence authority.
use super::{CompileDiagnostic, InterpretOptions, InterpretationRecord, diagnostic};
use semantic_plan::{
    graph::{GraphOperation, GraphQuery, GraphRequestEvidence, GraphRequirementRef},
    typed::{RequestEvidence, RequestSpan, RowQuery},
};
use serde::Serialize;
use std::collections::BTreeSet;

/// Privacy-safe provenance for an exact unique-text offset correction.
#[derive(Debug, Serialize)]
pub struct SpanNormalization {
    pub model_attempt: usize,
    pub requirement_position: usize,
    pub evidence_position: usize,
    pub previous: Vec<RequestSpan>,
    pub replacement: RequestSpan,
}
struct Budget<'a> {
    options: &'a InterpretOptions,
    visits: usize,
    bytes: usize,
}
impl Budget<'_> {
    fn charge(&mut self, bytes: usize) -> Result<(), CompileDiagnostic> {
        self.options.check()?;
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes
            > self
                .options
                .max_nodes
                .min(8192)
                .saturating_mul(self.options.max_input_bytes)
        {
            return Err(diagnostic(
                "work_limit",
                "Evidence normalization work budget exhausted",
            ));
        }
        Ok(())
    }
    fn visit(&mut self, spans: usize) -> Result<(), CompileDiagnostic> {
        self.visits = self.visits.saturating_add(1.max(spans));
        if self.visits > self.options.max_nodes.min(8192) {
            return Err(diagnostic(
                "work_limit",
                "Evidence normalization entry budget exhausted",
            ));
        }
        self.charge(0)
    }
}
fn valid(
    request: &str,
    source: &str,
    spans: &[RequestSpan],
    budget: &mut Budget<'_>,
) -> Result<bool, CompileDiagnostic> {
    if spans.is_empty() {
        return Ok(false);
    }
    let mut previous_end = 0;
    let mut offset = 0;
    for (index, span) in spans.iter().enumerate() {
        if span.start >= span.end || span.start < previous_end {
            return Ok(false);
        }
        let Some(value) = request.get(span.start..span.end) else {
            return Ok(false);
        };
        budget.charge(value.len())?;
        if value.trim().is_empty() {
            return Ok(false);
        }
        if index > 0 {
            if source.as_bytes().get(offset) != Some(&b' ') {
                return Ok(false);
            }
            offset += 1;
        }
        if source.get(offset..offset.saturating_add(value.len())) != Some(value) {
            return Ok(false);
        }
        offset += value.len();
        previous_end = span.end;
    }
    Ok(offset == source.len())
}
fn unique(
    request: &str,
    source: &str,
    budget: &mut Budget<'_>,
) -> Result<Option<RequestSpan>, CompileDiagnostic> {
    if source.trim().is_empty() || source.len() > request.len() {
        return Ok(None);
    }
    let mut found = None;
    for (start, _) in request.char_indices() {
        if source.len() > request.len() - start {
            break;
        }
        budget.charge(source.len())?;
        if request[start..].starts_with(source) {
            if found.is_some() {
                return Ok(None);
            }
            found = Some(RequestSpan {
                start,
                end: start + source.len(),
            });
        }
    }
    Ok(found)
}
fn normalize(
    request: &str,
    source: &str,
    spans: &mut Vec<RequestSpan>,
    position: (usize, usize),
    budget: &mut Budget<'_>,
    record: &mut InterpretationRecord,
) -> Result<(), CompileDiagnostic> {
    budget.visit(spans.len())?;
    if spans.is_empty() {
        return Ok(());
    }
    if valid(request, source, spans, budget)? {
        return Ok(());
    }
    if let Some(replacement) = unique(request, source, budget)? {
        let previous = std::mem::replace(spans, vec![replacement.clone()]);
        record.work.span_normalizations += 1;
        record.span_normalizations.push(SpanNormalization {
            model_attempt: record.work.model_calls,
            requirement_position: position.0,
            evidence_position: position.1,
            previous,
            replacement,
        });
    }
    Ok(())
}
pub(super) fn row(
    query: &RowQuery,
    evidence: &mut RequestEvidence,
    options: &InterpretOptions,
    record: &mut InterpretationRecord,
) -> Result<(), CompileDiagnostic> {
    let mut budget = Budget {
        options,
        visits: 0,
        bytes: 0,
    };
    options.check()?;
    if query.requirements.len() > options.max_nodes.min(8192)
        || evidence.requirement_spans.len() > options.max_nodes.min(8192)
    {
        return Err(diagnostic(
            "work_limit",
            "Evidence normalization entry budget exhausted",
        ));
    }
    let mut requirements = Vec::new();
    semantic_plan::typed::visit_requirements(&query.requirements, &mut |requirement, depth, _| {
        options.compiler.check()?;
        if depth > options.compiler.max_depth || requirements.len() >= options.compiler.max_nodes {
            return Err(diagnostic(
                "work_limit",
                "Related normalization budget exhausted",
            ));
        }
        requirements.push(requirement);
        Ok(())
    })?;
    let ids: BTreeSet<_> = requirements.iter().map(|r| &r.id).collect();
    // Leave duplicate requirements and orphan/missing mappings for strict rejection.
    if ids.len() != requirements.len()
        || evidence.requirement_spans.len() != ids.len()
        || evidence
            .requirement_spans
            .keys()
            .any(|id| !ids.contains(id))
    {
        return Ok(());
    }
    let result = (|| {
        for (position, requirement) in requirements.iter().enumerate() {
            if let Some(spans) = evidence.requirement_spans.get_mut(&requirement.id) {
                normalize(
                    &evidence.original_request,
                    &requirement.source_text,
                    spans,
                    (position, position),
                    &mut budget,
                    record,
                )?;
            }
        }
        Ok(())
    })();
    record.work.span_normalization_bytes = record
        .work
        .span_normalization_bytes
        .saturating_add(budget.bytes);
    result
}
pub(super) fn graph(
    query: &GraphQuery,
    evidence: &mut GraphRequestEvidence,
    options: &InterpretOptions,
    record: &mut InterpretationRecord,
) -> Result<(), CompileDiagnostic> {
    let mut budget = Budget {
        options,
        visits: 0,
        bytes: 0,
    };
    options.check()?;
    if query.nodes.len() > options.max_nodes.min(8192)
        || evidence.requirements.len() > options.max_nodes.min(8192)
    {
        return Err(diagnostic(
            "work_limit",
            "Evidence normalization entry budget exhausted",
        ));
    }
    let targets: BTreeSet<_> = evidence.requirements.iter().map(|e| &e.target).collect();
    let nodes: BTreeSet<_> = query.nodes.iter().map(|n| &n.id).collect();
    if targets.len() != evidence.requirements.len() || nodes.len() != query.nodes.len() {
        return Ok(());
    }
    let result = (|| {
        for (evidence_position, entry) in evidence.requirements.iter_mut().enumerate() {
            let located = match &entry.target {
                GraphRequirementRef::Node { node } => query
                    .nodes
                    .iter()
                    .enumerate()
                    .find(|(_, n)| &n.id == node)
                    .map(|(index, n)| (index, n.source_text.as_str())),
                GraphRequirementRef::Leaf { node, requirement } => query
                    .nodes
                    .iter()
                    .find(|n| &n.id == node)
                    .and_then(|n| match &n.operation {
                        GraphOperation::Rows { query } => {
                            let mut matching = Vec::new();
                            let mut position = 0usize;
                            semantic_plan::typed::visit_requirements(
                                &query.requirements,
                                &mut |r, depth, _| {
                                    options.compiler.check()?;
                                    if depth > options.compiler.max_depth
                                        || position >= options.compiler.max_nodes
                                    {
                                        return Err(diagnostic(
                                            "work_limit",
                                            "Related normalization budget exhausted",
                                        ));
                                    }
                                    if &r.id == requirement {
                                        matching.push((position, r));
                                    }
                                    position += 1;
                                    Ok(())
                                },
                            )
                            .ok()?;
                            if matching.len() == 1 {
                                Some((matching[0].0, matching[0].1.source_text.as_str()))
                            } else {
                                None
                            }
                        }
                        _ => None,
                    }),
                _ => None,
            };
            if let Some((position, source)) = located {
                normalize(
                    &evidence.original_request,
                    source,
                    &mut entry.source_spans,
                    (position, evidence_position),
                    &mut budget,
                    record,
                )?;
            } else {
                budget.visit(entry.source_spans.len())?;
            }
        }
        Ok(())
    })();
    record.work.span_normalization_bytes = record
        .work
        .span_normalization_bytes
        .saturating_add(budget.bytes);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn budget(options: &InterpretOptions) -> Budget<'_> {
        Budget {
            options,
            visits: 0,
            bytes: 0,
        }
    }
    #[test]
    fn exact_unique_search_includes_overlapping_occurrences_and_utf8_offsets() {
        let options = InterpretOptions::default();
        assert_eq!(
            unique("List Zürich IDs", "Zürich", &mut budget(&options)).unwrap(),
            Some(RequestSpan { start: 5, end: 12 })
        );
        for (request, source) in [
            ("aaa", "aa"),
            ("one one", "one"),
            ("one", "ONE"),
            ("one", " "),
            ("one", "missing"),
        ] {
            assert_eq!(
                unique(request, source, &mut budget(&options)).unwrap(),
                None
            );
        }
    }
    #[test]
    fn valid_discontinuous_evidence_is_preserved_without_search() {
        let options = InterpretOptions::default();
        let spans = vec![
            RequestSpan { start: 0, end: 3 },
            RequestSpan { start: 11, end: 14 },
        ];
        assert!(valid("one middle two", "one two", &spans, &mut budget(&options)).unwrap());
        assert!(!valid("one middle two", "one  two", &spans, &mut budget(&options)).unwrap());
        assert!(
            !valid(
                "Zürich",
                "Zürich",
                &[RequestSpan { start: 0, end: 2 }],
                &mut budget(&options)
            )
            .unwrap()
        );
    }
    #[test]
    fn search_and_entry_work_are_bounded_and_cancellable() {
        let mut options = InterpretOptions::default();
        options.compiler.max_nodes = 1;
        options.compiler.max_input_bytes = 1;
        assert_eq!(
            unique("abcdef", "xyz", &mut budget(&options))
                .unwrap_err()
                .code,
            "work_limit"
        );
        assert_eq!(budget(&options).visit(2).unwrap_err().code, "work_limit");
        options.compiler.cancellation.cancel();
        assert_eq!(
            unique("one", "one", &mut budget(&options))
                .unwrap_err()
                .code,
            "cancelled"
        );
    }
}
