use std::sync::Arc;

use datafusion::{
    arrow::{
        array::Date32Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datasource::MemTable,
};
use semantic_catalog::{Relation, RelationSemantics, ViewTemporalCoverage};
use semantic_compiler::typed::{
    Calendar, CompileOptions, ContextOrigin, RequestContext, TypedOutcome, compile_rows,
};
use semantic_engine::{Engine, RelationBackend, TableProvider};
use semantic_plan::typed::*;

struct Backend(Arc<MemTable>);
impl RelationBackend for Backend {
    async fn resolve(
        &self,
        _relation: &Relation,
    ) -> datafusion::error::Result<Arc<dyn TableProvider>> {
        Ok(self.0.clone())
    }
}

async fn engine(grain: Option<CalendarUnit>) -> Engine {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "day",
        DataType::Date32,
        false,
    )]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(Date32Array::from(vec![19723, 19724]))],
    )
    .unwrap();
    let backend = Backend(Arc::new(
        MemTable::try_new(schema.clone(), vec![vec![batch]]).unwrap(),
    ));
    let base = Relation::base("events", schema.clone(), "memory");
    let mut view = Relation::view("daily", schema, "SELECT day FROM events");
    if let Some(grain) = grain {
        view.semantics = Some(RelationSemantics {
            view_coverage: Some(ViewTemporalCoverage {
                field: "day".into(),
                grain,
                start: Literal::Date32(19723),
                end: Literal::Date32(19754),
                source_refs: vec![],
            }),
            ..Default::default()
        });
    }
    Engine::from_catalog([base, view], &backend).await.unwrap()
}

fn query() -> RowQuery {
    RowQuery {
        version: 1,
        input: RelationInput {
            relation: "daily".into(),
            instance: "d".into(),
        },
        requirements: vec![
            Requirement {
                id: "day".into(),
                source_text: "on this day".into(),
                operation: RowOperation::CalendarFilter {
                    field: FieldRef {
                        instance: "d".into(),
                        field: "day".into(),
                    },
                    period: CalendarPeriod {
                        unit: CalendarUnit::Day,
                        offset: 0,
                        count: 1,
                    },
                },
            },
            Requirement {
                id: "output".into(),
                source_text: "day".into(),
                operation: RowOperation::Project {
                    field: FieldRef {
                        instance: "d".into(),
                        field: "day".into(),
                    },
                    alias: "day".into(),
                },
            },
        ],
        unresolved: vec![],
    }
}

fn options(reference_unix_millis: i64) -> CompileOptions {
    let mut options = CompileOptions::default();
    options.request_context = Some(RequestContext {
        reference_unix_millis,
        timezone: "UTC".into(),
        calendar: Calendar::Gregorian,
        origin: ContextOrigin::Caller,
    });
    options
}

#[tokio::test]
async fn view_calendar_grain_and_coverage_are_checked_before_lowering() {
    let january = 1_704_067_200_000;
    let daily = engine(Some(CalendarUnit::Day)).await;
    assert!(matches!(
        compile_rows(&daily, query(), options(january))
            .await
            .outcome,
        TypedOutcome::Compiled { .. }
    ));
    let monthly = engine(Some(CalendarUnit::Month)).await;
    assert!(matches!(
        compile_rows(&monthly, query(), options(january)).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "view_applicability"
    ));
    assert!(matches!(
        compile_rows(&daily, query(), options(1_706_745_600_000)).await.outcome,
        TypedOutcome::Rejected { diagnostic } if diagnostic.code == "view_applicability"
    ));
    let unknown = engine(None).await;
    assert!(matches!(
        compile_rows(&unknown, query(), options(january)).await.outcome,
        TypedOutcome::Unresolved { diagnostic } if diagnostic.code == "view_coverage_unproven"
    ));
}
