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

#[derive(Clone, Copy)]
enum Case {
    Complete,
    DuplicateSource,
    MissingBridge,
    PolicyFiltered,
    DuplicateTargetInflatedExpectation,
    MultiTarget,
}

fn allocation() -> AllocationContract {
    AllocationContract {
        version: ALLOCATION_VERSION,
        id: "allocations/tenant-order-category".into(),
        source_relation: "orders".into(),
        bridge_relation: "order_category".into(),
        source_entity_fields: vec!["tenant".into(), "order_id".into()],
        bridge_source_fields: vec!["tenant".into(), "order_id".into()],
        target_dimensions: vec!["category".into()],
        source_amount_field: "amount_minor".into(),
        amount_unit: "minor_currency_unit".into(),
        weight_field: "weight".into(),
        eligible_population: AllocationEligibility::AllRows,
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

fn policy() -> RowPolicy {
    RowPolicy {
        id: "policies/visible".into(),
        filters: vec![GovernedFilter {
            field: "visible".into(),
            operator: Comparison::Eq,
            value: Literal::Boolean(true),
        }],
        source_refs: vec![],
    }
}

fn fixture(case: Case) -> Engine {
    let mut source_tenants = vec!["A", "B", "A"];
    let mut source_ids = vec![1, 1, 2];
    let mut amounts = vec![101, 101, -5];
    let mut counts = vec![2, 2, 2];
    let mut totals = vec![3, 4, 2];
    if matches!(case, Case::DuplicateSource) {
        source_tenants.push("A");
        source_ids.push(1);
        amounts.push(101);
        counts.push(2);
        totals.push(3);
    }
    if matches!(case, Case::DuplicateTargetInflatedExpectation) {
        counts[1] = 3;
        totals[1] = 7;
    }
    let source_schema = Arc::new(Schema::new(vec![
        Field::new("tenant", DataType::Utf8, false),
        Field::new("order_id", DataType::Int64, false),
        Field::new("amount_minor", DataType::Int64, false),
        Field::new("eligible_count", DataType::Int64, false),
        Field::new("eligible_weight", DataType::Int64, false),
        Field::new("visible", DataType::Boolean, false),
    ]));
    let source_batch = RecordBatch::try_new(
        source_schema.clone(),
        vec![
            Arc::new(StringArray::from(source_tenants)) as ArrayRef,
            Arc::new(Int64Array::from(source_ids)),
            Arc::new(Int64Array::from(amounts)),
            Arc::new(Int64Array::from(counts)),
            Arc::new(Int64Array::from(totals)),
            Arc::new(BooleanArray::from(vec![
                true;
                if matches!(case, Case::DuplicateSource) {
                    4
                } else {
                    3
                }
            ])),
        ],
    )
    .unwrap();
    let mut source = Relation::base("orders", source_schema.clone(), "memory");
    let mut contract = allocation();
    if matches!(case, Case::MultiTarget) {
        contract.target_dimensions.push("tenant".into());
    }
    source.semantics = Some(RelationSemantics {
        allocations: [("category".into(), contract)].into(),
        declared_primary_key: vec!["tenant".into(), "order_id".into()],
        row_policies: vec![policy()],
        ..Default::default()
    });

    let mut tenants = vec!["A", "A", "B", "B", "A", "A"];
    let mut ids = vec![1, 1, 1, 1, 2, 2];
    let mut categories = vec!["X", "Y", "X", "Y", "X", "Y"];
    let mut weights = vec![1, 2, 3, 1, 1, 1];
    let mut visible = vec![true; 6];
    if matches!(case, Case::MissingBridge) {
        tenants.remove(3);
        ids.remove(3);
        categories.remove(3);
        weights.remove(3);
        visible.remove(3);
    }
    if matches!(case, Case::PolicyFiltered) {
        visible[1] = false;
    }
    if matches!(case, Case::DuplicateTargetInflatedExpectation) {
        tenants.push("B");
        ids.push(1);
        categories.push("X");
        weights.push(3);
        visible.push(true);
    }
    let bridge_schema = Arc::new(Schema::new(vec![
        Field::new("tenant", DataType::Utf8, false),
        Field::new("order_id", DataType::Int64, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("weight", DataType::Int64, false),
        Field::new("visible", DataType::Boolean, false),
    ]));
    let bridge_batch = RecordBatch::try_new(
        bridge_schema.clone(),
        vec![
            Arc::new(StringArray::from(tenants)) as ArrayRef,
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(categories)),
            Arc::new(Int64Array::from(weights)),
            Arc::new(BooleanArray::from(visible)),
        ],
    )
    .unwrap();
    let mut bridge = Relation::base("order_category", bridge_schema.clone(), "memory");
    bridge.semantics = Some(RelationSemantics {
        row_policies: vec![policy()],
        ..Default::default()
    });

    let mut engine = Engine::new();
    engine
        .register_table(
            bridge,
            Arc::new(MemTable::try_new(bridge_schema, vec![vec![bridge_batch]]).unwrap()),
        )
        .unwrap();
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
            id: "allocate".into(),
            source_text: "Allocate each tenant order across categories".into(),
            operation: RowOperation::Allocate {
                allocation: "category".into(),
                target_aliases: vec!["category".into()],
                amount_alias: "minor".into(),
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
                (0..batch.num_columns())
                    .map(|column| array_value_to_string(batch.column(column), row).unwrap())
                    .collect()
            })
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

#[tokio::test]
async fn composite_source_key_isolated_by_all_components_and_conserves_signed_amount() {
    let engine = fixture(Case::Complete);
    let result = compile_rows(&engine, query(), CompileOptions::default()).await;
    let query = match result.outcome {
        TypedOutcome::Compiled { query } => query,
        other => panic!("{other:?}"),
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
    let expected = vec![vec!["X", "107"], vec!["Y", "90"]];
    assert_eq!(rows(&direct), expected);
    assert_eq!(rows(&sql), expected);
}

#[tokio::test]
async fn composite_source_key_rejects_duplicate_missing_and_policy_filtered_population() {
    for case in [
        Case::DuplicateSource,
        Case::MissingBridge,
        Case::PolicyFiltered,
        Case::DuplicateTargetInflatedExpectation,
    ] {
        let engine = fixture(case);
        let result = compile_rows(&engine, query(), CompileOptions::default()).await;
        let query = match result.outcome {
            TypedOutcome::Compiled { query } => query,
            other => panic!("{other:?}"),
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
async fn target_alias_count_must_match_authored_dimensions() {
    let engine = fixture(Case::MultiTarget);
    let result = compile_rows(&engine, query(), CompileOptions::default()).await;
    assert!(
        matches!(result.outcome, TypedOutcome::Rejected { diagnostic } if diagnostic.code == "allocation_profile")
    );
}
