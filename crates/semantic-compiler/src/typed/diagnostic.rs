//! Stable, safe classification of compiler diagnostics for clients and repair.
//! The code and message remain the existing public diagnostic identity.
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticStage {
    Input,
    Intent,
    Context,
    Binding,
    Validation,
    Lowering,
    Replay,
    Provider,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticKind {
    Clarification,
    Unsupported,
    Unresolved,
    CatalogInvalid,
    ProviderFailure,
    InvalidProposal,
    InternalFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Recoverability {
    UserInput,
    ContextExpansion,
    ProposalRepair,
    CatalogChange,
    ProviderRetry,
    NewRequest,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NextAction {
    AskUser,
    ExpandContext,
    RepairProposal,
    FixCatalog,
    RetryProvider,
    ReviseRequest,
    Investigate,
}

/// Current wire outcome. Kept distinct from semantic kind so adding diagnostic
/// detail does not silently change an established compile outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticTerminal {
    Unresolved,
    ProviderFailure,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DiagnosticDetails {
    pub stage: DiagnosticStage,
    pub kind: DiagnosticKind,
    pub recoverability: Recoverability,
    pub next_action: NextAction,
    pub terminal: DiagnosticTerminal,
    /// Scoped identities may be attached by a stage with direct evidence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<String>,
}

impl DiagnosticDetails {
    const fn new(
        stage: DiagnosticStage,
        kind: DiagnosticKind,
        recoverability: Recoverability,
        next_action: NextAction,
    ) -> Self {
        Self {
            stage,
            kind,
            recoverability,
            next_action,
            terminal: DiagnosticTerminal::Rejected,
            requirement_ref: None,
            object_ref: None,
            source_ref: None,
        }
    }
}

/// Classify known diagnostic families without exposing internal error text to a
/// model repair prompt. Unknown codes fail closed as internal failures.
pub fn diagnostic_details(code: &str) -> DiagnosticDetails {
    use DiagnosticKind as K;
    use DiagnosticStage as S;
    use NextAction as A;
    use Recoverability as R;
    let mut details = match code {
        "provider_failure" => DiagnosticDetails::new(
            S::Provider,
            K::ProviderFailure,
            R::ProviderRetry,
            A::RetryProvider,
        ),
        "unresolved_alternatives"
        | "ambiguous_metric"
        | "ambiguous_calendar_boundary"
        | "ambiguous_concept"
        | "ambiguous_conversion"
        | "ambiguous_business_calendar" => {
            DiagnosticDetails::new(S::Intent, K::Clarification, R::UserInput, A::AskUser)
        }
        "unresolved_time_context" | "invalid_time_context" | "metric_unit" => {
            DiagnosticDetails::new(S::Binding, K::Clarification, R::UserInput, A::AskUser)
        }
        "interpretation_unresolved"
        | "unresolved_terms"
        | "unresolved_value"
        | "unknown_concept"
        | "unknown_conversion"
        | "unknown_allocation"
        | "unknown_business_calendar"
        | "view_coverage_unproven"
        | "partial_catalog" => DiagnosticDetails::new(
            S::Context,
            K::Unresolved,
            R::ContextExpansion,
            A::ExpandContext,
        ),
        "context_limit"
        | "search_limit"
        | "expansion_limit"
        | "graph_expansion_limit"
        | "model_call_limit"
        | "model_input_limit"
        | "model_output_limit"
        | "model_context_limit"
        | "input_limit"
        | "work_limit"
        | "requirement_limit"
        | "index_limit"
        | "capture_limit"
        | "sql_limit"
        | "page_limit"
        | "calendar_spine_limit" => {
            DiagnosticDetails::new(S::Context, K::Unresolved, R::NewRequest, A::ReviseRequest)
        }
        "deadline" | "cancelled" | "admission_closed" => {
            DiagnosticDetails::new(S::Internal, K::Unresolved, R::NewRequest, A::ReviseRequest)
        }
        "catalog_invalid"
        | "catalog_capability"
        | "metric_contract"
        | "conversion_contract"
        | "allocation_contract"
        | "business_calendar_contract"
        | "calendar_reference"
        | "enum_domain"
        | "policy_contract"
        | "relationship_contract"
        | "relationship_reference_system"
        | "view_lineage"
        | "cache_configuration" => DiagnosticDetails::new(
            S::Binding,
            K::CatalogInvalid,
            R::CatalogChange,
            A::FixCatalog,
        ),
        "unsupported_version"
        | "unsupported_order_type"
        | "unsupported_metric_state"
        | "unsupported_temporal_path"
        | "conversion_grain"
        | "conversion_type"
        | "allocation_profile"
        | "calendar_group_profile"
        | "calendar_group_type"
        | "calendar_fill_profile"
        | "calendar_spine_range"
        | "business_calendar_profile"
        | "unsupported_business_calendar_basis" => {
            DiagnosticDetails::new(S::Input, K::Unsupported, R::NewRequest, A::ReviseRequest)
        }
        "comparison_profile" => {
            DiagnosticDetails::new(S::Binding, K::Unsupported, R::NewRequest, A::ReviseRequest)
        }
        "backend_validation"
        | "invalid_relational_plan"
        | "invalid_relational_schema"
        | "invalid_relational_analysis"
        | "output_contract" => {
            DiagnosticDetails::new(S::Lowering, K::InternalFailure, R::None, A::Investigate)
        }
        "replay_context"
        | "replay_evidence"
        | "replay_incomplete"
        | "replay_mismatch"
        | "replay_profile"
        | "replay_version"
        | "replay_stage_missing"
        | "replay_capture_invalid"
        | "replay_unavailable"
        | "snapshot_mismatch" => DiagnosticDetails::new(
            S::Replay,
            K::InvalidProposal,
            R::NewRequest,
            A::ReviseRequest,
        ),
        "request_coverage"
        | "request_evidence"
        | "request_span"
        | "requirement_coverage"
        | "missing_requirements"
        | "empty_request"
        | "invalid_requirement" => DiagnosticDetails::new(
            S::Intent,
            K::InvalidProposal,
            R::ProposalRepair,
            A::RepairProposal,
        ),
        "invalid_proposal"
        | "invalid_context_request"
        | "invalid_output"
        | "empty_boolean"
        | "decimal_literal"
        | "timestamp_timezone"
        | "parameter_contract"
        | "unbound_parameter" => DiagnosticDetails::new(
            S::Input,
            K::InvalidProposal,
            R::ProposalRepair,
            A::RepairProposal,
        ),
        "parameter_count" | "parameter_missing" | "parameter_extra" | "parameter_type" => {
            DiagnosticDetails::new(S::Input, K::InvalidProposal, R::UserInput, A::AskUser)
        }
        "unknown_relation"
        | "unknown_field"
        | "unknown_metric"
        | "unknown_relationship"
        | "unknown_value_mapping"
        | "unknown_output_slot"
        | "missing_projection"
        | "graph_root"
        | "graph_edge"
        | "graph_cycle"
        | "graph_slot"
        | "graph_mapping"
        | "graph_identity"
        | "graph_evidence"
        | "graph_coverage"
        | "graph_columns"
        | "graph_order"
        | "graph_output"
        | "graph_calculation"
        | "graph_cast"
        | "graph_cast_type"
        | "graph_cast_scope"
        | "graph_null_test"
        | "graph_null_test_type"
        | "graph_null_test_scope"
        | "graph_comparison"
        | "graph_comparison_type"
        | "graph_comparison_scope"
        | "graph_comparison_unit"
        | "graph_ratio_type"
        | "graph_ratio_unit"
        | "set_type"
        | "set_unit"
        | "composition_grain"
        | "composition_key_origin"
        | "composition_key_type"
        | "composition_relationship"
        | "missing_group_type"
        | "conflicting_limits"
        | "access_scope"
        | "execution_scope"
        | "invalid_scope"
        | "aggregate_arguments"
        | "aggregate_grain"
        | "aggregate_order"
        | "aggregate_type"
        | "comparison_operator"
        | "comparison_type"
        | "cumulative_frame"
        | "cumulative_order"
        | "filter_stage"
        | "output_filter_scope"
        | "lookup_grain"
        | "path_lookup_contract"
        | "path_lookup_grain"
        | "path_lookup_keys"
        | "path_lookup_limit"
        | "metric_applicability"
        | "metric_coverage"
        | "metric_dimensions"
        | "metric_grain"
        | "metric_grounding"
        | "metric_identity"
        | "metric_rollup"
        | "metric_scope"
        | "metric_time_grain"
        | "view_applicability"
        | "ratio_grain"
        | "ratio_type"
        | "relationship_key_type"
        | "temporal_field_type"
        | "temporal_range"
        | "value_mapping_scope"
        | "enum_literal"
        | "window_contract"
        | "window_grain"
        | "window_key_type"
        | "window_order"
        | "window_scope" => DiagnosticDetails::new(
            S::Validation,
            K::InvalidProposal,
            R::ProposalRepair,
            A::RepairProposal,
        ),
        "index_revision" => DiagnosticDetails::new(
            S::Context,
            K::Unresolved,
            R::ContextExpansion,
            A::ExpandContext,
        ),
        _ => DiagnosticDetails::new(S::Internal, K::InternalFailure, R::None, A::Investigate),
    };
    details.terminal = if code == "provider_failure" {
        DiagnosticTerminal::ProviderFailure
    } else if code.ends_with("limit")
        || matches!(
            code,
            "deadline"
                | "cancelled"
                | "unresolved_terms"
                | "unresolved_value"
                | "unresolved_time_context"
                | "unresolved_alternatives"
                | "view_coverage_unproven"
                | "partial_catalog"
        )
    {
        DiagnosticTerminal::Unresolved
    } else {
        DiagnosticTerminal::Rejected
    };
    details
}
