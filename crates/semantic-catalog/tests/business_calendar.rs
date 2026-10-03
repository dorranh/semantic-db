use std::{collections::BTreeSet, sync::Arc};

use semantic_catalog::{
    BUSINESS_CALENDAR_VERSION, BusinessCalendarError, BusinessCalendarRule, CalendarSourceBasis,
    Catalog, CatalogMutation, DataType, Field, PublicationLimits, Relation, RelationSemantics,
    Schema, SearchOptions,
};

fn rule() -> BusinessCalendarRule {
    BusinessCalendarRule {
        version: BUSINESS_CALENDAR_VERSION,
        id: "calendar/orders-fiscal".into(),
        source_relation: "orders".into(),
        calendar_relation: "fiscal_days".into(),
        source_date_field: "ordered_at".into(),
        calendar_date_field: "date".into(),
        fiscal_year_field: "fiscal_year".into(),
        fiscal_period_field: "fiscal_period".into(),
        business_day_field: "business_day".into(),
        source_basis: CalendarSourceBasis::UtcInstantMicros,
        timezone: "Europe/Zurich".into(),
        mapping_revision: "FY26-v1".into(),
        source_refs: vec![],
    }
}

fn relations() -> [Relation; 2] {
    let mut orders = Relation::base(
        "orders",
        Arc::new(Schema::new(vec![Field::new(
            "ordered_at",
            DataType::Timestamp(arrow_schema::TimeUnit::Microsecond, Some("UTC".into())),
            false,
        )])),
        "memory",
    );
    orders.semantics = Some(RelationSemantics {
        business_calendars: [("fiscal".into(), rule())].into(),
        ..Default::default()
    });
    let calendar = Relation::base(
        "fiscal_days",
        Arc::new(Schema::new(vec![
            Field::new("date", DataType::Date32, false),
            Field::new("fiscal_year", DataType::Int32, false),
            Field::new("fiscal_period", DataType::Int16, false),
            Field::new("business_day", DataType::Boolean, false),
        ])),
        "memory",
    );
    [orders, calendar]
}

#[test]
fn authored_calendar_is_pinned_discoverable_and_invalidates_dependents() {
    let [orders, calendar] = relations();
    let mut catalog = Catalog::from_relations([orders, calendar.clone()]).unwrap();
    catalog.validate(&PublicationLimits::default()).unwrap();
    let snapshot = catalog.snapshot();
    assert_eq!(
        snapshot
            .relation("orders")
            .unwrap()
            .definition_reference("business_calendar", "fiscal")
            .unwrap()
            .id,
        "calendar/orders-fiscal"
    );
    let index = snapshot.search_index(|| Ok::<_, ()>(())).unwrap();
    let report = index
        .search("fiscal", &SearchOptions::default(), || Ok::<_, ()>(()))
        .unwrap();
    assert!(
        report
            .hits
            .iter()
            .any(|hit| hit.object.relation.as_ref() == "orders")
    );
    let mut changed = calendar;
    changed.description = Some("FY26 mapping update".into());
    let publication = catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(changed))],
            &PublicationLimits::default(),
        )
        .unwrap();
    assert!(publication.affected.contains("orders"));
    let mut changed_orders = catalog.relation("orders").unwrap().clone();
    changed_orders
        .semantics
        .as_mut()
        .unwrap()
        .business_calendars
        .get_mut("fiscal")
        .unwrap()
        .mapping_revision = "FY26-v2".into();
    let next = catalog
        .apply_changes(
            [CatalogMutation::Put(Box::new(changed_orders))],
            &PublicationLimits::default(),
        )
        .unwrap();
    assert!(next.changed.contains("orders"));
}

#[test]
fn calendar_requires_exact_types_and_authorized_relation() {
    let [orders, mut calendar] = relations();
    let snapshot = Catalog::from_relations([orders.clone(), calendar.clone()])
        .unwrap()
        .snapshot();
    assert!(rule().validate(&snapshot, None).is_ok());
    assert_eq!(
        rule()
            .validate(&snapshot, Some(&BTreeSet::from(["orders".into()])))
            .unwrap_err(),
        BusinessCalendarError::Scope
    );
    let mut bad_zone = rule();
    bad_zone.timezone = "Mars/Base".into();
    assert_eq!(
        bad_zone.validate(&snapshot, None).unwrap_err(),
        BusinessCalendarError::Timezone
    );
    calendar.schema = Arc::new(Schema::new(vec![
        Field::new("date", DataType::Date32, false),
        Field::new("fiscal_year", DataType::Int32, false),
        Field::new("fiscal_period", DataType::Int64, false),
        Field::new("business_day", DataType::Boolean, false),
    ]));
    assert!(
        Catalog::from_relations([orders, calendar])
            .unwrap()
            .validate(&PublicationLimits::default())
            .is_err()
    );
}
