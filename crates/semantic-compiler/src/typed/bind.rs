use std::collections::BTreeSet;

use semantic_catalog::{CatalogSnapshot, DataType, Field, ObjectRef, SnapshotRelation};
use semantic_plan::typed::*;
use serde::Serialize;

use super::{CompileDiagnostic, CompileOptions, Work, diagnostic};

/// Constructible only by validation; serialized copies must re-enter as proposals.
#[derive(Debug, Clone, Serialize)]
pub struct BoundQuery {
    pub(super) version: u32,
    pub(super) acceptance_profile: &'static str,
    pub(super) snapshot_id: String,
    pub(super) input: ObjectRef,
    pub(super) instance: String,
    pub(super) requirements: Vec<BoundRequirement>,
    pub(super) definitions: Vec<ObjectRef>,
}
impl BoundQuery {
    pub fn snapshot_id(&self) -> &str {
        &self.snapshot_id
    }
    pub fn input(&self) -> &ObjectRef {
        &self.input
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundRequirement {
    pub id: String,
    pub operation: BoundOperation,
}
#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundField {
    pub instance: String,
    pub field: Field,
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum BoundOperation {
    FilterOutput {
        stage: OutputFilterStage,
        predicate: BoundPredicate,
    },
    Lookup {
        lookup: Box<BoundLookup>,
        alias: String,
        group_output: Option<BoundField>,
    },
    CalendarFilter {
        predicate: BoundPredicate,
        resolution: super::TemporalResolution,
    },
    Window {
        window: BoundWindow,
        alias: String,
        output: BoundField,
    },
    Ratio {
        numerator: Box<BoundRatioInput>,
        denominator: Box<BoundRatioInput>,
        zero: ZeroDivision,
        alias: String,
        output: BoundField,
    },
    Related {
        relationship: BoundRelationship,
    },
    Group {
        field: BoundField,
        alias: String,
        output: BoundField,
    },
    Aggregate {
        function: AggregateFunction,
        field: Option<BoundField>,
        distinct: bool,
        alias: String,
        output: BoundField,
        filter: Option<BoundPredicate>,
    },
    Project {
        field: BoundField,
        alias: String,
    },
    Filter {
        predicate: BoundPredicate,
    },
    Order {
        field: BoundField,
        direction: Direction,
        nulls: NullOrder,
    },
    Limit {
        count: u32,
    },
}
#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundLookup {
    pub relationship: BoundRelationship,
    pub value: BoundField,
    pub output: BoundField,
    pub missing: MissingMatch,
    pub obligation: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundWindow {
    pub function: WindowFunction,
    pub input: Option<BoundField>,
    pub partition_by: Vec<BoundField>,
    pub order_by: Vec<(BoundField, Direction, NullOrder)>,
    pub frame: WindowFrame,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundRatioInput {
    pub function: AggregateFunction,
    pub field: Option<BoundField>,
    pub distinct: bool,
    pub filter: Option<BoundPredicate>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundRelationship {
    pub definition: ObjectRef,
    pub right: ObjectRef,
    pub instance: String,
    pub mode: ExistenceMode,
    pub keys: Vec<(BoundField, BoundField)>,
    pub null_keys_match: bool,
    pub predicate: Option<BoundPredicate>,
    pub policies: Vec<ObjectRef>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum BoundPredicate {
    /// Retains parameter provenance; lowering preserves the exact child value.
    Mapped {
        definition: ObjectRef,
        phrase: String,
        predicate: Box<BoundPredicate>,
    },
    Compare {
        field: BoundField,
        operator: Comparison,
        value: Literal,
    },
    IsNull {
        field: BoundField,
        negated: bool,
    },
    All {
        predicates: Vec<BoundPredicate>,
    },
    Any {
        predicates: Vec<BoundPredicate>,
    },
    Not {
        predicate: Box<BoundPredicate>,
    },
}

#[tracing::instrument(name = "semantic.bind", skip_all)]
pub(super) fn bind(
    snapshot: &CatalogSnapshot,
    query: &RowQuery,
    options: &CompileOptions,
    work: &mut Work,
) -> Result<BoundQuery, CompileDiagnostic> {
    if !super::context::allowed(&query.input.relation, options) {
        return Err(diagnostic(
            "access_scope",
            "Relation is outside the compiler access scope",
        ));
    }
    if query.version != ROW_QUERY_VERSION {
        return Err(diagnostic(
            "unsupported_version",
            "Expected row query version 1",
        ));
    }
    if !query.unresolved.is_empty() {
        return Err(diagnostic(
            "unresolved_terms",
            "Resolve all business choices before binding",
        ));
    }
    if query.input.instance.trim().is_empty() || query.input.instance.starts_with('$') {
        return Err(diagnostic(
            "invalid_scope",
            "A relation occurrence needs a nonempty instance ID",
        ));
    }
    if query.requirements.is_empty() {
        return Err(diagnostic(
            "missing_requirements",
            "At least one request requirement is required",
        ));
    }
    if query.requirements.len() > options.max_nodes {
        return Err(diagnostic(
            "requirement_limit",
            "Requirement count exceeds the work budget",
        ));
    }
    work.relations_looked_up += 1;
    let relation = snapshot.relation(&query.input.relation).ok_or_else(|| {
        diagnostic(
            "unknown_relation",
            "The input relation is not in this catalog snapshot",
        )
    })?;
    if relation
        .definition()
        .semantics
        .as_ref()
        .and_then(|s| s.capability.as_ref())
        .is_some_and(|capability| {
            !matches!(capability, semantic_catalog::Capability::Executable { .. })
        })
    {
        return Err(diagnostic(
            "catalog_capability",
            "Selected relation is not executable in this catalog snapshot",
        ));
    }
    let mut binder = Binder {
        relation,
        instance: &query.input.instance,
        options,
        work,
    };
    let mut instances = BTreeSet::from([query.input.instance.as_str()]);
    let mut ids = BTreeSet::new();
    let mut aliases = BTreeSet::new();
    let aggregate_query = query.requirements.iter().any(|r| {
        matches!(
            r.operation,
            RowOperation::Ratio { .. }
                | RowOperation::Group { .. }
                | RowOperation::Aggregate { .. }
                | RowOperation::Metric { .. }
                | RowOperation::Lookup {
                    usage: LookupUsage::Group,
                    ..
                }
        )
    });
    let mut definitions = Vec::new();
    let mut metrics = std::collections::BTreeMap::new();
    let mut ratios = std::collections::BTreeMap::new();
    let mut lookups = std::collections::BTreeMap::new();
    let lookup_dimensions: Vec<_> = query
        .requirements
        .iter()
        .filter_map(|r| {
            if let RowOperation::Lookup {
                relationship,
                field,
                missing,
                usage: LookupUsage::Group,
                ..
            } = &r.operation
            {
                Some(semantic_catalog::MetricLookupDimension {
                    relationship: relationship.clone(),
                    field: field.clone(),
                    missing: *missing,
                })
            } else {
                None
            }
        })
        .collect();
    let grouping_fields: BTreeSet<_> = query
        .requirements
        .iter()
        .filter_map(|r| {
            if let RowOperation::Group { field, .. } = &r.operation {
                Some(field.field.as_str())
            } else {
                None
            }
        })
        .collect();
    let mut output_slots = std::collections::BTreeMap::new();
    for (index, requirement) in query.requirements.iter().enumerate() {
        let output_type = match &requirement.operation {
            RowOperation::Lookup {
                relationship,
                role,
                instance,
                field,
                missing,
                usage: LookupUsage::Group,
                ..
            } => {
                let lookup = binder.lookup(
                    snapshot,
                    relationship,
                    role,
                    instance,
                    field,
                    *missing,
                    index,
                )?;
                let ty = lookup.output.field.data_type().clone();
                lookups.insert(requirement.id.clone(), lookup);
                Some((ty, true))
            }
            RowOperation::Ratio {
                numerator,
                denominator,
                zero,
                ..
            } => {
                let numerator = binder.ratio_input(numerator)?;
                let denominator = binder.ratio_input(denominator)?;
                ratios.insert(requirement.id.clone(), (numerator, denominator, *zero));
                Some((DataType::Decimal128(38, 18), true))
            }
            RowOperation::Metric { name, .. } => {
                let semantics = relation.definition().semantics.as_ref().ok_or_else(|| {
                    diagnostic("unknown_metric", "Metric is not declared on this relation")
                })?;
                if let Some(ratio) = semantics.ratio_metrics.get(name) {
                    if ratio.id.is_empty() || semantics.metrics.contains_key(name) {
                        return Err(diagnostic(
                            "metric_contract",
                            "Governed metric identities must be explicit and unambiguous",
                        ));
                    }
                    let numerator =
                        binder.metric(&ratio.numerator, &grouping_fields, &lookup_dimensions)?;
                    let denominator =
                        binder.metric(&ratio.denominator, &grouping_fields, &lookup_dimensions)?;
                    if numerator.1.0 != DataType::Int64 || denominator.1.0 != DataType::Int64 {
                        return Err(diagnostic(
                            "ratio_type",
                            "Governed ratio components require Int64 aggregates",
                        ));
                    }
                    if semantics.metrics[&ratio.numerator].source_grain
                        != semantics.metrics[&ratio.denominator].source_grain
                    {
                        return Err(diagnostic(
                            "ratio_grain",
                            "Single-source ratio components must share their declared source grain",
                        ));
                    }
                    definitions.extend([
                        relation
                            .definition_reference("ratio", name)
                            .expect("indexed definition")
                            .clone(),
                        numerator.2,
                        denominator.2,
                    ]);
                    ratios.insert(
                        requirement.id.clone(),
                        (numerator.0, denominator.0, ratio.zero),
                    );
                    Some((DataType::Decimal128(38, 18), true))
                } else {
                    let (input, ty, reference) =
                        binder.metric(name, &grouping_fields, &lookup_dimensions)?;
                    definitions.push(reference);
                    metrics.insert(requirement.id.clone(), input);
                    Some(ty)
                }
            }
            RowOperation::Group { field, .. } => {
                let field = binder.field(field)?;
                Some((field.field.data_type().clone(), field.field.is_nullable()))
            }
            RowOperation::Aggregate {
                function,
                field,
                distinct,
                ..
            } => {
                let field = field.as_ref().map(|f| binder.field(f)).transpose()?;
                Some(aggregate_type(*function, field.as_ref(), *distinct)?)
            }
            _ => None,
        };
        if let Some((data_type, nullable)) = output_type {
            output_slots.insert(
                requirement.id.clone(),
                BoundField {
                    instance: "$output".into(),
                    field: Field::new(format!("__semantic_output_{index}"), data_type, nullable),
                },
            );
        }
    }
    let mut windows = std::collections::BTreeMap::new();
    // Resolve every window against the same pre-window schema, independent of
    // proposal order. Cycles/window nesting cannot acquire a valid output slot.
    for (index, requirement) in query.requirements.iter().enumerate() {
        if let RowOperation::Window { window, .. } = &requirement.operation {
            validate_window_rollup(window, query, relation, options)?;
            let (window, ty, nullable) = binder.window(window, aggregate_query, &output_slots)?;
            let mut name = format!("__semantic_window_{index}");
            while relation.field(&name).is_some() {
                name.push('_');
            }
            windows.insert(
                requirement.id.clone(),
                (
                    window,
                    BoundField {
                        instance: "$output".into(),
                        field: Field::new(name, ty, nullable),
                    },
                ),
            );
        }
    }
    let aggregate_slots = output_slots.clone();
    output_slots.extend(
        windows
            .iter()
            .map(|(id, (_, output))| (id.clone(), output.clone())),
    );
    let mut limited = false;
    let mut requirements = Vec::new();
    for requirement in &query.requirements {
        binder.visit(0)?;
        if requirement.id.trim().is_empty()
            || !ids.insert(&requirement.id)
            || requirement.source_text.trim().is_empty()
        {
            return Err(diagnostic(
                "invalid_requirement",
                "Requirements need unique nonempty IDs and source text",
            ));
        }
        let operation = match &requirement.operation {
            RowOperation::FilterOutput { stage, predicate } => {
                let slots = match stage {
                    OutputFilterStage::AfterAggregate if aggregate_query => &aggregate_slots,
                    OutputFilterStage::AfterWindow
                        if !windows.is_empty()
                            || query
                                .requirements
                                .iter()
                                .any(|r| matches!(r.operation, RowOperation::Window { .. })) =>
                    {
                        &output_slots
                    }
                    _ => {
                        return Err(diagnostic(
                            "filter_stage",
                            "The selected output filter stage must exist in this query",
                        ));
                    }
                };
                let predicate = binder.predicate_with(predicate, 1, &|binder, field: &OutputRef| {
                    binder.visit(0)?;
                    slots.get(&field.slot).cloned().ok_or_else(|| diagnostic("output_filter_scope", "Output predicate references a slot unavailable at the selected stage"))
                })?;
                BoundOperation::FilterOutput {
                    stage: *stage,
                    predicate,
                }
            }
            RowOperation::Lookup {
                relationship,
                role,
                instance,
                field,
                alias,
                missing,
                usage,
            } => {
                if aggregate_query && *usage == LookupUsage::Project {
                    return Err(diagnostic(
                        "lookup_grain",
                        "Aggregate queries require lookup usage group for every dimension output",
                    ));
                }
                if instance.trim().is_empty()
                    || instance.starts_with('$')
                    || !instances.insert(instance.as_str())
                {
                    return Err(diagnostic(
                        "invalid_scope",
                        "Lookup occurrences need distinct nonempty instance IDs",
                    ));
                }
                check_alias(alias, &mut aliases)?;
                let lookup = if *usage == LookupUsage::Group {
                    lookups.remove(&requirement.id).ok_or_else(|| {
                        diagnostic("invalid_requirement", "Lookup requirements need unique IDs")
                    })?
                } else {
                    binder.lookup(
                        snapshot,
                        relationship,
                        role,
                        instance,
                        field,
                        *missing,
                        requirements.len(),
                    )?
                };
                definitions.push(lookup.relationship.definition.clone());
                definitions.extend(lookup.relationship.policies.iter().cloned());
                definitions.push(ObjectRef {
                    id: "functions/semantic_assert_single_v1".into(),
                    revision: "1".into(),
                });
                BoundOperation::Lookup {
                    lookup: Box::new(lookup),
                    alias: alias.clone(),
                    group_output: (*usage == LookupUsage::Group)
                        .then(|| output_slots[&requirement.id].clone()),
                }
            }
            RowOperation::CalendarFilter { field, period } => {
                let field = binder.field(field)?;
                let context = options.request_context.as_ref().ok_or_else(|| diagnostic("unresolved_time_context", "Relative periods require a pinned reference instant, timezone and calendar"))?;
                let (start, end, resolution) =
                    super::temporal::resolve(period, context, field.field.data_type())?;
                BoundOperation::CalendarFilter {
                    predicate: BoundPredicate::All {
                        predicates: vec![
                            BoundPredicate::Compare {
                                field: field.clone(),
                                operator: Comparison::GtEq,
                                value: start,
                            },
                            BoundPredicate::Compare {
                                field,
                                operator: Comparison::Lt,
                                value: end,
                            },
                        ],
                    },
                    resolution,
                }
            }
            RowOperation::Window { alias, .. } => {
                check_alias(alias, &mut aliases)?;
                let (window, output) = windows.remove(&requirement.id).ok_or_else(|| {
                    diagnostic("invalid_requirement", "Window requirements need unique IDs")
                })?;
                BoundOperation::Window {
                    window,
                    output,
                    alias: alias.clone(),
                }
            }
            RowOperation::Ratio { zero, alias, .. } => {
                check_alias(alias, &mut aliases)?;
                let (numerator, denominator, _) =
                    ratios.remove(&requirement.id).ok_or_else(|| {
                        diagnostic("invalid_requirement", "Ratio requirements need unique IDs")
                    })?;
                BoundOperation::Ratio {
                    numerator: Box::new(numerator),
                    denominator: Box::new(denominator),
                    zero: *zero,
                    alias: alias.clone(),
                    output: output_slots[&requirement.id].clone(),
                }
            }
            RowOperation::Related {
                relationship,
                role,
                instance,
                mode,
                predicate,
            } => {
                if instance.trim().is_empty()
                    || instance.starts_with('$')
                    || !instances.insert(instance.as_str())
                {
                    return Err(diagnostic(
                        "invalid_scope",
                        "Related occurrences need distinct nonempty instance IDs",
                    ));
                }
                let relationship = binder.relationship(
                    snapshot,
                    relationship,
                    role,
                    instance,
                    *mode,
                    predicate.as_ref(),
                )?;
                definitions.push(relationship.definition.clone());
                definitions.extend(relationship.policies.iter().cloned());
                BoundOperation::Related { relationship }
            }

            RowOperation::Metric { alias, .. } => {
                check_alias(alias, &mut aliases)?;
                if let Some((numerator, denominator, zero)) = ratios.remove(&requirement.id) {
                    BoundOperation::Ratio {
                        numerator: Box::new(numerator),
                        denominator: Box::new(denominator),
                        zero,
                        alias: alias.clone(),
                        output: output_slots[&requirement.id].clone(),
                    }
                } else {
                    let input = metrics.remove(&requirement.id).ok_or_else(|| {
                        diagnostic("invalid_requirement", "Metric requirements need unique IDs")
                    })?;
                    BoundOperation::Aggregate {
                        function: input.function,
                        field: input.field,
                        distinct: input.distinct,
                        alias: alias.clone(),
                        output: output_slots[&requirement.id].clone(),
                        filter: input.filter,
                    }
                }
            }
            RowOperation::Group { field, alias } => {
                check_alias(alias, &mut aliases)?;
                BoundOperation::Group {
                    field: binder.field(field)?,
                    alias: alias.clone(),
                    output: output_slots[&requirement.id].clone(),
                }
            }
            RowOperation::Aggregate {
                function,
                field,
                distinct,
                alias,
            } => {
                check_alias(alias, &mut aliases)?;
                BoundOperation::Aggregate {
                    function: *function,
                    field: field.as_ref().map(|f| binder.field(f)).transpose()?,
                    distinct: *distinct,
                    alias: alias.clone(),
                    output: output_slots[&requirement.id].clone(),
                    filter: None,
                }
            }
            RowOperation::OrderOutput {
                slot,
                direction,
                nulls,
            } => {
                let field = output_slots.get(slot).ok_or_else(|| {
                    diagnostic(
                        "unknown_output_slot",
                        "Ordering must reference a grouping, aggregate or window requirement ID",
                    )
                })?;
                BoundOperation::Order {
                    field: field.clone(),
                    direction: *direction,
                    nulls: *nulls,
                }
            }
            RowOperation::Project { field, alias } => {
                if aggregate_query {
                    return Err(diagnostic(
                        "aggregate_grain",
                        "Aggregate queries must explicitly group each dimension output",
                    ));
                }
                if alias.trim().is_empty() || !aliases.insert(alias.as_str()) {
                    return Err(diagnostic(
                        "invalid_output",
                        "This profile requires unique nonempty output aliases",
                    ));
                }
                BoundOperation::Project {
                    field: binder.field(field)?,
                    alias: alias.clone(),
                }
            }
            RowOperation::Filter { predicate } => BoundOperation::Filter {
                predicate: binder.predicate(predicate, 1)?,
            },
            RowOperation::Order {
                field,
                direction,
                nulls,
            } => {
                if aggregate_query {
                    return Err(diagnostic(
                        "aggregate_order",
                        "Aggregate ordering must reference an output slot",
                    ));
                }
                let field = binder.field(field)?;
                if !matches!(
                    field.field.data_type(),
                    DataType::Boolean
                        | DataType::Int16
                        | DataType::Int32
                        | DataType::Int64
                        | DataType::Utf8
                        | DataType::Decimal128(_, _)
                        | DataType::Date32
                        | DataType::Timestamp(_, _)
                ) {
                    return Err(diagnostic(
                        "unsupported_order_type",
                        "Ordering requires a supported exact scalar field type",
                    ));
                }
                BoundOperation::Order {
                    field,
                    direction: *direction,
                    nulls: *nulls,
                }
            }
            RowOperation::Limit { count } => {
                if limited {
                    return Err(diagnostic(
                        "conflicting_limits",
                        "A query can have only one limit requirement",
                    ));
                }
                limited = true;
                BoundOperation::Limit { count: *count }
            }
        };
        requirements.push(BoundRequirement {
            id: requirement.id.clone(),
            operation,
        });
    }
    if aliases.is_empty() {
        return Err(diagnostic(
            "missing_projection",
            "At least one explicit projection is required",
        ));
    }
    if let Some(semantics) = &relation.definition().semantics {
        if semantics.row_policies.len() > options.max_nodes {
            return Err(diagnostic(
                "work_limit",
                "Policy count exceeds the work budget",
            ));
        }
        let mut policy_ids = BTreeSet::new();
        for (index, policy) in semantics.row_policies.iter().enumerate() {
            if policy.id.is_empty() || !policy_ids.insert(&policy.id) {
                return Err(diagnostic(
                    "policy_contract",
                    "Policies require unique nonempty identities",
                ));
            }
            let predicate = binder.governed_filters(&policy.filters)?.ok_or_else(|| {
                diagnostic(
                    "policy_contract",
                    "An executable row policy requires a predicate",
                )
            })?;
            let mut id = format!("__policy_{index}");
            while ids.contains(&id) {
                id.push('_');
            }
            definitions.push(
                relation
                    .definition_reference("policy", &index.to_string())
                    .expect("indexed policy")
                    .clone(),
            );
            requirements.push(BoundRequirement {
                id,
                operation: BoundOperation::Filter { predicate },
            });
        }
    }
    if requirements
        .iter()
        .any(|r| matches!(r.operation, BoundOperation::Ratio { .. }))
    {
        definitions.push(ObjectRef {
            id: "functions/semantic_ratio_i64_v1".into(),
            revision: "1".into(),
        });
    }
    if requirements
        .iter()
        .any(|requirement| match &requirement.operation {
            BoundOperation::Aggregate { function, .. } => *function == AggregateFunction::Sum,
            BoundOperation::Ratio {
                numerator,
                denominator,
                ..
            } => {
                numerator.function == AggregateFunction::Sum
                    || denominator.function == AggregateFunction::Sum
            }
            BoundOperation::Window { window, .. } => window.function == WindowFunction::Sum,
            _ => false,
        })
    {
        definitions.push(ObjectRef {
            id: "functions/semantic_sum_v1".into(),
            revision: "1".into(),
        });
    }
    for requirement in &requirements {
        let mut predicates = Vec::new();
        match &requirement.operation {
            BoundOperation::Filter { predicate }
            | BoundOperation::FilterOutput { predicate, .. }
            | BoundOperation::CalendarFilter { predicate, .. } => predicates.push(predicate),
            BoundOperation::Aggregate { filter, .. } => predicates.extend(filter),
            BoundOperation::Related { relationship } => predicates.extend(&relationship.predicate),
            BoundOperation::Lookup { lookup, .. } => {
                predicates.extend(&lookup.relationship.predicate)
            }
            BoundOperation::Ratio {
                numerator,
                denominator,
                ..
            } => {
                predicates.extend(&numerator.filter);
                predicates.extend(&denominator.filter);
            }
            _ => {}
        }
        while let Some(predicate) = predicates.pop() {
            match predicate {
                BoundPredicate::Mapped {
                    definition,
                    predicate,
                    ..
                } => {
                    definitions.push(definition.clone());
                    predicates.push(predicate);
                }
                BoundPredicate::All {
                    predicates: children,
                }
                | BoundPredicate::Any {
                    predicates: children,
                } => predicates.extend(children),
                BoundPredicate::Not { predicate } => predicates.push(predicate),
                _ => {}
            }
        }
    }
    definitions.sort_by(|a, b| (&a.id, &a.revision).cmp(&(&b.id, &b.revision)));
    definitions.dedup();
    Ok(BoundQuery {
        acceptance_profile: "strict/v1",
        definitions,
        version: ROW_QUERY_VERSION,
        snapshot_id: snapshot.id().into(),
        input: relation.reference().clone(),
        instance: query.input.instance.clone(),
        requirements,
    })
}

struct Binder<'a> {
    relation: &'a SnapshotRelation,
    instance: &'a str,
    options: &'a CompileOptions,
    work: &'a mut Work,
}
impl Binder<'_> {
    #[allow(clippy::too_many_arguments)]
    fn lookup(
        &mut self,
        snapshot: &CatalogSnapshot,
        relationship: &str,
        role: &str,
        instance: &str,
        field: &str,
        missing: MissingMatch,
        index: usize,
    ) -> Result<BoundLookup, CompileDiagnostic> {
        let relationship = self.relationship(
            snapshot,
            relationship,
            role,
            instance,
            ExistenceMode::Exists,
            None,
        )?;
        let right = snapshot
            .relation(&relationship.right.id)
            .expect("bound endpoint");
        let value = Binder {
            relation: right,
            instance,
            options: self.options,
            work: self.work,
        }
        .field(&FieldRef {
            instance: instance.into(),
            field: field.into(),
        })?;
        // MIN never chooses among multiple matches: the grouped-key guard fails.
        aggregate_type(AggregateFunction::Min, Some(&value), false)?;
        let mut name = format!("__semantic_lookup_{index}");
        while self.relation.field(&name).is_some() {
            name.push('_');
        }
        let output = BoundField {
            instance: "$output".into(),
            field: Field::new(name, value.field.data_type().clone(), true),
        };
        Ok(BoundLookup {
            relationship,
            value,
            output,
            missing,
            obligation: "same-query/grouped-right-key-count-at-most-one/v1",
        })
    }
    fn relationship(
        &mut self,
        snapshot: &CatalogSnapshot,
        relationship: &str,
        role: &str,
        instance: &str,
        mode: ExistenceMode,
        predicate: Option<&RowPredicate>,
    ) -> Result<BoundRelationship, CompileDiagnostic> {
        let relationship_ref = self
            .relation
            .definition_reference("relationship", relationship)
            .cloned();
        let relationship = self
            .relation
            .definition()
            .semantics
            .as_ref()
            .and_then(|s| s.relationships.get(relationship))
            .ok_or_else(|| {
                diagnostic(
                    "unknown_relationship",
                    "Only an authored relationship can authorize related-row queries",
                )
            })?;
        if relationship.id.is_empty()
            || role.is_empty()
            || role != relationship.role
            || relationship.key_pairs.is_empty()
        {
            return Err(diagnostic(
                "relationship_contract",
                "Relationship identity, role and key pairs must be explicit",
            ));
        }
        if relationship.key_pairs.len() > self.options.max_nodes {
            return Err(diagnostic(
                "work_limit",
                "Relationship key budget exhausted",
            ));
        }
        if !super::context::allowed(&relationship.right_relation, self.options) {
            return Err(diagnostic(
                "access_scope",
                "Related relation is outside the compiler access scope",
            ));
        }
        self.work.relations_looked_up += 1;
        let right = snapshot
            .relation(&relationship.right_relation)
            .ok_or_else(|| diagnostic("unknown_relation", "Relationship endpoint is missing"))?;
        if right
            .definition()
            .semantics
            .as_ref()
            .and_then(|s| s.capability.as_ref())
            .is_some_and(|c| !matches!(c, semantic_catalog::Capability::Executable { .. }))
        {
            return Err(diagnostic(
                "catalog_capability",
                "Related relation is not executable",
            ));
        }
        let mut keys = Vec::new();
        for key in &relationship.key_pairs {
            let left = self.field(&FieldRef {
                instance: self.instance.into(),
                field: key.left_field.clone(),
            })?;
            let mut right_binder = Binder {
                relation: right,
                instance,
                options: self.options,
                work: self.work,
            };
            let right = right_binder.field(&FieldRef {
                instance: instance.to_owned(),
                field: key.right_field.clone(),
            })?;
            if left.field.data_type() != right.field.data_type()
                || !supported_key_type(left.field.data_type())
            {
                return Err(diagnostic(
                    "relationship_key_type",
                    "Relationship keys require identical supported scalar comparison types",
                ));
            }
            keys.push((left, right));
        }
        let mut right_binder = Binder {
            relation: right,
            instance,
            options: self.options,
            work: self.work,
        };
        let mut predicates = predicate
            .map(|p| right_binder.predicate(p, 1))
            .transpose()?
            .into_iter()
            .collect::<Vec<_>>();
        let mut policies = Vec::new();
        if let Some(semantics) = &right.definition().semantics {
            if semantics.row_policies.len() > self.options.max_nodes {
                return Err(diagnostic("work_limit", "Related policy budget exhausted"));
            }
            let mut policy_ids = BTreeSet::new();
            for (index, policy) in semantics.row_policies.iter().enumerate() {
                if policy.id.is_empty() || !policy_ids.insert(&policy.id) {
                    return Err(diagnostic(
                        "policy_contract",
                        "Policies require unique nonempty identities",
                    ));
                }
                let predicate =
                    right_binder
                        .governed_filters(&policy.filters)?
                        .ok_or_else(|| {
                            diagnostic(
                                "policy_contract",
                                "An executable policy requires a predicate",
                            )
                        })?;
                policies.push(
                    right
                        .definition_reference("policy", &index.to_string())
                        .expect("indexed policy")
                        .clone(),
                );
                predicates.push(predicate);
            }
        }
        let predicate = (!predicates.is_empty()).then_some(BoundPredicate::All { predicates });
        Ok(BoundRelationship {
            definition: relationship_ref.expect("indexed relationship"),
            right: right.reference().clone(),
            instance: instance.to_owned(),
            mode,
            keys,
            null_keys_match: relationship.null_keys_match,
            predicate,
            policies,
        })
    }
    fn visit(&mut self, depth: usize) -> Result<(), CompileDiagnostic> {
        self.options.check()?;
        self.work.nodes_visited += 1;
        if depth > self.options.max_depth || self.work.nodes_visited > self.options.max_nodes {
            return Err(diagnostic(
                "work_limit",
                "Expression depth or node budget exhausted",
            ));
        }
        Ok(())
    }
    fn field(&mut self, field: &FieldRef) -> Result<BoundField, CompileDiagnostic> {
        self.visit(0)?;
        if field.instance != self.instance {
            return Err(diagnostic(
                "invalid_scope",
                "Field reference belongs to a different relation occurrence",
            ));
        }
        self.work.fields_looked_up += 1;
        let definition = self.relation.field(&field.field).ok_or_else(|| {
            diagnostic(
                "unknown_field",
                "Field is missing or ambiguous in the selected relation",
            )
        })?;
        Ok(BoundField {
            instance: field.instance.clone(),
            field: definition.clone(),
        })
    }
    fn window(
        &mut self,
        spec: &WindowSpec,
        aggregate: bool,
        slots: &std::collections::BTreeMap<String, BoundField>,
    ) -> Result<(BoundWindow, DataType, bool), CompileDiagnostic> {
        if spec.partition_by.len().saturating_add(spec.order_by.len()) > self.options.max_nodes {
            return Err(diagnostic("work_limit", "Window key budget exhausted"));
        }
        let mut resolve = |input: &WindowInput| -> Result<BoundField, CompileDiagnostic> {
            self.visit(0)?;
            match input {
                WindowInput::Field { field } if !aggregate => self.field(field),
                WindowInput::Output { slot } if aggregate => {
                    slots.get(slot).cloned().ok_or_else(|| {
                        diagnostic(
                            "window_scope",
                            "Window input must be a grouping or aggregate output",
                        )
                    })
                }
                _ => Err(diagnostic(
                    "window_grain",
                    "Window inputs must belong to the current row or aggregate grain",
                )),
            }
        };
        let input = spec.input.as_ref().map(&mut resolve).transpose()?;
        let partition_by = spec
            .partition_by
            .iter()
            .map(&mut resolve)
            .collect::<Result<Vec<_>, _>>()?;
        let order_by = spec
            .order_by
            .iter()
            .map(|order| Ok((resolve(&order.input)?, order.direction, order.nulls)))
            .collect::<Result<Vec<_>, CompileDiagnostic>>()?;
        if partition_by
            .iter()
            .chain(order_by.iter().map(|o| &o.0))
            .any(|field| !supported_key_type(field.field.data_type()))
        {
            return Err(diagnostic(
                "window_key_type",
                "Window keys require supported exact scalar types",
            ));
        }
        let rank = matches!(
            spec.function,
            WindowFunction::Rank | WindowFunction::DenseRank
        );
        if (rank || spec.frame == WindowFrame::ThroughCurrentPeer) && order_by.is_empty() {
            return Err(diagnostic(
                "window_order",
                "Rank and running windows require explicit ordering",
            ));
        }
        let (ty, nullable) = if rank {
            if input.is_some() || spec.frame != WindowFrame::ThroughCurrentPeer {
                return Err(diagnostic(
                    "window_contract",
                    "Ranks take no input and require the through-current-peer frame",
                ));
            }
            (DataType::UInt64, false)
        } else {
            aggregate_type(
                match spec.function {
                    WindowFunction::Count => AggregateFunction::Count,
                    WindowFunction::Sum => AggregateFunction::Sum,
                    WindowFunction::Min => AggregateFunction::Min,
                    WindowFunction::Max => AggregateFunction::Max,
                    _ => unreachable!(),
                },
                input.as_ref(),
                false,
            )?
        };
        Ok((
            BoundWindow {
                function: spec.function,
                input,
                partition_by,
                order_by,
                frame: spec.frame,
            },
            ty,
            nullable,
        ))
    }
    fn metric(
        &mut self,
        name: &str,
        grouping_fields: &BTreeSet<&str>,
        lookup_dimensions: &[semantic_catalog::MetricLookupDimension],
    ) -> Result<(BoundRatioInput, (DataType, bool), ObjectRef), CompileDiagnostic> {
        let metric = self
            .relation
            .definition()
            .semantics
            .as_ref()
            .and_then(|s| s.metrics.get(name))
            .ok_or_else(|| {
                diagnostic("unknown_metric", "Aggregate metric dependency is missing")
            })?;
        if metric.id.is_empty() || metric.source_grain.is_empty() {
            return Err(diagnostic(
                "metric_contract",
                "Metric requires a durable identity and explicit source grain",
            ));
        }
        if metric.source_grain.len() > self.options.max_nodes
            || metric.row_filters.len() > self.options.max_nodes
        {
            return Err(diagnostic(
                "work_limit",
                "Metric contract exceeds the work budget",
            ));
        }
        for field in &metric.source_grain {
            self.field(&FieldRef {
                instance: self.instance.into(),
                field: field.clone(),
            })?;
        }
        if grouping_fields
            .iter()
            .any(|field| !metric.compatible_dimensions.contains(*field))
        {
            return Err(diagnostic(
                "metric_dimensions",
                "Grouping is outside this metric's compatible dimensions",
            ));
        }
        if metric.compatible_lookup_dimensions.len() > self.options.max_nodes {
            return Err(diagnostic(
                "work_limit",
                "Metric dimension contract exceeds the work budget",
            ));
        }
        if lookup_dimensions
            .iter()
            .any(|d| !metric.compatible_lookup_dimensions.contains(d))
        {
            return Err(diagnostic(
                "metric_dimensions",
                "Lookup role, field or missing-match behavior is outside this metric's compatible dimensions",
            ));
        }
        let field = metric
            .field
            .as_ref()
            .map(|field| {
                self.field(&FieldRef {
                    instance: self.instance.into(),
                    field: field.clone(),
                })
            })
            .transpose()?;
        let ty = aggregate_type(metric.function, field.as_ref(), metric.distinct)?;
        let empty = if metric.function == AggregateFunction::Count {
            semantic_catalog::EmptyBehavior::Zero
        } else {
            semantic_catalog::EmptyBehavior::Null
        };
        if ty.0 != metric.result_type || metric.empty_behavior != empty {
            return Err(diagnostic(
                "metric_contract",
                "Metric result type or empty-input contract does not match its executable definition",
            ));
        }
        let filter = self.governed_filters(&metric.row_filters)?;
        Ok((
            BoundRatioInput {
                function: metric.function,
                field,
                distinct: metric.distinct,
                filter,
            },
            ty,
            self.relation
                .definition_reference("metric", name)
                .expect("indexed metric")
                .clone(),
        ))
    }
    fn ratio_input(
        &mut self,
        operand: &AggregateOperand,
    ) -> Result<BoundRatioInput, CompileDiagnostic> {
        let field = operand.field.as_ref().map(|f| self.field(f)).transpose()?;
        if aggregate_type(operand.function, field.as_ref(), operand.distinct)?.0 != DataType::Int64
        {
            return Err(diagnostic(
                "ratio_type",
                "This ratio profile requires Int64 aggregate components",
            ));
        }
        Ok(BoundRatioInput {
            function: operand.function,
            field,
            distinct: operand.distinct,
            filter: None,
        })
    }
    fn governed_filters(
        &mut self,
        filters: &[semantic_catalog::GovernedFilter],
    ) -> Result<Option<BoundPredicate>, CompileDiagnostic> {
        if filters.len() > self.options.max_nodes {
            return Err(diagnostic(
                "work_limit",
                "Governed filter count exceeds the work budget",
            ));
        }
        let predicates = filters
            .iter()
            .map(|filter| {
                self.predicate(
                    &RowPredicate::Compare {
                        field: FieldRef {
                            instance: self.instance.into(),
                            field: filter.field.clone(),
                        },
                        operator: filter.operator,
                        value: filter.value.clone(),
                    },
                    1,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(if predicates.is_empty() {
            None
        } else {
            Some(BoundPredicate::All { predicates })
        })
    }
    fn predicate(
        &mut self,
        predicate: &RowPredicate,
        depth: usize,
    ) -> Result<BoundPredicate, CompileDiagnostic> {
        self.predicate_with(predicate, depth, &|binder, field| binder.field(field))
    }
    fn predicate_with<F>(
        &mut self,
        predicate: &RowPredicate<F>,
        depth: usize,
        resolve: &impl Fn(&mut Self, &F) -> Result<BoundField, CompileDiagnostic>,
    ) -> Result<BoundPredicate, CompileDiagnostic> {
        self.visit(depth)?;
        Ok(match predicate {
            RowPredicate::CompareMapped {
                field,
                operator,
                mapping,
                phrase,
            } => {
                let field = resolve(self, field)?;
                if field.instance != self.instance || *field.field.data_type() != DataType::Utf8 {
                    return Err(diagnostic(
                        "value_mapping_scope",
                        "Value mappings require their exact source occurrence and Utf8 field",
                    ));
                }
                let definition = self
                    .relation
                    .definition()
                    .semantics
                    .as_ref()
                    .and_then(|s| s.value_mappings.get(mapping))
                    .ok_or_else(|| {
                        diagnostic(
                            "unknown_value_mapping",
                            "The value mapping is not authored on this relation",
                        )
                    })?;
                if definition.id.is_empty() || definition.field != *field.field.name() {
                    return Err(diagnostic(
                        "value_mapping_scope",
                        "Value mapping does not govern this field",
                    ));
                }
                let code = definition.codes.get(phrase).ok_or_else(|| {
                    diagnostic(
                        "unresolved_value",
                        "The exact phrase has no entry in this authored value mapping",
                    )
                })?;
                self.work.mapped_value_bytes =
                    self.work.mapped_value_bytes.saturating_add(code.len());
                if self.work.mapped_value_bytes > self.options.max_input_bytes {
                    return Err(diagnostic(
                        "input_limit",
                        "Mapped parameter exceeds the input byte budget",
                    ));
                }
                BoundPredicate::Mapped {
                    definition: self
                        .relation
                        .definition_reference("value_mapping", mapping)
                        .expect("indexed mapping")
                        .clone(),
                    phrase: phrase.clone(),
                    predicate: Box::new(BoundPredicate::Compare {
                        field,
                        operator: *operator,
                        value: Literal::Utf8(code.clone()),
                    }),
                }
            }
            RowPredicate::Compare {
                field,
                operator,
                value,
            } => {
                let field = resolve(self, field)?;
                let value_type = super::literal::checked_scalar(value)?.data_type();
                let valid = field.field.data_type() == &value_type;
                if !valid {
                    return Err(diagnostic(
                        "comparison_type",
                        "Comparison requires an exact supported field/literal type; implicit casts are disabled",
                    ));
                }
                if matches!(value, Literal::Boolean(_))
                    && !matches!(operator, Comparison::Eq | Comparison::NotEq)
                {
                    return Err(diagnostic(
                        "comparison_operator",
                        "Boolean comparisons support only equality and inequality",
                    ));
                }
                BoundPredicate::Compare {
                    field,
                    operator: *operator,
                    value: value.clone(),
                }
            }
            RowPredicate::IsNull { field, negated } => BoundPredicate::IsNull {
                field: resolve(self, field)?,
                negated: *negated,
            },
            RowPredicate::Not { predicate } => BoundPredicate::Not {
                predicate: Box::new(self.predicate_with(predicate, depth + 1, resolve)?),
            },
            RowPredicate::All { predicates } | RowPredicate::Any { predicates } => {
                if predicates.is_empty() {
                    return Err(diagnostic(
                        "empty_boolean",
                        "Boolean conjunctions/disjunctions must not be empty",
                    ));
                }
                let children = predicates
                    .iter()
                    .map(|p| self.predicate_with(p, depth + 1, resolve))
                    .collect::<Result<_, _>>()?;
                if matches!(predicate, RowPredicate::All { .. }) {
                    BoundPredicate::All {
                        predicates: children,
                    }
                } else {
                    BoundPredicate::Any {
                        predicates: children,
                    }
                }
            }
        })
    }
}

// A scalar window sum is a merge of subgroup results. Grouping has made
// source rows disjoint, but that does not make distinct sets or finalized
// ratios additive. Named metrics additionally need authored dimensional scope.
fn validate_window_rollup(
    window: &WindowSpec,
    query: &RowQuery,
    relation: &SnapshotRelation,
    options: &CompileOptions,
) -> Result<(), CompileDiagnostic> {
    if window.function != WindowFunction::Sum {
        return Ok(());
    }
    let Some(WindowInput::Output { slot }) = &window.input else {
        return Ok(());
    };
    let Some(input) = query.requirements.iter().find(|r| r.id == *slot) else {
        return Ok(()); // The regular scope validator reports missing references.
    };
    let unsupported = || {
        diagnostic(
            "metric_rollup",
            "Scalar sum cannot merge this metric's finalized results at the requested grain",
        )
    };
    match &input.operation {
        RowOperation::Ratio { .. } => Err(unsupported()),
        RowOperation::Aggregate {
            function: AggregateFunction::Count | AggregateFunction::Sum,
            distinct: true,
            ..
        } => Err(unsupported()),
        RowOperation::Metric { name, .. } => {
            let metric = relation
                .definition()
                .semantics
                .as_ref()
                .and_then(|s| s.metrics.get(name))
                .ok_or_else(unsupported)?;
            let dimensions = metric
                .sum_rollup_dimensions
                .as_ref()
                .ok_or_else(unsupported)?;
            if dimensions.len() > options.max_nodes {
                return Err(diagnostic(
                    "work_limit",
                    "Metric merge contract exceeds the work budget",
                ));
            }
            if metric.distinct
                || !matches!(
                    metric.function,
                    AggregateFunction::Sum | AggregateFunction::Count
                )
            {
                return Err(unsupported());
            }
            for group in &query.requirements {
                options.check()?;
                if matches!(
                    group.operation,
                    RowOperation::Lookup {
                        usage: LookupUsage::Group,
                        ..
                    }
                ) && !window
                    .partition_by
                    .iter()
                    .any(|p| matches!(p, WindowInput::Output { slot } if *slot == group.id))
                {
                    return Err(unsupported());
                }
                if let RowOperation::Group { field, .. } = &group.operation {
                    let retained = window
                        .partition_by
                        .iter()
                        .any(|p| matches!(p, WindowInput::Output { slot } if *slot == group.id));
                    if !retained && !dimensions.contains(&field.field) {
                        return Err(unsupported());
                    }
                }
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn check_alias<'a>(
    alias: &'a str,
    aliases: &mut BTreeSet<&'a str>,
) -> Result<(), CompileDiagnostic> {
    if alias.trim().is_empty() || !aliases.insert(alias) {
        return Err(diagnostic(
            "invalid_output",
            "Outputs need unique nonempty aliases",
        ));
    }
    Ok(())
}
fn aggregate_type(
    function: AggregateFunction,
    field: Option<&BoundField>,
    distinct: bool,
) -> Result<(DataType, bool), CompileDiagnostic> {
    if function == AggregateFunction::Count {
        if field.is_none() && distinct {
            return Err(diagnostic(
                "aggregate_arguments",
                "Distinct count requires an explicit field",
            ));
        }
        return Ok((DataType::Int64, false));
    }
    let ty = field
        .ok_or_else(|| diagnostic("aggregate_arguments", "This aggregate requires a field"))?
        .field
        .data_type();
    let result = match (function, ty) {
        (AggregateFunction::Sum, DataType::Int16 | DataType::Int32 | DataType::Int64) => {
            DataType::Int64
        }
        (AggregateFunction::Sum, DataType::Decimal128(p, s)) => {
            DataType::Decimal128((*p + 10).min(38), *s)
        }
        (
            AggregateFunction::Min | AggregateFunction::Max,
            DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::Utf8
            | DataType::Decimal128(_, _)
            | DataType::Date32
            | DataType::Timestamp(_, _),
        ) => ty.clone(),
        _ => {
            return Err(diagnostic(
                "aggregate_type",
                "Aggregate is unavailable for this field type",
            ));
        }
    };
    Ok((result, true))
}

fn supported_key_type(ty: &DataType) -> bool {
    matches!(
        ty,
        DataType::Boolean
            | DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::Utf8
            | DataType::Decimal128(_, _)
            | DataType::Date32
            | DataType::Timestamp(_, _)
    )
}
