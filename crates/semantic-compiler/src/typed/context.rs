//! Context selection and authoritative hydration share one immutable snapshot.
use super::{CompileDiagnostic, CompileOptions, Work, bounded_json, diagnostic};
use semantic_catalog::{CatalogSnapshot, ObjectRef, RelationKind, SearchOptions, SearchReport};
use semantic_plan::typed::{RowOperation, RowPredicate, RowQuery};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

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
}
#[derive(Default, Clone)]
struct Selection {
    fields: BTreeSet<String>,
    reasons: BTreeSet<String>,
}
#[derive(Clone)]
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
                for metric in semantics.metrics.values() {
                    options.check()?;
                    if metric.source_grain.len() + metric.row_filters.len() + fields.len()
                        > options.max_context_edges
                    {
                        return Err(diagnostic(
                            "context_limit",
                            "Metric dependency closure exceeds the work budget",
                        ));
                    }
                    fields.extend(metric.source_grain.iter().cloned());
                    fields.extend(metric.field.iter().cloned());
                    fields.extend(metric.row_filters.iter().map(|filter| filter.field.clone()));
                    if let semantic_catalog::Presence::Value(temporal) = &metric.temporal {
                        fields.insert(temporal.field.clone());
                    }
                }
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
                    // Until checked view field lineage exists, hydrate complete
                    // dependency contracts rather than dropping governing facts.
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
            model_description: &'a Option<String>,
            model_ai_context: &'a Option<semantic_catalog::AiContext>,
            ai_context: &'a Option<semantic_catalog::AiContext>,
            declared_primary_key: &'a [String],
            declared_unique_keys: &'a [Vec<String>],
            origin: &'a Option<semantic_catalog::SemanticOrigin>,
            capability: &'a Option<semantic_catalog::Capability>,
            facts: &'a BTreeMap<String, semantic_catalog::FactResolution<serde_json::Value>>,
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
                    model_description: &s.model_description,
                    model_ai_context: &s.model_ai_context,
                    ai_context: &s.ai_context,
                    declared_primary_key: &s.declared_primary_key,
                    declared_unique_keys: &s.declared_unique_keys,
                    origin: &s.origin,
                    capability: &s.capability,
                    facts: &s.facts,
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
        let payload = bounded_json(&context, options.max_context_bytes).map_err(|_| {
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
        let manifest = ContextManifest {
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
        };
        Ok((payload, manifest))
    }
}
pub(super) fn allowed(name: &str, options: &CompileOptions) -> bool {
    options
        .allowed_relations
        .as_ref()
        .is_none_or(|names| names.contains(name))
}
