//! Mechanical audit of a rendered context and its pinned manifest.
//! Completeness here never proves that natural-language interpretation is enough.

use super::{ContextManifest, SelectionMode};
use semantic_catalog::{CatalogSnapshot, RelationKind};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextGapKind {
    SnapshotMismatch,
    PayloadMismatch,
    MissingObject,
    StaleObject,
    MissingField,
    IncompleteInventory,
    MissingFactGroup,
    MissingDependency,
    IncompleteSearch,
    FalseCompletenessClaim,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextGap {
    pub kind: ContextGapKind,
    pub relation: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ContextFact {
    pub relation: String,
    pub kind: String,
    pub id: String,
    pub revision: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContextAudit {
    /// Mechanical closure of the checked payload, selected objects and searches.
    pub complete: bool,
    pub facts: Vec<ContextFact>,
    pub gaps: Vec<ContextGap>,
    /// Search and dependency closure cannot prove intent sufficiency.
    pub semantic_sufficiency_proven: bool,
}
impl ContextAudit {
    pub(super) fn pending() -> Self {
        Self {
            complete: false,
            facts: Vec::new(),
            gaps: Vec::new(),
            semantic_sufficiency_proven: false,
        }
    }
}

fn gap(kind: ContextGapKind, relation: Option<&str>, detail: impl Into<String>) -> ContextGap {
    ContextGap {
        kind,
        relation: relation.map(str::to_owned),
        detail: detail.into(),
    }
}

/// Check the exact context text sent to interpretation against the snapshot and
/// manifest. Missing or partial evidence is incomplete, never unsupported.
pub fn audit_context_manifest(
    snapshot: &CatalogSnapshot,
    manifest: &ContextManifest,
    payload: &str,
) -> ContextAudit {
    let mut gaps = Vec::new();
    let mut facts = Vec::new();
    if manifest.snapshot_id != snapshot.id() {
        gaps.push(gap(ContextGapKind::SnapshotMismatch, None, "snapshot_id"));
    }
    if format!("{:x}", Sha256::digest(payload.as_bytes())) != manifest.payload_digest
        || payload.len() != manifest.payload_bytes
    {
        gaps.push(gap(
            ContextGapKind::PayloadMismatch,
            None,
            "payload_digest_or_bytes",
        ));
    }
    let parsed = serde_json::from_str::<Value>(payload).ok();
    let rendered_relations = parsed
        .as_ref()
        .and_then(|value| value.get("relations"))
        .and_then(Value::as_array);
    let payload_relations: BTreeMap<&str, &Value> = rendered_relations
        .into_iter()
        .flatten()
        .filter_map(|relation| Some((relation.get("name")?.as_str()?, relation)))
        .collect();
    if rendered_relations.is_none() {
        gaps.push(gap(ContextGapKind::PayloadMismatch, None, "relations"));
    } else if rendered_relations.is_some_and(|relations| relations.len() != payload_relations.len())
    {
        gaps.push(gap(
            ContextGapKind::PayloadMismatch,
            None,
            "duplicate_or_unnamed_relation",
        ));
    }
    if parsed
        .as_ref()
        .and_then(|value| value.get("snapshot_id"))
        .and_then(Value::as_str)
        != Some(snapshot.id())
    {
        gaps.push(gap(
            ContextGapKind::PayloadMismatch,
            None,
            "payload_snapshot_id",
        ));
    }
    if parsed
        .as_ref()
        .and_then(|value| value.get("version"))
        .and_then(Value::as_u64)
        != Some(u64::from(manifest.version))
        || parsed.as_ref().and_then(|value| value.get("coverage"))
            != Some(&serde_json::to_value(manifest.selection_mode).expect("mode serializes"))
        || parsed
            .as_ref()
            .and_then(|value| value.get("semantic_sufficiency_proven"))
            .and_then(Value::as_bool)
            != Some(false)
    {
        gaps.push(gap(
            ContextGapKind::PayloadMismatch,
            None,
            "payload_contract",
        ));
    }
    let manifested: BTreeSet<_> = manifest
        .included
        .iter()
        .map(|object| object.reference.id.as_str())
        .collect();
    for name in payload_relations.keys().copied() {
        if !manifested.contains(name) {
            gaps.push(gap(
                ContextGapKind::PayloadMismatch,
                Some(name),
                "unmanifested_relation",
            ));
        }
    }
    let mut included = BTreeSet::new();
    for object in &manifest.included {
        let name = object.reference.id.as_str();
        if !included.insert(name) {
            gaps.push(gap(
                ContextGapKind::MissingObject,
                Some(name),
                "duplicate_manifest_object",
            ));
            continue;
        }
        let Some(entry) = snapshot.relation(name) else {
            gaps.push(gap(
                ContextGapKind::MissingObject,
                Some(name),
                "absent_from_snapshot",
            ));
            continue;
        };
        if entry.reference() != &object.reference {
            gaps.push(gap(
                ContextGapKind::StaleObject,
                Some(name),
                "object_revision",
            ));
        }
        let Some(rendered) = payload_relations.get(name).copied() else {
            gaps.push(gap(
                ContextGapKind::MissingObject,
                Some(name),
                "absent_from_payload",
            ));
            continue;
        };
        if rendered.get("revision").and_then(Value::as_str)
            != Some(object.reference.revision.as_str())
        {
            gaps.push(gap(
                ContextGapKind::StaleObject,
                Some(name),
                "payload_revision",
            ));
        }
        let relation = entry.definition();
        if rendered.get("description")
            != Some(&serde_json::to_value(&relation.description).expect("description serializes"))
            || rendered.get("grain")
                != Some(&serde_json::to_value(&relation.grain).expect("grain serializes"))
        {
            gaps.push(gap(
                ContextGapKind::MissingFactGroup,
                Some(name),
                "relation_description_or_grain",
            ));
        }
        let actual_fields: BTreeSet<_> = relation
            .schema
            .fields()
            .iter()
            .map(|field| field.name().as_str())
            .collect();
        let selected_fields: BTreeSet<_> = object.fields.iter().map(String::as_str).collect();
        if selected_fields.len() != object.fields.len()
            || !selected_fields.is_subset(&actual_fields)
        {
            gaps.push(gap(
                ContextGapKind::MissingField,
                Some(name),
                "manifest_fields",
            ));
        }
        if object.field_inventory_complete != (selected_fields == actual_fields) {
            gaps.push(gap(
                ContextGapKind::IncompleteInventory,
                Some(name),
                "field_inventory_complete",
            ));
        }
        if rendered.get("fields_total").and_then(Value::as_u64) != Some(actual_fields.len() as u64)
            || rendered
                .get("field_inventory_complete")
                .and_then(Value::as_bool)
                != Some(object.field_inventory_complete)
        {
            gaps.push(gap(
                ContextGapKind::IncompleteInventory,
                Some(name),
                "payload_field_inventory",
            ));
        }
        let rendered_columns: Vec<_> = rendered
            .get("columns")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .collect();
        let rendered_fields: BTreeSet<_> = rendered
            .get("columns")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|column| column.get("name").and_then(Value::as_str))
            .collect();
        if rendered_fields != selected_fields || rendered_columns.len() != rendered_fields.len() {
            gaps.push(gap(
                ContextGapKind::MissingField,
                Some(name),
                "payload_columns",
            ));
        }
        for field in &object.fields {
            let rendered_column = rendered
                .get("columns")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .find(|column| column.get("name").and_then(Value::as_str) == Some(field.as_str()));
            let expected = relation
                .semantics
                .as_ref()
                .and_then(|semantics| semantics.fields.get(field))
                .map(|semantics| {
                    serde_json::to_value(semantics).expect("field semantics serialize")
                })
                .unwrap_or(Value::Null);
            let physical = entry.field(field).expect("checked manifest field");
            let expected_data_type = physical.data_type().to_string();
            if rendered_column
                .and_then(|column| column.get("data_type"))
                .and_then(Value::as_str)
                != Some(expected_data_type.as_str())
                || rendered_column
                    .and_then(|column| column.get("nullable"))
                    .and_then(Value::as_bool)
                    != Some(physical.is_nullable())
            {
                gaps.push(gap(
                    ContextGapKind::MissingField,
                    Some(name),
                    format!("field_contract/{field}"),
                ));
            }
            if rendered_column.and_then(|column| column.get("semantics")) != Some(&expected) {
                gaps.push(gap(
                    ContextGapKind::MissingFactGroup,
                    Some(name),
                    format!("field/{field}"),
                ));
            }
            facts.push(ContextFact {
                relation: name.into(),
                kind: "field".into(),
                id: field.clone(),
                revision: Some(entry.reference().revision.clone()),
            });
        }
        facts.push(ContextFact {
            relation: name.into(),
            kind: "relation".into(),
            id: name.into(),
            revision: Some(entry.reference().revision.clone()),
        });
        if relation.semantics.is_none() && rendered.get("semantics") != Some(&Value::Null) {
            gaps.push(gap(
                ContextGapKind::MissingFactGroup,
                Some(name),
                "unexpected_semantics",
            ));
        }
        if let Some(semantics) = &relation.semantics {
            let rendered_semantics = rendered.get("semantics");
            let expected_coverage =
                serde_json::to_value(&semantics.view_coverage).expect("view coverage serializes");
            if rendered_semantics.and_then(|value| value.get("view_coverage"))
                != Some(&expected_coverage)
            {
                gaps.push(gap(
                    ContextGapKind::MissingFactGroup,
                    Some(name),
                    "view_coverage",
                ));
            }
            if semantics.view_coverage.is_some() {
                facts.push(ContextFact {
                    relation: name.into(),
                    kind: "view_coverage".into(),
                    id: name.into(),
                    revision: entry
                        .definition_reference("view_coverage", name)
                        .map(|reference| reference.revision.clone()),
                });
            }
            let expected_lineage =
                serde_json::to_value(&semantics.view_lineage).expect("view lineage serializes");
            if rendered_semantics.and_then(|value| value.get("view_lineage"))
                != Some(&expected_lineage)
            {
                gaps.push(gap(
                    ContextGapKind::MissingFactGroup,
                    Some(name),
                    "view_lineage",
                ));
            }
            if semantics.view_lineage.is_some() {
                facts.push(ContextFact {
                    relation: name.into(),
                    kind: "view_lineage".into(),
                    id: name.into(),
                    revision: entry
                        .definition_reference("view_lineage", name)
                        .map(|reference| reference.revision.clone()),
                });
            }
            for (kind, expected) in [
                (
                    "declared_primary_key",
                    serde_json::to_value(&semantics.declared_primary_key),
                ),
                (
                    "declared_unique_keys",
                    serde_json::to_value(&semantics.declared_unique_keys),
                ),
                ("origin", serde_json::to_value(&semantics.origin)),
                ("capability", serde_json::to_value(&semantics.capability)),
                (
                    "model_description",
                    serde_json::to_value(&semantics.model_description),
                ),
                (
                    "model_ai_context",
                    serde_json::to_value(&semantics.model_ai_context),
                ),
                ("ai_context", serde_json::to_value(&semantics.ai_context)),
            ] {
                let expected = expected.expect("catalog context metadata serializes");
                if rendered_semantics.and_then(|value| value.get(kind)) != Some(&expected) {
                    gaps.push(gap(ContextGapKind::MissingFactGroup, Some(name), kind));
                }
            }
            if !semantics.declared_primary_key.is_empty() {
                facts.push(ContextFact {
                    relation: name.into(),
                    kind: "declared_primary_key".into(),
                    id: name.into(),
                    revision: Some(entry.reference().revision.clone()),
                });
            }
            for index in 0..semantics.declared_unique_keys.len() {
                facts.push(ContextFact {
                    relation: name.into(),
                    kind: "declared_unique_key".into(),
                    id: index.to_string(),
                    revision: Some(entry.reference().revision.clone()),
                });
            }
            for (kind, expected) in [
                ("facts", serde_json::to_value(&semantics.facts)),
                (
                    "entity_identity",
                    serde_json::to_value(&semantics.entity_identity),
                ),
                ("metrics", serde_json::to_value(&semantics.metrics)),
                (
                    "ratio_metrics",
                    serde_json::to_value(&semantics.ratio_metrics),
                ),
                (
                    "value_mappings",
                    serde_json::to_value(&semantics.value_mappings),
                ),
                (
                    "relationships",
                    serde_json::to_value(&semantics.relationships),
                ),
                ("concepts", serde_json::to_value(&semantics.concepts)),
                ("conversions", serde_json::to_value(&semantics.conversions)),
                (
                    "business_calendars",
                    serde_json::to_value(&semantics.business_calendars),
                ),
                ("allocations", serde_json::to_value(&semantics.allocations)),
                (
                    "currency_rates",
                    serde_json::to_value(&semantics.currency_rates),
                ),
                (
                    "row_policies",
                    serde_json::to_value(&semantics.row_policies),
                ),
            ] {
                let expected = expected.expect("catalog facts serialize");
                if rendered_semantics.and_then(|value| value.get(kind)) != Some(&expected) {
                    gaps.push(gap(ContextGapKind::MissingFactGroup, Some(name), kind));
                } else if kind == "entity_identity" {
                    if let Some(identity) = &semantics.entity_identity {
                        facts.push(ContextFact {
                            relation: name.into(),
                            kind: "entity_identity".into(),
                            id: identity.id.0.clone(),
                            revision: Some(entry.reference().revision.clone()),
                        });
                    }
                } else if let Some(values) = expected.as_object() {
                    for id in values.keys() {
                        let reference = entry.definition_reference(
                            match kind {
                                "metrics" => "metric",
                                "ratio_metrics" => "ratio",
                                "value_mappings" => "value_mapping",
                                "relationships" => "relationship",
                                "concepts" => "concept",
                                "conversions" => "conversion",
                                "business_calendars" => "business_calendar",
                                "allocations" => "allocation",
                                "currency_rates" => "currency_rate",
                                _ => kind,
                            },
                            id,
                        );
                        facts.push(ContextFact {
                            relation: name.into(),
                            kind: kind.into(),
                            id: id.clone(),
                            revision: reference.map(|reference| reference.revision.clone()),
                        });
                    }
                } else if kind == "row_policies" {
                    for index in 0..semantics.row_policies.len() {
                        let reference = entry.definition_reference("policy", &index.to_string());
                        facts.push(ContextFact {
                            relation: name.into(),
                            kind: "row_policy".into(),
                            id: index.to_string(),
                            revision: reference.map(|reference| reference.revision.clone()),
                        });
                    }
                }
            }
        }
        if let RelationKind::View { sql, .. } = &relation.kind {
            if rendered.get("view_sql").and_then(Value::as_str) != Some(sql.as_str()) {
                gaps.push(gap(
                    ContextGapKind::MissingFactGroup,
                    Some(name),
                    "view_sql",
                ));
            }
            facts.push(ContextFact {
                relation: name.into(),
                kind: "view_restriction".into(),
                id: name.into(),
                revision: Some(entry.reference().revision.clone()),
            });
        } else if rendered.get("view_sql") != Some(&Value::Null) {
            gaps.push(gap(
                ContextGapKind::MissingFactGroup,
                Some(name),
                "unexpected_view_sql",
            ));
        }
        let mut required = BTreeSet::new();
        if let RelationKind::View { dependencies, .. } = &relation.kind {
            required.extend(dependencies.iter().map(String::as_str));
        }
        if let Some(semantics) = &relation.semantics {
            required.extend(
                semantics
                    .relationships
                    .values()
                    .map(|relationship| relationship.right_relation.as_str()),
            );
            required.extend(
                semantics
                    .allocations
                    .values()
                    .map(|allocation| allocation.bridge_relation.as_str()),
            );
            required.extend(
                semantics
                    .currency_rates
                    .values()
                    .map(|rate| rate.rate_relation.as_str()),
            );
            required.extend(
                semantics
                    .business_calendars
                    .values()
                    .map(|calendar| calendar.calendar_relation.as_str()),
            );
        }
        for target in required {
            if !included.contains(target)
                && !manifest
                    .included
                    .iter()
                    .any(|object| object.reference.id == target)
            {
                gaps.push(gap(ContextGapKind::MissingDependency, Some(name), target));
            }
            if !manifest
                .dependencies
                .iter()
                .any(|edge| edge.from == name && edge.to == target && edge.satisfied)
            {
                gaps.push(gap(ContextGapKind::MissingDependency, Some(name), target));
            }
        }
    }
    for search in &manifest.searches {
        if !search.exhausted || search.truncated_candidates {
            gaps.push(gap(ContextGapKind::IncompleteSearch, None, "search_budget"));
        }
    }
    if manifest.selection_mode == SelectionMode::Retrieved
        && (manifest.searches.is_empty()
            || manifest.index_revision.as_deref() != Some(snapshot.id()))
    {
        gaps.push(gap(
            ContextGapKind::IncompleteSearch,
            None,
            "retrieval_evidence",
        ));
    }
    if !manifest.declared_dependencies_complete
        || manifest.dependencies.iter().any(|edge| !edge.satisfied)
    {
        gaps.push(gap(
            ContextGapKind::MissingDependency,
            None,
            "declared_dependency_closure",
        ));
    }
    if manifest.semantic_sufficiency_proven {
        gaps.push(gap(
            ContextGapKind::FalseCompletenessClaim,
            None,
            "semantic_sufficiency_proven",
        ));
    }
    ContextAudit {
        complete: gaps.is_empty(),
        facts,
        gaps,
        semantic_sufficiency_proven: false,
    }
}
