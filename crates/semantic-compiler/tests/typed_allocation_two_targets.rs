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
    Missing,
    PolicyFiltered,
    DuplicateTupleInflated,
}

fn allocation() -> AllocationContract {
    AllocationContract {
        version: ALLOCATION_VERSION,
        id: "allocations/tenant-order-category-channel".into(),
        source_relation: "orders".into(),
        bridge_relation: "order_targets".into(),
        source_entity_fields: vec!["tenant".into(), "order_id".into()],
        bridge_source_fields: vec!["tenant".into(), "order_id".into()],
        target_dimensions: vec!["category".into(), "channel".into()],
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
    let source_schema = Arc::new(Schema::new(vec![
        Field::new("tenant", DataType::Utf8, false),
        Field::new("order_id", DataType::Int64, false),
        Field::new("amount_minor", DataType::Int64, false),
        Field::new("eligible_count", DataType::Int64, false),
        Field::new("eligible_weight", DataType::Int64, false),
        Field::new("visible", DataType::Boolean, false),
    ]));
    let inflated = matches!(case, Case::DuplicateTupleInflated);
    let source_batch = RecordBatch::try_new(
        source_schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["A", "B"])) as ArrayRef,
            Arc::new(Int64Array::from(vec![1, 1])),
            Arc::new(Int64Array::from(vec![101, -5])),
            Arc::new(Int64Array::from(vec![if inflated { 4 } else { 3 }, 2])),
            Arc::new(Int64Array::from(vec![if inflated { 6 } else { 4 }, 2])),
            Arc::new(BooleanArray::from(vec![true, true])),
        ],
    )
    .unwrap();
    let mut source = Relation::base("orders", source_schema.clone(), "memory");
    source.semantics = Some(RelationSemantics {
        allocations: [("category_channel".into(), allocation())].into(),
        declared_primary_key: vec!["tenant".into(), "order_id".into()],
        row_policies: vec![policy()],
        ..Default::default()
    });

    let mut tenants = vec!["A", "A", "A", "B", "B"];
    let mut categories = vec!["X", "X", "Y", "X", "Y"];
    let mut channels = vec!["web", "retail", "web", "web", "web"];
    let mut weights = vec![1, 2, 1, 1, 1];
    let mut visible = vec![true; 5];
    if matches!(case, Case::Missing) {
        tenants.remove(2);
        categories.remove(2);
        channels.remove(2);
        weights.remove(2);
        visible.remove(2);
    }
    if matches!(case, Case::PolicyFiltered) {
        visible[2] = false;
    }
    if inflated {
        tenants.push("A");
        categories.push("X");
        channels.push("retail");
        weights.push(2);
        visible.push(true);
    }
    let order_ids = tenants.iter().map(|_| 1).collect::<Vec<i64>>();
    let bridge_schema = Arc::new(Schema::new(vec![
        Field::new("tenant", DataType::Utf8, false),
        Field::new("order_id", DataType::Int64, false),
        Field::new("category", DataType::Utf8, false),
        Field::new("channel", DataType::Utf8, false),
        Field::new("weight", DataType::Int64, false),
        Field::new("visible", DataType::Boolean, false),
    ]));
    let bridge_batch = RecordBatch::try_new(
        bridge_schema.clone(),
        vec![
            Arc::new(StringArray::from(tenants)) as ArrayRef,
            Arc::new(Int64Array::from(order_ids)),
            Arc::new(StringArray::from(categories)),
            Arc::new(StringArray::from(channels)),
            Arc::new(Int64Array::from(weights)),
            Arc::new(BooleanArray::from(visible)),
        ],
    )
    .unwrap();
    let mut bridge = Relation::base("order_targets", bridge_schema.clone(), "memory");
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
            source_text: "Allocate each order across category and channel".into(),
            operation: RowOperation::Allocate {
                allocation: "category_channel".into(),
                target_aliases: vec!["category".into(), "channel".into()],
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
async fn two_target_dimensions_have_independent_columns_and_exact_signed_shares() {
    let engine = fixture(Case::Complete);
    let result = compile_rows(&engine, query(), CompileOptions::default()).await;
    let query = match result.outcome {
        TypedOutcome::Compiled { query } => query,
        other => panic!("{other:?}"),
    };
    let output = query.sql().expected_output();
    assert_eq!(
        output
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        vec!["category", "channel", "minor"]
    );
    assert_eq!(
        output
            .iter()
            .map(|field| field.data_type.as_str())
            .collect::<Vec<_>>(),
        vec!["Utf8", "Utf8", "Int64"]
    );
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
    let expected = vec![
        vec!["X", "retail", "51"],
        vec!["X", "web", "22"],
        vec!["Y", "web", "23"],
    ];
    assert_eq!(rows(&direct), expected);
    assert_eq!(rows(&sql), expected);
}

#[tokio::test]
async fn two_target_population_fails_on_missing_policy_filtered_and_duplicate_tuple() {
    for case in [
        Case::Missing,
        Case::PolicyFiltered,
        Case::DuplicateTupleInflated,
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
