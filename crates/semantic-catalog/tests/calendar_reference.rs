use arrow_schema::TimeUnit;
use std::sync::Arc;

use semantic_catalog::{
    CalendarReference, CalendarSystem, Catalog, DataType, Field, FieldSemantics, PublicationError,
    PublicationLimits, Relation, RelationSemantics, Schema,
};

fn relation(fiscal_id: &str, fiscal_zone: &str) -> Relation {
    let timestamp = DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into()));
    let mut relation = Relation::base(
        "events",
        Arc::new(Schema::new(vec![
            Field::new("gregorian_at", timestamp.clone(), false),
            Field::new("fiscal_at", timestamp, false),
        ])),
        "memory",
    );
    relation.semantics = Some(RelationSemantics {
        fields: [
            (
                "gregorian_at".into(),
                FieldSemantics {
                    calendar_reference: Some(CalendarReference {
                        system: CalendarSystem::Gregorian,
                        timezone: "UTC".into(),
                    }),
                    ..Default::default()
                },
            ),
            (
                "fiscal_at".into(),
                FieldSemantics {
                    calendar_reference: Some(CalendarReference {
                        system: CalendarSystem::Fiscal {
                            id: fiscal_id.into(),
                        },
                        timezone: fiscal_zone.into(),
                    }),
                    ..Default::default()
                },
            ),
        ]
        .into(),
        ..Default::default()
    });
    relation
}

#[test]
fn authored_calendar_reference_is_validated_and_revisioned() {
    let valid = Catalog::from_relations([relation("company-445", "Europe/Zurich")]).unwrap();
    valid.validate(&PublicationLimits::default()).unwrap();
    let changed = Catalog::from_relations([relation("company-454", "Europe/Zurich")]).unwrap();
    changed.validate(&PublicationLimits::default()).unwrap();
    assert_ne!(
        valid.snapshot().relation("events").unwrap().reference(),
        changed.snapshot().relation("events").unwrap().reference()
    );

    for (fiscal_id, zone) in [("", "Europe/Zurich"), ("company-445", "Mars/Olympus")] {
        let invalid = Catalog::from_relations([relation(fiscal_id, zone)]).unwrap();
        assert!(matches!(
            invalid.validate(&PublicationLimits::default()),
            Err(PublicationError::InvalidDefinition {
                code: "invalid_calendar_reference",
                ..
            })
        ));
    }
}
