//! Versioned semantics required by compiler-produced plans.
//!
//! This is intentionally a small, closed profile rather than a connector plugin
//! framework. Connectors may push an operation down only when they implement the
//! same behavior; otherwise DataFusion remains the execution boundary.

/// Stable identity recorded by compiler artifacts and connector capability
/// declarations. Change this value whenever an observable semantic below changes.
pub const MVP_EXECUTION_PROFILE_REVISION: &str = "semantic-datafusion-typed-v14";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IntegerSumSemantics {
    /// Accumulate wider than the result type and reject an unrepresentable result.
    CheckedAtResult,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RatioSemantics {
    /// Decimal(38, 18), with exact integer intermediates and truncation toward zero.
    Decimal38Scale18Truncate,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NullComparisonSemantics {
    /// Ordinary SQL comparisons produce unknown for null operands. Join equality
    /// therefore does not match two null keys.
    SqlThreeValued,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NullOrderingSemantics {
    /// Every compiler-produced sort records FIRST or LAST rather than inheriting
    /// a backend default.
    ExplicitPerSort,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSemantics {
    /// UTF-8 values use Arrow/DataFusion binary ordering. A remote collation is
    /// not assumed equivalent.
    BinaryUtf8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimestampSemantics {
    /// Compiler timestamps are UTC instants. Naive/local timestamps require an
    /// explicit conversion contract and are outside this profile.
    UtcInstant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AggregateNullSemantics {
    /// Null inputs are ignored and an empty/all-null SUM yields null.
    IgnoreNullsNullOnEmpty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowSemantics {
    /// Partitioning, ordering (including null placement), and frame are explicit.
    ExplicitFrameAndOrdering,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterSemantics {
    /// Parameters are positional and carry a concrete DataFusion type before
    /// execution; values are never interpolated into SQL.
    TypedPositional,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FunctionKind {
    Scalar,
    Aggregate,
    Window,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FunctionPlacement {
    /// The function has versioned local semantics and must not be emitted to a
    /// remote backend under this profile.
    LocalOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FunctionCapability {
    pub name: &'static str,
    pub kind: FunctionKind,
    pub placement: FunctionPlacement,
}

static COMPILER_FUNCTIONS: [FunctionCapability; 45] = [
    FunctionCapability {
        name: "semantic_ratio_i64_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_assert_single_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_assert_exactly_one_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_decimal_rate_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_scale_i64_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_utc_month_us_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_local_date_us_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_allocation_floor_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_allocation_remainder_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_assert_allocation_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_allocation_share_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_sum_v1",
        kind: FunctionKind::Aggregate,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_mean_i64_v1",
        kind: FunctionKind::Aggregate,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_weighted_mean_i64_v1",
        kind: FunctionKind::Aggregate,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_exact_count_i64_v1",
        kind: FunctionKind::Aggregate,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_snapshot_balance_i64_v1",
        kind: FunctionKind::Aggregate,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_sum_v1",
        kind: FunctionKind::Window,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_add_int8_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_add_int16_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_add_int32_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_add_int64_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_add_uint8_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_add_uint16_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_add_uint32_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_add_uint64_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_sub_int8_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_sub_int16_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_sub_int32_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_sub_int64_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_sub_uint8_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_sub_uint16_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_sub_uint32_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_sub_uint64_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_mul_int8_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_mul_int16_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_mul_int32_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_mul_int64_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_mul_uint8_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_mul_uint16_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_mul_uint32_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_mul_uint64_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_neg_int8_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_neg_int16_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_neg_int32_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
    FunctionCapability {
        name: "semantic_checked_neg_int64_v1",
        kind: FunctionKind::Scalar,
        placement: FunctionPlacement::LocalOnly,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ExecutionProfile {
    pub revision: &'static str,
    pub integer_sum: IntegerSumSemantics,
    pub ratio: RatioSemantics,
    pub null_comparison: NullComparisonSemantics,
    pub null_ordering: NullOrderingSemantics,
    pub text: TextSemantics,
    pub timestamp: TimestampSemantics,
    pub aggregate_nulls: AggregateNullSemantics,
    pub windows: WindowSemantics,
    pub parameters: ParameterSemantics,
}

impl ExecutionProfile {
    pub fn compiler_function(&self, kind: FunctionKind, name: &str) -> Option<FunctionCapability> {
        COMPILER_FUNCTIONS
            .iter()
            .copied()
            .find(|function| function.kind == kind && function.name == name)
    }

    pub fn compiler_functions(&self) -> &'static [FunctionCapability] {
        &COMPILER_FUNCTIONS
    }
}

pub const MVP_EXECUTION_PROFILE: ExecutionProfile = ExecutionProfile {
    revision: MVP_EXECUTION_PROFILE_REVISION,
    integer_sum: IntegerSumSemantics::CheckedAtResult,
    ratio: RatioSemantics::Decimal38Scale18Truncate,
    null_comparison: NullComparisonSemantics::SqlThreeValued,
    null_ordering: NullOrderingSemantics::ExplicitPerSort,
    text: TextSemantics::BinaryUtf8,
    timestamp: TimestampSemantics::UtcInstant,
    aggregate_nulls: AggregateNullSemantics::IgnoreNullsNullOnEmpty,
    windows: WindowSemantics::ExplicitFrameAndOrdering,
    parameters: ParameterSemantics::TypedPositional,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_compiler_function_is_explicitly_local() {
        assert_eq!(MVP_EXECUTION_PROFILE.compiler_functions().len(), 45);
        assert!(
            MVP_EXECUTION_PROFILE
                .compiler_functions()
                .iter()
                .all(|function| {
                    function.placement == FunctionPlacement::LocalOnly
                        && function.name.ends_with("_v1")
                })
        );
        assert_eq!(
            MVP_EXECUTION_PROFILE.compiler_function(FunctionKind::Scalar, "semantic_ratio_i64_v1"),
            Some(FunctionCapability {
                name: "semantic_ratio_i64_v1",
                kind: FunctionKind::Scalar,
                placement: FunctionPlacement::LocalOnly,
            })
        );
        assert_eq!(
            MVP_EXECUTION_PROFILE
                .compiler_function(FunctionKind::Scalar, "semantic_assert_exactly_one_v1",),
            Some(FunctionCapability {
                name: "semantic_assert_exactly_one_v1",
                kind: FunctionKind::Scalar,
                placement: FunctionPlacement::LocalOnly,
            })
        );
        assert_eq!(
            MVP_EXECUTION_PROFILE.compiler_function(FunctionKind::Scalar, "lower"),
            None
        );
    }
}
