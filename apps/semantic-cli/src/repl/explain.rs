use semantic_compiler::typed::TypedOutcome;
use semantic_interpreter::typed::InterpretedCompilation;
use semantic_plan::typed::{AggregateFunction, RowOperation};

pub(crate) fn format_compilation(compilation: &InterpretedCompilation) -> Option<String> {
    let mut lines =
        vec!["Interpretation (model-proposed, compiler-checked operations):".to_owned()];
    match &compilation.outcome {
        TypedOutcome::Compiled { query } => {
            for requirement in &query.intent().requirements {
                lines.push(format!(
                    "  {} → {}",
                    quoted_excerpt(&requirement.source_text),
                    operation_summary(&requirement.operation)
                ));
            }
        }
        TypedOutcome::CompiledGraph { .. } => lines.push(format!(
            "  Graph plan: {} checked requirement(s)",
            compilation.record.requirement_dispositions.len()
        )),
        _ => return None,
    }

    let definitions: std::collections::BTreeSet<_> = compilation
        .record
        .definition_refs
        .iter()
        .map(|reference| reference.id.as_str())
        .collect();
    if !definitions.is_empty() {
        let shown = definitions.iter().take(6).copied().collect::<Vec<_>>();
        let more = definitions.len().saturating_sub(shown.len());
        lines.push(format!(
            "  Catalog: {}{}",
            shown.join(", "),
            if more > 0 {
                format!(" (+{more} more)")
            } else {
                String::new()
            }
        ));
    }
    if !compilation.record.execution_obligations.is_empty() {
        lines.push(format!(
            "  Runtime checks required: {}",
            compilation.record.execution_obligations.len()
        ));
    }
    lines.push(if compilation.record.request_spans_validated {
        "  Request mapping: phrase spans checked; interpretation completeness is unproven".into()
    } else {
        "  Request mapping: original wording coverage not verified".into()
    });
    Some(lines.join("\n"))
}

fn quoted_excerpt(text: &str) -> String {
    let mut chars = text.chars();
    let mut excerpt = chars.by_ref().take(100).collect::<String>();
    if chars.next().is_some() {
        excerpt.push('…');
    }
    format!("{excerpt:?}")
}

fn operation_summary(operation: &RowOperation) -> String {
    match operation {
        RowOperation::Group { field, .. } => format!("group by {}", field.field),
        RowOperation::Aggregate {
            function,
            field,
            distinct,
            ..
        } => {
            let name = match function {
                AggregateFunction::Count => "count",
                AggregateFunction::Sum => "sum",
                AggregateFunction::Min => "minimum",
                AggregateFunction::Max => "maximum",
                AggregateFunction::Avg => "average",
            };
            match field {
                Some(field) => format!(
                    "{name} {}{}",
                    if *distinct { "distinct " } else { "" },
                    field.field
                ),
                None => format!("{name} rows"),
            }
        }
        RowOperation::Project { field, .. } => format!("select {}", field.field),
        RowOperation::Filter { .. } => "filter rows".into(),
        RowOperation::Order { field, .. } => format!("sort by {}", field.field),
        RowOperation::OrderOutput { .. } => "sort results".into(),
        RowOperation::Limit { count } => format!("limit to {count} rows"),
        other => serde_json::to_value(other)
            .ok()
            .and_then(|value| value["kind"].as_str().map(str::to_owned))
            .unwrap_or_else(|| "checked operation".into())
            .replace('_', " "),
    }
}

#[cfg(test)]
mod tests {
    use semantic_plan::typed::{FieldRef, RowOperation};

    use super::{operation_summary, quoted_excerpt};

    #[test]
    fn formats_a_grouped_count_for_a_demo_without_internal_ids() {
        assert_eq!(
            operation_summary(&RowOperation::Group {
                field: FieldRef {
                    instance: "r".into(),
                    field: "basin".into(),
                },
                alias: "basin".into(),
            }),
            "group by basin"
        );
        assert_eq!(
            operation_summary(&RowOperation::Aggregate {
                function: semantic_plan::typed::AggregateFunction::Count,
                field: None,
                distinct: false,
                alias: "well_count".into(),
            }),
            "count rows"
        );
        assert_eq!(quoted_excerpt("by basin"), "\"by basin\"");
        assert!(quoted_excerpt("\u{1b}[2J").contains("\\u{1b}"));
    }
}
