use std::sync::Arc;

use semantic_catalog::{
    ApplicabilityError, Catalog, DataType, Field, PublicationLimits, Relation, RelationSemantics,
    Schema, ViewTemporalCoverage,
};
use semantic_plan::typed::{CalendarUnit, Literal};

fn coverage(grain: CalendarUnit) -> ViewTemporalCoverage {
    ViewTemporalCoverage {
        field: "day".into(),
        grain,
        start: Literal::Date32(100),
        end: Literal::Date32(200),
        source_refs: vec![],
    }
}

#[test]
fn half_open_coverage_and_calendar_refinement_are_exact() {
    let day = coverage(CalendarUnit::Day);
    assert_eq!(
        day.contains(
            "day",
            CalendarUnit::Month,
            &Literal::Date32(100),
            &Literal::Date32(200)
        ),
        Ok(())
    );
    assert_eq!(
        day.contains(
            "day",
            CalendarUnit::Day,
            &Literal::Date32(99),
            &Literal::Date32(101)
        ),
        Err(ApplicabilityError::Coverage)
    );
    assert_eq!(
        day.contains(
            "other",
            CalendarUnit::Day,
            &Literal::Date32(100),
            &Literal::Date32(101)
        ),
        Err(ApplicabilityError::Field)
    );
    let month = coverage(CalendarUnit::Month);
    assert_eq!(
        month.contains(
            "day",
            CalendarUnit::Day,
            &Literal::Date32(100),
            &Literal::Date32(101)
        ),
        Err(ApplicabilityError::Grain)
    );
    let week = coverage(CalendarUnit::IsoWeek);
    assert_eq!(
        week.contains(
            "day",
            CalendarUnit::Year,
            &Literal::Date32(100),
            &Literal::Date32(101)
        ),
        Err(ApplicabilityError::Grain)
    );
}

fn view(value: ViewTemporalCoverage) -> Relation {
    let schema = Arc::new(Schema::new(vec![Field::new(
        "day",
        DataType::Date32,
        false,
    )]));
    let mut relation = Relation::view("daily", schema, "SELECT day FROM events");
    relation.semantics = Some(RelationSemantics {
        view_coverage: Some(value),
        ..Default::default()
    });
    relation
}

#[test]
fn publication_checks_view_contract_and_revisions_it() {
    let mut first = view(coverage(CalendarUnit::Day));
    let catalog = Catalog::from_relations([first.clone()]).unwrap();
    catalog.validate(&PublicationLimits::default()).unwrap();
    let reference = catalog
        .snapshot()
        .relation("daily")
        .unwrap()
        .definition_reference("view_coverage", "daily")
        .unwrap()
        .clone();
    first
        .semantics
        .as_mut()
        .unwrap()
        .view_coverage
        .as_mut()
        .unwrap()
        .end = Literal::Date32(201);
    let changed = Catalog::from_relations([first]).unwrap();
    changed.validate(&PublicationLimits::default()).unwrap();
    assert_ne!(
        reference,
        *changed
            .snapshot()
            .relation("daily")
            .unwrap()
            .definition_reference("view_coverage", "daily")
            .unwrap()
    );

    let invalid = Catalog::from_relations([view(ViewTemporalCoverage {
        end: Literal::Date32(100),
        ..coverage(CalendarUnit::Day)
    })])
    .unwrap();
    assert!(invalid.validate(&PublicationLimits::default()).is_err());
    let mut base = Relation::base(
        "events",
        Arc::new(Schema::new(vec![Field::new(
            "day",
            DataType::Date32,
            false,
        )])),
        "src",
    );
    base.semantics = Some(RelationSemantics {
        view_coverage: Some(coverage(CalendarUnit::Day)),
        ..Default::default()
    });
    let invalid = Catalog::from_relations([base]).unwrap();
    assert!(invalid.validate(&PublicationLimits::default()).is_err());
}
