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
        for (name, mapping) in &semantics.value_mappings {
            check_field(&mapping.field, edges)?;
            if name.is_empty()
                || mapping.id.is_empty()
                || !identities.insert(mapping.id.as_str())
                || entry
                    .field(&mapping.field)
                    .is_none_or(|f| *f.data_type() != crate::DataType::Utf8)
            {
                return Err(invalid("invalid_value_mapping"));
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
        for (name, metric) in &semantics.metrics {
            if name.is_empty()
                || metric.id.is_empty()
                || !identities.insert(metric.id.as_str())
                || metric.source_grain.is_empty()
            {
                return Err(invalid("invalid_metric_identity_or_grain"));
            }
            for field in metric
                .source_grain
                .iter()
                .chain(metric.compatible_dimensions.iter())
                .chain(metric.sum_rollup_dimensions.iter().flatten())
                .chain(metric.field.iter())
                .chain(metric.row_filters.iter().map(|filter| &filter.field))
            {
                check_field(field, edges)?;
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
            }
            reverse
                .entry(relationship.right_relation.clone())
                .or_default()
                .insert(relation.name.clone());
        }
    }
    Ok(())
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
        for name in views.iter().chain(relationships) {
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
