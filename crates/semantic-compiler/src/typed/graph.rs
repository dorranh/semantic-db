//! Checked query DAGs. Leaves bind independently to one pinned catalog snapshot;
//! graph operators consume validated slots and never model-produced expressions.
use super::*;
mod calculate;
mod compose;
mod intent;
pub use intent::compile_graph_intent;
mod replay;
pub use replay::GraphReplayBundle;
mod set;
use datafusion::{
    common::Column,
    logical_expr::Expr,
    sql::sqlparser::{ast, dialect::GenericDialect, parser::Parser},
};
use semantic_catalog::{DataType, FactResolution, Field, Presence, SlotMeaning};
use semantic_plan::{graph::*, typed::*};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize)]
struct Slot {
    id: String,
    field: Field,
    origin: Option<(String, String)>,
    meaning: SlotMeaning,
}
#[derive(Debug, Clone, Serialize)]
struct CheckedNode {
    id: String,
    operation: CheckedOperation,
    slots: Vec<Slot>,
    /// Exactly these slots identify the result grain, when proved by grouping.
    group_keys: Option<BTreeSet<String>>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CheckedOperation {
    Calculate {
        calculation: Box<calculate::Calculation>,
    },
    Conditional {
        conditional: Box<calculate::Conditional>,
    },
    Cast {
        cast: Box<calculate::Cast>,
    },
    NullTest {
        test: Box<calculate::NullTest>,
    },
    CompareSlots {
        comparison: Box<calculate::SlotComparison>,
    },
    Filter {
        filter: Box<calculate::Filter>,
    },
    Compose {
        composition: Box<compose::Composition>,
    },
    Rows {
        bound: Box<BoundQuery>,
        plan: RelationalPlan,
    },
    Set {
        left: usize,
        right: usize,
        operator: SetOperator,
        duplicates: Duplicates,
        columns: Vec<SetColumn>,
    },
}
#[derive(Clone, Serialize)]
pub struct CompiledGraph {
    proposal: GraphQuery,
    snapshot_id: String,
    nodes: Vec<CheckedNode>,
    root: usize,
    execution_profile_revision: &'static str,
    request_context: Option<RequestContext>,
    request_evidence: Option<GraphRequestEvidence>,
    required_relations: BTreeSet<String>,
    restricted_scope: bool,
    sql: SqlArtifact,
}
impl std::fmt::Debug for CompiledGraph {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CompiledGraph")
            .field("node_count", &self.nodes.len())
            .field("relation_count", &self.required_relations.len())
            .field("output_count", &self.sql.expected_output().len())
            .finish_non_exhaustive()
    }
}
impl CompiledGraph {
    /// Checked semantic meaning of a root output slot, independent of Arrow type.
    pub fn slot_meaning(&self, slot_id: &str) -> Option<&SlotMeaning> {
        self.nodes[self.root]
            .slots
            .iter()
            .find(|slot| slot.id == slot_id)
            .map(|slot| &slot.meaning)
    }
    pub fn sql(&self) -> &SqlArtifact {
        &self.sql
    }
    pub fn execution_profile_revision(&self) -> &str {
        self.execution_profile_revision
    }
    fn check_snapshot(&self, engine: &Engine) -> Result<(), CompileDiagnostic> {
        if engine.catalog().snapshot().id() != self.snapshot_id {
            return Err(diagnostic(
                "snapshot_mismatch",
                "Graph execution requires its pinned catalog snapshot",
            ));
        }
        Ok(())
    }
    fn check_execution_scope(
        &self,
        allowed_relations: Option<&BTreeSet<String>>,
    ) -> Result<(), CompileDiagnostic> {
        if let Some(allowed) = allowed_relations {
            if self.required_relations.is_subset(allowed) {
                return Ok(());
            }
            return Err(diagnostic(
                "execution_scope",
                "Current execution scope does not authorize every compiled relation",
            ));
        }
        if self.restricted_scope {
            return Err(diagnostic(
                "execution_scope",
                "A scope-restricted artifact requires current execution authorization",
            ));
        }
        Ok(())
    }
    pub async fn execute(
        &self,
        engine: &Engine,
        options: QueryOptions,
    ) -> Result<QueryExecution, CompileDiagnostic> {
        self.check_snapshot(engine)?;
        self.check_execution_scope(None)?;
        engine
            .execute_parameters(self.sql.statement(), self.sql.values(), options)
            .await
            .map_err(lower::backend_error)
    }
    pub async fn execute_authorized(
        &self,
        engine: &Engine,
        allowed_relations: &BTreeSet<String>,
        options: QueryOptions,
    ) -> Result<QueryExecution, CompileDiagnostic> {
        self.check_snapshot(engine)?;
        self.check_execution_scope(Some(allowed_relations))?;
        engine
            .execute_parameters(self.sql.statement(), self.sql.values(), options)
            .await
            .map_err(lower::backend_error)
    }
    pub async fn execute_read(
        &self,
        engine: &Engine,
        options: semantic_engine::ReadOptions,
    ) -> Result<semantic_engine::ReadExecution, CompileDiagnostic> {
        self.check_snapshot(engine)?;
        self.check_execution_scope(None)?;
        engine
            .execute_read(self.sql.statement(), self.sql.values(), options)
            .await
            .map_err(lower::backend_error)
    }
    pub async fn execute_read_authorized(
        &self,
        engine: &Engine,
        allowed_relations: &BTreeSet<String>,
        options: semantic_engine::ReadOptions,
    ) -> Result<semantic_engine::ReadExecution, CompileDiagnostic> {
        self.check_snapshot(engine)?;
        self.check_execution_scope(Some(allowed_relations))?;
        engine
            .execute_read(self.sql.statement(), self.sql.values(), options)
            .await
            .map_err(lower::backend_error)
    }
    pub async fn plan_direct(&self, engine: &Engine) -> Result<DataFrame, CompileDiagnostic> {
        self.check_snapshot(engine)?;
        self.check_execution_scope(None)?;
        self.plan_direct_unchecked(engine).await
    }
    pub async fn plan_direct_authorized(
        &self,
        engine: &Engine,
        allowed_relations: &BTreeSet<String>,
    ) -> Result<DataFrame, CompileDiagnostic> {
        self.check_snapshot(engine)?;
        self.check_execution_scope(Some(allowed_relations))?;
        self.plan_direct_unchecked(engine).await
    }
    async fn plan_direct_unchecked(&self, engine: &Engine) -> Result<DataFrame, CompileDiagnostic> {
        let mut frames: Vec<DataFrame> = Vec::new();
        for node in &self.nodes {
            let frame = match &node.operation {
                CheckedOperation::Calculate { calculation } => {
                    calculate::plan_calculation(calculation, frames[calculation.input].clone())?
                }
                CheckedOperation::Conditional { conditional } => {
                    calculate::plan_conditional(conditional, frames[conditional.input].clone())?
                }
                CheckedOperation::Cast { cast } => {
                    calculate::plan_cast(cast, frames[cast.input].clone())?
                }
                CheckedOperation::NullTest { test } => {
                    calculate::plan_null_test(test, frames[test.input].clone())?
                }
                CheckedOperation::CompareSlots { comparison } => {
                    calculate::plan_slot_comparison(comparison, frames[comparison.input].clone())?
                }
                CheckedOperation::Filter { filter } => {
                    calculate::plan_filter(filter, frames[filter.input].clone())?
                }
                CheckedOperation::Compose { composition } => compose::plan(
                    composition,
                    frames[composition.left].clone(),
                    frames[composition.right].clone(),
                )?,
                CheckedOperation::Rows { plan, .. } => plan.plan_direct(engine).await?,
                CheckedOperation::Set {
                    left,
                    right,
                    operator,
                    duplicates,
                    columns,
                } => {
                    let l = project_set(
                        frames[*left].clone(),
                        &self.nodes[*left],
                        columns,
                        Side::Left,
                    )?;
                    let r = project_set(
                        frames[*right].clone(),
                        &self.nodes[*right],
                        columns,
                        Side::Right,
                    )?;
                    match (operator, duplicates) {
                        (SetOperator::Union, Duplicates::All) => l.union(r),
                        (SetOperator::Union, Duplicates::Distinct) => l.union_distinct(r),
                        (SetOperator::Intersect, Duplicates::All) => {
                            let frame = set::bag(l, r, columns, *operator)?;
                            verify_root_output_contract(&node.slots, frame.schema().as_arrow())?;
                            frames.push(frame);
                            continue;
                        }
                        (SetOperator::Intersect, Duplicates::Distinct) => l.intersect_distinct(r),
                        (SetOperator::Except, Duplicates::All) => {
                            let frame = set::bag(l, r, columns, *operator)?;
                            verify_root_output_contract(&node.slots, frame.schema().as_arrow())?;
                            frames.push(frame);
                            continue;
                        }
                        (SetOperator::Except, Duplicates::Distinct) => l.except_distinct(r),
                    }
                    .map_err(lower::backend_error)?
                }
            };
            verify_root_output_contract(&node.slots, frame.schema().as_arrow())?;
            frames.push(frame);
        }
        let mut root = frames[self.root].clone();
        if !self.proposal.ordering.is_empty() {
            root = root
                .sort(
                    self.proposal
                        .ordering
                        .iter()
                        .map(|order| {
                            let slot = slot(&self.nodes[self.root], &order.slot)
                                .expect("checked ordering");
                            col(slot.field.name()).sort(
                                order.direction == Direction::Asc,
                                order.nulls == NullOrder::First,
                            )
                        })
                        .collect(),
                )
                .map_err(lower::backend_error)?;
        }
        if let Some(limit) = self.proposal.limit {
            root = root
                .limit(0, Some(limit as usize))
                .map_err(lower::backend_error)?;
        }
        Ok(root)
    }
}
fn col(name: &str) -> Expr {
    Expr::Column(Column::new_unqualified(name))
}
fn slot<'a>(node: &'a CheckedNode, id: &str) -> Result<&'a Slot, CompileDiagnostic> {
    node.slots.iter().find(|s| s.id == id).ok_or_else(|| {
        diagnostic(
            "graph_slot",
            "Graph edge references an unavailable output slot",
        )
    })
}
fn project_set(
    frame: DataFrame,
    node: &CheckedNode,
    columns: &[SetColumn],
    side: Side,
) -> Result<DataFrame, CompileDiagnostic> {
    frame
        .select(
            columns
                .iter()
                .map(|c| {
                    let id = if side == Side::Left {
                        &c.left
                    } else {
                        &c.right
                    };
                    col(slot(node, id).expect("checked slot").field.name()).alias(&c.alias)
                })
                .collect::<Vec<_>>(),
        )
        .map_err(lower::backend_error)
}

#[tracing::instrument(name = "semantic.compile_graph", skip_all)]
pub async fn compile_graph(
    engine: &Engine,
    query: GraphQuery,
    options: CompileOptions,
) -> TypedCompilation {
    let options = options.start();
    let mut request_guard = options.metrics.as_ref().map(|metrics| metrics.begin());
    let start = Instant::now();
    let mut record = CompilationRecord::new("structured_graph_v1");
    let result = {
        let work = build(engine, query, &options, &mut record);
        tokio::select! {
            biased;
            _ = options.cancellation.cancelled() => Err(diagnostic("cancelled", "Compilation was cancelled")),
            result = tokio::time::timeout(options.timeout, work) => result.unwrap_or_else(|_| Err(diagnostic("deadline", "Compilation deadline exhausted"))),
        }
    };
    let completed = finish(
        result.map(|query| TypedOutcome::CompiledGraph {
            query: Box::new(query),
        }),
        record,
        start,
        options.metrics.as_deref(),
    );
    if let Some(guard) = &mut request_guard {
        guard.complete();
    }
    completed
}
pub(super) fn preflight_graph(
    query: &GraphQuery,
    options: &CompileOptions,
) -> Result<(), CompileDiagnostic> {
    options.check()?;
    if query.version != 1 {
        return Err(diagnostic(
            "unsupported_version",
            "Expected graph query version 1",
        ));
    }
    if !query.unresolved.is_empty() {
        return Err(diagnostic(
            "unresolved_terms",
            "Resolve every graph business choice before binding",
        ));
    }
    if options.request_evidence.is_some() {
        return Err(diagnostic(
            "graph_evidence",
            "Graph requirements require their own evidence contract; row evidence cannot cover a graph",
        ));
    }
    if query.nodes.is_empty()
        || query.nodes.len() > options.max_nodes
        || query.ordering.len() > options.max_nodes
    {
        return Err(diagnostic(
            "work_limit",
            "Graph size exceeds its work budget",
        ));
    }
    // Graph evidence is validated against the whole graph, never against a leaf.
    let mut leaf_options = options.clone();
    leaf_options.graph_request_evidence = None;
    // Preflight expression depth before bounded serialization recurses into leaves.
    for node in &query.nodes {
        if let GraphOperation::Rows { query } = &node.operation {
            preflight(query, &leaf_options)?;
        }
    }
    bounded_json(
        &(query, &options.graph_request_evidence),
        options.max_input_bytes,
    )
    .map_err(|_| {
        diagnostic(
            "input_limit",
            "Graph proposal and evidence exceed their byte budget",
        )
    })?;
    if let Some(evidence) = &options.graph_request_evidence {
        intent::validate(query, evidence, options)?;
    }
    Ok(())
}
pub(super) async fn build(
    engine: &Engine,
    query: GraphQuery,
    options: &CompileOptions,
    record: &mut CompilationRecord,
) -> Result<CompiledGraph, CompileDiagnostic> {
    record.bound_digest = None;
    record.request_digest = None;
    record.request_spans_validated = false;
    record.relational_digest = None;
    record.artifact_digest = None;
    record.definition_refs.clear();
    record.execution_obligations.clear();
    record.requirement_dispositions.clear();
    preflight_graph(&query, options)?;
    if let Some(evidence) = &options.graph_request_evidence {
        record.request_digest = Some(semantic_catalog::canonical_digest(&serde_json::json!(
            evidence.original_request
        )));
        record.request_spans_validated = true;
    }
    let snapshot = engine.catalog().snapshot();
    record.snapshot_id = Some(snapshot.id().into());
    record.work.graph_nodes_visited = record
        .work
        .graph_nodes_visited
        .saturating_add(query.nodes.len());
    record.work.graph_edges_visited = record.work.graph_edges_visited.saturating_add(
        query
            .nodes
            .iter()
            .map(|node| match &node.operation {
                GraphOperation::Rows { .. } => 0,
                GraphOperation::Calculate { .. }
                | GraphOperation::Conditional { .. }
                | GraphOperation::Cast { .. }
                | GraphOperation::NullTest { .. }
                | GraphOperation::CompareSlots { .. }
                | GraphOperation::Filter { .. } => 1,
                GraphOperation::Set { .. } | GraphOperation::Compose { .. } => 2,
            })
            .sum::<usize>(),
    );
    record.work.graph_outputs_checked = record.work.graph_outputs_checked.saturating_add(
        query
            .nodes
            .iter()
            .map(|node| match &node.operation {
                GraphOperation::Rows { .. } => 0,
                GraphOperation::Set { columns, .. } => columns.len(),
                GraphOperation::Compose { keys, outputs, .. } => keys.len() + outputs.len(),
                GraphOperation::Calculate {
                    passthrough,
                    ratios,
                    ..
                } => passthrough.len() + ratios.len(),
                GraphOperation::Conditional {
                    passthrough,
                    outputs,
                    ..
                } => passthrough.len() + outputs.len(),
                GraphOperation::Cast {
                    passthrough, casts, ..
                } => passthrough.len() + casts.len(),
                GraphOperation::NullTest {
                    passthrough, tests, ..
                } => passthrough.len() + tests.len(),
                GraphOperation::CompareSlots {
                    passthrough,
                    comparisons,
                    ..
                } => passthrough.len() + comparisons.len(),
                GraphOperation::Filter { .. } => 0,
            })
            .sum(),
    );
    let order = topological(&query, options)?;
    let mut indexes = BTreeMap::new();
    let mut nodes: Vec<CheckedNode> = Vec::new();
    let start = Instant::now();
    for position in order {
        options.check()?;
        let node = &query.nodes[position];
        let (operation, slots, group_keys) = match &node.operation {
            GraphOperation::Rows { query } => {
                let bound = bind::bind(&snapshot, query, options, &mut record.work)?;
                let plan = lower::lower(&bound)?;
                let (slots, keys) = leaf_slots(&bound);
                let mut leaf_record = CompilationRecord::new("graph_leaf");
                record_bound(
                    &bound,
                    &mut leaf_record,
                    RequirementScope::GraphLeaf(&node.id),
                );
                record.definition_refs.extend(leaf_record.definition_refs);
                record
                    .execution_obligations
                    .extend(leaf_record.execution_obligations);
                record
                    .requirement_dispositions
                    .extend(leaf_record.requirement_dispositions);
                (
                    CheckedOperation::Rows {
                        bound: Box::new(bound),
                        plan,
                    },
                    slots,
                    keys,
                )
            }
            GraphOperation::Set {
                left,
                right,
                operator,
                duplicates,
                columns,
            } => {
                let l = indexes[left];
                let r = indexes[right];
                if columns.is_empty() || columns.len() > options.max_nodes {
                    return Err(diagnostic(
                        "graph_columns",
                        "Set alignment requires bounded nonempty columns",
                    ));
                }
                let mut ids = BTreeSet::new();
                let mut aliases = BTreeSet::new();
                let mut slots = Vec::new();
                for c in columns {
                    options.check()?;
                    unique_output(&c.id, &c.alias, &mut ids, &mut aliases)?;
                    let a = slot(&nodes[l], &c.left)?;
                    let b = slot(&nodes[r], &c.right)?;
                    if a.field.data_type() != b.field.data_type()
                        || !exact_type(a.field.data_type())
                    {
                        return Err(diagnostic(
                            "set_type",
                            "Set columns require matching exact scalar types; implicit casts are disabled",
                        ));
                    }
                    slots.push(Slot {
                        id: c.id.clone(),
                        field: Field::new(
                            &c.alias,
                            a.field.data_type().clone(),
                            a.field.is_nullable() || b.field.is_nullable(),
                        ),
                        origin: (a.origin == b.origin).then(|| a.origin.clone()).flatten(),
                        meaning: aligned_meaning(&a.meaning, &b.meaning)?,
                    });
                }
                (
                    CheckedOperation::Set {
                        left: l,
                        right: r,
                        operator: *operator,
                        duplicates: *duplicates,
                        columns: columns.clone(),
                    },
                    slots,
                    None,
                )
            }
            GraphOperation::Compose { left, right, .. } => {
                let (composition, slots, grain) = compose::bind(
                    &snapshot,
                    &node.operation,
                    indexes[left],
                    indexes[right],
                    &nodes,
                    options,
                )?;
                if let Some(definition) = &composition.definition {
                    record.definition_refs.push(definition.clone());
                }
                (
                    CheckedOperation::Compose {
                        composition: Box::new(composition),
                    },
                    slots,
                    grain,
                )
            }
            GraphOperation::Calculate {
                input,
                passthrough,
                ratios,
            } => {
                let source = indexes[input];
                let (calculation, slots, grain) = calculate::bind_calculation(
                    source,
                    &nodes[source],
                    passthrough,
                    ratios,
                    options,
                )?;
                (
                    CheckedOperation::Calculate {
                        calculation: Box::new(calculation),
                    },
                    slots,
                    grain,
                )
            }
            GraphOperation::Conditional {
                input,
                passthrough,
                outputs,
            } => {
                let source = indexes[input];
                let (conditional, slots, grain) = calculate::bind_conditional(
                    source,
                    &nodes[source],
                    passthrough,
                    outputs,
                    options,
                )?;
                (
                    CheckedOperation::Conditional {
                        conditional: Box::new(conditional),
                    },
                    slots,
                    grain,
                )
            }
            GraphOperation::Cast {
                input,
                passthrough,
                casts,
            } => {
                let source = indexes[input];
                let (cast, slots, grain) =
                    calculate::bind_cast(source, &nodes[source], passthrough, casts, options)?;
                (
                    CheckedOperation::Cast {
                        cast: Box::new(cast),
                    },
                    slots,
                    grain,
                )
            }
            GraphOperation::NullTest {
                input,
                passthrough,
                tests,
            } => {
                let source = indexes[input];
                let (test, slots, grain) =
                    calculate::bind_null_test(source, &nodes[source], passthrough, tests, options)?;
                (
                    CheckedOperation::NullTest {
                        test: Box::new(test),
                    },
                    slots,
                    grain,
                )
            }
            GraphOperation::CompareSlots {
                input,
                passthrough,
                comparisons,
            } => {
                let source = indexes[input];
                let (comparison, slots, grain) = calculate::bind_slot_comparison(
                    source,
                    &nodes[source],
                    passthrough,
                    comparisons,
                    options,
                )?;
                (
                    CheckedOperation::CompareSlots {
                        comparison: Box::new(comparison),
                    },
                    slots,
                    grain,
                )
            }
            GraphOperation::Filter { input, predicate } => {
                let source = indexes[input];
                let filter = calculate::bind_filter(source, &nodes[source], predicate, options)?;
                (
                    CheckedOperation::Filter {
                        filter: Box::new(filter),
                    },
                    nodes[source].slots.clone(),
                    nodes[source].group_keys.clone(),
                )
            }
        };
        indexes.insert(node.id.clone(), nodes.len());
        nodes.push(CheckedNode {
            id: node.id.clone(),
            operation,
            slots,
            group_keys,
        });
    }
    let root = indexes[&query.root];
    for order in &query.ordering {
        if !exact_type(slot(&nodes[root], &order.slot)?.field.data_type()) {
            return Err(diagnostic(
                "graph_order",
                "Graph ordering requires an exact scalar output",
            ));
        }
    }
    for target in intent::requirements(&query, options)?.into_keys() {
        let rule = match &target {
            GraphRequirementRef::Leaf { .. } => continue,
            GraphRequirementRef::Node { .. } => "graph.checked_operator.v1",
            GraphRequirementRef::Output { .. } => "graph.output_slot.v1",
            GraphRequirementRef::Order { .. } => "graph.final_order.v1",
            GraphRequirementRef::Limit => "graph.final_limit.v1",
        };
        record
            .requirement_dispositions
            .push(RequirementDisposition {
                requirement_id: safe_requirement_id(&target),
                rule,
                result: "lowered_and_verified",
            });
    }
    record
        .definition_refs
        .sort_by(|a, b| (&a.id, &a.revision).cmp(&(&b.id, &b.revision)));
    record.definition_refs.dedup();
    record.bound_digest = Some(semantic_catalog::canonical_digest(
        &serde_json::to_value(&nodes).expect("bound graph serializes"),
    ));
    record.relational_digest = Some(semantic_catalog::canonical_digest(&serde_json::json!({
        "nodes": &nodes,
        "root": root,
        "ordering": &query.ordering,
        "limit": query.limit,
    })));
    record.stage("graph_bind_lower", start, &Ok::<_, CompileDiagnostic>(()));
    let sql = emit(&nodes, root, &query, &snapshot, options)?;
    if sql.statement().len() > options.max_sql_bytes {
        return Err(diagnostic("sql_limit", "Graph SQL exceeds its byte budget"));
    }
    let required_relations = nodes
        .iter()
        .filter_map(|node| match &node.operation {
            CheckedOperation::Rows { bound, .. } => Some(bound_required_relations(bound)),
            _ => None,
        })
        .flatten()
        .collect();
    let result = CompiledGraph {
        proposal: query,
        snapshot_id: snapshot.id().into(),
        nodes,
        root,
        execution_profile_revision: MVP_EXECUTION_PROFILE_REVISION,
        request_context: options.request_context.clone(),
        request_evidence: options.graph_request_evidence.clone(),
        required_relations,
        restricted_scope: options.allowed_relations.is_some(),
        sql,
    };
    let start = Instant::now();
    let direct = result.plan_direct_unchecked(engine).await?;
    let emitted = engine
        .plan_generated_sql(result.sql.statement())
        .await
        .map_err(lower::backend_error)?;
    verify_root_output_contract(&result.nodes[root].slots, direct.schema().as_arrow())?;
    // The SQL planner may conservatively infer nullable parameterized CASE
    // outputs while the direct planner proves both literal branches non-null.
    // Both schemas must satisfy the same bound portable slot contract.
    verify_root_output_contract(&result.nodes[root].slots, emitted.schema().as_arrow())?;
    record.stage("graph_backend", start, &Ok::<_, CompileDiagnostic>(()));
    for disposition in &mut record.requirement_dispositions {
        disposition.result = "lowered_and_verified";
    }
    record.artifact_digest = Some(semantic_catalog::canonical_digest(
        &serde_json::json!({"pipeline":PIPELINE_REVISION,"artifact":result}),
    ));
    options.check()?;
    Ok(result)
}
fn verify_root_output_contract(
    expected: &[Slot],
    actual: &datafusion::arrow::datatypes::Schema,
) -> Result<(), CompileDiagnostic> {
    if actual.fields().len() != expected.len()
        || expected.iter().zip(actual.fields()).any(|(slot, field)| {
            slot.field.name() != field.name()
                || slot.field.data_type() != field.data_type()
                // A nullable backend field cannot satisfy a non-null slot
                // promise. A nullable slot over a non-null field is safe.
                || (!slot.field.is_nullable() && field.is_nullable())
        })
    {
        return Err(diagnostic(
            "output_contract",
            "Graph output differs from its bound slot contract",
        ));
    }
    Ok(())
}
fn unique_output<'a>(
    id: &'a str,
    alias: &'a str,
    ids: &mut BTreeSet<&'a str>,
    aliases: &mut BTreeSet<&'a str>,
) -> Result<(), CompileDiagnostic> {
    if id.trim().is_empty() || alias.trim().is_empty() || !ids.insert(id) || !aliases.insert(alias)
    {
        return Err(diagnostic(
            "graph_output",
            "Graph outputs require unique nonempty IDs and aliases",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod output_contract_tests {
    use super::*;
    use datafusion::arrow::datatypes::{Field as ArrowField, Schema};

    #[test]
    fn root_contract_rejects_unsafe_nonnull_promise() {
        let mut slot = Slot {
            id: "value".into(),
            field: Field::new("value", DataType::Int64, true),
            origin: None,
            meaning: SlotMeaning::default(),
        };
        let nullable = Schema::new(vec![ArrowField::new("value", DataType::Int64, true)]);
        verify_root_output_contract(&[slot.clone()], &nullable).unwrap();

        slot.field = Field::new("value", DataType::Int64, false);
        assert_eq!(
            verify_root_output_contract(&[slot.clone()], &nullable)
                .unwrap_err()
                .code,
            "output_contract"
        );

        let nonnull = Schema::new(vec![ArrowField::new("value", DataType::Int64, false)]);
        verify_root_output_contract(&[slot.clone()], &nonnull).unwrap();
        slot.field = Field::new("value", DataType::Int64, true);
        verify_root_output_contract(&[slot.clone()], &nonnull).unwrap();

        slot.field = Field::new("value", DataType::Utf8, true);
        assert_eq!(
            verify_root_output_contract(&[slot], &nonnull)
                .unwrap_err()
                .code,
            "output_contract"
        );
    }

    #[test]
    fn intermediate_mismatch_fails_even_if_final_root_contract_is_valid() {
        let intermediate_schema =
            Schema::new(vec![ArrowField::new("middle", DataType::Utf8, true)]);
        let root_schema = Schema::new(vec![ArrowField::new("result", DataType::Int64, false)]);
        let root = Slot {
            id: "root".into(),
            field: Field::new("result", DataType::Int64, false),
            origin: None,
            meaning: SlotMeaning::default(),
        };
        verify_root_output_contract(&[root], &root_schema).unwrap();

        let intermediate = Slot {
            id: "middle".into(),
            field: Field::new("middle", DataType::Utf8, false),
            origin: None,
            meaning: SlotMeaning::default(),
        };
        assert_eq!(
            verify_root_output_contract(&[intermediate], &intermediate_schema)
                .unwrap_err()
                .code,
            "output_contract"
        );
    }
}
fn exact_type(ty: &DataType) -> bool {
    matches!(
        ty,
        DataType::Boolean
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt64
            | DataType::Utf8
            | DataType::Decimal128(_, _)
            | DataType::Date32
            | DataType::Timestamp(_, _)
    )
}
fn aligned_meaning(
    left: &SlotMeaning,
    right: &SlotMeaning,
) -> Result<SlotMeaning, CompileDiagnostic> {
    let unit = match (&left.unit, &right.unit) {
        (FactResolution::Conflicting { .. }, _)
        | (_, FactResolution::Conflicting { .. })
        | (
            FactResolution::Known {
                value: Presence::Null,
                ..
            },
            _,
        )
        | (
            _,
            FactResolution::Known {
                value: Presence::Null,
                ..
            },
        ) => {
            return Err(diagnostic(
                "set_unit",
                "Set alignment has conflicting or null authored units",
            ));
        }
        (
            FactResolution::Known {
                value: Presence::Value(left),
                contributors: left_facts,
            },
            FactResolution::Known {
                value: Presence::Value(right),
                contributors: right_facts,
            },
        ) if left == right => {
            let mut contributors = left_facts.clone();
            contributors.extend(right_facts.iter().cloned());
            contributors.sort_by(|a, b| (&a.scope, &a.id).cmp(&(&b.scope, &b.id)));
            contributors.dedup();
            FactResolution::Known {
                value: Presence::Value(left.clone()),
                contributors,
            }
        }
        (
            FactResolution::Known {
                value: Presence::Value(_),
                ..
            },
            FactResolution::Known {
                value: Presence::Value(_),
                ..
            },
        ) => {
            return Err(diagnostic(
                "set_unit",
                "Set alignment requires identical authored units",
            ));
        }
        _ => FactResolution::Unknown,
    };
    Ok(SlotMeaning {
        unit,
        source_grain: if left.source_grain == right.source_grain {
            left.source_grain.clone()
        } else {
            FactResolution::Unknown
        },
        entity: if left.entity == right.entity {
            left.entity.clone()
        } else {
            FactResolution::Unknown
        },
    })
}
fn leaf_slots(bound: &BoundQuery) -> (Vec<Slot>, Option<BTreeSet<String>>) {
    use bind::BoundOperation as O;
    let mut slots = Vec::new();
    let mut groups = BTreeSet::new();
    let mut aggregate = false;
    for r in &bound.requirements {
        let (field, alias, origin) = match &r.operation {
            O::Project { field, alias } => (
                field,
                alias,
                Some((bound.input.id.clone(), field.field.name().clone())),
            ),
            O::Group {
                field,
                alias,
                output,
            } => {
                aggregate = true;
                groups.insert(r.id.clone());
                (
                    output,
                    alias,
                    Some((bound.input.id.clone(), field.field.name().clone())),
                )
            }
            O::Aggregate { output, alias, .. } | O::Ratio { output, alias, .. } => {
                aggregate = true;
                (output, alias, None)
            }
            O::Window { output, alias, .. } => (output, alias, None),
            O::Lookup {
                lookup,
                alias,
                group_output,
            } => {
                if group_output.is_some() {
                    aggregate = true;
                    groups.insert(r.id.clone());
                }
                (
                    group_output.as_ref().unwrap_or(&lookup.output),
                    alias,
                    Some((
                        lookup.relationship.right.id.clone(),
                        lookup.value.field.name().clone(),
                    )),
                )
            }
            _ => continue,
        };
        slots.push(Slot {
            id: r.id.clone(),
            field: Field::new(
                alias,
                field.field.data_type().clone(),
                field.field.is_nullable(),
            ),
            origin,
            meaning: bound
                .output_meanings
                .get(&r.id)
                .cloned()
                .unwrap_or_default(),
        });
    }
    (slots, aggregate.then_some(groups))
}
fn topological(
    query: &GraphQuery,
    options: &CompileOptions,
) -> Result<Vec<usize>, CompileDiagnostic> {
    let mut positions = BTreeMap::new();
    for (i, n) in query.nodes.iter().enumerate() {
        if n.id.trim().is_empty()
            || n.source_text.trim().is_empty()
            || positions.insert(n.id.as_str(), i).is_some()
        {
            return Err(diagnostic(
                "graph_identity",
                "Graph nodes require unique IDs and source text",
            ));
        }
    }
    if !positions.contains_key(query.root.as_str()) {
        return Err(diagnostic("graph_root", "Graph root is missing"));
    }
    let mut pending = vec![0; query.nodes.len()];
    let mut reverse = vec![Vec::new(); query.nodes.len()];
    let mut dependencies = vec![Vec::new(); query.nodes.len()];
    for (i, n) in query.nodes.iter().enumerate() {
        let children = match &n.operation {
            GraphOperation::Rows { .. } => vec![],
            GraphOperation::Calculate { input, .. }
            | GraphOperation::Conditional { input, .. }
            | GraphOperation::Cast { input, .. }
            | GraphOperation::NullTest { input, .. }
            | GraphOperation::CompareSlots { input, .. }
            | GraphOperation::Filter { input, .. } => {
                vec![input]
            }
            GraphOperation::Set { left, right, .. }
            | GraphOperation::Compose { left, right, .. } => vec![left, right],
        };
        for child in children {
            let j = *positions
                .get(child.as_str())
                .ok_or_else(|| diagnostic("graph_edge", "Graph dependency is missing"))?;
            dependencies[i].push(j);
            reverse[j].push(i);
            pending[i] += 1;
        }
    }
    let mut reachable = BTreeSet::new();
    let mut stack = vec![positions[query.root.as_str()]];
    while let Some(i) = stack.pop() {
        options.check()?;
        if reachable.insert(i) {
            stack.extend(&dependencies[i]);
        }
    }
    if reachable.len() != query.nodes.len() {
        return Err(diagnostic(
            "graph_coverage",
            "Every graph requirement must contribute to the root",
        ));
    }
    let mut ready: BTreeSet<_> = pending
        .iter()
        .enumerate()
        .filter_map(|(i, n)| (*n == 0).then_some(i))
        .collect();
    let mut order = Vec::new();
    let mut expanded = vec![1usize; query.nodes.len()];
    let mut depths = vec![1; query.nodes.len()];
    while let Some(i) = ready.pop_first() {
        options.check()?;
        order.push(i);
        for &j in &reverse[i] {
            expanded[j] = expanded[j].saturating_add(expanded[i]);
            if expanded[j] > options.max_nodes {
                return Err(diagnostic(
                    "graph_expansion_limit",
                    "Backend expansion of shared graph nodes exceeds the work budget",
                ));
            }
            depths[j] = depths[j].max(depths[i] + 1);
            if depths[j] > options.max_depth {
                return Err(diagnostic("work_limit", "Graph depth budget exhausted"));
            }
            pending[j] -= 1;
            if pending[j] == 0 {
                ready.insert(j);
            }
        }
    }
    if order.len() != query.nodes.len() {
        return Err(diagnostic(
            "graph_cycle",
            "Recursive semantic query graphs are unsupported",
        ));
    }
    Ok(order)
}
fn ident(s: &str) -> ast::Ident {
    ast::Ident::with_quote('"', s)
}
fn parsed(sql: &str) -> Box<ast::Query> {
    let ast::Statement::Query(q) = Parser::parse_sql(&GenericDialect {}, sql)
        .expect("compiler-owned SQL")
        .remove(0)
    else {
        unreachable!()
    };
    q
}
fn table_query(index: usize, names: &[String]) -> Box<ast::Query> {
    let mut q = parsed("SELECT * FROM t AS src");
    let ast::SetExpr::Select(s) = q.body.as_mut() else {
        unreachable!()
    };
    let ast::TableFactor::Table { name, .. } = &mut s.from[0].relation else {
        unreachable!()
    };
    *name = ast::ObjectName::from(vec![ident(&names[index])]);
    q
}
fn set_select(
    index: usize,
    node: &CheckedNode,
    columns: &[SetColumn],
    side: Side,
    names: &[String],
) -> Box<ast::Query> {
    let mut q = table_query(index, names);
    let ast::SetExpr::Select(s) = q.body.as_mut() else {
        unreachable!()
    };
    s.projection = columns
        .iter()
        .map(|c| {
            let id = if side == Side::Left {
                &c.left
            } else {
                &c.right
            };
            ast::SelectItem::ExprWithAlias {
                expr: ast::Expr::CompoundIdentifier(vec![
                    ident("src"),
                    ident(slot(node, id).expect("checked slot").field.name()),
                ]),
                alias: ident(&c.alias),
            }
        })
        .collect();
    q
}
fn emit(
    nodes: &[CheckedNode],
    root: usize,
    proposal: &GraphQuery,
    snapshot: &CatalogSnapshot,
    options: &CompileOptions,
) -> Result<SqlArtifact, CompileDiagnostic> {
    let mut names = Vec::new();
    for i in 0..nodes.len() {
        let mut name = format!("__semantic_graph_{i}");
        let mut collisions = 0;
        while snapshot.relation(&name).is_some() {
            options.check()?;
            collisions += 1;
            if collisions > options.max_nodes {
                return Err(diagnostic(
                    "work_limit",
                    "SQL scope allocation exhausted its budget",
                ));
            }
            name.push('_');
        }
        names.push(name);
    }
    let mut query = table_query(root, &names);
    let mut template = parsed("WITH x AS (SELECT 1) SELECT * FROM x");
    let mut with = template.with.take().unwrap();
    let cte = with.cte_tables.remove(0);
    let mut parameters = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let q = match &node.operation {
            CheckedOperation::Calculate { calculation } => {
                calculate::emit_calculation(calculation, &names)
            }
            CheckedOperation::Conditional { conditional } => {
                calculate::emit_conditional(conditional, &names, &mut parameters)
            }
            CheckedOperation::Cast { cast } => calculate::emit_cast(cast, &names),
            CheckedOperation::NullTest { test } => calculate::emit_null_test(test, &names),
            CheckedOperation::CompareSlots { comparison } => {
                calculate::emit_slot_comparison(comparison, &names)
            }
            CheckedOperation::Filter { filter } => {
                calculate::emit_filter(filter, &names, &mut parameters)
            }
            CheckedOperation::Compose { composition } => compose::emit(composition, &names),
            CheckedOperation::Rows { plan, .. } => {
                let sql = plan.emit(snapshot.id());
                let mut q = parsed(sql.statement());
                let offset = parameters.len();
                let _ = ast::visit_expressions_mut(q.as_mut(), |expr| {
                    if let ast::Expr::Value(value) = expr
                        && let ast::Value::Placeholder(id) = &mut value.value
                    {
                        let n = id[1..].parse::<usize>().expect("generated parameter");
                        *id = format!("${}", offset + n);
                    }
                    std::ops::ControlFlow::<()>::Continue(())
                });
                parameters.extend_from_slice(sql.parameters());
                q
            }
            CheckedOperation::Set {
                left,
                right,
                operator,
                duplicates,
                columns,
            } => {
                let l = set_select(*left, &nodes[*left], columns, Side::Left, &names);
                let r = set_select(*right, &nodes[*right], columns, Side::Right, &names);
                if *duplicates == Duplicates::All && *operator != SetOperator::Union {
                    let mut entry = cte.clone();
                    entry.alias.name = ident(&names[i]);
                    entry.query = set::sql_bag(l, r, columns, *operator);
                    with.cte_tables.push(entry);
                    continue;
                }
                let mut q = parsed("SELECT 1");
                q.body = Box::new(ast::SetExpr::SetOperation {
                    op: match operator {
                        SetOperator::Union => ast::SetOperator::Union,
                        SetOperator::Intersect => ast::SetOperator::Intersect,
                        SetOperator::Except => ast::SetOperator::Except,
                    },
                    set_quantifier: if *duplicates == Duplicates::All {
                        ast::SetQuantifier::All
                    } else {
                        ast::SetQuantifier::Distinct
                    },
                    left: Box::new(ast::SetExpr::Query(l)),
                    right: Box::new(ast::SetExpr::Query(r)),
                });
                q
            }
        };
        let mut entry = cte.clone();
        entry.alias.name = ident(&names[i]);
        entry.query = q;
        with.cte_tables.push(entry);
    }
    query.with = Some(with);
    if !proposal.ordering.is_empty() {
        let mut template = parsed("SELECT 1 ORDER BY x ASC NULLS LAST");
        let mut order = template.order_by.take().unwrap();
        let ast::OrderByKind::Expressions(keys) = &mut order.kind else {
            unreachable!()
        };
        let key = keys.remove(0);
        for o in &proposal.ordering {
            let mut k = key.clone();
            k.expr = ast::Expr::CompoundIdentifier(vec![
                ident("src"),
                ident(slot(&nodes[root], &o.slot)?.field.name()),
            ]);
            k.options.asc = Some(o.direction == Direction::Asc);
            k.options.nulls_first = Some(o.nulls == NullOrder::First);
            keys.push(k);
        }
        query.order_by = Some(order);
    }
    if let Some(count) = proposal.limit {
        query.limit_clause = parsed(&format!("SELECT 1 LIMIT {count}")).limit_clause;
    }
    Ok(SqlArtifact::generated(
        snapshot.id(),
        query.to_string(),
        parameters,
    ))
}
