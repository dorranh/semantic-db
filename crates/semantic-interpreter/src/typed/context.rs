//! Context selection and authoritative hydration share one immutable snapshot.
use super::{CompileDiagnostic, CompileOptions, Work, bounded_json, diagnostic};
use semantic_catalog::{CatalogSnapshot, ObjectRef, RelationKind, SearchOptions, SearchReport};
use semantic_plan::typed::{RowOperation, RowPredicate, RowQuery};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use semantic_compiler::typed::{
    AnalysisCache, AnalysisCacheError, AnalysisCacheIdentity, AnalysisCacheLimits, LookupDependency,
};
use std::sync::atomic::{AtomicBool, Ordering};

type InitialContext = (ContextState, String, ContextManifest);

pub(crate) struct ContextCache {
    cache: AnalysisCache<InitialContext>,
    full_single_relation_selection: AnalysisCache<ContextState>,
}

pub(super) enum ContextReuse {
    Miss,
    SameSnapshot,
    SelectionRerendered,
}

impl ContextCache {
    pub(crate) fn new() -> Self {
        Self {
            cache: AnalysisCache::new(AnalysisCacheLimits {
                max_entries: 32,
                max_bytes: 16 * 1024 * 1024,
                max_lookups: 1,
                max_active_keys: 32,
            })
            .expect("fixed context cache limits"),
            full_single_relation_selection: AnalysisCache::new(AnalysisCacheLimits {
                max_entries: 32,
                max_bytes: 16 * 1024 * 1024,
                max_lookups: 1,
                max_active_keys: 32,
            })
            .expect("fixed selection cache limits"),
        }
    }

    pub(super) async fn initial(
        &self,
        snapshot: &CatalogSnapshot,
        request: &str,
        options: &CompileOptions,
        work: &mut Work,
    ) -> Result<(ContextState, (String, ContextManifest), ContextReuse), CompileDiagnostic> {
        options.check()?;
        let selection_input = serde_json::json!({
            "request": request,
            "selection": options.selection_mode,
            "budgets": {
                "max_input_bytes": options.max_input_bytes,
                "max_context_bytes": options.max_context_bytes,
                "max_context_fields": options.max_context_fields,
                "max_context_relations": options.max_context_relations,
                "max_context_edges": options.max_context_edges,
                "initial_fields_per_relation": options.initial_fields_per_relation,
                "small_relation_fields": options.small_relation_fields,
                "max_index_objects": options.max_index_objects,
                "max_index_bytes": options.max_index_bytes,
                "max_search_postings": options.max_search_postings,
                "max_search_candidates": options.max_search_candidates,
                "max_search_terms": options.max_search_terms,
            },
        });
        let identity = AnalysisCacheIdentity {
            stage: "initial_context_v1".into(),
            input_digest: semantic_catalog::canonical_digest(&serde_json::json!({
                "snapshot": snapshot.id(),
                "input": selection_input.clone(),
            })),
            access_scope_revision: semantic_catalog::canonical_digest(&serde_json::json!(
                options.allowed_relations
            )),
            parameter_digest: semantic_catalog::canonical_digest(&serde_json::json!(
                options.request_context
            )),
            renderer_revision: "context-render-v1".into(),
            function_revision: "context-functions-v1".into(),
            acceptance_revision: "strict-v1".into(),
        };
        // Ranked retrieval has namespace/index competitors. Only exact Full
        // selection with one explicitly allowed relation has a complete
        // relation-level dependency, including the missing-name result.
        let selected_name = (options.selection_mode == SelectionMode::Full)
            .then_some(options.allowed_relations.as_ref())
            .flatten()
            .filter(|relations| relations.len() == 1)
            .and_then(|relations| relations.iter().next());
        let selection_identity = AnalysisCacheIdentity {
            stage: "full_single_relation_selection_v1".into(),
            input_digest: semantic_catalog::canonical_digest(&selection_input),
            ..identity.clone()
        };
        let built = AtomicBool::new(false);
        let built_for_build = &built;
        let selection_hit = AtomicBool::new(false);
        let selection_hit_for_build = &selection_hit;
        let selection_cache = &self.full_single_relation_selection;
        let work_for_build = &mut *work;
        let result = self
            .cache
            .get_or_build(snapshot, identity, move || async move {
                built_for_build.store(true, Ordering::Relaxed);
                let state = if let Some(name) = selected_name {
                    let selection_built = AtomicBool::new(false);
                    let selection_built_for_build = &selection_built;
                    let selection_work = &mut *work_for_build;
                    let selected = selection_cache
                        .get_or_build(snapshot, selection_identity, move || async move {
                            selection_built_for_build.store(true, Ordering::Relaxed);
                            let state =
                                ContextState::initial(snapshot, request, options, selection_work)?;
                            let bytes = serde_json::to_vec(&state)
                                .expect("selection state serializes")
                                .len();
                            Ok::<_, CompileDiagnostic>((
                                state,
                                vec![LookupDependency::Relation {
                                    name: name.as_str().to_owned(),
                                }],
                                bytes,
                            ))
                        })
                        .await
                        .map_err(cache_error)?;
                    if !selection_built.load(Ordering::Relaxed) {
                        selection_hit_for_build.store(true, Ordering::Relaxed);
                    }
                    (*selected).clone()
                } else {
                    ContextState::initial(snapshot, request, options, work_for_build)?
                };
                let (payload, manifest) =
                    state.render(snapshot, request, options, work_for_build)?;
                let bytes = payload.len()
                    + serde_json::to_vec(&manifest)
                        .expect("context manifest serializes")
                        .len();
                Ok::<_, CompileDiagnostic>((
                    (state, payload, manifest),
                    vec![LookupDependency::Snapshot],
                    bytes,
                ))
            })
            .await
            .map_err(cache_error)?;
        options.check()?;
        let hit = !built.load(Ordering::Relaxed);
        if hit {
            work.context_relations += result.2.included.len();
            work.context_fields += result
                .2
                .included
                .iter()
                .map(|object| object.fields.len())
                .sum::<usize>();
            work.context_bytes += result.1.len();
        }
        let reuse = if hit {
            ContextReuse::SameSnapshot
        } else if selection_hit.load(Ordering::Relaxed) {
            ContextReuse::SelectionRerendered
        } else {
            ContextReuse::Miss
        };
        Ok((
            result.0.clone(),
            (result.1.clone(), result.2.clone()),
            reuse,
        ))
    }
}

fn cache_error(error: AnalysisCacheError<CompileDiagnostic>) -> CompileDiagnostic {
    match error {
        AnalysisCacheError::Build(error) => error,
        AnalysisCacheError::Dependencies(_) => diagnostic(
            "cache_configuration",
            "Context cache dependency limit failed",
        ),
        AnalysisCacheError::Admission => diagnostic(
            "admission_closed",
            "Too many distinct context analyses are already in progress",
        ),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMode {
    #[default]
    Full,
    Retrieved,
    Auto,
}
#[derive(Debug, Clone, Serialize)]
pub struct ContextObject {
    pub reference: ObjectRef,
    pub fields: Vec<String>,
    pub field_inventory_complete: bool,
    pub reasons: BTreeSet<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ContextDependency {
    pub from: String,
    pub to: String,
    pub satisfied: bool,
}
#[derive(Debug, Clone, Serialize)]
pub struct ContextManifest {
    pub version: u32,
    pub snapshot_id: String,
    pub selection_mode: SelectionMode,
    pub index_revision: Option<String>,
    pub access_scope_revision: String,
    pub included: Vec<ContextObject>,
    pub dependencies: Vec<ContextDependency>,
    pub searches: Vec<SearchReport>,
    pub declared_dependencies_complete: bool,
    /// Always false: mechanical closure does not prove NL semantic sufficiency.
    pub semantic_sufficiency_proven: bool,
    pub payload_digest: String,
    pub payload_bytes: usize,
    /// Mechanical evidence audit; it never proves semantic sufficiency.
    pub audit: super::ContextAudit,
}
#[derive(Default, Clone, Serialize)]
struct Selection {
    fields: BTreeSet<String>,
    reasons: BTreeSet<String>,
}
#[derive(Clone, Serialize)]
pub(super) struct ContextState {
    mode: SelectionMode,
    selected: BTreeMap<String, Selection>,
    searches: Vec<SearchReport>,
    dependencies: Vec<ContextDependency>,
}
impl ContextState {
    pub fn expand(
        &mut self,
        requests: &[semantic_plan::typed::ContextRequest],
        snapshot: &CatalogSnapshot,
        request: &str,
        options: &CompileOptions,
        work: &mut Work,
    ) -> Result<(String, ContextManifest), CompileDiagnostic> {
        if requests.is_empty() || requests.len() > options.max_context_requests {
            return Err(diagnostic(
                "invalid_context_request",
                "Context requests must be nonempty and bounded",
            ));
        }
        if work.context_expansions >= options.max_expansions {
            return Err(diagnostic(
                "expansion_limit",
                "Context expansion round budget exhausted",
            ));
        }
        let mut next = self.clone();
        for item in requests {
            use semantic_plan::typed::ContextRequest;
            match item {
                ContextRequest::Search { terms, relation } => {
                    next.search(snapshot, terms, relation.as_deref(), options, work)?
                }
                ContextRequest::Hydrate { relation, fields } => {
                    next.hydrate(snapshot, relation, fields, options)?
                }
                ContextRequest::Inventory {
                    relation,
                    offset,
                    count,
                } => next.inventory(snapshot, relation, *offset, *count, options)?,
            }
        }
        if next
            .selected
            .iter()
            .map(|(r, s)| (r, &s.fields))
            .eq(self.selected.iter().map(|(r, s)| (r, &s.fields)))
        {
            return Err(diagnostic(
                "expansion_limit",
                "Context expansion found no additional authoritative context",
            ));
        }
        let bundle = next.render(snapshot, request, options, work)?;
        *self = next;
        work.context_expansions += 1;
        Ok(bundle)
    }
    pub fn initial(
        snapshot: &CatalogSnapshot,
        request: &str,
        options: &CompileOptions,
        work: &mut Work,
    ) -> Result<Self, CompileDiagnostic> {
        let mut state = Self {
            mode: options.selection_mode,
            selected: BTreeMap::new(),
            searches: Vec::new(),
            dependencies: Vec::new(),
        };
        match options.selection_mode {
            SelectionMode::Full => {
                state.full(snapshot, options)?;
            }
            SelectionMode::Retrieved => {
                state.search(snapshot, request, None, options, work)?;
            }
            SelectionMode::Auto => {
                state.mode = SelectionMode::Full;
                if let Err(error) = state
                    .full(snapshot, options)
                    .and_then(|_| state.render(snapshot, request, options, work).map(|_| ()))
                {
                    if error.code != "context_limit" {
                        return Err(error);
                    }
                    options.check()?;
                    state.selected.clear();
                    state.dependencies.clear();
                    state.mode = SelectionMode::Retrieved;
                    state.search(snapshot, request, None, options, work)?;
                }
            }
        }
        Ok(state)
    }
    fn full(
        &mut self,
        snapshot: &CatalogSnapshot,
        options: &CompileOptions,
    ) -> Result<(), CompileDiagnostic> {
        for relation in snapshot.relations() {
            options.check()?;
            let name = &relation.definition().name;
            if allowed(name, options) {
                self.add_relation(snapshot, name, true, "full_catalog", options)?;
            }
        }
        self.close_dependencies(snapshot, options)
    }
    fn add_relation(
        &mut self,
        snapshot: &CatalogSnapshot,
        name: &str,
        all_fields: bool,
        reason: &str,
        options: &CompileOptions,
    ) -> Result<(), CompileDiagnostic> {
        options.check()?;
        if !allowed(name, options) {
            return Err(diagnostic(
                "access_scope",
                "Relation is outside the compiler access scope",
            ));
        }
        let relation = snapshot
            .relation(name)
            .ok_or_else(|| {
                diagnostic(
                    "unknown_relation",
                    "Requested context relation is absent from the pinned snapshot",
                )
            })?
            .definition();
        let selection = self.selected.entry(name.into()).or_default();
        selection.reasons.insert(reason.into());
        if all_fields {
            if relation.schema.fields().len() > options.max_context_fields {
                return Err(diagnostic(
                    "context_limit",
                    "Required relation field inventory exceeds the context budget",
                ));
            }
            for field in relation.schema.fields() {
                options.check()?;
                selection.fields.insert(field.name().clone());
            }
        } else if selection.fields.is_empty() {
            for field in relation
                .schema
                .fields()
                .iter()
                .take(options.initial_fields_per_relation)
            {
                selection.fields.insert(field.name().clone());
            }
        }
        self.check_size(options)
    }
    fn check_size(&self, options: &CompileOptions) -> Result<(), CompileDiagnostic> {
        if self.selected.len() > options.max_context_relations
            || self
                .selected
                .values()
                .map(|s| s.fields.len())
                .sum::<usize>()
                > options.max_context_fields
        {
            return Err(diagnostic(
                "context_limit",
                "Required objects or fields exceed the context budget",
            ));
        }
        Ok(())
    }
    pub fn search(
        &mut self,
        snapshot: &CatalogSnapshot,
        terms: &str,
        relation: Option<&str>,
        options: &CompileOptions,
        work: &mut Work,
    ) -> Result<(), CompileDiagnostic> {
        if terms.trim().is_empty() || terms.len() > options.max_input_bytes {
            return Err(diagnostic(
                "invalid_context_request",
                "Search requires bounded nonempty terms",
            ));
        }
        let index = snapshot.search_index_budgeted(|objects, bytes| {
            options.check()?;
            work.index_objects_visited = work.index_objects_visited.saturating_add(objects);
            work.index_bytes_visited = work.index_bytes_visited.saturating_add(bytes);
            if work.index_objects_visited > options.max_index_objects
                || work.index_bytes_visited > options.max_index_bytes
            {
                return Err(diagnostic(
                    "index_limit",
                    "Index construction object or byte budget exhausted",
                ));
            }
            Ok(())
        })?;
        if index.snapshot_id() != snapshot.id() {
            return Err(diagnostic(
                "index_revision",
                "Index does not match the pinned snapshot",
            ));
        }
        let report = index.search(
            terms,
            &SearchOptions {
                max_postings: options
                    .max_search_postings
                    .saturating_sub(work.search_postings_visited),
                max_candidates: options.max_search_candidates,
                max_terms: options.max_search_terms,
                relation: relation.map(str::to_owned),
                allowed_relations: options.allowed_relations.clone(),
            },
            || options.check(),
        )?;
        work.search_postings_visited += report.postings_visited;
        let limited = !report.exhausted || report.truncated_candidates;
        // Preserve every known candidate/alias alternative or fail explicitly.
        // A partial posting list is never evidence of catalog absence.
        if limited {
            self.searches.push(report);
            return Err(diagnostic(
                "search_limit",
                "Search or alternative-candidate budget exhausted",
            ));
        }
        for hit in &report.hits {
            options.check()?;
            let name = hit.object.relation.as_ref();
            let definition = snapshot
                .relation(name)
                .expect("index references pinned definitions")
                .definition();
            let all = definition.schema.fields().len() <= options.small_relation_fields;
            self.add_relation(
                snapshot,
                name,
                all,
                if hit.exact {
                    "exact_or_alias"
                } else {
                    "lexical"
                },
                options,
            )?;
            if let Some(field) = &hit.object.field {
                self.selected
                    .get_mut(name)
                    .expect("selected relation")
                    .fields
                    .insert(field.to_string());
            }
        }
        self.searches.push(report);
        self.check_size(options)?;
        self.close_dependencies(snapshot, options)
    }
    pub fn hydrate(
        &mut self,
        snapshot: &CatalogSnapshot,
        relation: &str,
        fields: &[String],
        options: &CompileOptions,
    ) -> Result<(), CompileDiagnostic> {
        self.add_relation(snapshot, relation, false, "explicit_hydration", options)?;
        let entry = snapshot.relation(relation).expect("checked relation");
        if fields.len() > options.max_context_fields {
            return Err(diagnostic(
                "context_limit",
                "Requested field count exceeds context budget",
            ));
        }
        for field in fields {
            options.check()?;
            if entry.field(field).is_none() {
                return Err(diagnostic(
                    "unknown_field",
                    "Requested context field is missing or ambiguous",
                ));
            }
            self.selected
                .get_mut(relation)
                .expect("selected relation")
                .fields
                .insert(field.clone());
        }
        self.check_size(options)?;
        self.close_dependencies(snapshot, options)
    }
    pub fn inventory(
        &mut self,
        snapshot: &CatalogSnapshot,
        relation: &str,
        offset: usize,
        count: usize,
        options: &CompileOptions,
    ) -> Result<(), CompileDiagnostic> {
        if count == 0 || count > options.max_context_fields {
            return Err(diagnostic(
                "context_limit",
                "Inventory page size is outside the context budget",
            ));
        }
        self.add_relation(snapshot, relation, false, "field_inventory_page", options)?;
        let entry = snapshot.relation(relation).expect("checked relation");
        let fields = entry
            .definition()
            .schema
            .fields()
            .iter()
            .skip(offset)
            .take(count)
            .map(|f| f.name().clone())
            .collect::<Vec<_>>();
        self.hydrate(snapshot, relation, &fields, options)
    }
    fn close_dependencies(
        &mut self,
        snapshot: &CatalogSnapshot,
        options: &CompileOptions,
    ) -> Result<(), CompileDiagnostic> {
        let mut queue: VecDeque<_> = self.selected.keys().cloned().collect();
        let mut seen = BTreeSet::new();
        let mut dependencies = Vec::new();
        while let Some(name) = queue.pop_front() {
            options.check()?;
            if !seen.insert(name.clone()) {
                continue;
            }
            if seen.len() > options.max_context_relations {
                return Err(diagnostic(
                    "context_limit",
                    "Dependency closure node budget exhausted",
                ));
            }
            let relation = snapshot
                .relation(&name)
                .expect("selected definitions")
                .definition();
            if let Some(semantics) = &relation.semantics {
                let mut fields = BTreeSet::new();
                for relationship in semantics.relationships.values() {
                    options.check()?;
                    if dependencies.len() >= options.max_context_edges {
                        return Err(diagnostic(
                            "context_limit",
                            "Relationship closure edge budget exhausted",
                        ));
                    }
                    if relationship.key_pairs.len() + fields.len() > options.max_context_edges {
                        return Err(diagnostic(
                            "context_limit",
                            "Relationship key closure exceeds the work budget",
                        ));
                    }
                    fields.extend(
                        relationship
                            .key_pairs
                            .iter()
                            .map(|key| key.left_field.clone()),
                    );
                    self.add_relation(
                        snapshot,
                        &relationship.right_relation,
                        true,
                        "relationship_endpoint",
                        options,
                    )?;
                    dependencies.push(ContextDependency {
                        from: name.clone(),
                        to: relationship.right_relation.clone(),
                        satisfied: true,
                    });
                    queue.push_back(relationship.right_relation.clone());
                }
                for allocation in semantics.allocations.values() {
                    options.check()?;
                    let needed = allocation.source_entity_fields.len() + 3;
                    if dependencies.len() >= options.max_context_edges
                        || needed + fields.len() > options.max_context_edges
                    {
                        return Err(diagnostic(
                            "context_limit",
                            "Allocation dependency closure exceeds the work budget",
                        ));
                    }
                    fields.extend(allocation.source_entity_fields.iter().cloned());
                    fields.insert(allocation.source_amount_field.clone());
                    fields.insert(allocation.expected_membership_count_field.clone());
                    fields.insert(allocation.expected_weight_total_field.clone());
                    self.add_relation(
                        snapshot,
                        &allocation.bridge_relation,
                        true,
                        "allocation_bridge",
                        options,
                    )?;
                    dependencies.push(ContextDependency {
                        from: name.clone(),
                        to: allocation.bridge_relation.clone(),
                        satisfied: true,
                    });
                    queue.push_back(allocation.bridge_relation.clone());
                }
                for rate in semantics.currency_rates.values() {
                    options.check()?;
                    if dependencies.len() >= options.max_context_edges
                        || fields.len() + 3 > options.max_context_edges
                    {
                        return Err(diagnostic(
                            "context_limit",
                            "Currency rate dependency closure exceeds the work budget",
                        ));
                    }
                    fields.insert(rate.source_amount_field.clone());
                    fields.insert(rate.source_currency_field.clone());
                    fields.insert(rate.source_time_field.clone());
                    self.add_relation(
                        snapshot,
                        &rate.rate_relation,
                        true,
                        "currency_rate_relation",
                        options,
                    )?;
                    dependencies.push(ContextDependency {
                        from: name.clone(),
                        to: rate.rate_relation.clone(),
                        satisfied: true,
                    });
                    queue.push_back(rate.rate_relation.clone());
                }
                for calendar in semantics.business_calendars.values() {
                    options.check()?;
                    if dependencies.len() >= options.max_context_edges
                        || fields.len() + 1 > options.max_context_edges
                    {
                        return Err(diagnostic(
                            "context_limit",
                            "Business calendar dependency closure exceeds the work budget",
                        ));
                    }
                    fields.insert(calendar.source_date_field.clone());
                    self.add_relation(
                        snapshot,
                        &calendar.calendar_relation,
                        true,
                        "business_calendar_relation",
                        options,
                    )?;
                    dependencies.push(ContextDependency {
                        from: name.clone(),
                        to: calendar.calendar_relation.clone(),
                        satisfied: true,
                    });
                    queue.push_back(calendar.calendar_relation.clone());
                }
                for metric in semantics.metrics.values() {
                    options.check()?;
                    if metric.source_grain.keys.len() + metric.row_filters.len() + fields.len()
                        > options.max_context_edges
                    {
                        return Err(diagnostic(
                            "context_limit",
                            "Metric dependency closure exceeds the work budget",
                        ));
                    }
                    fields.extend(metric.source_grain.keys.iter().map(|key| key.field.clone()));
                    fields.extend(metric.field.iter().cloned());
                    fields.extend(metric.row_filters.iter().map(|filter| filter.field.clone()));
                    if let semantic_catalog::Presence::Value(temporal) = &metric.temporal {
                        fields.insert(temporal.field.clone());
                    }
                }
                if let Some(identity) = &semantics.entity_identity {
                    if identity.source_grain.keys.len() + fields.len() > options.max_context_edges {
                        return Err(diagnostic(
                            "context_limit",
                            "Entity key dependency closure exceeds the work budget",
                        ));
                    }
                    fields.extend(
                        identity
                            .source_grain
                            .keys
                            .iter()
                            .map(|key| key.field.clone()),
                    );
                }
                if semantics.conversions.len() + fields.len() > options.max_context_edges {
                    return Err(diagnostic(
                        "context_limit",
                        "Conversion dependency closure exceeds the work budget",
                    ));
                }
                fields.extend(
                    semantics
                        .conversions
                        .values()
                        .map(|conversion| conversion.field.clone()),
                );
                for policy in &semantics.row_policies {
                    options.check()?;
                    if policy.filters.len() + fields.len() > options.max_context_edges {
                        return Err(diagnostic(
                            "context_limit",
                            "Policy dependency closure exceeds the work budget",
                        ));
                    }
                    fields.extend(policy.filters.iter().map(|filter| filter.field.clone()));
                }
                let entry = snapshot.relation(&name).expect("selected relation");
                for field in fields {
                    if entry.field(&field).is_none() {
                        return Err(diagnostic(
                            "catalog_invalid",
                            "Governed definition depends on a missing or ambiguous field",
                        ));
                    }
                    self.selected
                        .get_mut(&name)
                        .expect("selected relation")
                        .fields
                        .insert(field);
                }
                self.check_size(options)?;
            }
            if let RelationKind::View {
                dependencies: inputs,
                ..
            } = &relation.kind
            {
                for dependency in inputs {
                    options.check()?;
                    if dependencies.len() >= options.max_context_edges {
                        return Err(diagnostic(
                            "context_limit",
                            "Dependency closure edge budget exhausted",
                        ));
                    }
                    // Even with checked output lineage, hydrate the complete
                    // dependency contract so nested governance remains visible.
                    self.add_relation(snapshot, dependency, true, "view_dependency", options)?;
                    dependencies.push(ContextDependency {
                        from: name.clone(),
                        to: dependency.clone(),
                        satisfied: true,
                    });
                    queue.push_back(dependency.clone());
                }
            }
        }
        self.dependencies = dependencies;
        Ok(())
    }
    pub fn missing(&self, query: &RowQuery) -> BTreeSet<String> {
        fn visit(predicate: &RowPredicate, fields: &mut BTreeSet<String>) {
            match predicate {
                RowPredicate::Compare { field, .. }
                | RowPredicate::CompareParameter { field, .. }
                | RowPredicate::CompareMapped { field, .. }
                | RowPredicate::IsNull { field, .. } => {
                    fields.insert(field.field.clone());
                }
                RowPredicate::All { predicates } | RowPredicate::Any { predicates } => {
                    for p in predicates {
                        visit(p, fields);
                    }
                }
                RowPredicate::Not { predicate } => visit(predicate, fields),
            }
        }
        let mut fields = BTreeSet::new();
        for requirement in &query.requirements {
            match &requirement.operation {
                RowOperation::Window { window, .. } => {
                    for input in window
                        .input
                        .iter()
                        .chain(&window.partition_by)
                        .chain(window.order_by.iter().map(|o| &o.input))
                    {
                        if let semantic_plan::typed::WindowInput::Field { field } = input {
                            fields.insert(field.field.clone());
                        }
                    }
                }
                RowOperation::CalendarFilter { field, .. }
                | RowOperation::CalendarGroup { field, .. }
                | RowOperation::Project { field, .. }
                | RowOperation::Order { field, .. }
                | RowOperation::Group { field, .. }
                | RowOperation::Aggregate {
                    field: Some(field), ..
                } => {
                    fields.insert(field.field.clone());
                }
                RowOperation::Ratio {
                    numerator,
                    denominator,
                    ..
                } => {
                    fields.extend(
                        numerator
                            .field
                            .iter()
                            .chain(denominator.field.iter())
                            .map(|f| f.field.clone()),
                    );
                }
                RowOperation::Filter { predicate } => visit(predicate, &mut fields),
                _ => {}
            }
        }
        if let Some(selected) = self.selected.get(&query.input.relation) {
            fields.retain(|field| !selected.fields.contains(field));
        }
        fields
    }
    pub fn contains_relation(&self, name: &str) -> bool {
        self.selected.contains_key(name)
    }
    pub fn render(
        &self,
        snapshot: &CatalogSnapshot,
        request: &str,
        options: &CompileOptions,
        work: &mut Work,
    ) -> Result<(String, ContextManifest), CompileDiagnostic> {
        #[derive(Serialize)]
        struct Context<'a> {
            version: u32,
            snapshot_id: &'a str,
            request: &'a str,
            request_context: &'a Option<super::RequestContext>,
            coverage: SelectionMode,
            semantic_sufficiency_proven: bool,
            relations: Vec<RelationContext<'a>>,
        }
        #[derive(Serialize)]
        struct RelationContext<'a> {
            name: &'a str,
            revision: &'a str,
            description: &'a Option<String>,
            grain: &'a Option<String>,
            semantics: Option<ScopedSemantics<'a>>,
            view_sql: Option<&'a str>,
            fields_total: usize,
            field_inventory_complete: bool,
            columns: Vec<ColumnContext<'a>>,
        }
        #[derive(Serialize)]
        struct ScopedSemantics<'a> {
            view_coverage: &'a Option<semantic_catalog::ViewTemporalCoverage>,
            view_lineage: &'a Option<semantic_catalog::ViewOutputLineage>,
            model_description: &'a Option<String>,
            model_ai_context: &'a Option<semantic_catalog::AiContext>,
            ai_context: &'a Option<semantic_catalog::AiContext>,
            declared_primary_key: &'a [String],
            declared_unique_keys: &'a [Vec<String>],
            entity_identity: &'a Option<semantic_catalog::EntityIdentity>,
            origin: &'a Option<semantic_catalog::SemanticOrigin>,
            capability: &'a Option<semantic_catalog::Capability>,
            facts: &'a BTreeMap<String, semantic_catalog::FactResolution<serde_json::Value>>,
            concepts: &'a BTreeMap<String, semantic_catalog::ConceptDefinition>,
            conversions: &'a BTreeMap<String, semantic_catalog::UnitConversion>,
            business_calendars: &'a BTreeMap<String, semantic_catalog::BusinessCalendarRule>,
            allocations: &'a BTreeMap<String, semantic_catalog::AllocationContract>,
            currency_rates: &'a BTreeMap<String, semantic_catalog::CurrencyRateRule>,
            metrics: &'a BTreeMap<String, semantic_catalog::MetricDefinition>,
            value_mappings: &'a BTreeMap<String, semantic_catalog::ValueMapping>,
            ratio_metrics: &'a BTreeMap<String, semantic_catalog::RatioDefinition>,
            relationships: &'a BTreeMap<String, semantic_catalog::RelationshipDefinition>,
            row_policies: &'a [semantic_catalog::RowPolicy],
        }
        #[derive(Serialize)]
        struct ColumnContext<'a> {
            name: &'a str,
            data_type: String,
            nullable: bool,
            semantics: Option<&'a semantic_catalog::FieldSemantics>,
        }
        let mut context = Context {
            version: 1,
            snapshot_id: snapshot.id(),
            request,
            request_context: &options.request_context,
            coverage: self.mode,
            semantic_sufficiency_proven: false,
            relations: Vec::new(),
        };
        let mut included = Vec::new();
        for (name, selection) in &self.selected {
            options.check()?;
            let entry = snapshot.relation(name).expect("selected definition");
            let relation = entry.definition();
            let mut columns = Vec::new();
            for name in &selection.fields {
                options.check()?;
                let field = entry.field(name).ok_or_else(|| {
                    diagnostic(
                        "catalog_invalid",
                        "Selected field is ambiguous in the catalog",
                    )
                })?;
                columns.push(ColumnContext {
                    name: field.name(),
                    data_type: field.data_type().to_string(),
                    nullable: field.is_nullable(),
                    semantics: relation.semantics.as_ref().and_then(|s| s.fields.get(name)),
                });
                work.context_fields += 1;
            }
            work.context_relations += 1;
            let complete = selection.fields.len() == relation.schema.fields().len();
            context.relations.push(RelationContext {
                name: &relation.name,
                revision: &entry.reference().revision,
                description: &relation.description,
                grain: &relation.grain,
                semantics: relation.semantics.as_ref().map(|s| ScopedSemantics {
                    view_coverage: &s.view_coverage,
                    view_lineage: &s.view_lineage,
                    model_description: &s.model_description,
                    model_ai_context: &s.model_ai_context,
                    ai_context: &s.ai_context,
                    declared_primary_key: &s.declared_primary_key,
                    declared_unique_keys: &s.declared_unique_keys,
                    entity_identity: &s.entity_identity,
                    origin: &s.origin,
                    capability: &s.capability,
                    facts: &s.facts,
                    concepts: &s.concepts,
                    conversions: &s.conversions,
                    business_calendars: &s.business_calendars,
                    allocations: &s.allocations,
                    currency_rates: &s.currency_rates,
                    metrics: &s.metrics,
                    value_mappings: &s.value_mappings,
                    ratio_metrics: &s.ratio_metrics,
                    relationships: &s.relationships,
                    row_policies: &s.row_policies,
                }),
                view_sql: match &relation.kind {
                    RelationKind::View { sql, .. } => Some(sql),
                    _ => None,
                },
                fields_total: relation.schema.fields().len(),
                field_inventory_complete: complete,
                columns,
            });
            included.push(ContextObject {
                reference: entry.reference().clone(),
                fields: selection.fields.iter().cloned().collect(),
                field_inventory_complete: complete,
                reasons: selection.reasons.clone(),
            });
        }
        // Reserve the fixed protocol and the full configured output allowance
        // before selecting context. Auto mode can then try retrieved context
        // when a full catalog cannot fit one model call's host byte envelope.
        let per_call_context_bytes = options
            .max_model_call_bytes
            .saturating_sub(options.max_model_output_bytes)
            .saturating_sub(include_str!("prompt.txt").len());
        let payload = bounded_json(
            &context,
            options.max_context_bytes.min(per_call_context_bytes),
        )
        .map_err(|_| {
            diagnostic(
                "context_limit",
                "Required context facts exceed the byte budget",
            )
        })?;
        work.context_bytes += payload.len();
        let payload_digest = format!("{:x}", Sha256::digest(payload.as_bytes()));
        let access_scope_revision = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&options.allowed_relations).expect("scope serialization")
            )
        );
        let mut manifest = ContextManifest {
            version: 1,
            snapshot_id: snapshot.id().into(),
            selection_mode: self.mode,
            index_revision: (self.mode == SelectionMode::Retrieved).then(|| snapshot.id().into()),
            access_scope_revision,
            included,
            dependencies: self.dependencies.clone(),
            searches: self.searches.clone(),
            declared_dependencies_complete: true,
            semantic_sufficiency_proven: false,
            payload_digest,
            payload_bytes: payload.len(),
            audit: super::ContextAudit::pending(),
        };
        let audit = super::audit_context_manifest(snapshot, &manifest, &payload);
        if !audit.complete {
            return Err(diagnostic(
                "partial_catalog",
                "Rendered context has incomplete pinned facts or dependencies",
            ));
        }
        manifest.audit = audit;
        Ok((payload, manifest))
    }
}
pub(super) fn allowed(name: &str, options: &CompileOptions) -> bool {
    options
        .allowed_relations
        .as_ref()
        .is_none_or(|names| names.contains(name))
}
