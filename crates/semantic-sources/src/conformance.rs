//! Reusable result checks for connector tests. Use deterministic fixture sources,
//! import both engines through Ossie, and give queries an explicit ORDER BY when
//! they return multiple rows. Pagination may change batch boundaries, not rows.
use crate::{Result, SourceError};
use datafusion::{arrow::compute::concat_batches, error::DataFusionError};
use semantic_engine::Engine;
use std::sync::Arc;

/// Compare schemas and complete query results, ignoring Arrow batch boundaries.
/// Run the same cases against a local reference and a pushdown-disabled provider.
/// This executes queries; use fixture data, not a mutable production source.
pub async fn check_query_equivalence(
    actual: &Engine,
    expected: &Engine,
    queries: &[&str],
) -> Result<()> {
    for sql in queries {
        let actual_frame = actual.plan_sql(sql).await?;
        let expected_frame = expected.plan_sql(sql).await?;
        let actual_schema = Arc::new(actual_frame.schema().as_arrow().clone());
        let expected_schema = Arc::new(expected_frame.schema().as_arrow().clone());
        if actual_schema != expected_schema {
            return Err(SourceError::configuration(
                "conformance_schema",
                "/queries",
                format!("schema mismatch for {sql}"),
            ));
        }
        let actual = concat_batches(&actual_schema, &actual.query(sql).await?)
            .map_err(DataFusionError::from)?;
        let expected = concat_batches(&expected_schema, &expected.query(sql).await?)
            .map_err(DataFusionError::from)?;
        if actual != expected {
            return Err(SourceError::configuration(
                "conformance_rows",
                "/queries",
                format!("result mismatch for {sql}"),
            ));
        }
    }
    Ok(())
}
