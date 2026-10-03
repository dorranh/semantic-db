//! Architecture §12.2: observed UTC months, current billing region, and exact money.
use std::{collections::BTreeMap, sync::Arc};

use datafusion::{
    arrow::{
        array::{
            Array, ArrayRef, Decimal128Array, Int64Array, StringArray, TimestampMicrosecondArray,
        },
        datatypes::{DataType, Field, Schema, TimeUnit},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    CalendarReference, CalendarSystem, EmptyBehavior, FactResolution, FieldSemantics, GrainKey,
    MetricDefinition, MetricLookupDimension, MetricTemporalApplicability, Presence, Relation,
    RelationSemantics, RelationshipDefinition, RelationshipKey, SourceGrain, Unit,
};
use semantic_compiler::typed::{
    Calendar, CompileOptions, ContextOrigin, RequestContext, TypedOutcome, compile_rows,
};
use semantic_engine::{Engine, QueryOptions};
use semantic_plan::typed::*;

const JAN_1: i64 = 1_767_225_600_000_000;
const FEB_1: i64 = 1_769_904_000_000_000;
const MAR_1: i64 = 1_772_323_200_000_000;
const APR_1: i64 = 1_775_001_600_000_000;

fn timestamp(ticks: i64) -> Literal {
    Literal::Timestamp {
        ticks,
        unit: TimestampUnit::Microsecond,
        timezone: Some("UTC".into()),
    }
}

fn engine() -> Engine {
    let order_schema = Arc::new(Schema::new(vec![
        Field::new("order_id", DataType::Int64, false),
        Field::new("billing_customer_id", DataType::Int64, true),
        Field::new(
            "ordered_at",
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
        Field::new("net_amount_usd", DataType::Decimal128(20, 2), false),
    ]));
    let amounts = Decimal128Array::from(vec![10000i128, 5000, 5000, 700, 3000, 90000])
        .with_precision_and_scale(20, 2)
        .unwrap();
    let order_batch = RecordBatch::try_new(
        order_schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3, 4, 5, 6])) as ArrayRef,
            Arc::new(Int64Array::from(vec![
                Some(10),
                Some(20),
                Some(20),
                None,
                Some(20),
                Some(10),
            ])) as ArrayRef,
            Arc::new(
                TimestampMicrosecondArray::from(vec![
                    JAN_1 + 4 * 86_400_000_000,
                    JAN_1 + 19 * 86_400_000_000,
                    JAN_1 + 24 * 86_400_000_000,
                    JAN_1 + 29 * 86_400_000_000,
                    MAR_1 + 86_400_000_000,
                    APR_1,
                ])
                .with_timezone_opt(Some("UTC")),
            ) as ArrayRef,
            Arc::new(amounts) as ArrayRef,
        ],
    )
    .unwrap();
    let mut orders = Relation::base("orders", order_schema.clone(), "memory");
    orders.semantics = Some(RelationSemantics {
        fields: [(
            "ordered_at".into(),
            FieldSemantics {
                calendar_reference: Some(CalendarReference {
                    system: CalendarSystem::Gregorian,
                    timezone: "UTC".into(),
                }),
                ..Default::default()
            },
        )]
        .into(),
        metrics: [(
            "net_revenue".into(),
            MetricDefinition {
                id: "sales/net-revenue".into(),
                description: "Authored net order amount in USD".into(),
                aliases: vec![],
                function: AggregateFunction::Sum,
                field: Some("net_amount_usd".into()),
                distinct: false,
                source_grain: SourceGrain {
                    entity: None,
                    keys: vec![GrainKey {
                        relation: "orders".into(),
                        field: "order_id".into(),
                    }],
                },
                compatible_dimensions: ["ordered_at".into()].into(),
                compatible_lookup_dimensions: vec![MetricLookupDimension {
                    relationship: "billing_customer".into(),
                    field: "region".into(),
                    missing: MissingMatch::Null,
                }],
                sum_rollup_dimensions: None,
                state: None,
                row_filters: vec![],
                result_type: DataType::Decimal128(30, 2),
                unit: Presence::Value(Unit::Currency { code: "USD".into() }),
                temporal: Presence::Value(MetricTemporalApplicability {
                    field: "ordered_at".into(),
                    grain: CalendarUnit::Month,
                    coverage_start: timestamp(JAN_1),
                    coverage_end: timestamp(APR_1),
                }),
                empty_behavior: EmptyBehavior::Null,
                source_refs: vec![],
            },
        )]
        .into(),
        relationships: [(
            "billing_customer".into(),
            RelationshipDefinition {
                ai_context: None,
                id: "relationships/billing-customer".into(),
                right_relation: "customers".into(),
                role: "billing_customer".into(),
                key_pairs: vec![RelationshipKey {
                    left_field: "billing_customer_id".into(),
                    right_field: "customer_id".into(),
                }],
                null_keys_match: false,
                cardinality: FactResolution::Unknown,
                source_refs: vec![],
            },
        )]
        .into(),
        ..Default::default()
    });

    let customer_schema = Arc::new(Schema::new(vec![
        Field::new("customer_id", DataType::Int64, false),
        Field::new("region", DataType::Utf8, true),
    ]));
    let customer_batch = RecordBatch::try_new(
        customer_schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![10, 20])) as ArrayRef,
            Arc::new(StringArray::from(vec![Some("North"), Some("South")])) as ArrayRef,
        ],
    )
    .unwrap();

    let mut engine = Engine::new();
    engine
        .register_table(
            orders,
            Arc::new(MemTable::try_new(order_schema, vec![vec![order_batch]]).unwrap()),
        )
        .unwrap();
    engine
        .register_table(
            Relation::base("customers", customer_schema.clone(), "memory"),
            Arc::new(MemTable::try_new(customer_schema, vec![vec![customer_batch]]).unwrap()),
        )
        .unwrap();
    engine
}

fn query() -> RowQuery {
    let at = FieldRef {
        instance: "o".into(),
        field: "ordered_at".into(),
    };
    let requirement = |id: &str, source_text: &str, operation| Requirement {
        id: id.into(),
        source_text: source_text.into(),
        operation,
    };
    RowQuery {
        version: ROW_QUERY_VERSION,
        input: RelationInput {
            relation: "orders".into(),
            instance: "o".into(),
        },
        requirements: vec![
            requirement(
                "quarter",
                "January through March 2026",
                RowOperation::CalendarFilter {
                    field: at.clone(),
                    period: CalendarPeriod {
                        unit: CalendarUnit::Month,
                        offset: -3,
                        count: 3,
                    },
                },
            ),
            requirement(
                "month",
                "by UTC month",
                RowOperation::CalendarGroup {
                    field: at,
                    grain: CalendarUnit::Month,
                    timezone: "UTC".into(),
                    alias: "month".into(),
                },
            ),
            requirement(
                "region",
                "current billing-customer region",
                RowOperation::Lookup {
                    relationship: "billing_customer".into(),
                    role: "billing_customer".into(),
                    instance: "billing".into(),
                    field: "region".into(),
                    alias: "region".into(),
                    missing: MissingMatch::Null,
                    usage: LookupUsage::Group,
                },
            ),
            requirement(
                "revenue",
                "net revenue",
                RowOperation::Metric {
                    name: "net_revenue".into(),
                    alias: "net_revenue".into(),
                    applicability: MetricApplicability {
                        required_unit: Some(Unit::Currency { code: "USD".into() }),
                        required_source_grain: Some(SourceGrain {
                            entity: None,
                            keys: vec![GrainKey {
                                relation: "orders".into(),
                                field: "order_id".into(),
                            }],
                        }),
                    },
                },
            ),
        ],
        unresolved: vec![],
    }
}

fn options() -> CompileOptions {
    let mut options = CompileOptions::default();
    options.request_context = Some(RequestContext {
        reference_unix_millis: 1_776_211_200_000, // 2026-04-15T00:00:00Z
        timezone: "UTC".into(),
        calendar: Calendar::Gregorian,
        origin: ContextOrigin::Caller,
    });
    options
}

fn rows(batches: &[RecordBatch]) -> BTreeMap<(i64, Option<String>), i128> {
    let mut rows = BTreeMap::new();
    for batch in batches {
        let months = batch
            .column(0)
            .as_any()
            .downcast_ref::<TimestampMicrosecondArray>()
            .unwrap();
        let regions = batch
            .column(1)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        let amounts = batch
            .column(2)
            .as_any()
            .downcast_ref::<Decimal128Array>()
            .unwrap();
        for index in 0..batch.num_rows() {
            assert!(!amounts.is_null(index));
            let region = (!regions.is_null(index)).then(|| regions.value(index).to_owned());
            assert!(
                rows.insert((months.value(index), region), amounts.value(index))
                    .is_none()
            );
        }
    }
    rows
}

#[tokio::test]
async fn observed_month_current_region_revenue_has_exact_sql_and_direct_rows() {
    let engine = engine();
    let result = compile_rows(&engine, query(), options()).await;
    let TypedOutcome::Compiled { query } = result.outcome else {
        panic!("§12.2 metric rejected: {:?}", result.outcome)
    };
    assert_eq!(
        query
            .sql()
            .expected_output()
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>(),
        ["month", "region", "net_revenue"]
    );
    let expected = BTreeMap::from([
        ((JAN_1, None), 700),
        ((JAN_1, Some("North".into())), 10000),
        ((JAN_1, Some("South".into())), 10000),
        ((MAR_1, Some("South".into())), 3000),
    ]);
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
    assert_eq!(rows(&direct), expected);
    assert_eq!(rows(&sql), expected);
    assert!(
        !rows(&sql)
            .keys()
            .any(|(month, _)| *month == FEB_1 || *month == APR_1)
    );
}

#[tokio::test]
async fn historical_region_cannot_reuse_the_current_region_role() {
    let engine = engine();
    let mut historical = query();
    let RowOperation::Lookup {
        relationship, role, ..
    } = &mut historical.requirements[2].operation
    else {
        unreachable!()
    };
    *relationship = "billing_customer_at_order_time".into();
    *role = "billing_customer_at_order_time".into();
    assert!(matches!(
        compile_rows(&engine, historical, options()).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "unknown_relationship"
    ));
}
