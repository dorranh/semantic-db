//! PostgreSQL pushdown capabilities for the engine MVP execution profile.

use datafusion::arrow::datatypes::{DataType, TimeUnit};
use semantic_engine::MVP_EXECUTION_PROFILE_REVISION;

/// Versioned declaration used by every Postgres federation eligibility check.
/// A compiler artifact can pin `semantic_profile_revision`; connector-specific
/// changes additionally bump `revision`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PostgresExecutionProfile {
    pub revision: &'static str,
    pub semantic_profile_revision: &'static str,
}

impl PostgresExecutionProfile {
    /// Exact parameter types supported by the connector wire conversion.
    pub(crate) fn parameter_type(&self, ty: &DataType) -> Option<&'static str> {
        Some(match ty {
            DataType::Boolean => "boolean",
            DataType::Int16 => "smallint",
            DataType::Int32 => "integer",
            DataType::Int64 => "bigint",
            DataType::Utf8 => "text",
            DataType::Date32 => "date",
            DataType::Timestamp(TimeUnit::Microsecond, None) => "timestamp",
            DataType::Timestamp(TimeUnit::Microsecond, Some(tz)) if tz.as_ref() == "UTC" => {
                "timestamptz"
            }
            _ => return None,
        })
    }

    /// Types whose ordinary comparison, grouping, distinctness and ordering are
    /// equivalent for this connector. Text deliberately stays local because the
    /// database collation is not pinned to DataFusion's binary UTF-8 ordering.
    pub(crate) fn comparable(&self, ty: &DataType) -> bool {
        matches!(
            ty,
            DataType::Boolean
                | DataType::Int16
                | DataType::Int32
                | DataType::Int64
                | DataType::Date32
                | DataType::Timestamp(TimeUnit::Microsecond, None)
        ) || matches!(
            ty,
            DataType::Timestamp(TimeUnit::Microsecond, Some(tz)) if tz.as_ref() == "UTC"
        )
    }

    pub(crate) fn safe_cast(&self, source: &DataType, target: &DataType) -> bool {
        matches!(
            (source, target),
            (DataType::Int16, DataType::Int32 | DataType::Int64)
                | (DataType::Int32, DataType::Int64)
        )
    }
}

pub const POSTGRES_EXECUTION_PROFILE: PostgresExecutionProfile = PostgresExecutionProfile {
    revision: "postgres-semantic-mvp-v1",
    semantic_profile_revision: MVP_EXECUTION_PROFILE_REVISION,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_is_pinned_and_conservative() {
        assert_eq!(
            POSTGRES_EXECUTION_PROFILE.semantic_profile_revision,
            MVP_EXECUTION_PROFILE_REVISION
        );
        assert!(POSTGRES_EXECUTION_PROFILE.comparable(&DataType::Int64));
        assert!(!POSTGRES_EXECUTION_PROFILE.comparable(&DataType::Utf8));
        assert!(!POSTGRES_EXECUTION_PROFILE.comparable(&DataType::Float64));
        assert!(POSTGRES_EXECUTION_PROFILE.comparable(&DataType::Timestamp(
            TimeUnit::Microsecond,
            Some("UTC".into())
        )));
        assert!(!POSTGRES_EXECUTION_PROFILE.comparable(&DataType::Timestamp(
            TimeUnit::Microsecond,
            Some("Europe/Zurich".into())
        )));
        assert!(POSTGRES_EXECUTION_PROFILE.safe_cast(&DataType::Int16, &DataType::Int64));
        assert!(!POSTGRES_EXECUTION_PROFILE.safe_cast(&DataType::Int64, &DataType::Int32));
    }
}
