//! Deterministic lowering of the deliberately small authored-view query contract.

use std::collections::BTreeSet;

use semantic_catalog::{DataType, Relation, RelationKind};
use semantic_engine::{Engine, EngineError};
use semantic_plan::{
    Comparison, GroundedQuery, GroundingEvidence, RequestLiteral, SortDirection, ViewFilter,
    ViewSelection,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ViewSelectionError {
    #[error("select an existing authored view: {0:?}")]
    UnknownView(String),
    #[error("the view binding phrase must be a nonempty exact phrase from the request")]
    InvalidPhrase,
    #[error("select at least one column, without duplicates")]
    InvalidProjection,
    #[error("unknown column {column:?} in view {view:?}")]
    UnknownColumn { view: String, column: String },
    #[error("filter literal {0:?} must occur verbatim in the request as a complete value")]
    MissingLiteral(String),
    #[error(
        "invalid literal spelling or incompatible type for column {0:?}; no implicit conversions are supported"
    )]
    InvalidLiteral(String),
    #[error(transparent)]
    Planning(#[from] EngineError),
}

/// Apply a proposed selection to exactly one registered view, without collecting
/// rows. Preserves the view definition by construction; this does not prove that
/// the model selected the right view or preserved every natural-language clause.
pub async fn lower_view_selection(
    engine: &Engine,
    request: &str,
    selection: &ViewSelection,
) -> Result<GroundedQuery, ViewSelectionError> {
    let relation = engine
        .catalog()
        .relation(&selection.view)
        .filter(|relation| matches!(relation.kind, RelationKind::View { .. }))
        .ok_or_else(|| ViewSelectionError::UnknownView(selection.view.clone()))?;
    if selection.phrase.trim().is_empty() || !request.contains(&selection.phrase) {
        return Err(ViewSelectionError::InvalidPhrase);
    }
    if selection.columns.is_empty()
        || selection.columns.iter().collect::<BTreeSet<_>>().len() != selection.columns.len()
    {
        return Err(ViewSelectionError::InvalidProjection);
    }
    let columns = selection
        .columns
        .iter()
        .map(|name| column(relation, name))
        .collect::<Result<Vec<_>, _>>()?;
    let mut sql = format!(
        "SELECT {} FROM {}",
        columns.join(", "),
        identifier(&relation.name)
    );
    let mut filters = Vec::new();
    for filter in &selection.filters {
        filters.push(match filter {
            ViewFilter::Compare {
                column: name,
                op,
                value,
            } => {
                let quoted = column(relation, name)?;
                // column() has already checked existence.
                let field = relation
                    .schema
                    .field_with_name(name)
                    .expect("checked column");
                let literal = literal_sql(request, name, field.data_type(), value)?;
                let operator = match op {
                    Comparison::Eq => "=",
                    Comparison::NotEq => "<>",
                    Comparison::Lt => "<",
                    Comparison::Lte => "<=",
                    Comparison::Gt => ">",
                    Comparison::Gte => ">=",
                };
                format!("{quoted} {operator} {literal}")
            }
            ViewFilter::IsNull { column: name } => format!("{} IS NULL", column(relation, name)?),
            ViewFilter::IsNotNull { column: name } => {
                format!("{} IS NOT NULL", column(relation, name)?)
            }
        });
    }
    if !filters.is_empty() {
        sql.push_str(&format!(" WHERE {}", filters.join(" AND ")));
    }
    let order = selection
        .order_by
        .iter()
        .map(|order| {
            let direction = match order.direction {
                SortDirection::Asc => "ASC",
                SortDirection::Desc => "DESC",
            };
            Ok(format!(
                "{} {direction} NULLS LAST",
                column(relation, &order.column)?
            ))
        })
        .collect::<Result<Vec<_>, ViewSelectionError>>()?;
    if !order.is_empty() {
        sql.push_str(&format!(" ORDER BY {}", order.join(", ")));
    }
    engine.plan_generated_sql(&sql).await?;
    let RelationKind::View {
        sql: definition, ..
    } = &relation.kind
    else {
        unreachable!("checked view")
    };
    Ok(GroundedQuery {
        sql,
        // Generated from the actual binding, never accepted as model-authored proof.
        evidence: vec![GroundingEvidence {
            phrase: selection.phrase.clone(),
            catalog_reference: relation.name.clone(),
            interpretation: format!("Applied authored view definition unchanged: {definition}"),
        }],
    })
}

fn identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn column(relation: &Relation, name: &str) -> Result<String, ViewSelectionError> {
    relation
        .schema
        .field_with_name(name)
        .map_err(|_| ViewSelectionError::UnknownColumn {
            view: relation.name.clone(),
            column: name.into(),
        })?;
    Ok(identifier(name))
}

fn literal_sql(
    request: &str,
    column: &str,
    datatype: &DataType,
    value: &RequestLiteral,
) -> Result<String, ViewSelectionError> {
    let (RequestLiteral::Text(text) | RequestLiteral::Number(text) | RequestLiteral::Boolean(text)) =
        value;
    // A numeric fragment such as 500 in 2500, -500 or 500.5 is not an explicit
    // user value. Text values likewise cannot be substrings of a larger word.
    let numeric = matches!(value, RequestLiteral::Number(_));
    let continuation = |c: char| {
        c.is_alphanumeric() || c == '_' || (numeric && matches!(c, '.' | ',' | '+' | '-'))
    };
    let present = !text.is_empty()
        && request.match_indices(text).any(|(start, _)| {
            let mut following = request[start + text.len()..].chars();
            let next = following.next();
            // Sentence punctuation after a whole number is fine; a decimal or
            // grouping separator followed by a digit is part of that number.
            let continues_after = if numeric && matches!(next, Some('.' | ',')) {
                following.next().is_some_and(|c| c.is_ascii_digit())
            } else {
                next.is_some_and(continuation)
            };
            !request[..start]
                .chars()
                .next_back()
                .is_some_and(continuation)
                && !continues_after
        });
    if !present {
        return Err(ViewSelectionError::MissingLiteral(text.clone()));
    }
    match value {
        RequestLiteral::Text(_)
            if matches!(
                datatype,
                DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
            ) =>
        {
            Ok(format!("'{}'", text.replace('\'', "''")))
        }
        RequestLiteral::Number(_)
            if datatype.is_numeric() && text.parse::<serde_json::Number>().is_ok() =>
        {
            Ok(text.clone())
        }
        RequestLiteral::Boolean(_)
            if *datatype == DataType::Boolean && matches!(text.as_str(), "true" | "false") =>
        {
            Ok(text.to_uppercase())
        }
        _ => Err(ViewSelectionError::InvalidLiteral(column.into())),
    }
}
