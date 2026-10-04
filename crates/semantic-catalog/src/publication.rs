//! Atomic metadata publication. Live provider/schema validation belongs to Engine.
use crate::{Catalog, CatalogSnapshot, Relation, RelationKind, SnapshotRelation};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, RwLock},
};

#[derive(Debug)]
pub enum CatalogMutation {
    Put(Box<Relation>),
    Remove(String),
}
#[derive(Debug, Clone, Serialize)]
pub struct PublicationReport {
    pub previous_snapshot: String,
    pub snapshot: String,
    pub changed: BTreeSet<String>,
    pub affected: BTreeSet<String>,
    pub objects_validated: usize,
    pub edges_visited: usize,
}
#[derive(Debug, thiserror::Error)]
pub enum PublicationError {
    #[error("catalog publication raced with another revision")]
    Conflict,
    #[error("invalid semantic definition on relation {relation:?}: {code}")]
    InvalidDefinition {
        relation: String,
        code: &'static str,
    },
    #[error("relation {relation:?} depends on missing relation {dependency:?}")]
    MissingDependency {
        relation: String,
        dependency: String,
    },
    #[error("cyclic view definitions: {0:?}")]
    Cycle(Vec<String>),
    #[error("catalog update work budget exhausted")]
    WorkLimit,
    #[error("cannot remove missing relation: {0}")]
    MissingRelation(String),
}
#[derive(Debug, Clone)]
pub struct PublicationLimits {
    pub max_objects: usize,
    pub max_edges: usize,
}
impl Default for PublicationLimits {
    fn default() -> Self {
        Self {
            max_objects: 1_000_000,
            max_edges: 4_000_000,
        }
    }
}

impl Catalog {
    /// Validate all declared semantic references without acquiring providers.
    pub fn validate(
        &self,
        limits: &PublicationLimits,
    ) -> Result<PublicationReport, PublicationError> {
        let (objects_validated, edges_visited) = if self.analysis.get().is_some() {
            (0, 0)
        } else {
            let (reverse, objects, edges) = validate(self, limits)?;
            let _ = self
                .analysis
                .set(Arc::new(DependencyAnalysis::from_reverse(reverse)));
            (objects, edges)
        };
        let snapshot = self.snapshot().id().to_owned();
        Ok(PublicationReport {
            previous_snapshot: snapshot.clone(),
            snapshot,
            changed: BTreeSet::new(),
            affected: BTreeSet::new(),
            objects_validated,
            edges_visited,
        })
    }

    /// Apply a batch atomically to this catalog value, preserving old snapshots.
    /// This validates supplied dependency edges, not arbitrary authored SQL.
    /// Engine loading independently derives and validates SQL dependencies/types.
    pub fn apply_changes(
        &mut self,
        changes: impl IntoIterator<Item = CatalogMutation>,
        limits: &PublicationLimits,
    ) -> Result<PublicationReport, PublicationError> {
        let previous = self.snapshot();
        let mut draft = self.clone();
        let mut changed = BTreeSet::new();
        for (index, change) in changes.into_iter().enumerate() {
            if index >= limits.max_objects {
                return Err(PublicationError::WorkLimit);
            }
            match change {
                CatalogMutation::Put(relation) => {
                    let entry = Arc::new(SnapshotRelation::new(*relation));
                    let name = entry.definition().name.clone();
                    if draft
                        .root
                        .get(&name)
                        .is_none_or(|old| old.reference() != entry.reference())
                    {
                        draft.root = draft.root.insert(name.clone(), entry);
                        changed.insert(name);
                    }
                }
                CatalogMutation::Remove(name) => {
                    if draft.root.get(&name).is_none() {
                        return Err(PublicationError::MissingRelation(name));
                    }
                    draft.root = draft.root.remove(&name);
                    changed.insert(name);
                }
            }
        }
        if changed.is_empty() {
            return Ok(PublicationReport {
                previous_snapshot: previous.id().into(),
                snapshot: previous.id().into(),
                changed: changed.clone(),
                affected: changed,
                objects_validated: 0,
                edges_visited: 0,
            });
        }
        let (analysis, affected, objects_validated, edges_visited) =
            if let Some(analysis) = self.analysis.get() {
                incremental(self, &draft, analysis, &changed, limits)?
            } else {
                let (dependents, objects_validated, mut edges_visited) = validate(&draft, limits)?;
                let analysis = DependencyAnalysis::from_reverse(dependents.clone());
                let mut affected = changed.clone();
                let mut queue: VecDeque<_> = changed.iter().cloned().collect();
                // Old edges matter for removed dependencies as well as new edges.
                let mut old_reverse: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
                for entry in previous.relations() {
                    if let RelationKind::View { dependencies, .. } = &entry.definition().kind {
                        for dependency in dependencies {
                            edges_visited += 1;
                            if edges_visited > limits.max_edges {
                                return Err(PublicationError::WorkLimit);
                            }
                            old_reverse
                                .entry(dependency.clone())
                                .or_default()
                                .insert(entry.definition().name.clone());
                        }
                    }
                }
                for entry in previous.relations() {
                    if let Some(semantics) = &entry.definition().semantics {
                        for relationship in semantics.relationships.values() {
                            edges_visited += 1;
                            if edges_visited > limits.max_edges {
                                return Err(PublicationError::WorkLimit);
                            }
                            old_reverse
                                .entry(relationship.right_relation.clone())
                                .or_default()
                                .insert(entry.definition().name.clone());
                        }
                        for allocation in semantics.allocations.values() {
                            edges_visited += 1;
                            if edges_visited > limits.max_edges {
                                return Err(PublicationError::WorkLimit);
                            }
                            old_reverse
                                .entry(allocation.bridge_relation.clone())
                                .or_default()
                                .insert(entry.definition().name.clone());
                        }
                        for rule in semantics.currency_rates.values() {
                            edges_visited += 1;
                            if edges_visited > limits.max_edges {
                                return Err(PublicationError::WorkLimit);
                            }
                            old_reverse
                                .entry(rule.rate_relation.clone())
                                .or_default()
                                .insert(entry.definition().name.clone());
                        }
                        for calendar in semantics.business_calendars.values() {
                            edges_visited += 1;
                            if edges_visited > limits.max_edges {
                                return Err(PublicationError::WorkLimit);
                            }
                            old_reverse
                                .entry(calendar.calendar_relation.clone())
                                .or_default()
                                .insert(entry.definition().name.clone());
                        }
                    }
                }
                while let Some(name) = queue.pop_front() {
                    for dependent in dependents
                        .get(&name)
                        .into_iter()
                        .flatten()
                        .chain(old_reverse.get(&name).into_iter().flatten())
                    {
                        edges_visited += 1;
                        if edges_visited > limits.max_edges {
                            return Err(PublicationError::WorkLimit);
                        }
                        if affected.insert(dependent.clone()) {
                            queue.push_back(dependent.clone());
                        }
                    }
                }
                (analysis, affected, objects_validated, edges_visited)
            };
        draft.analysis.take();
        let _ = draft.analysis.set(Arc::new(analysis));
        draft.snapshot.take();
        let next_snapshot = draft.snapshot();
        next_snapshot.seed_search(&previous, &changed);
        let snapshot = next_snapshot.id().to_owned();
        *self = draft;
        Ok(PublicationReport {
            previous_snapshot: previous.id().into(),
            snapshot,
            changed,
            affected,
            objects_validated,
            edges_visited,
        })
    }
}
type ReverseEdges = BTreeMap<String, BTreeSet<String>>;
fn validate(
    catalog: &Catalog,
    limits: &PublicationLimits,
) -> Result<(ReverseEdges, usize, usize), PublicationError> {
    let mut reverse: ReverseEdges = BTreeMap::new();
    let mut indegree = BTreeMap::new();
    let mut edges = 0;
    for relation in catalog.relations() {
        if indegree.len() >= limits.max_objects {
            return Err(PublicationError::WorkLimit);
        }
        let dependencies = match &relation.kind {
            RelationKind::Base { .. } => BTreeSet::new(),
            RelationKind::View { dependencies, .. } => dependencies.iter().collect(),
        };
        for dependency in &dependencies {
            edges += 1;
            if edges > limits.max_edges {
                return Err(PublicationError::WorkLimit);
            }
            if catalog.relation(dependency).is_none() {
                return Err(PublicationError::MissingDependency {
                    relation: relation.name.clone(),
                    dependency: (**dependency).clone(),
                });
            }
            reverse
                .entry((**dependency).clone())
                .or_default()
                .insert(relation.name.clone());
        }
        indegree.insert(relation.name.clone(), dependencies.len());
    }
    let objects = indegree.len();
    let mut ready: BTreeSet<_> = indegree
        .iter()
        .filter(|(_, n)| **n == 0)
        .map(|(name, _)| name.clone())
        .collect();
    while let Some(name) = ready.pop_first() {
        indegree.remove(&name);
        for child in reverse.get(&name).into_iter().flatten() {
            let degree = indegree.get_mut(child).expect("dependent not yet emitted");
            *degree -= 1;
            if *degree == 0 {
                ready.insert(child.clone());
            }
        }
    }
    if !indegree.is_empty() {
        return Err(PublicationError::Cycle(indegree.into_keys().collect()));
    }
    validate_definitions(
        catalog,
        catalog.relations(),
        limits,
        &mut edges,
        &mut reverse,
    )?;
    Ok((reverse, objects, edges))
}

fn validate_definitions<'a>(
    catalog: &Catalog,
    relations: impl Iterator<Item = &'a Relation>,
    limits: &PublicationLimits,
    edges: &mut usize,
    reverse: &mut ReverseEdges,
) -> Result<(), PublicationError> {
    let snapshot = catalog.snapshot();
    for relation in relations {
        let Some(semantics) = &relation.semantics else {
            continue;
        };
        let entry = catalog.root.get(&relation.name).expect("catalog entry");
        let invalid = |code| PublicationError::InvalidDefinition {
            relation: relation.name.clone(),
            code,
        };
        let mut identities = BTreeSet::new();
        let check_field = |name: &str, edges: &mut usize| -> Result<(), PublicationError> {
            *edges += 1;
            if *edges > limits.max_edges {
                return Err(PublicationError::WorkLimit);
            }
            if entry.field(name).is_none() {
                return Err(invalid("missing_or_ambiguous_field"));
            }
            Ok(())
        };
        if let Some(entity) = &semantics.entity_identity {
            if entity.id.0.trim().is_empty()
                || entity.id.0.len() > 256
                || !identities.insert(entity.id.0.as_str())
                || entity.relation != relation.name
                || entity.source_grain.entity.as_ref() != Some(&entity.id)
                || entity.source_grain.keys.is_empty()
            {
                return Err(invalid("invalid_entity_identity"));
            }
            let mut names = BTreeSet::new();
            let mut key_tuple = Vec::with_capacity(entity.source_grain.keys.len());
            for key in &entity.source_grain.keys {
                if key.relation != relation.name || !names.insert(key.field.as_str()) {
                    return Err(invalid("invalid_entity_key"));
                }
                check_field(&key.field, edges)?;
                if entry
                    .field(&key.field)
                    .expect("checked entity key")
                    .is_nullable()
                {
                    return Err(invalid("invalid_entity_key"));
                }
                key_tuple.push(key.field.as_str());
            }
            if semantics
                .declared_primary_key
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                != key_tuple
                && !semantics
                    .declared_unique_keys
                    .iter()
                    .any(|keys| keys.iter().map(String::as_str).collect::<Vec<_>>() == key_tuple)
            {
                return Err(invalid("invalid_entity_key"));
            }
            match &entity.key_evidence {
                crate::FactResolution::Known {
                    value: crate::KeyEvidence::AuthoredDeclaration,
                    contributors,
                } if !contributors.is_empty()
                    && contributors.iter().all(|fact| {
                        !fact.id.trim().is_empty()
                            && fact.scope == relation.name
                            && fact.value == crate::KeyEvidence::AuthoredDeclaration
                            && fact.authority == crate::Authority::Authored
                            && fact.evidence.is_empty()
                    }) => {}
                _ => return Err(invalid("unauthenticated_key_evidence")),
            }
        }
        for (field, meaning) in &semantics.fields {
            if let Some(unit) = &meaning.unit {
                check_field(field, edges)?;
                if !crate::valid_unit(unit)
                    || !(matches!(
                        entry.field(field).expect("checked unit field").data_type(),
                        crate::DataType::Int16
                            | crate::DataType::Int32
                            | crate::DataType::Int64
                            | crate::DataType::Decimal128(_, _)
                    ) || matches!(
                        entry.field(field).expect("checked unit field").data_type(),
                        crate::DataType::Float32 | crate::DataType::Float64
                    ) && matches!(unit, crate::Unit::Named { .. }))
                {
                    return Err(invalid("invalid_field_unit"));
                }
            }
            if let Some(domain) = &meaning.enum_domain {
                check_field(field, edges)?;
                if domain.id.trim().is_empty()
                    || entry
                        .field(field)
                        .is_none_or(|field| *field.data_type() != crate::DataType::Utf8)
                {
                    return Err(invalid("invalid_enum_domain"));
                }
            }
            if let Some(reference_system) = &meaning.reference_system {
                check_field(field, edges)?;
                if reference_system.id.trim().is_empty() {
                    return Err(invalid("invalid_reference_system"));
                }
            }
            if let Some(calendar_reference) = &meaning.calendar_reference {
                check_field(field, edges)?;
                if !crate::valid_calendar_reference(calendar_reference)
                    || !matches!(
                        entry
                            .field(field)
                            .expect("checked calendar field")
                            .data_type(),
                        crate::DataType::Date32 | crate::DataType::Timestamp(_, _)
                    )
                {
                    return Err(invalid("invalid_calendar_reference"));
                }
            }
        }
        if let Some(lineage) = &semantics.view_lineage {
            let RelationKind::View { sql, dependencies } = &relation.kind else {
                return Err(invalid("invalid_view_lineage"));
            };
            *edges += 1;
            if *edges > limits.max_edges {
                return Err(PublicationError::WorkLimit);
            }
            let source = snapshot
                .relation(&lineage.source.id)
                .ok_or_else(|| invalid("invalid_view_lineage"))?;
            if dependencies.len() != 1
                || dependencies[0] != lineage.source.id
                || source.reference() != &lineage.source
                || lineage.canonical_sql(&relation.schema).as_deref() != Some(sql.as_str())
            {
                return Err(invalid("invalid_view_lineage"));
            }
            for (output, input) in &lineage.columns {
                check_field(output, edges)?;
                *edges += 1;
                if *edges > limits.max_edges {
                    return Err(PublicationError::WorkLimit);
                }
                let Some(source_field) = source.field(input) else {
                    return Err(invalid("invalid_view_lineage"));
                };
                let output_field = entry.field(output).expect("checked view output field");
                if output_field.data_type() != source_field.data_type()
                    || output_field.is_nullable() != source_field.is_nullable()
                {
                    return Err(invalid("invalid_view_lineage"));
                }
            }
        }
        if let Some(coverage) = &semantics.view_coverage {
            if !matches!(relation.kind, RelationKind::View { .. }) {
                return Err(invalid("view_coverage_on_base"));
            }
            check_field(&coverage.field, edges)?;
            if coverage.validate().is_err()
                || !concept_value_matches(
                    entry
                        .field(&coverage.field)
                        .expect("checked view time field")
                        .data_type(),
                    &coverage.start,
                )
                || !concept_value_matches(
                    entry
                        .field(&coverage.field)
                        .expect("checked view time field")
                        .data_type(),
                    &coverage.end,
                )
            {
                return Err(invalid("invalid_view_coverage"));
            }
        }
        for (name, conversion) in &semantics.conversions {
            check_field(&conversion.field, edges)?;
            if name.trim().is_empty()
                || conversion.validate().is_err()
                || !identities.insert(conversion.id.as_str())
                || entry
                    .field(&conversion.field)
                    .is_none_or(|field| *field.data_type() != crate::DataType::Int64)
            {
                return Err(invalid("invalid_unit_conversion"));
            }
            if semantics
                .fields
                .get(&conversion.field)
                .and_then(|field| field.unit.as_ref())
                .is_some_and(|unit| unit != &conversion.from_unit)
            {
                return Err(invalid("conversion_source_unit"));
            }
        }
        for (name, allocation) in &semantics.allocations {
            *edges = edges
                .checked_add(
                    allocation.source_entity_fields.len()
                        + allocation.bridge_source_fields.len()
                        + allocation.target_dimensions.len()
                        + 8,
                )
                .ok_or(PublicationError::WorkLimit)?;
            if *edges > limits.max_edges {
                return Err(PublicationError::WorkLimit);
            }
            if catalog.relation(&allocation.bridge_relation).is_none() {
                return Err(PublicationError::MissingDependency {
                    relation: relation.name.clone(),
                    dependency: allocation.bridge_relation.clone(),
                });
            }
            if name.trim().is_empty()
                || !identities.insert(allocation.id.as_str())
                || allocation.source_relation != relation.name
                || allocation.validate(&snapshot, None).is_err()
            {
                return Err(invalid("invalid_allocation"));
            }
            reverse
                .entry(allocation.bridge_relation.clone())
                .or_default()
                .insert(relation.name.clone());
        }
        for (name, rule) in &semantics.currency_rates {
            *edges = edges.checked_add(12).ok_or(PublicationError::WorkLimit)?;
            if *edges > limits.max_edges {
                return Err(PublicationError::WorkLimit);
            }
            if catalog.relation(&rule.rate_relation).is_none() {
                return Err(PublicationError::MissingDependency {
                    relation: relation.name.clone(),
                    dependency: rule.rate_relation.clone(),
                });
            }
            if name.trim().is_empty()
                || !identities.insert(rule.id.as_str())
                || rule.source_relation != relation.name
                || rule.validate(&snapshot, None).is_err()
            {
                return Err(invalid("invalid_currency_rate"));
            }
            reverse
                .entry(rule.rate_relation.clone())
                .or_default()
                .insert(relation.name.clone());
        }
        for (name, calendar) in &semantics.business_calendars {
            *edges = edges.checked_add(5).ok_or(PublicationError::WorkLimit)?;
            if *edges > limits.max_edges {
                return Err(PublicationError::WorkLimit);
            }
            if catalog.relation(&calendar.calendar_relation).is_none() {
                return Err(PublicationError::MissingDependency {
                    relation: relation.name.clone(),
                    dependency: calendar.calendar_relation.clone(),
                });
            }
            if name.trim().is_empty()
                || !identities.insert(calendar.id.as_str())
                || calendar.source_relation != relation.name
                || calendar.validate(&snapshot, None).is_err()
            {
                return Err(invalid("invalid_business_calendar"));
            }
            reverse
                .entry(calendar.calendar_relation.clone())
                .or_default()
                .insert(relation.name.clone());
        }
        for (name, mapping) in &semantics.value_mappings {
            check_field(&mapping.field, edges)?;
            let field_domain = semantics
                .fields
                .get(&mapping.field)
                .and_then(|field| field.enum_domain.as_ref());
            if name.is_empty()
                || mapping.id.is_empty()
                || !identities.insert(mapping.id.as_str())
                || entry
                    .field(&mapping.field)
                    .is_none_or(|f| *f.data_type() != crate::DataType::Utf8)
            {
                return Err(invalid("invalid_value_mapping"));
            }
            if !crate::enum_domains_compatible(field_domain, mapping.enum_domain.as_ref()) {
                return Err(invalid("invalid_value_mapping_domain"));
            }
            for phrase in mapping.codes.keys() {
                *edges += 1;
                if *edges > limits.max_edges {
                    return Err(PublicationError::WorkLimit);
                }
                if phrase.trim().is_empty() {
                    return Err(invalid("empty_value_phrase"));
                }
            }
        }
        for (name, concept) in &semantics.concepts {
            if name.trim().is_empty()
                || concept.id.trim().is_empty()
                || !identities.insert(concept.id.as_str())
            {
                return Err(invalid("invalid_concept_identity"));
            }
            if concept.alternatives.len() > 16 {
                return Err(invalid("invalid_concept_alternatives"));
            }
            let mut alternatives = BTreeSet::new();
            for alternative in &concept.alternatives {
                *edges += 1;
                if *edges > limits.max_edges {
                    return Err(PublicationError::WorkLimit);
                }
                if alternative == name
                    || !alternatives.insert(alternative.as_str())
                    || !semantics.concepts.contains_key(alternative)
                {
                    return Err(invalid("invalid_concept_alternatives"));
                }
            }
            let mut parameter_types = BTreeMap::new();
            let mut pending = vec![&concept.predicate];
            while let Some(predicate) = pending.pop() {
                *edges += 1;
                if *edges > limits.max_edges {
                    return Err(PublicationError::WorkLimit);
                }
                use semantic_plan::typed::RowPredicate;
                match predicate {
                    RowPredicate::Compare {
                        field,
                        operator,
                        value,
                    } => {
                        check_field(field, edges)?;
                        let physical = entry.field(field).expect("checked concept field");
                        if semantics
                            .fields
                            .get(field)
                            .is_some_and(|semantics| semantics.enum_domain.is_some())
                            && matches!(value, semantic_plan::typed::Literal::Utf8(_))
                        {
                            return Err(invalid("invalid_concept_enum_domain"));
                        }
                        if !concept_value_matches(physical.data_type(), value)
                            || matches!(value, semantic_plan::typed::Literal::Boolean(_))
                                && !matches!(
                                    operator,
                                    semantic_plan::typed::Comparison::Eq
                                        | semantic_plan::typed::Comparison::NotEq
                                )
                        {
                            return Err(invalid("invalid_concept_comparison"));
                        }
                    }
                    RowPredicate::CompareParameter {
                        field,
                        operator,
                        parameter,
                    } => {
                        check_field(field, edges)?;
                        let physical = entry.field(field).expect("checked concept field");
                        let data_type = physical.data_type();
                        if parameter.is_empty()
                            || parameter.len() > 64
                            || !parameter
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                            || !parameter
                                .as_bytes()
                                .first()
                                .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
                            || !matches!(
                                data_type,
                                crate::DataType::Boolean
                                    | crate::DataType::Int64
                                    | crate::DataType::Utf8
                                    | crate::DataType::Date32
                            )
                            || *data_type == crate::DataType::Boolean
                                && !matches!(
                                    operator,
                                    semantic_plan::typed::Comparison::Eq
                                        | semantic_plan::typed::Comparison::NotEq
                                )
                            || semantics.fields.get(field).is_some_and(|semantics| {
                                semantics.enum_domain.is_some()
                                    || matches!(
                                        semantics.comparison_profile.as_ref(),
                                        Some(crate::ComparisonProfile::Locale { .. })
                                    )
                            })
                            || parameter_types
                                .get(parameter.as_str())
                                .is_some_and(|previous| *previous != data_type)
                        {
                            return Err(invalid("invalid_concept_parameter"));
                        }
                        parameter_types.insert(parameter.as_str(), data_type);
                        if parameter_types.len() > 16 {
                            return Err(invalid("invalid_concept_parameter"));
                        }
                    }
                    RowPredicate::CompareMapped {
                        field,
                        mapping,
                        phrase,
                        ..
                    } => {
                        check_field(field, edges)?;
                        if semantics
                            .value_mappings
                            .get(mapping)
                            .is_none_or(|definition| {
                                definition.field != *field || !definition.codes.contains_key(phrase)
                            })
                        {
                            return Err(invalid("invalid_concept_value_mapping"));
                        }
                    }
                    RowPredicate::IsNull { field, .. } => check_field(field, edges)?,
                    RowPredicate::All { predicates } | RowPredicate::Any { predicates } => {
                        if predicates.is_empty() {
                            return Err(invalid("empty_concept_boolean"));
                        }
                        pending.extend(predicates);
                    }
                    RowPredicate::Not { predicate } => pending.push(predicate),
                }
            }
        }
        for (name, metric) in &semantics.metrics {
            if name.is_empty()
                || metric.id.is_empty()
                || !identities.insert(metric.id.as_str())
                || metric.source_grain.keys.is_empty()
            {
                return Err(invalid("invalid_metric_identity_or_grain"));
            }
            if metric.source_grain.entity.is_some()
                && semantics.entity_identity.as_ref().is_none_or(|identity| {
                    metric.source_grain.entity.as_ref() != Some(&identity.id)
                        || metric.source_grain != identity.source_grain
                })
            {
                return Err(invalid("invalid_metric_entity_grain"));
            }
            let mut grain_keys = BTreeSet::new();
            for key in &metric.source_grain.keys {
                if key.relation != relation.name || !grain_keys.insert(key.field.as_str()) {
                    return Err(invalid("invalid_metric_source_grain"));
                }
            }
            for field in metric
                .source_grain
                .keys
                .iter()
                .map(|key| &key.field)
                .chain(metric.compatible_dimensions.iter())
                .chain(metric.sum_rollup_dimensions.iter().flatten())
                .chain(metric.field.iter())
                .chain(metric.row_filters.iter().map(|filter| &filter.field))
            {
                check_field(field, edges)?;
            }
            if matches!(&metric.unit, crate::Presence::Value(unit) if !crate::valid_unit(unit)) {
                return Err(invalid("invalid_metric_unit"));
            }
            if let crate::Presence::Value(temporal) = &metric.temporal {
                check_field(&temporal.field, edges)?;
                let field = entry.field(&temporal.field).expect("checked field");
                if !valid_temporal_coverage(
                    field.data_type(),
                    &temporal.coverage_start,
                    &temporal.coverage_end,
                ) {
                    return Err(invalid("invalid_metric_temporal_applicability"));
                }
            }
            for dimension in &metric.compatible_lookup_dimensions {
                *edges += 1;
                if *edges > limits.max_edges {
                    return Err(PublicationError::WorkLimit);
                }
                let relationship = semantics
                    .relationships
                    .get(&dimension.relationship)
                    .ok_or_else(|| invalid("missing_metric_dimension_relationship"))?;
                let right = catalog
                    .root
                    .get(&relationship.right_relation)
                    .ok_or_else(|| invalid("missing_metric_dimension_relation"))?;
                if right.field(&dimension.field).is_none() {
                    return Err(invalid("missing_metric_dimension_field"));
                }
            }
            if let Some(dimensions) = &metric.sum_rollup_dimensions
                && (metric.distinct
                    || !matches!(
                        metric.function,
                        semantic_plan::typed::AggregateFunction::Sum
                            | semantic_plan::typed::AggregateFunction::Count
                    )
                    || !dimensions.is_subset(&metric.compatible_dimensions))
            {
                return Err(invalid("invalid_metric_scalar_merge"));
            }
            if let Some(state) = &metric.state {
                use crate::MetricStateKind;
                state
                    .validate()
                    .map_err(|_| invalid("invalid_metric_state"))?;
                if metric.sum_rollup_dimensions.is_some()
                    || !state
                        .merge_dimensions
                        .is_subset(&metric.compatible_dimensions)
                {
                    return Err(invalid("invalid_metric_state_scope"));
                }
                for field in &state.merge_dimensions {
                    check_field(field, edges)?;
                }
                match &state.state {
                    MetricStateKind::SumCountAverage => {
                        if metric.function != semantic_plan::typed::AggregateFunction::Sum
                            || metric.distinct
                            || metric.empty_behavior != crate::EmptyBehavior::Null
                            || metric.result_type != crate::DataType::Decimal128(38, 18)
                            || metric
                                .field
                                .as_ref()
                                .and_then(|field| entry.field(field))
                                .is_none_or(|field| *field.data_type() != crate::DataType::Int64)
                        {
                            return Err(invalid("invalid_metric_state_type"));
                        }
                    }
                    MetricStateKind::WeightedAverage { weight_field, zero } => {
                        check_field(weight_field, edges)?;
                        if metric.function != semantic_plan::typed::AggregateFunction::Sum
                            || metric.distinct
                            || metric.result_type != crate::DataType::Decimal128(38, 18)
                            || metric.empty_behavior
                                != match zero {
                                    crate::ZeroWeight::Null => crate::EmptyBehavior::Null,
                                    crate::ZeroWeight::Zero => crate::EmptyBehavior::Zero,
                                }
                            || metric
                                .field
                                .as_ref()
                                .and_then(|field| entry.field(field))
                                .is_none_or(|field| *field.data_type() != crate::DataType::Int64)
                            || entry
                                .field(weight_field)
                                .is_none_or(|field| *field.data_type() != crate::DataType::Int64)
                        {
                            return Err(invalid("invalid_metric_state_type"));
                        }
                    }
                    MetricStateKind::ExactDistinct { identity_fields } => {
                        for field in identity_fields {
                            check_field(field, edges)?;
                            if !exact_state_field_type(
                                entry.field(field).expect("checked field").data_type(),
                            ) {
                                return Err(invalid("invalid_metric_state_field_type"));
                            }
                        }
                        if identity_fields.len() != 1
                            || metric.function != semantic_plan::typed::AggregateFunction::Count
                            || metric.field.as_deref() != Some(identity_fields[0].as_str())
                            || metric.distinct
                            || metric.empty_behavior != crate::EmptyBehavior::Zero
                            || metric.result_type != crate::DataType::Int64
                            || entry
                                .field(&identity_fields[0])
                                .is_none_or(|field| *field.data_type() != crate::DataType::Int64)
                        {
                            return Err(invalid("invalid_metric_state_type"));
                        }
                    }
                    MetricStateKind::SnapshotBalance {
                        time_field,
                        tie_break_fields,
                    } => {
                        if !metric_numeric_type(&metric.result_type) {
                            return Err(invalid("invalid_metric_state_type"));
                        }
                        check_field(time_field, edges)?;
                        if !matches!(
                            entry.field(time_field).expect("checked field").data_type(),
                            crate::DataType::Date32 | crate::DataType::Timestamp(_, _)
                        ) {
                            return Err(invalid("invalid_metric_state_time_type"));
                        }
                        for field in tie_break_fields {
                            check_field(field, edges)?;
                            if !exact_state_field_type(
                                entry.field(field).expect("checked field").data_type(),
                            ) {
                                return Err(invalid("invalid_metric_state_field_type"));
                            }
                        }
                    }
                }
            }
        }
        for policy in &semantics.row_policies {
            if policy.id.is_empty()
                || !identities.insert(policy.id.as_str())
                || policy.filters.is_empty()
            {
                return Err(invalid("invalid_policy"));
            }
            for filter in &policy.filters {
                check_field(&filter.field, edges)?;
            }
        }
        for (name, ratio) in &semantics.ratio_metrics {
            *edges += 2;
            if *edges > limits.max_edges {
                return Err(PublicationError::WorkLimit);
            }
            if name.is_empty()
                || ratio.id.is_empty()
                || !identities.insert(ratio.id.as_str())
                || semantics.metrics.contains_key(name)
            {
                return Err(invalid("invalid_ratio_identity"));
            }
            if !semantics.metrics.contains_key(&ratio.numerator)
                || !semantics.metrics.contains_key(&ratio.denominator)
            {
                return Err(invalid("missing_ratio_dependency"));
            }
            if matches!(&ratio.unit, crate::Presence::Value(unit) if !crate::valid_unit(unit)) {
                return Err(invalid("invalid_ratio_unit"));
            }
        }
        for (name, relationship) in &semantics.relationships {
            if name.is_empty()
                || relationship.id.is_empty()
                || !identities.insert(relationship.id.as_str())
                || relationship.role.is_empty()
                || relationship.key_pairs.is_empty()
            {
                return Err(invalid("invalid_relationship"));
            }
            let right = catalog
                .root
                .get(&relationship.right_relation)
                .ok_or_else(|| PublicationError::MissingDependency {
                    relation: relation.name.clone(),
                    dependency: relationship.right_relation.clone(),
                })?;
            for key in &relationship.key_pairs {
                check_field(&key.left_field, edges)?;
                let right_field = right
                    .field(&key.right_field)
                    .ok_or_else(|| invalid("missing_relationship_key"))?;
                if entry
                    .field(&key.left_field)
                    .expect("checked key")
                    .data_type()
                    != right_field.data_type()
                {
                    return Err(invalid("relationship_key_type"));
                }
                let left_system = semantics
                    .fields
                    .get(&key.left_field)
                    .and_then(|field| field.reference_system.as_ref());
                let right_system = right
                    .definition()
                    .semantics
                    .as_ref()
                    .and_then(|semantics| semantics.fields.get(&key.right_field))
                    .and_then(|field| field.reference_system.as_ref());
                if !crate::reference_systems_compatible(left_system, right_system) {
                    return Err(invalid("relationship_reference_system"));
                }
            }
            reverse
                .entry(relationship.right_relation.clone())
                .or_default()
                .insert(relation.name.clone());
        }
    }
    Ok(())
}

fn metric_numeric_type(ty: &crate::DataType) -> bool {
    matches!(
        ty,
        crate::DataType::Int64 | crate::DataType::Decimal128(_, _)
    )
}

fn exact_state_field_type(ty: &crate::DataType) -> bool {
    matches!(
        ty,
        crate::DataType::Boolean
            | crate::DataType::Int16
            | crate::DataType::Int32
            | crate::DataType::Int64
            | crate::DataType::UInt64
            | crate::DataType::Utf8
            | crate::DataType::Decimal128(_, _)
            | crate::DataType::Date32
            | crate::DataType::Timestamp(_, _)
    )
}

fn valid_temporal_coverage(
    field_type: &crate::DataType,
    start: &semantic_plan::typed::Literal,
    end: &semantic_plan::typed::Literal,
) -> bool {
    use arrow_schema::TimeUnit;
    use semantic_plan::typed::{Literal, TimestampUnit};
    match (field_type, start, end) {
        (crate::DataType::Date32, Literal::Date32(start), Literal::Date32(end)) => start < end,
        (
            crate::DataType::Timestamp(field_unit, Some(field_zone)),
            Literal::Timestamp {
                ticks: start,
                unit: start_unit,
                timezone: Some(start_zone),
            },
            Literal::Timestamp {
                ticks: end,
                unit: end_unit,
                timezone: Some(end_zone),
            },
        ) => {
            let unit = match field_unit {
                TimeUnit::Second => TimestampUnit::Second,
                TimeUnit::Millisecond => TimestampUnit::Millisecond,
                TimeUnit::Microsecond => TimestampUnit::Microsecond,
                TimeUnit::Nanosecond => TimestampUnit::Nanosecond,
            };
            *start < *end
                && *start_unit == unit
                && *end_unit == unit
                && start_zone == field_zone.as_ref()
                && end_zone == field_zone.as_ref()
                && matches!(field_zone.as_ref(), "UTC" | "+00:00")
        }
        _ => false,
    }
}

fn concept_value_matches(field: &crate::DataType, literal: &semantic_plan::typed::Literal) -> bool {
    use arrow_schema::TimeUnit;
    use semantic_plan::typed::{Literal, TimestampUnit};
    match (field, literal) {
        (crate::DataType::Boolean, Literal::Boolean(_))
        | (crate::DataType::Int16, Literal::Int16(_))
        | (crate::DataType::Int32, Literal::Int32(_))
        | (crate::DataType::Int64, Literal::Int64(_))
        | (crate::DataType::UInt64, Literal::UInt64(_))
        | (crate::DataType::Utf8, Literal::Utf8(_))
        | (crate::DataType::Date32, Literal::Date32(_)) => true,
        (
            crate::DataType::Decimal128(precision, scale),
            Literal::Decimal128 {
                precision: value_precision,
                scale: value_scale,
                ..
            },
        ) => {
            precision == value_precision
                && *scale >= 0
                && u8::try_from(*scale).ok() == Some(*value_scale)
        }
        (
            crate::DataType::Timestamp(field_unit, field_zone),
            Literal::Timestamp {
                unit: value_unit,
                timezone: value_zone,
                ..
            },
        ) => {
            let unit = match field_unit {
                TimeUnit::Second => TimestampUnit::Second,
                TimeUnit::Millisecond => TimestampUnit::Millisecond,
                TimeUnit::Microsecond => TimestampUnit::Microsecond,
                TimeUnit::Nanosecond => TimestampUnit::Nanosecond,
            };
            unit == *value_unit
                && field_zone.as_ref().map(|zone| zone.as_ref()) == value_zone.as_deref()
        }
        _ => false,
    }
}

/// One current snapshot; retained readers own old generations through Arc. No
/// unbounded history list is kept. Publication uses optimistic revision checks.
#[derive(Debug)]
pub struct CatalogStore {
    current: RwLock<Catalog>,
}
impl CatalogStore {
    pub fn new(catalog: Catalog) -> Result<Self, PublicationError> {
        catalog.validate(&PublicationLimits::default())?;
        Ok(Self {
            current: RwLock::new(catalog),
        })
    }
    pub fn snapshot(&self) -> Arc<CatalogSnapshot> {
        self.current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot()
    }
    pub fn publish(
        &self,
        expected: &str,
        changes: impl IntoIterator<Item = CatalogMutation>,
        limits: &PublicationLimits,
    ) -> Result<PublicationReport, PublicationError> {
        let mut draft = self
            .current
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if draft.snapshot().id() != expected {
            return Err(PublicationError::Conflict);
        }
        let report = draft.apply_changes(changes, limits)?;
        let mut current = self.current.write().unwrap_or_else(|e| e.into_inner());
        if current.snapshot().id() != expected {
            return Err(PublicationError::Conflict);
        }
        *current = draft;
        Ok(report)
    }
}

#[derive(Debug)]
struct Dependents {
    names: BTreeSet<String>,
    revision: String,
}
impl crate::tree::Revisioned for Dependents {
    fn revision(&self) -> &str {
        &self.revision
    }
}
impl Dependents {
    fn new(names: BTreeSet<String>) -> Self {
        Self {
            revision: crate::canonical_digest(&serde_json::json!(names)),
            names,
        }
    }
}
#[derive(Debug, Clone, Default)]
pub(crate) struct DependencyAnalysis {
    reverse: crate::tree::Root<Dependents>,
}
impl DependencyAnalysis {
    fn from_reverse(reverse: ReverseEdges) -> Self {
        let mut analysis = Self::default();
        for (key, names) in reverse {
            analysis.reverse = analysis
                .reverse
                .insert(key, Arc::new(Dependents::new(names)));
        }
        analysis
    }
    fn dependents(&self, name: &str) -> impl Iterator<Item = &String> {
        self.reverse
            .get(name)
            .into_iter()
            .flat_map(|entry| &entry.names)
    }
    fn edit(
        &mut self,
        dependency: &str,
        dependent: &str,
        insert: bool,
        edges: &mut usize,
        limits: &PublicationLimits,
    ) -> Result<(), PublicationError> {
        let old = self.reverse.get(dependency);
        *edges = edges.saturating_add(old.map_or(0, |entry| entry.names.len()));
        if *edges > limits.max_edges {
            return Err(PublicationError::WorkLimit);
        }
        let mut names = old.map_or_else(BTreeSet::new, |entry| entry.names.clone());
        if insert {
            names.insert(dependent.into());
        } else {
            names.remove(dependent);
        }
        self.reverse = if names.is_empty() {
            self.reverse.remove(dependency)
        } else {
            self.reverse
                .insert(dependency.into(), Arc::new(Dependents::new(names)))
        };
        Ok(())
    }
}
fn dependencies(
    relation: Option<&Relation>,
    edges: &mut usize,
    limits: &PublicationLimits,
) -> Result<BTreeSet<String>, PublicationError> {
    let mut names = BTreeSet::new();
    if let Some(relation) = relation {
        let views = if let RelationKind::View { dependencies, .. } = &relation.kind {
            dependencies.as_slice()
        } else {
            &[]
        };
        let relationships = relation
            .semantics
            .as_ref()
            .into_iter()
            .flat_map(|s| s.relationships.values())
            .map(|r| &r.right_relation);
        let allocations = relation
            .semantics
            .as_ref()
            .into_iter()
            .flat_map(|s| s.allocations.values())
            .map(|a| &a.bridge_relation);
        let currency_rates = relation
            .semantics
            .as_ref()
            .into_iter()
            .flat_map(|s| s.currency_rates.values())
            .map(|r| &r.rate_relation);
        let business_calendars = relation
            .semantics
            .as_ref()
            .into_iter()
            .flat_map(|s| s.business_calendars.values())
            .map(|calendar| &calendar.calendar_relation);
        for name in views
            .iter()
            .chain(relationships)
            .chain(allocations)
            .chain(currency_rates)
            .chain(business_calendars)
        {
            *edges += 1;
            if *edges > limits.max_edges {
                return Err(PublicationError::WorkLimit);
            }
            names.insert(name.clone());
        }
    }
    Ok(names)
}
fn incremental(
    old: &Catalog,
    draft: &Catalog,
    previous: &DependencyAnalysis,
    changed: &BTreeSet<String>,
    limits: &PublicationLimits,
) -> Result<(DependencyAnalysis, BTreeSet<String>, usize, usize), PublicationError> {
    let mut analysis = previous.clone();
    let mut edges = 0;
    for name in changed {
        let before = dependencies(old.relation(name), &mut edges, limits)?;
        let after = dependencies(draft.relation(name), &mut edges, limits)?;
        for dependency in before.difference(&after) {
            analysis.edit(dependency, name, false, &mut edges, limits)?;
        }
        for dependency in after.difference(&before) {
            analysis.edit(dependency, name, true, &mut edges, limits)?;
        }
    }
    let mut affected = changed.clone();
    let mut queue: VecDeque<_> = changed.iter().cloned().collect();
    while let Some(name) = queue.pop_front() {
        for dependent in previous.dependents(&name).chain(analysis.dependents(&name)) {
            edges += 1;
            if edges > limits.max_edges {
                return Err(PublicationError::WorkLimit);
            }
            if affected.insert(dependent.clone()) {
                queue.push_back(dependent.clone());
            }
        }
        if affected.len() > limits.max_objects {
            return Err(PublicationError::WorkLimit);
        }
    }
    let mut objects = 0;
    for name in &affected {
        if let Some(relation) = draft.relation(name) {
            objects += 1;
            for dependency in dependencies(Some(relation), &mut edges, limits)? {
                if draft.relation(&dependency).is_none() {
                    return Err(PublicationError::MissingDependency {
                        relation: name.clone(),
                        dependency,
                    });
                }
            }
            validate_definitions(
                draft,
                std::iter::once(relation),
                limits,
                &mut edges,
                &mut BTreeMap::new(),
            )?;
        }
    }
    // The previous view graph was acyclic. Only changed outgoing edges can
    // create cycles; unchanged metadata edits need no graph traversal.
    let mut done = BTreeSet::new();
    let mut active = BTreeSet::new();
    for name in changed {
        let view_deps = |relation: Option<&Relation>| match relation.map(|r| &r.kind) {
            Some(RelationKind::View { dependencies, .. }) => Some(dependencies.clone()),
            _ => None,
        };
        if view_deps(old.relation(name)) == view_deps(draft.relation(name)) {
            continue;
        }
        let mut stack = vec![(name.clone(), false)];
        while let Some((name, exiting)) = stack.pop() {
            if exiting {
                active.remove(&name);
                done.insert(name);
                continue;
            }
            if done.contains(&name) {
                continue;
            }
            if !active.insert(name.clone()) {
                return Err(PublicationError::Cycle(active.into_iter().collect()));
            }
            if active.len() + done.len() > limits.max_objects {
                return Err(PublicationError::WorkLimit);
            }
            stack.push((name.clone(), true));
            if let Some(Relation {
                kind: RelationKind::View { dependencies, .. },
                ..
            }) = draft.relation(&name)
            {
                for dependency in dependencies {
                    edges += 1;
                    if edges > limits.max_edges {
                        return Err(PublicationError::WorkLimit);
                    }
                    stack.push((dependency.clone(), false));
                }
            }
        }
    }
    Ok((analysis, affected, objects, edges))
}
