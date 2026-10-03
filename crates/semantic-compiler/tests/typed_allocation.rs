use std::sync::Arc;

use datafusion::{
    arrow::{
        array::{ArrayRef, BooleanArray, Int64Array, StringArray},
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    ALLOCATION_VERSION, AllocationConservation, AllocationContract, AllocationDenominator,
    AllocationEligibility, AllocationRounding, GovernedFilter, NullWeight, Relation,
    RelationSemantics, RowPolicy, ZeroDenominator,
};
use semantic_compiler::typed::{CompileOptions, TypedOutcome, compile_rows};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::{
    Comparison, Literal, RelationInput, Requirement, RowOperation, RowQuery,
};

fn allocation() -> AllocationContract {
    AllocationContract {
        version: ALLOCATION_VERSION,
        id: "allocations/order-category".into(),
        source_relation: "orders".into(),
        bridge_relation: "order_category".into(),
        source_entity_fields: vec!["id".into()],
        bridge_source_fields: vec!["order_id".into()],
        target_dimensions: vec!["category".into()],
        source_amount_field: "amount_minor".into(),
        amount_unit: "minor_currency_unit".into(),
        weight_field: "weight".into(),
        eligible_population: AllocationEligibility::Utf8Equals {
            field: "status".into(),
            value: "eligible".into(),
        },
        expected_membership_count_field: "eligible_count".into(),
        expected_weight_total_field: "eligible_weight".into(),
        denominator: AllocationDenominator::SumEligibleWeightsPerSource,
        null_weight: NullWeight::Reject,
        zero_denominator: ZeroDenominator::Reject,
        rounding: AllocationRounding::LargestRemainderMinorUnit,
        conservation: AllocationConservation::ExactPerSource,
        source_refs: vec![],
    }
}

fn visible_policy(id: &str) -> RowPolicy {
    RowPolicy {
        id: id.into(),
        filters: vec![GovernedFilter {
            field: "visible".into(),
            operator: Comparison::Eq,
            value: Literal::Boolean(true),
        }],
        source_refs: vec![],
    }
}

#[derive(Clone, Copy)]
enum Case {
    Complete,
    Missing,
    Duplicate,
    PolicyFiltered,
    DuplicateSource,
    DuplicateSourceInflatedExpectations,
    NullWeight,
    NegativeWeight,
    SignedTie,
}

fn fixture(case: Case) -> Engine {
    let mut engine = Engine::new();
    let bridge_schema = Arc::new(Schema::new(vec![
        Field::new("order_id", DataType::Int64, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("weight", DataType::Int64, true),
        Field::new("status", DataType::Utf8, false),
        Field::new("visible", DataType::Boolean, false),
    ]));
    let (ids, categories, weights, visibility) = match case {
        Case::Complete | Case::DuplicateSource | Case::DuplicateSourceInflatedExpectations => (
            vec![1, 1, 2, 2, 3],
            vec!["A", "B", "A", "B", "A"],
            vec![1, 2, 1, 1, 1],
            vec![true; 5],
        ),
        Case::Missing => (
            vec![1, 2, 2, 3],
            vec!["A", "A", "B", "A"],
            vec![1, 1, 1, 1],
            vec![true; 4],
        ),
        Case::Duplicate => (
            vec![1, 1, 2, 2, 3],
            vec!["A", "A", "A", "B", "A"],
            vec![1, 2, 1, 1, 1],
            vec![true; 5],
        ),
        Case::PolicyFiltered => (
            vec![1, 1, 2, 2, 3],
            vec!["A", "B", "A", "B", "A"],
            vec![1, 2, 1, 1, 1],
            vec![true, false, true, true, true],
        ),
        Case::NullWeight | Case::NegativeWeight | Case::SignedTie => (
            vec![1, 1, 2, 2, 3],
            vec!["A", "B", "A", "B", "A"],
            vec![1, 1, 1, 1, 1],
            vec![true; 5],
        ),
    };
    let mut weights = weights.into_iter().map(Some).collect::<Vec<_>>();
    if matches!(case, Case::NullWeight) {
        weights[0] = None;
    }
    if matches!(case, Case::NegativeWeight) {
        weights[0] = Some(-1);
    }
    let status = vec!["eligible"; ids.len()];
    let bridge_batch = RecordBatch::try_new(
        bridge_schema.clone(),
        vec![
            Arc::new(Int64Array::from(ids)) as ArrayRef,
            Arc::new(StringArray::from(categories)),
            Arc::new(Int64Array::from(weights)),
            Arc::new(StringArray::from(status)),
            Arc::new(BooleanArray::from(visibility)),
        ],
    )
    .unwrap();
    let mut bridge = Relation::base("order_category", bridge_schema.clone(), "memory");
    bridge.semantics = Some(RelationSemantics {
        row_policies: vec![visible_policy("policies/visible-bridge")],
        ..Default::default()
    });
    engine
        .register_table(
            bridge,
            Arc::new(MemTable::try_new(bridge_schema, vec![vec![bridge_batch]]).unwrap()),
        )
        .unwrap();
    let source_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("amount_minor", DataType::Int64, false),
        Field::new("eligible_count", DataType::Int64, false),
        Field::new("eligible_weight", DataType::Int64, false),
        Field::new("visible", DataType::Boolean, false),
    ]));
    let source_rows = if matches!(
        case,
        Case::DuplicateSource | Case::DuplicateSourceInflatedExpectations
    ) {
        let inflated = matches!(case, Case::DuplicateSourceInflatedExpectations);
        (
            vec![1, 1, 2, 3],
            vec![101, 101, 99, 1000],
            vec![
                if inflated { 4 } else { 2 },
                if inflated { 4 } else { 2 },
                2,
                1,
            ],
            vec![
                if inflated { 6 } else { 3 },
                if inflated { 6 } else { 3 },
                2,
                1,
            ],
            vec![true, true, true, false],
        )
    } else if matches!(case, Case::SignedTie) {
        (
            vec![1, 2, 3],
            vec![-101, 99, 1000],
            vec![2, 2, 1],
            vec![2, 2, 1],
            vec![true, true, false],
        )
    } else {
        (
            vec![1, 2, 3],
            vec![101, 99, 1000],
            vec![2, 2, 1],
            vec![3, 2, 1],
            vec![true, true, false],
        )
    };
    let source_batch = RecordBatch::try_new(
        source_schema.clone(),
        vec![
            Arc::new(Int64Array::from(source_rows.0)),
            Arc::new(Int64Array::from(source_rows.1)),
            Arc::new(Int64Array::from(source_rows.2)),
            Arc::new(Int64Array::from(source_rows.3)),
            Arc::new(BooleanArray::from(source_rows.4)),
        ],
    )
    .unwrap();
    let mut source = Relation::base("orders", source_schema.clone(), "memory");
    source.semantics = Some(RelationSemantics {
        allocations: [("category".into(), allocation())].into(),
        declared_primary_key: vec!["id".into()],
        row_policies: vec![visible_policy("policies/visible-orders")],
        ..Default::default()
    });
    engine
        .register_table(
            source,
            Arc::new(MemTable::try_new(source_schema, vec![vec![source_batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn query() -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "orders".into(),
            instance: "o".into(),
        },
        requirements: vec![Requirement {
            id: "allocated_category_total".into(),
            source_text: "Allocate each order amount across categories, then total by category"
                .into(),
            operation: RowOperation::Allocate {
                allocation: "category".into(),
                target_aliases: vec!["category".into()],
                amount_alias: "allocated_minor".into(),
            },
        }],
        unresolved: vec![],
    }
}

fn rows(batches: &[RecordBatch]) -> Vec<Vec<String>> {
    let mut rows = batches
        .iter()
        .flat_map(|batch| {
            (0..batch.num_rows()).map(|row| {
                batch
                    .columns()
                    .iter()
                    .map(|column| array_value_to_string(column, row).unwrap())
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

#[tokio::test]
async fn allocation_checks_each_order_then_totals_target_with_sql_direct_parity() {
    let engine = fixture(Case::Complete);
    let compilation = compile_rows(&engine, query(), CompileOptions::default()).await;
    assert_eq!(compilation.record.execution_obligations.len(), 1);
    let query = match compilation.outcome {
        TypedOutcome::Compiled { query } => query,
        other => panic!("allocation rejected: {other:?}"),
    };
    let direct = query
        .plan_direct(&engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = query
        .execute(&engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = vec![vec!["A", "84"], vec!["B", "116"]];
    assert_eq!(rows(&direct), expected);
    assert_eq!(rows(&sql), expected);
}

#[tokio::test]
async fn allocation_rejects_missing_duplicate_and_policy_filtered_bridge_rows() {
    for case in [
        Case::Missing,
        Case::Duplicate,
        Case::PolicyFiltered,
        Case::DuplicateSource,
        Case::DuplicateSourceInflatedExpectations,
        Case::NullWeight,
        Case::NegativeWeight,
    ] {
        let engine = fixture(case);
        let compilation = compile_rows(&engine, query(), CompileOptions::default()).await;
        let query = match compilation.outcome {
            TypedOutcome::Compiled { query } => query,
            other => panic!("allocation proposal rejected: {other:?}"),
        };
        assert!(
            query
                .plan_direct(&engine)
                .await
                .unwrap()
                .collect()
                .await
                .is_err()
        );
        assert!(
            query
                .execute(&engine, QueryOptions::default())
                .await
                .unwrap()
                .collect()
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn negative_amount_with_equal_remainders_conserves_exact_minor_units() {
    let engine = fixture(Case::SignedTie);
    let compilation = compile_rows(&engine, query(), CompileOptions::default()).await;
    let query = match compilation.outcome {
        TypedOutcome::Compiled { query } => query,
        other => panic!("signed tie allocation rejected: {other:?}"),
    };
    let direct = query
        .plan_direct(&engine)
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let sql = query
        .execute(&engine, QueryOptions::default())
        .await
        .unwrap()
        .collect()
        .await
        .unwrap();
    let expected = vec![vec!["A", "-1"], vec!["B", "-1"]];
    assert_eq!(rows(&direct), expected);
    assert_eq!(rows(&sql), expected);
}

#[tokio::test]
async fn allocation_is_terminal_and_requires_authored_contract() {
    let engine = fixture(Case::Complete);
    let mut proposal = query();
    if let RowOperation::Allocate { allocation, .. } = &mut proposal.requirements[0].operation {
        *allocation = "unlisted".into();
    }
    assert!(
        matches!(compile_rows(&engine, proposal, CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "unknown_allocation")
    );
    let mut proposal = query();
    proposal.requirements.push(Requirement {
        id: "other".into(),
        source_text: "other".into(),
        operation: RowOperation::Limit { count: 1 },
    });
    assert!(
        matches!(compile_rows(&engine, proposal, CompileOptions::default()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "allocation_profile")
    );
}
