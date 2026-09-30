//! Repeatable held-out compiler evaluation. `--scripted` is deterministic
//! orchestration coverage; `--live` uses an explicitly configured provider.

use std::{error::Error, sync::Arc, time::Duration};

use datafusion::{
    arrow::{
        array::{
            ArrayRef, BooleanArray, Date32Array, Int16Array, Int32Array, Int64Array, StringArray,
            TimestampMicrosecondArray,
        },
        datatypes::{DataType, Field, Schema, TimeUnit},
        record_batch::RecordBatch,
        util::display::array_value_to_string,
    },
    datasource::MemTable,
};
use semantic_catalog::{
    BUSINESS_CALENDAR_VERSION, BusinessCalendarRule, CalendarSourceBasis, ConceptDefinition,
    ConversionRounding, EmptyBehavior, FactResolution, GovernedFilter, MetricDefinition, Presence,
    Relation, RelationSemantics, RelationshipDefinition, RelationshipKey, RowPolicy,
    UNIT_CONVERSION_VERSION, UnitConversion,
};
use semantic_compiler::{
    Compiler,
    provider::{Message, ModelProvider, OpenAiConfig, OpenAiProvider, ProviderError},
    typed::{CompileOptions, SelectionMode, TypedOutcome},
};
use semantic_engine::Engine;
use semantic_plan::typed::{AggregateFunction, Comparison, Literal, RowPredicate};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const FIXTURE: &str = include_str!("../tests/fixtures/held_out_eval.json");

#[derive(Deserialize)]
struct Manifest {
    profile: String,
    claim: String,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct Case {
    id: String,
    request: String,
    #[serde(default = "orders_profile")]
    fixture: String,
    #[serde(default = "both_modes")]
    modes: Vec<String>,
    competing_alias: bool,
    policy: bool,
    required_facts: Vec<Fact>,
    scripted_output: Value,
    expected: ExpectedByMode,
}
fn orders_profile() -> String {
    "orders".into()
}
fn both_modes() -> Vec<String> {
    vec!["full".into(), "retrieved".into()]
}
#[derive(Deserialize)]
struct ExpectedByMode {
    full: Expected,
    retrieved: Expected,
}
#[derive(Deserialize)]
struct Expected {
    outcome: String,
    diagnostic: Option<String>,
    rows: Option<Vec<Vec<String>>>,
}
#[derive(Deserialize)]
struct Fact {
    relation: String,
    kind: String,
    id: String,
}

enum Provider {
    Scripted(String),
    Live(OpenAiProvider),
}
impl ModelProvider for Provider {
    async fn complete(&self, messages: &[Message]) -> Result<String, ProviderError> {
        match self {
            Self::Scripted(response) => Ok(response.clone()),
            Self::Live(provider) => provider.complete(messages).await,
        }
    }
}

#[derive(Serialize)]
struct CaseReport {
    id: String,
    context: &'static str,
    expected: String,
    actual: String,
    actual_diagnostic: Option<String>,
    outcome_match: bool,
    diagnostic_match: bool,
    rows_match: Option<bool>,
    incorrect_accept: bool,
    false_refusal: bool,
    required_facts: usize,
    recalled_facts: usize,
    model_calls: usize,
    elapsed_micros: u128,
    input_tokens: u128,
    output_tokens: u128,
    token_usage_incomplete: bool,
}
#[derive(Serialize)]
struct Report {
    profile: String,
    provider: &'static str,
    claim: String,
    cases: Vec<CaseReport>,
    outcome_matches: usize,
    incorrect_accepts: usize,
    false_refusals: usize,
    required_facts: usize,
    recalled_facts: usize,
}

fn concept(name: &str, state: &str) -> ConceptDefinition {
    ConceptDefinition {
        id: format!("orders/concepts/{name}"),
        description: format!("Orders in {state} state"),
        aliases: vec!["open".into()],
        alternatives: vec![],
        predicate: RowPredicate::Compare {
            field: "state".into(),
            operator: Comparison::Eq,
            value: Literal::Utf8(state.into()),
        },
        source_refs: vec![],
    }
}

fn orders_engine(case: &Case) -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("state", DataType::Utf8, false),
        Field::new("amount", DataType::Int64, false),
    ]));
    let mut orders = Relation::base("orders", schema.clone(), "fixture:orders");
    let mut semantics = RelationSemantics {
        concepts: [("active".into(), concept("active", "open"))].into(),
        ..Default::default()
    };
    if case.competing_alias {
        semantics
            .concepts
            .insert("pending".into(), concept("pending", "pending"));
    }
    if case.policy {
        semantics.row_policies.push(RowPolicy {
            id: "orders/policies/visible".into(),
            filters: vec![GovernedFilter {
                field: "state".into(),
                operator: Comparison::Eq,
                value: Literal::Utf8("open".into()),
            }],
            source_refs: vec![],
        });
    }
    orders.semantics = Some(semantics);
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])) as ArrayRef,
            Arc::new(StringArray::from(vec!["open", "closed", "open"])) as ArrayRef,
            Arc::new(Int64Array::from(vec![10, 20, 30])) as ArrayRef,
        ],
    )
    .expect("fixed fixture rows");
    let mut engine = Engine::new();
    engine
        .register_table(
            orders,
            Arc::new(MemTable::try_new(schema, vec![vec![batch]]).expect("fixed fixture")),
        )
        .expect("fixed fixture registration");
    engine
}

fn table(schema: Arc<Schema>, columns: Vec<ArrayRef>) -> Arc<MemTable> {
    let batch = RecordBatch::try_new(schema.clone(), columns).expect("fixed evaluation columns");
    Arc::new(MemTable::try_new(schema, vec![vec![batch]]).expect("fixed evaluation table"))
}

fn calendar_engine(hidden_duplicate: bool) -> Engine {
    let source_schema = Arc::new(Schema::new(vec![Field::new(
        "occurred_at",
        DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
        false,
    )]));
    let instants = [
        "2026-03-28T22:30:00Z",
        "2026-03-28T23:30:00Z",
        "2026-03-29T22:30:00Z",
    ]
    .map(|value| {
        chrono::DateTime::parse_from_rfc3339(value)
            .unwrap()
            .timestamp_micros()
    });
    let mut source = Relation::base("events", source_schema.clone(), "fixture:events");
    source.semantics = Some(RelationSemantics {
        business_calendars: [(
            "fiscal".into(),
            BusinessCalendarRule {
                version: BUSINESS_CALENDAR_VERSION,
                id: "calendar/events-fiscal".into(),
                source_relation: "events".into(),
                calendar_relation: "fiscal_days".into(),
                source_date_field: "occurred_at".into(),
                calendar_date_field: "date".into(),
                fiscal_year_field: "fiscal_year".into(),
                fiscal_period_field: "fiscal_period".into(),
                business_day_field: "business_day".into(),
                source_basis: CalendarSourceBasis::UtcInstantMicros,
                timezone: "Europe/Zurich".into(),
                mapping_revision: "FY26-held-out-v1".into(),
                source_refs: vec![],
            },
        )]
        .into(),
        ..Default::default()
    });
    // Independent fixture values are generated from calendar dates, not from
    // the compiler's timezone UDF.
    let mut days = [(2026, 3, 28), (2026, 3, 29), (2026, 3, 30)]
        .map(|(y, m, d)| {
            chrono::NaiveDate::from_ymd_opt(y, m, d)
                .unwrap()
                .signed_duration_since(chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
                .num_days() as i32
        })
        .to_vec();
    let mut periods = vec![28, 29, 30];
    let mut visible = vec![true; 3];
    if hidden_duplicate {
        days.push(days[1]);
        periods.push(99);
        visible.push(false);
    }
    let calendar_schema = Arc::new(Schema::new(vec![
        Field::new("date", DataType::Date32, false),
        Field::new("fiscal_year", DataType::Int32, false),
        Field::new("fiscal_period", DataType::Int16, false),
        Field::new("business_day", DataType::Boolean, false),
        Field::new("visible", DataType::Boolean, false),
    ]));
    let mut calendar = Relation::base(
        "fiscal_days",
        calendar_schema.clone(),
        "fixture:fiscal_days",
    );
    calendar.semantics = Some(RelationSemantics {
        row_policies: vec![RowPolicy {
            id: "calendar/visible".into(),
            filters: vec![GovernedFilter {
                field: "visible".into(),
                operator: Comparison::Eq,
                value: Literal::Boolean(true),
            }],
            source_refs: vec![],
        }],
        ..Default::default()
    });
    let mut engine = Engine::new();
    engine
        .register_table(
            calendar,
            table(
                calendar_schema,
                vec![
                    Arc::new(Date32Array::from(days)),
                    Arc::new(Int32Array::from(vec![2026; periods.len()])),
                    Arc::new(Int16Array::from(periods)),
                    Arc::new(BooleanArray::from(vec![true; visible.len()])),
                    Arc::new(BooleanArray::from(visible)),
                ],
            ),
        )
        .unwrap();
    engine
        .register_table(
            source,
            table(
                source_schema,
                vec![Arc::new(
                    TimestampMicrosecondArray::from(instants.to_vec()).with_timezone("UTC"),
                )],
            ),
        )
        .unwrap();
    engine
}

fn conversion_engine() -> Engine {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "amount",
        DataType::Int64,
        false,
    )]));
    let mut relation = Relation::base("amounts", schema.clone(), "fixture:amounts");
    relation.semantics = Some(RelationSemantics {
        conversions: [(
            "thirds".into(),
            UnitConversion {
                version: UNIT_CONVERSION_VERSION,
                id: "conversion/thirds".into(),
                field: "amount".into(),
                from_unit: semantic_catalog::Unit::Named { id: "whole".into() },
                to_unit: semantic_catalog::Unit::Named { id: "third".into() },
                numerator: 1,
                denominator: 3,
                rounding: ConversionRounding::HalfEven,
                source_refs: vec![],
            },
        )]
        .into(),
        ..Default::default()
    });
    let mut engine = Engine::new();
    engine
        .register_table(
            relation,
            table(schema, vec![Arc::new(Int64Array::from(vec![1, 2]))]),
        )
        .unwrap();
    engine
}

fn metric_engine() -> Engine {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("score", DataType::Int64, false),
    ]));
    let mut relation = Relation::base("scores", schema.clone(), "fixture:scores");
    relation.semantics = Some(RelationSemantics {
        metrics: [(
            "total_score".into(),
            MetricDefinition {
                id: "metrics/total-score".into(),
                description: "Sum of scores at score row grain".into(),
                aliases: vec![],
                function: AggregateFunction::Sum,
                field: Some("score".into()),
                distinct: false,
                source_grain: semantic_catalog::SourceGrain {
                    entity: None,
                    keys: vec![semantic_catalog::GrainKey {
                        relation: "scores".into(),
                        field: "id".into(),
                    }],
                },
                compatible_dimensions: Default::default(),
                compatible_lookup_dimensions: vec![],
                sum_rollup_dimensions: None,
                state: None,
                row_filters: vec![],
                result_type: DataType::Int64,
                unit: Presence::Value(semantic_catalog::Unit::Named {
                    id: "points".into(),
                }),
                temporal: Presence::Missing,
                empty_behavior: EmptyBehavior::Null,
                source_refs: vec![],
            },
        )]
        .into(),
        ..Default::default()
    });
    let mut engine = Engine::new();
    engine
        .register_table(
            relation,
            table(
                schema,
                vec![
                    Arc::new(Int64Array::from(vec![1, 2])),
                    Arc::new(Int64Array::from(vec![10, 20])),
                ],
            ),
        )
        .unwrap();
    engine
}

fn relationship(role: &str, right: &str, key: &str) -> RelationshipDefinition {
    RelationshipDefinition {
        ai_context: None,
        id: format!("relationships/{role}"),
        right_relation: right.into(),
        role: role.into(),
        key_pairs: vec![RelationshipKey {
            left_field: key.into(),
            right_field: "id".into(),
        }],
        null_keys_match: false,
        cardinality: FactResolution::Unknown,
        source_refs: vec![],
    }
}

fn temporal_path_engine() -> Engine {
    let order_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("bill", DataType::Int64, false),
    ]));
    let mut orders = Relation::base("orders", order_schema.clone(), "fixture:orders");
    orders.semantics = Some(RelationSemantics {
        relationships: [("bill".into(), relationship("bill", "customers", "bill"))].into(),
        ..Default::default()
    });
    let customer_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("manager", DataType::Int64, false),
    ]));
    let mut customers = Relation::base("customers", customer_schema.clone(), "fixture:customers");
    customers.semantics = Some(RelationSemantics {
        relationships: [(
            "manager".into(),
            relationship("manager", "managers", "manager"),
        )]
        .into(),
        ..Default::default()
    });
    let manager_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("label", DataType::Utf8, false),
        Field::new("valid_from", DataType::Date32, false),
        Field::new("valid_to", DataType::Date32, false),
    ]));
    let mut engine = Engine::new();
    engine
        .register_table(
            orders,
            table(
                order_schema,
                vec![
                    Arc::new(Int64Array::from(vec![1])),
                    Arc::new(Int64Array::from(vec![10])),
                ],
            ),
        )
        .unwrap();
    engine
        .register_table(
            customers,
            table(
                customer_schema,
                vec![
                    Arc::new(Int64Array::from(vec![10])),
                    Arc::new(Int64Array::from(vec![100])),
                ],
            ),
        )
        .unwrap();
    engine
        .register_table(
            Relation::base("managers", manager_schema.clone(), "fixture:managers"),
            table(
                manager_schema,
                vec![
                    Arc::new(Int64Array::from(vec![100])),
                    Arc::new(StringArray::from(vec!["Ada"])),
                    Arc::new(Date32Array::from(vec![0])),
                    Arc::new(Date32Array::from(vec![100])),
                ],
            ),
        )
        .unwrap();
    engine
}

fn engine(case: &Case) -> Engine {
    match case.fixture.as_str() {
        "orders" => orders_engine(case),
        "calendar_dst" => calendar_engine(false),
        "calendar_hidden_duplicate" => calendar_engine(true),
        "conversion" => conversion_engine(),
        "metric_grain" => metric_engine(),
        "temporal_path" => temporal_path_engine(),
        other => panic!("unsupported held-out fixture {other}"),
    }
}

fn provider(live: bool, scripted: &Value) -> Result<Provider, Box<dyn Error>> {
    if !live {
        return Ok(Provider::Scripted(scripted.to_string()));
    }
    let key = std::env::var("OPENAI_API_KEY")?;
    let model = std::env::var("SEMANTIC_EVAL_MODEL")?;
    let mut config = OpenAiConfig::new(key, model);
    config.timeout = Duration::from_secs(30);
    Ok(Provider::Live(OpenAiProvider::new(config)?))
}

async fn evaluate(
    case: &Case,
    mode: SelectionMode,
    live: bool,
) -> Result<CaseReport, Box<dyn Error>> {
    let context = if mode == SelectionMode::Full {
        "full"
    } else {
        "retrieved"
    };
    let expected = if context == "full" {
        &case.expected.full
    } else {
        &case.expected.retrieved
    };
    let engine = engine(case);
    let mut options = CompileOptions::default();
    options.selection_mode = mode;
    options.timeout = Duration::from_secs(30);
    // Retrieved context may need one bounded hydration round before the same
    // scripted proposal can bind. This is still capped across all attempts.
    options.max_model_calls = 2;
    options.max_expansions = 1;
    options.max_total_model_input_bytes = 64 * 1024;
    options.max_total_model_output_bytes = 8 * 1024;
    let compilation = Compiler::new(provider(live, &case.scripted_output)?)
        .with_max_repairs(0)
        .compile_typed(&engine, &case.request, options)
        .await;
    let actual = compilation.record.outcome.to_string();
    let diagnostic = match &compilation.outcome {
        TypedOutcome::Rejected { diagnostic } | TypedOutcome::Unresolved { diagnostic } => {
            Some(diagnostic.code.as_str())
        }
        _ => None,
    };
    let rows_match = match (&compilation.outcome, &expected.rows) {
        (TypedOutcome::Compiled { query }, Some(expected_rows)) => {
            let batches = query.plan_direct(&engine).await?.collect().await?;
            let mut actual_rows = Vec::new();
            for batch in &batches {
                for row in 0..batch.num_rows() {
                    actual_rows.push(
                        (0..batch.num_columns())
                            .map(|column| array_value_to_string(batch.column(column), row))
                            .collect::<Result<Vec<_>, _>>()?,
                    );
                }
            }
            let mut expected_rows = expected_rows.clone();
            actual_rows.sort();
            expected_rows.sort();
            Some(actual_rows == expected_rows)
        }
        (TypedOutcome::Compiled { .. }, None) => Some(false),
        _ => None,
    };
    let recalled_facts = case
        .required_facts
        .iter()
        .filter(|required| {
            compilation.record.contexts.iter().any(|context| {
                context.audit.facts.iter().any(|fact| {
                    fact.relation == required.relation
                        && fact.kind == required.kind
                        && fact.id == required.id
                })
            })
        })
        .count();
    Ok(CaseReport {
        id: case.id.clone(),
        context,
        outcome_match: actual == expected.outcome,
        diagnostic_match: diagnostic == expected.diagnostic.as_deref(),
        rows_match,
        incorrect_accept: expected.outcome != "compiled" && actual == "compiled",
        false_refusal: expected.outcome == "compiled" && actual != "compiled",
        expected: expected.outcome.clone(),
        actual,
        actual_diagnostic: diagnostic.map(str::to_owned),
        required_facts: case.required_facts.len(),
        recalled_facts,
        model_calls: compilation.record.work.model_calls,
        elapsed_micros: compilation.record.elapsed_micros,
        input_tokens: compilation.record.token_accounting.reported_input_tokens,
        output_tokens: compilation.record.token_accounting.reported_output_tokens,
        token_usage_incomplete: compilation
            .record
            .token_accounting
            .calls_missing_input_usage
            > 0
            || compilation
                .record
                .token_accounting
                .calls_missing_output_usage
                > 0,
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let live = match args.as_slice() {
        [] => false,
        [mode] if mode == "--scripted" => false,
        [mode] if mode == "--live" => true,
        _ => return Err("usage: held_out_eval [--scripted|--live]".into()),
    };
    let manifest: Manifest = serde_json::from_str(FIXTURE)?;
    let mut cases = Vec::new();
    for case in &manifest.cases {
        for mode in &case.modes {
            let mode = match mode.as_str() {
                "full" => SelectionMode::Full,
                "retrieved" => SelectionMode::Retrieved,
                other => return Err(format!("unsupported held-out context mode {other}").into()),
            };
            cases.push(evaluate(case, mode, live).await?);
        }
    }
    let report = Report {
        profile: manifest.profile,
        provider: if live { "live_openai" } else { "scripted" },
        claim: manifest.claim,
        outcome_matches: cases
            .iter()
            .filter(|case| {
                case.outcome_match && case.diagnostic_match && case.rows_match != Some(false)
            })
            .count(),
        incorrect_accepts: cases.iter().filter(|case| case.incorrect_accept).count(),
        false_refusals: cases.iter().filter(|case| case.false_refusal).count(),
        required_facts: cases.iter().map(|case| case.required_facts).sum(),
        recalled_facts: cases.iter().map(|case| case.recalled_facts).sum(),
        cases,
    };
    println!("{}", serde_json::to_string_pretty(&report)?);
    if !live
        && (report.outcome_matches != report.cases.len()
            || report.incorrect_accepts != 0
            || report.false_refusals != 0
            || report.recalled_facts != report.required_facts)
    {
        return Err("scripted held-out quality gate failed".into());
    }
    Ok(())
}
