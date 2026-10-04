//! Immutable compiler metadata. Indexing/hashing happens at registration, not binding.
use std::{collections::BTreeMap, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::{Field, Relation, RelationKind};

/// Names are durable only within this catalog namespace; a rename creates an ID.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectRef {
    pub id: String,
    pub revision: String,
}

#[derive(Debug)]
pub struct SnapshotRelation {
    relation: Relation,
    reference: ObjectRef,
    semantic_revision: String,
    binding_revision: String,
    // None records duplicate names rather than silently picking a field.
    fields: BTreeMap<String, Option<usize>>,
    definitions: BTreeMap<String, ObjectRef>,
}

impl SnapshotRelation {
    pub(crate) fn new(relation: Relation) -> Self {
        let mut fields = BTreeMap::new();
        for (index, field) in relation.schema.fields().iter().enumerate() {
            fields
                .entry(field.name().clone())
                .and_modify(|v| *v = None)
                .or_insert(Some(index));
        }
        let kind = match &relation.kind {
            RelationKind::Base { source } => serde_json::json!({"base": source}),
            RelationKind::View { sql, dependencies } => {
                serde_json::json!({"view": sql, "dependencies": dependencies})
            }
        };
        // JSON Values canonicalize nested metadata maps before hashing. Include
        // physical bindings in the digest, but never expose them in model context.
        let content = serde_json::json!({
            "version": 1, "name": relation.name, "schema": relation.schema,
            "kind": kind, "description": relation.description, "owner": relation.owner,
            "grain": relation.grain, "semantics": relation.semantics,
        });
        let binding_revision =
            digest(&serde_json::json!({"version": 1, "kind": kind, "schema": relation.schema}));
        let mut semantic_content = content.clone();
        // Provenance changes do not change semantic definitions. The full object
        // revision above still pins source locations for reproducible explanations.
        if let Some(semantics) = semantic_content
            .get_mut("semantics")
            .and_then(serde_json::Value::as_object_mut)
        {
            semantics.remove("source_refs");
            if let Some(origin) = semantics
                .get_mut("origin")
                .and_then(serde_json::Value::as_object_mut)
            {
                origin.remove("document_sha256");
            }
            if let Some(fields) = semantics
                .get_mut("fields")
                .and_then(serde_json::Value::as_object_mut)
            {
                for field in fields.values_mut() {
                    if let Some(field) = field.as_object_mut() {
                        field.remove("source_refs");
                    }
                }
            }
            if let Some(mappings) = semantics
                .get_mut("value_mappings")
                .and_then(serde_json::Value::as_object_mut)
            {
                for mapping in mappings.values_mut() {
                    if let Some(mapping) = mapping.as_object_mut() {
                        mapping.remove("source_refs");
                    }
                }
            }
            if let Some(concepts) = semantics
                .get_mut("concepts")
                .and_then(serde_json::Value::as_object_mut)
            {
                for concept in concepts.values_mut() {
                    if let Some(concept) = concept.as_object_mut() {
                        concept.remove("source_refs");
                    }
                }
            }
            if let Some(coverage) = semantics
                .get_mut("view_coverage")
                .and_then(serde_json::Value::as_object_mut)
            {
                coverage.remove("source_refs");
            }
            if let Some(lineage) = semantics
                .get_mut("view_lineage")
                .and_then(serde_json::Value::as_object_mut)
            {
                lineage.remove("source_refs");
            }
            if let Some(conversions) = semantics
                .get_mut("conversions")
                .and_then(serde_json::Value::as_object_mut)
            {
                for conversion in conversions.values_mut() {
                    if let Some(conversion) = conversion.as_object_mut() {
                        conversion.remove("source_refs");
                    }
                }
            }
            if let Some(allocations) = semantics
                .get_mut("allocations")
                .and_then(serde_json::Value::as_object_mut)
            {
                for allocation in allocations.values_mut() {
                    if let Some(allocation) = allocation.as_object_mut() {
                        allocation.remove("source_refs");
                    }
                }
            }
            if let Some(rates) = semantics
                .get_mut("exact_decimal_rates")
                .and_then(serde_json::Value::as_object_mut)
            {
                for rule in rates.values_mut() {
                    if let Some(rule) = rule.as_object_mut() {
                        rule.remove("source_refs");
                    }
                }
            }
            if let Some(currency_rates) = semantics
                .get_mut("currency_rates")
                .and_then(serde_json::Value::as_object_mut)
            {
                for rule in currency_rates.values_mut() {
                    if let Some(rule) = rule.as_object_mut() {
                        rule.remove("source_refs");
                    }
                }
            }
            if let Some(calendars) = semantics
                .get_mut("business_calendars")
                .and_then(serde_json::Value::as_object_mut)
            {
                for calendar in calendars.values_mut() {
                    if let Some(calendar) = calendar.as_object_mut() {
                        calendar.remove("source_refs");
                    }
                }
            }
            if let Some(metrics) = semantics
                .get_mut("metrics")
                .and_then(serde_json::Value::as_object_mut)
            {
                for metric in metrics.values_mut() {
                    if let Some(metric) = metric.as_object_mut() {
                        metric.remove("source_refs");
                    }
                }
            }
            if let Some(ratios) = semantics
                .get_mut("ratio_metrics")
                .and_then(serde_json::Value::as_object_mut)
            {
                for ratio in ratios.values_mut() {
                    if let Some(ratio) = ratio.as_object_mut() {
                        ratio.remove("source_refs");
                    }
                }
            }
            if let Some(relationships) = semantics
                .get_mut("relationships")
                .and_then(serde_json::Value::as_object_mut)
            {
                for relationship in relationships.values_mut() {
                    if let Some(relationship) = relationship.as_object_mut() {
                        relationship.remove("source_refs");
                    }
                }
            }
            if let Some(policies) = semantics
                .get_mut("row_policies")
                .and_then(serde_json::Value::as_array_mut)
            {
                for policy in policies {
                    if let Some(policy) = policy.as_object_mut() {
                        policy.remove("source_refs");
                    }
                }
            }
            if let Some(facts) = semantics.get_mut("facts") {
                strip_fact_origins(facts);
            }
        }
        if matches!(relation.kind, RelationKind::Base { .. }) {
            semantic_content["kind"] = serde_json::json!({"base": true});
        }
        let semantic_revision = digest(&semantic_content);
        let mut definitions = BTreeMap::new();
        if let Some(semantics) = &relation.semantics {
            if let Some(coverage) = &semantics.view_coverage {
                definitions.insert(
                    format!("view_coverage/{}", relation.name),
                    coverage.reference(&relation.name),
                );
            }
            if let Some(lineage) = &semantics.view_lineage {
                definitions.insert(
                    format!("view_lineage/{}", relation.name),
                    lineage.reference(&relation.name),
                );
            }
            for (name, conversion) in &semantics.conversions {
                definitions.insert(format!("conversion/{name}"), conversion.reference());
            }
            for (name, allocation) in &semantics.allocations {
                definitions.insert(format!("allocation/{name}"), allocation.reference());
            }
            for (name, rule) in &semantics.exact_decimal_rates {
                definitions.insert(format!("exact_decimal_rate/{name}"), rule.reference());
            }
            for (name, rule) in &semantics.currency_rates {
                definitions.insert(format!("currency_rate/{name}"), rule.reference());
            }
            for (name, calendar) in &semantics.business_calendars {
                definitions.insert(format!("business_calendar/{name}"), calendar.reference());
            }
            for (name, metric) in &semantics.metrics {
                definitions.insert(format!("metric/{name}"), metric.reference());
            }
            for (name, mapping) in &semantics.value_mappings {
                definitions.insert(format!("value_mapping/{name}"), mapping.reference());
            }
            for (name, concept) in &semantics.concepts {
                definitions.insert(format!("concept/{name}"), concept.reference());
            }
            for (name, ratio) in &semantics.ratio_metrics {
                definitions.insert(format!("ratio/{name}"), ratio.reference());
            }
            for (name, relationship) in &semantics.relationships {
                definitions.insert(format!("relationship/{name}"), relationship.reference());
            }
            for (index, policy) in semantics.row_policies.iter().enumerate() {
                definitions.insert(format!("policy/{index}"), policy.reference());
            }
        }
        Self {
            semantic_revision,
            binding_revision,
            reference: ObjectRef {
                id: relation.name.clone(),
                revision: digest(&content),
            },
            relation,
            fields,
            definitions,
        }
    }

    /// Pinned at publication so binding never hashes unbounded definition text.
    pub fn definition_reference(&self, kind: &str, name: &str) -> Option<&ObjectRef> {
        self.definitions.get(&format!("{kind}/{name}"))
    }
    pub fn semantic_revision(&self) -> &str {
        &self.semantic_revision
    }
    pub fn binding_revision(&self) -> &str {
        &self.binding_revision
    }
    pub fn definition(&self) -> &Relation {
        &self.relation
    }
    pub fn reference(&self) -> &ObjectRef {
        &self.reference
    }
    /// Indexed, exact-case lookup. Missing and ambiguous fields both fail closed.
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields
            .get(name)
            .copied()
            .flatten()
            .map(|i| self.relation.schema.field(i))
    }
}

#[derive(Debug)]
pub struct CatalogSnapshot {
    id: String,
    root: crate::tree::Root,
    search: std::sync::Mutex<Option<Arc<crate::SearchIndex>>>,
    search_seed: std::sync::Mutex<Option<SearchSeed>>,
}

#[derive(Debug, Clone)]
struct SearchSeed {
    base: Arc<crate::SearchIndex>,
    changed: std::collections::BTreeSet<String>,
}

impl CatalogSnapshot {
    pub(crate) fn new(root: crate::tree::Root) -> Self {
        let id = root.digest().to_owned();
        Self {
            id,
            root,
            search: std::sync::Mutex::new(None),
            search_seed: std::sync::Mutex::new(None),
        }
    }
    /// Retain at most one completed index, never a chain of ancestor snapshots.
    pub(crate) fn seed_search(
        &self,
        previous: &Self,
        changed: &std::collections::BTreeSet<String>,
    ) {
        if let Some(base) = previous
            .search
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            *self.search_seed.lock().unwrap_or_else(|e| e.into_inner()) = Some(SearchSeed {
                base: base.clone(),
                changed: changed.clone(),
            });
        }
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn relation(&self, name: &str) -> Option<&SnapshotRelation> {
        self.root.get(name).map(AsRef::as_ref)
    }
    pub fn relations(&self) -> impl Iterator<Item = &SnapshotRelation> {
        self.root.iter().map(AsRef::as_ref)
    }
    pub fn len(&self) -> usize {
        self.root.len()
    }
    pub fn is_empty(&self) -> bool {
        self.root.len() == 0
    }

    /// Reuse an index only for this pinned snapshot. The callback bounds/cancels
    /// cold construction at every object; failed builds are never published.
    pub fn search_index<E>(
        &self,
        mut check: impl FnMut() -> Result<(), E>,
    ) -> Result<Arc<crate::SearchIndex>, E> {
        self.search_index_budgeted(|objects, _bytes| {
            if objects > 0 {
                check()?;
            }
            Ok(())
        })
    }

    /// Charges object visits and text bytes before index allocations. A warm
    /// index requires no construction budget; query work is charged separately.
    pub fn search_index_budgeted<E>(
        &self,
        check: impl FnMut(usize, usize) -> Result<(), E>,
    ) -> Result<Arc<crate::SearchIndex>, E> {
        if let Some(index) = self
            .search
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
        {
            return Ok(index.clone());
        }
        let seed = self
            .search_seed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let index = Arc::new(if let Some(seed) = seed {
            seed.base.update(self, &seed.changed, check)?
        } else {
            crate::SearchIndex::build(self, check)?
        });
        let mut slot = self.search.lock().unwrap_or_else(|e| e.into_inner());
        let published = slot.get_or_insert(index).clone();
        self.search_seed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        Ok(published)
    }
}

fn digest(value: &serde_json::Value) -> String {
    crate::canonical_digest(value)
}

fn strip_fact_origins(value: &mut serde_json::Value) {
    let Some(facts) = value.as_object_mut() else {
        return;
    };
    for resolution in facts.values_mut() {
        for key in ["contributors", "alternatives"] {
            if let Some(facts) = resolution
                .get_mut(key)
                .and_then(serde_json::Value::as_array_mut)
            {
                for fact in facts {
                    if let Some(fact) = fact.as_object_mut() {
                        fact.remove("origins");
                    }
                }
            }
        }
    }
}
