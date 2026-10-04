use semantic_catalog::{
    Authority, Catalog, CatalogSnapshot, DataType, EntityId, EntityIdentity, Fact, FactResolution,
    Field, GrainKey, KeyEvidence, Relation, RelationKind, RelationSemantics, Schema, SearchReport,
    SourceGrain, ViewOutputLineage,
};
use semantic_interpreter::typed::{
    ContextAudit, ContextDependency, ContextGapKind, ContextManifest, ContextObject, SelectionMode,
    audit_context_manifest,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

fn schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![Field::new("id", DataType::Int64, false)]))
}

fn fixture() -> Arc<CatalogSnapshot> {
    let mut items = Relation::base("items", schema(), "fixture:items");
    let mut semantics = RelationSemantics::default();
    semantics
        .facts
        .insert("status_scope".into(), FactResolution::Unknown);
    semantics.declared_primary_key = vec!["id".into()];
    let identity = EntityId("entities/item".into());
    semantics.entity_identity = Some(EntityIdentity {
        id: identity.clone(),
        relation: "items".into(),
        source_grain: SourceGrain {
            entity: Some(identity),
            keys: vec![GrainKey {
                relation: "items".into(),
                field: "id".into(),
            }],
        },
        key_evidence: FactResolution::Known {
            value: KeyEvidence::AuthoredDeclaration,
            contributors: vec![Fact {
                id: "item-key".into(),
                scope: "items".into(),
                value: KeyEvidence::AuthoredDeclaration,
                authority: Authority::Authored,
                origins: vec![],
                evidence: vec![],
            }],
        },
        source_refs: vec![],
    });
    items.semantics = Some(semantics);
    let source = Catalog::from_relations([items.clone()])
        .unwrap()
        .snapshot()
        .relation("items")
        .unwrap()
        .reference()
        .clone();
    let mut view = Relation::view("item_view", schema(), "SELECT id AS id FROM items");
    view.kind = RelationKind::View {
        sql: "SELECT id AS id FROM items".into(),
        dependencies: vec!["items".into()],
    };
    view.semantics = Some(RelationSemantics {
        view_lineage: Some(ViewOutputLineage {
            source,
            columns: BTreeMap::from([("id".into(), "id".into())]),
            source_refs: vec![],
        }),
        ..Default::default()
    });
    Catalog::from_relations([items, view]).unwrap().snapshot()
}

fn rendered(snapshot: &CatalogSnapshot, names: &[&str], omit_facts: bool) -> String {
    let relations: Vec<_> = names
        .iter()
        .map(|name| {
            let entry = snapshot.relation(name).unwrap();
            let semantics = entry.definition().semantics.as_ref().map(|semantics| {
                let mut value = json!({
                    "facts":semantics.facts,
                    "entity_identity":semantics.entity_identity,
                    "declared_primary_key":semantics.declared_primary_key,
                    "declared_unique_keys":semantics.declared_unique_keys,
                    "origin":semantics.origin,
                    "capability":semantics.capability,
                    "model_description":semantics.model_description,
                    "model_ai_context":semantics.model_ai_context,
                    "ai_context":semantics.ai_context,
                    "view_coverage":semantics.view_coverage,
                    "view_lineage":semantics.view_lineage,
                    "metrics":semantics.metrics,
                    "ratio_metrics":semantics.ratio_metrics,
                    "value_mappings":semantics.value_mappings,
                    "relationships":semantics.relationships,
                    "concepts":semantics.concepts,
                    "conversions":semantics.conversions,
                    "business_calendars":semantics.business_calendars,
                    "allocations":semantics.allocations,
                    "currency_rates":semantics.currency_rates,
                    "exact_decimal_rates":semantics.exact_decimal_rates,
                    "row_policies":semantics.row_policies
                });
                if omit_facts {
                    value.as_object_mut().unwrap().remove("facts");
                }
                value
            });
            json!({
                "name":name,
                "revision":entry.reference().revision,
                "description":entry.definition().description,
                "grain":entry.definition().grain,
                "columns":entry.definition().schema.fields().iter().map(|field| json!({"name":field.name(),"data_type":field.data_type().to_string(),"nullable":field.is_nullable(),"semantics":entry.definition().semantics.as_ref().and_then(|s|s.fields.get(field.name()))})).collect::<Vec<_>>(),
                "fields_total":entry.definition().schema.fields().len(),
                "field_inventory_complete":true,
                "view_sql":match &entry.definition().kind {
                    RelationKind::View { sql, .. } => Some(sql.as_str()),
                    RelationKind::Base { .. } => None,
                },
                "semantics":semantics
            })
        })
        .collect();
    json!({
        "version":1,
        "snapshot_id":snapshot.id(),
        "coverage":"full",
        "semantic_sufficiency_proven":false,
        "relations":relations
    })
    .to_string()
}

fn manifest(snapshot: &CatalogSnapshot, payload: &str, names: &[&str]) -> ContextManifest {
    ContextManifest {
        version: 1,
        snapshot_id: snapshot.id().into(),
        selection_mode: SelectionMode::Full,
        index_revision: None,
        access_scope_revision: "scope".into(),
        included: names
            .iter()
            .map(|name| ContextObject {
                reference: snapshot.relation(name).unwrap().reference().clone(),
                fields: snapshot
                    .relation(name)
                    .unwrap()
                    .definition()
                    .schema
                    .fields()
                    .iter()
                    .map(|field| field.name().clone())
                    .collect(),
                field_inventory_complete: true,
                reasons: BTreeSet::from(["full_catalog".into()]),
            })
            .collect(),
        dependencies: vec![ContextDependency {
            from: "item_view".into(),
            to: "items".into(),
            satisfied: true,
        }],
        searches: vec![],
        declared_dependencies_complete: true,
        semantic_sufficiency_proven: false,
        payload_digest: format!("{:x}", Sha256::digest(payload.as_bytes())),
        payload_bytes: payload.len(),
        audit: ContextAudit {
            complete: false,
            facts: vec![],
            gaps: vec![],
            semantic_sufficiency_proven: false,
        },
    }
}

#[test]
fn complete_payload_records_pinned_facts_without_claiming_semantic_sufficiency() {
    let snapshot = fixture();
    let payload = rendered(&snapshot, &["items", "item_view"], false);
    let manifest = manifest(&snapshot, &payload, &["items", "item_view"]);
    let audit = audit_context_manifest(&snapshot, &manifest, &payload);
    assert!(audit.complete, "{:?}", audit.gaps);
    assert!(audit.facts.iter().any(|fact| {
        fact.relation == "items" && fact.kind == "facts" && fact.id == "status_scope"
    }));
    assert!(audit.facts.iter().any(|fact| {
        fact.relation == "items" && fact.kind == "entity_identity" && fact.id == "entities/item"
    }));
    assert!(
        audit
            .facts
            .iter()
            .any(|fact| { fact.relation == "items" && fact.kind == "declared_primary_key" })
    );
    assert!(audit.facts.iter().any(|fact| {
        fact.relation == "item_view" && fact.kind == "view_lineage" && fact.revision.is_some()
    }));
    assert!(!audit.semantic_sufficiency_proven);
}

#[test]
fn omitted_governing_facts_and_view_inputs_are_incomplete() {
    let snapshot = fixture();
    let payload = rendered(&snapshot, &["item_view"], false);
    let view_manifest = manifest(&snapshot, &payload, &["item_view"]);
    let audit = audit_context_manifest(&snapshot, &view_manifest, &payload);
    assert!(!audit.complete);
    assert!(
        audit
            .gaps
            .iter()
            .any(|gap| gap.kind == ContextGapKind::MissingDependency)
    );

    let payload = rendered(&snapshot, &["items", "item_view"], true);
    let fact_manifest = manifest(&snapshot, &payload, &["items", "item_view"]);
    let audit = audit_context_manifest(&snapshot, &fact_manifest, &payload);
    assert!(!audit.complete);
    assert!(
        audit
            .gaps
            .iter()
            .any(|gap| { gap.kind == ContextGapKind::MissingFactGroup && gap.detail == "facts" })
    );
}

#[test]
fn omitted_entity_identity_is_an_incomplete_context() {
    let snapshot = fixture();
    let mut payload: Value =
        serde_json::from_str(&rendered(&snapshot, &["items", "item_view"], false)).unwrap();
    payload["relations"][0]["semantics"]
        .as_object_mut()
        .unwrap()
        .remove("entity_identity");
    let payload = payload.to_string();
    let manifest = manifest(&snapshot, &payload, &["items", "item_view"]);
    let audit = audit_context_manifest(&snapshot, &manifest, &payload);
    assert!(!audit.complete);
    assert!(audit.gaps.iter().any(|gap| {
        gap.kind == ContextGapKind::MissingFactGroup && gap.detail == "entity_identity"
    }));
}

#[test]
fn omitting_declared_key_metadata_is_not_a_complete_context() {
    let snapshot = fixture();
    let mut payload: Value =
        serde_json::from_str(&rendered(&snapshot, &["items", "item_view"], false)).unwrap();
    payload["relations"][0]["semantics"]
        .as_object_mut()
        .unwrap()
        .remove("declared_primary_key");
    let payload = payload.to_string();
    let manifest = manifest(&snapshot, &payload, &["items", "item_view"]);
    let audit = audit_context_manifest(&snapshot, &manifest, &payload);
    assert!(!audit.complete);
    assert!(audit.gaps.iter().any(|gap| {
        gap.kind == ContextGapKind::MissingFactGroup && gap.detail == "declared_primary_key"
    }));
}

#[test]
fn omitted_pinned_view_lineage_is_incomplete() {
    let snapshot = fixture();
    let mut payload: Value =
        serde_json::from_str(&rendered(&snapshot, &["items", "item_view"], false)).unwrap();
    payload["relations"][1]["semantics"]
        .as_object_mut()
        .unwrap()
        .remove("view_lineage");
    let payload = payload.to_string();
    let manifest = manifest(&snapshot, &payload, &["items", "item_view"]);
    let audit = audit_context_manifest(&snapshot, &manifest, &payload);
    assert!(!audit.complete);
    assert!(audit.gaps.iter().any(|gap| {
        gap.kind == ContextGapKind::MissingFactGroup && gap.detail == "view_lineage"
    }));
}

#[test]
fn duplicated_or_unmanifested_relations_cannot_claim_complete_context() {
    let snapshot = fixture();
    let mut payload: Value =
        serde_json::from_str(&rendered(&snapshot, &["items", "item_view"], false)).unwrap();
    let duplicate = payload["relations"][0].clone();
    payload["relations"].as_array_mut().unwrap().push(duplicate);
    let payload = payload.to_string();
    let duplicate_manifest = manifest(&snapshot, &payload, &["items", "item_view"]);
    let audit = audit_context_manifest(&snapshot, &duplicate_manifest, &payload);
    assert!(!audit.complete);
    assert!(audit.gaps.iter().any(|gap| {
        gap.kind == ContextGapKind::PayloadMismatch && gap.detail == "duplicate_or_unnamed_relation"
    }));

    let payload = rendered(&snapshot, &["items", "item_view"], false);
    let manifest = manifest(&snapshot, &payload, &["items"]);
    let audit = audit_context_manifest(&snapshot, &manifest, &payload);
    assert!(!audit.complete);
    assert!(audit.gaps.iter().any(|gap| {
        gap.kind == ContextGapKind::PayloadMismatch && gap.detail == "unmanifested_relation"
    }));
}

#[test]
fn changed_field_contract_or_inventory_cannot_claim_complete_context() {
    let snapshot = fixture();
    let mut payload: Value =
        serde_json::from_str(&rendered(&snapshot, &["items", "item_view"], false)).unwrap();
    payload["relations"][0]["columns"][0]["nullable"] = json!(true);
    payload["relations"][0]["field_inventory_complete"] = json!(false);
    let payload = payload.to_string();
    let manifest = manifest(&snapshot, &payload, &["items", "item_view"]);
    let audit = audit_context_manifest(&snapshot, &manifest, &payload);
    assert!(!audit.complete);
    assert!(audit.gaps.iter().any(|gap| {
        gap.kind == ContextGapKind::MissingField && gap.detail == "field_contract/id"
    }));
    assert!(audit.gaps.iter().any(|gap| {
        gap.kind == ContextGapKind::IncompleteInventory && gap.detail == "payload_field_inventory"
    }));
}

#[test]
fn mismatched_payload_header_or_relation_grain_cannot_claim_complete_context() {
    let snapshot = fixture();
    let mut payload: Value =
        serde_json::from_str(&rendered(&snapshot, &["items", "item_view"], false)).unwrap();
    payload["coverage"] = json!("retrieved");
    payload["relations"][0]["grain"] = json!("one row per invented entity");
    let payload = payload.to_string();
    let manifest = manifest(&snapshot, &payload, &["items", "item_view"]);
    let audit = audit_context_manifest(&snapshot, &manifest, &payload);
    assert!(!audit.complete);
    assert!(audit.gaps.iter().any(|gap| {
        gap.kind == ContextGapKind::PayloadMismatch && gap.detail == "payload_contract"
    }));
    assert!(audit.gaps.iter().any(|gap| {
        gap.kind == ContextGapKind::MissingFactGroup
            && gap.detail == "relation_description_or_grain"
    }));
}

#[test]
fn fabricated_view_sql_or_missing_view_lineage_cannot_claim_complete_context() {
    let snapshot = fixture();
    let mut payload: Value =
        serde_json::from_str(&rendered(&snapshot, &["items", "item_view"], false)).unwrap();
    payload["relations"][0]["view_sql"] = json!("SELECT id FROM invented");
    payload["relations"][1]["semantics"] = json!({"facts":{"invented":true}});
    let payload = payload.to_string();
    let manifest = manifest(&snapshot, &payload, &["items", "item_view"]);
    let audit = audit_context_manifest(&snapshot, &manifest, &payload);
    assert!(!audit.complete);
    assert!(audit.gaps.iter().any(|gap| {
        gap.kind == ContextGapKind::MissingFactGroup && gap.detail == "unexpected_view_sql"
    }));
    assert!(audit.gaps.iter().any(|gap| {
        gap.kind == ContextGapKind::MissingFactGroup && gap.detail == "view_lineage"
    }));
}

#[test]
fn partial_search_is_unknown_even_when_it_has_no_hits() {
    let snapshot = fixture();
    let payload = rendered(&snapshot, &["items", "item_view"], false);
    let mut manifest = manifest(&snapshot, &payload, &["items", "item_view"]);
    manifest.searches.push(SearchReport {
        hits: vec![],
        postings_visited: 1,
        exhausted: false,
        truncated_candidates: false,
        lookup_fingerprints: vec![],
    });
    let audit = audit_context_manifest(&snapshot, &manifest, &payload);
    assert!(!audit.complete);
    assert!(
        audit
            .gaps
            .iter()
            .any(|gap| gap.kind == ContextGapKind::IncompleteSearch)
    );
    let serialized: Value = serde_json::to_value(audit).unwrap();
    assert_eq!(serialized["semantic_sufficiency_proven"], false);
}

#[test]
fn retrieved_context_requires_a_pinned_completed_search() {
    let snapshot = fixture();
    let mut payload: Value =
        serde_json::from_str(&rendered(&snapshot, &["items", "item_view"], false)).unwrap();
    payload["coverage"] = json!("retrieved");
    let payload = payload.to_string();
    let mut manifest = manifest(&snapshot, &payload, &["items", "item_view"]);
    manifest.selection_mode = SelectionMode::Retrieved;
    let audit = audit_context_manifest(&snapshot, &manifest, &payload);
    assert!(!audit.complete);
    assert!(audit.gaps.iter().any(|gap| {
        gap.kind == ContextGapKind::IncompleteSearch && gap.detail == "retrieval_evidence"
    }));

    manifest.index_revision = Some(snapshot.id().into());
    manifest.searches.push(SearchReport {
        hits: vec![],
        postings_visited: 0,
        exhausted: true,
        truncated_candidates: false,
        lookup_fingerprints: vec![],
    });
    let audit = audit_context_manifest(&snapshot, &manifest, &payload);
    assert!(audit.complete, "{:?}", audit.gaps);
}

#[test]
fn exact_rate_fact_group_omission_is_incomplete_even_when_empty() {
    let snapshot = fixture();
    let mut value: Value =
        serde_json::from_str(&rendered(&snapshot, &["items", "item_view"], false)).unwrap();
    value["relations"][0]["semantics"]
        .as_object_mut()
        .unwrap()
        .remove("exact_decimal_rates");
    let payload = value.to_string();
    let manifest = manifest(&snapshot, &payload, &["items", "item_view"]);
    let audit = audit_context_manifest(&snapshot, &manifest, &payload);
    assert!(!audit.complete);
    assert!(
        audit
            .gaps
            .iter()
            .any(|gap| gap.kind == ContextGapKind::MissingFactGroup
                && gap.detail == "exact_decimal_rates")
    );
}

#[test]
fn published_exact_rate_context_pins_profile_and_rejects_omission_or_tampering() {
    let mut relation = Relation::base(
        "rates",
        Arc::new(Schema::new(vec![
            Field::new("currency", DataType::Utf8, false),
            Field::new("date", DataType::Date32, false),
            Field::new("rate", DataType::Decimal128(18, 6), true),
        ])),
        "fixture:rates",
    );
    let rule = semantic_catalog::ExactDecimalRateRule {
        version: 1,
        id: "rates/chf".into(),
        rate_relation: "rates".into(),
        source_currency_field: "currency".into(),
        date_field: "date".into(),
        rate_field: "rate".into(),
        target_currency: "CHF".into(),
        positive_only: true,
        null_rate: semantic_catalog::NullRatePolicy::Unavailable,
        source_refs: vec![],
    };
    relation.semantics = Some(RelationSemantics {
        exact_decimal_rates: [("chf".into(), rule.clone())].into(),
        ..Default::default()
    });
    let catalog = Catalog::from_relations([relation]).unwrap();
    catalog
        .validate(&semantic_catalog::PublicationLimits::default())
        .unwrap();
    let snapshot = catalog.snapshot();
    let payload = rendered(&snapshot, &["rates"], false);
    let mut receipt = manifest(&snapshot, &payload, &["rates"]);
    receipt.dependencies.clear();
    let audit = audit_context_manifest(&snapshot, &receipt, &payload);
    assert!(audit.complete, "{:?}", audit.gaps);
    assert!(audit.facts.iter().any(|fact| fact.relation == "rates"
        && fact.kind == "exact_decimal_rates"
        && fact.id == "chf"
        && fact.revision.as_deref() == Some(rule.reference().revision.as_str())));
    for remove in [true, false] {
        let mut value: Value = serde_json::from_str(&payload).unwrap();
        if remove {
            value["relations"][0]["semantics"]
                .as_object_mut()
                .unwrap()
                .remove("exact_decimal_rates");
        } else {
            value["relations"][0]["semantics"]["exact_decimal_rates"]["chf"]["target_currency"] =
                json!("EUR");
        }
        let changed = value.to_string();
        let mut receipt = manifest(&snapshot, &changed, &["rates"]);
        receipt.dependencies.clear();
        let audit = audit_context_manifest(&snapshot, &receipt, &changed);
        assert!(!audit.complete);
        assert!(
            audit
                .gaps
                .iter()
                .any(|gap| gap.kind == ContextGapKind::MissingFactGroup
                    && gap.detail == "exact_decimal_rates")
        );
    }
}
