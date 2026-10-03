use semantic_compiler::typed::{
    CompileOptions, DiagnosticKind, DiagnosticStage, DiagnosticTerminal, NextAction,
    Recoverability, TypedOutcome, compile_rows, diagnostic_details,
};
use semantic_engine::Engine;
use semantic_plan::typed::{RelationInput, RowQuery};

#[test]
fn diagnostic_families_have_distinct_safe_actions() {
    let cases = [
        (
            "unresolved_alternatives",
            DiagnosticStage::Intent,
            DiagnosticKind::Clarification,
            Recoverability::UserInput,
            NextAction::AskUser,
            DiagnosticTerminal::Unresolved,
        ),
        (
            "partial_catalog",
            DiagnosticStage::Context,
            DiagnosticKind::Unresolved,
            Recoverability::ContextExpansion,
            NextAction::ExpandContext,
            DiagnosticTerminal::Unresolved,
        ),
        (
            "catalog_invalid",
            DiagnosticStage::Binding,
            DiagnosticKind::CatalogInvalid,
            Recoverability::CatalogChange,
            NextAction::FixCatalog,
            DiagnosticTerminal::Rejected,
        ),
        (
            "relationship_reference_system",
            DiagnosticStage::Binding,
            DiagnosticKind::CatalogInvalid,
            Recoverability::CatalogChange,
            NextAction::FixCatalog,
            DiagnosticTerminal::Rejected,
        ),
        (
            "calendar_reference",
            DiagnosticStage::Binding,
            DiagnosticKind::CatalogInvalid,
            Recoverability::CatalogChange,
            NextAction::FixCatalog,
            DiagnosticTerminal::Rejected,
        ),
        (
            "enum_domain",
            DiagnosticStage::Binding,
            DiagnosticKind::CatalogInvalid,
            Recoverability::CatalogChange,
            NextAction::FixCatalog,
            DiagnosticTerminal::Rejected,
        ),
        (
            "enum_literal",
            DiagnosticStage::Validation,
            DiagnosticKind::InvalidProposal,
            Recoverability::ProposalRepair,
            NextAction::RepairProposal,
            DiagnosticTerminal::Rejected,
        ),
        (
            "provider_failure",
            DiagnosticStage::Provider,
            DiagnosticKind::ProviderFailure,
            Recoverability::ProviderRetry,
            NextAction::RetryProvider,
            DiagnosticTerminal::ProviderFailure,
        ),
        (
            "invalid_proposal",
            DiagnosticStage::Input,
            DiagnosticKind::InvalidProposal,
            Recoverability::ProposalRepair,
            NextAction::RepairProposal,
            DiagnosticTerminal::Rejected,
        ),
        (
            "unsupported_version",
            DiagnosticStage::Input,
            DiagnosticKind::Unsupported,
            Recoverability::NewRequest,
            NextAction::ReviseRequest,
            DiagnosticTerminal::Rejected,
        ),
        (
            "comparison_profile",
            DiagnosticStage::Binding,
            DiagnosticKind::Unsupported,
            Recoverability::NewRequest,
            NextAction::ReviseRequest,
            DiagnosticTerminal::Rejected,
        ),
        (
            "replay_capture_invalid",
            DiagnosticStage::Replay,
            DiagnosticKind::InvalidProposal,
            Recoverability::NewRequest,
            NextAction::ReviseRequest,
            DiagnosticTerminal::Rejected,
        ),
        (
            "invalid_relational_analysis",
            DiagnosticStage::Lowering,
            DiagnosticKind::InternalFailure,
            Recoverability::None,
            NextAction::Investigate,
            DiagnosticTerminal::Rejected,
        ),
        (
            "future_unrecognized_code",
            DiagnosticStage::Internal,
            DiagnosticKind::InternalFailure,
            Recoverability::None,
            NextAction::Investigate,
            DiagnosticTerminal::Rejected,
        ),
    ];
    for (code, stage, kind, recoverability, next_action, terminal) in cases {
        let details = diagnostic_details(code);
        assert_eq!(details.stage, stage, "{code}");
        assert_eq!(details.kind, kind, "{code}");
        assert_eq!(details.recoverability, recoverability, "{code}");
        assert_eq!(details.next_action, next_action, "{code}");
        assert_eq!(details.terminal, terminal, "{code}");
        assert!(details.requirement_ref.is_none());
        assert!(details.object_ref.is_none());
        assert!(details.source_ref.is_none());
    }
}

#[tokio::test]
async fn actual_compile_error_carries_typed_details_and_original_code() {
    let query = RowQuery {
        version: 99,
        input: RelationInput {
            relation: "unused".into(),
            instance: "r".into(),
        },
        requirements: vec![],
        unresolved: vec![],
    };
    let result = compile_rows(&Engine::new(), query, CompileOptions::default()).await;
    let TypedOutcome::Rejected { diagnostic } = result.outcome else {
        panic!("unsupported version must reject");
    };
    assert_eq!(diagnostic.code, "unsupported_version");
    assert_eq!(diagnostic.details.kind, DiagnosticKind::Unsupported);
    assert_eq!(diagnostic.details.next_action, NextAction::ReviseRequest);
}
