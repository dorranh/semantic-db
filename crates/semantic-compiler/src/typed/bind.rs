use std::collections::{BTreeMap, BTreeSet};

use semantic_catalog::{
    Authority, CatalogSnapshot, DataType, Fact, FactResolution, Field, ObjectRef, Presence,
    SlotMeaning, SnapshotRelation, SourceGrain, SourceRef, Unit,
};
use semantic_plan::typed::*;
use serde::Serialize;

use super::{CompileDiagnostic, CompileOptions, Work, diagnostic};
mod allocation;
mod business_calendar;
mod currency_rate;
mod path;

/// Constructible only by validation; serialized copies must re-enter as proposals.
#[derive(Clone, Serialize)]
pub struct BoundQuery {
    pub(super) version: u32,
    pub(super) acceptance_profile: &'static str,
    pub(super) snapshot_id: String,
    pub(super) input: ObjectRef,
    pub(super) instance: String,
    pub(super) requirements: Vec<BoundRequirement>,
    pub(super) definitions: Vec<ObjectRef>,
    pub(super) output_meanings: BTreeMap<String, SlotMeaning>,
}
impl std::fmt::Debug for BoundQuery {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BoundQuery")
            .field("version", &self.version)
            .field("requirement_count", &self.requirements.len())
            .field("definition_count", &self.definitions.len())
            .finish_non_exhaustive()
    }
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
// Operations retain their checked semantic state inline and are bounded by the
// compilation node budget. Keep the representation consistent across variants.
#[allow(clippy::large_enum_variant)]
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
    PathLookup {
        lookups: Vec<BoundLookup>,
        alias: String,
    },
    Allocate {
        allocation: Box<BoundAllocation>,
        target_aliases: Vec<String>,
        amount_alias: String,
    },
    Convert {
        conversion: BoundConversion,
        alias: String,
    },
    ConvertRate {
        conversion: BoundRateConversion,
        alias: String,
    },
    CurrencyConvert {
        rate: Box<BoundCurrencyRate>,
        alias: String,
    },
    BusinessCalendar {
        calendar: Box<BoundBusinessCalendar>,
        alias: String,
    },
    CalendarFilter {
        predicate: BoundPredicate,
        resolution: super::TemporalResolution,
    },
    CalendarGroup {
        source: BoundField,
        output: BoundField,
        alias: String,
    },
    CalendarFill {
        month: BoundField,
        count: BoundField,
        months: Vec<i64>,
        fill: i64,
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
        mean_state: bool,
        weighted: Option<BoundWeightedState>,
        exact_distinct: bool,
        snapshot: Option<BoundSnapshotState>,
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
    Page {
        offset: u32,
        fetch: u32,
    },
}
#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundLookup {
    pub relationship: BoundRelationship,
    pub value: BoundField,
    pub output: BoundField,
    /// A second field from the same uniquely checked right row, used as the
    /// intermediate time for a following as-of hop.
    pub extra_value: Option<BoundLookupExtra>,
    pub missing: MissingMatch,
    pub obligation: &'static str,
    pub as_of: Option<BoundAsOf>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundLookupExtra {
    pub value: BoundField,
    pub output: BoundField,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundAllocation {
    pub definition: ObjectRef,
    pub bridge: ObjectRef,
    pub source_key: Vec<BoundField>,
    pub bridge_key: Vec<BoundField>,
    pub source_amount: BoundField,
    pub expected_count: BoundField,
    pub expected_weight: BoundField,
    pub targets: Vec<BoundField>,
    pub weight: BoundField,
    pub predicate: Option<BoundPredicate>,
    pub policies: Vec<ObjectRef>,
    pub target_outputs: Vec<BoundField>,
    pub amount_output: BoundField,
    pub obligation: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundAsOf {
    pub fact_time: BoundField,
    pub valid_from: BoundField,
    pub valid_to: BoundField,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundRateConversion {
    pub source: BoundField,
    pub output: BoundField,
    pub amount: Literal,
    pub result_type: DecimalResultType,
    pub rounding: DecimalRounding,
    pub target_currency: String,
    pub profile: ObjectRef,
    pub source_refs: Vec<SourceRef>,
}
#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundConversion {
    pub source: BoundField,
    pub output: BoundField,
    pub numerator: i64,
    pub denominator: i64,
    pub half_even: bool,
    pub from_unit: Unit,
    pub to_unit: Unit,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundCurrencyRate {
    pub definition: ObjectRef,
    pub rate_relation: ObjectRef,
    pub source_amount: BoundField,
    pub source_currency: BoundField,
    pub source_time: BoundField,
    pub rate_from_currency: BoundField,
    pub rate_to_currency: BoundField,
    pub valid_from: BoundField,
    pub valid_to: BoundField,
    pub numerator: BoundField,
    pub denominator: BoundField,
    pub to_currency: String,
    pub half_even: bool,
    pub predicate: Option<BoundPredicate>,
    pub policies: Vec<ObjectRef>,
    pub output: BoundField,
    pub obligation: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundBusinessCalendar {
    pub definition: ObjectRef,
    pub calendar_relation: ObjectRef,
    pub source_date: BoundField,
    pub source_basis: semantic_catalog::CalendarSourceBasis,
    pub timezone: String,
    pub calendar_date: BoundField,
    pub calendar_value: BoundField,
    pub predicate: Option<BoundPredicate>,
    pub policies: Vec<ObjectRef>,
    pub output: BoundField,
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
    pub mean_state: bool,
    pub weighted: Option<BoundWeightedState>,
    pub exact_distinct: bool,
    pub snapshot: Option<BoundSnapshotState>,
    pub field: Option<BoundField>,
    pub distinct: bool,
    pub filter: Option<BoundPredicate>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundWeightedState {
    pub weight: BoundField,
    pub zero: semantic_catalog::ZeroWeight,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundSnapshotState {
    pub time: BoundField,
    pub tie: BoundField,
}

struct RequestedTemporalWindow {
    field: String,
    grain: CalendarUnit,
    start: Literal,
    end: Literal,
}

#[derive(Clone, Copy)]
enum GovernedMetricCandidate<'a> {
    Metric {
        relation: &'a SnapshotRelation,
        name: &'a str,
        definition: &'a semantic_catalog::MetricDefinition,
    },
    Ratio {
        relation: &'a SnapshotRelation,
        name: &'a str,
        definition: &'a semantic_catalog::RatioDefinition,
    },
}

impl<'a> GovernedMetricCandidate<'a> {
    fn relation(self) -> &'a SnapshotRelation {
        match self {
            Self::Metric { relation, .. } | Self::Ratio { relation, .. } => relation,
        }
    }
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
    pub target: Option<Box<BoundQuery>>,
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
    bind_scoped(snapshot, query, options, work, false)
}
fn bind_scoped(
    snapshot: &CatalogSnapshot,
    query: &RowQuery,
    options: &CompileOptions,
    work: &mut Work,
    target_scope: bool,
) -> Result<BoundQuery, CompileDiagnostic> {
    if !super::allowed(&query.input.relation, options) {
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
    if let Some(fill) = query
        .requirements
        .iter()
        .find(|requirement| matches!(requirement.operation, RowOperation::CalendarFill { .. }))
    {
        let RowOperation::CalendarFill {
            month_slot,
            count_slot,
            fill: fill_value,
            ..
        } = &fill.operation
        else {
            unreachable!()
        };
        let groups = query
            .requirements
            .iter()
            .filter(|requirement| {
                matches!(requirement.operation, RowOperation::CalendarGroup { .. })
            })
            .collect::<Vec<_>>();
        let counts = query
            .requirements
            .iter()
            .filter(|requirement| matches!(requirement.operation, RowOperation::Aggregate { .. }))
            .collect::<Vec<_>>();
        if groups.len() != 1
            || counts.len() != 1
            || groups[0].id != *month_slot
            || counts[0].id != *count_slot
            || *fill_value != 0
            || query.requirements.iter().any(|requirement| {
                !matches!(
                    requirement.operation,
                    RowOperation::CalendarGroup { .. }
                        | RowOperation::Aggregate { .. }
                        | RowOperation::CalendarFill { .. }
                        | RowOperation::Filter { .. }
                        | RowOperation::CalendarFilter { .. }
                        | RowOperation::ConceptFilter { .. }
                        | RowOperation::OrderOutput { .. }
                        | RowOperation::Limit { .. }
                        | RowOperation::Page { .. }
                )
            })
            || !matches!(
                counts[0].operation,
                RowOperation::Aggregate {
                    function: AggregateFunction::Count,
                    field: None,
                    distinct: false,
                    ..
                }
            )
            || query
                .requirements
                .iter()
                .filter(|requirement| {
                    matches!(requirement.operation, RowOperation::CalendarFill { .. })
                })
                .count()
                != 1
        {
            return Err(diagnostic(
                "calendar_fill_profile",
                "Calendar fill requires one UTC-month group, one COUNT(*), and explicit zero fill",
            ));
        }
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
                | RowOperation::CalendarGroup { .. }
                | RowOperation::Aggregate { .. }
                | RowOperation::Metric { .. }
                | RowOperation::Lookup {
                    usage: LookupUsage::Group,
                    ..
                }
        )
    });
    let mut definitions = Vec::new();
    if let Some(lineage) = relation
        .definition()
        .semantics
        .as_ref()
        .and_then(|semantics| semantics.view_lineage.as_ref())
    {
        let semantic_catalog::RelationKind::View { sql, dependencies } =
            &relation.definition().kind
        else {
            return Err(diagnostic(
                "view_lineage",
                "Authored view lineage requires a view relation",
            ));
        };
        let source = snapshot
            .relation(&lineage.source.id)
            .ok_or_else(|| diagnostic("view_lineage", "Authored view lineage source is missing"))?;
        if source.reference() != &lineage.source
            || dependencies.len() != 1
            || dependencies[0] != lineage.source.id
            || lineage
                .canonical_sql(&relation.definition().schema)
                .as_deref()
                != Some(sql.as_str())
            || lineage.columns.len() != relation.definition().schema.fields().len()
            || relation.definition().schema.fields().iter().any(|output| {
                lineage
                    .columns
                    .get(output.name())
                    .and_then(|name| source.field(name))
                    .is_none_or(|input| {
                        input.data_type() != output.data_type()
                            || input.is_nullable() != output.is_nullable()
                    })
            })
        {
            return Err(diagnostic(
                "view_lineage",
                "Authored view lineage is stale or incompatible with the pinned source",
            ));
        }
        definitions.push(
            relation
                .definition_reference("view_lineage", &relation.definition().name)
                .ok_or_else(|| {
                    diagnostic(
                        "view_lineage",
                        "Authored view lineage revision is unavailable",
                    )
                })?
                .clone(),
        );
    }
    let mut metrics = std::collections::BTreeMap::new();
    let mut ratios = std::collections::BTreeMap::new();
    let mut resolved_metric_names = std::collections::BTreeMap::new();
    let mut output_meanings = BTreeMap::new();
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
            if let RowOperation::Group { field, .. } | RowOperation::CalendarGroup { field, .. } =
                &r.operation
            {
                Some(field.field.as_str())
            } else {
                None
            }
        })
        .collect();
    let temporal_windows = query
        .requirements
        .iter()
        .filter_map(|requirement| {
            let RowOperation::CalendarFilter { field, period } = &requirement.operation else {
                return None;
            };
            Some((requirement, field, period))
        })
        .map(|(requirement, field, period)| {
            in_requirement(&requirement.id, || {
                let field = binder.field(field)?;
                let context = options.request_context.as_ref().ok_or_else(|| {
                    diagnostic(
                        "unresolved_time_context",
                        "Relative periods require a pinned reference instant, timezone and calendar",
                    )
                })?;
                let (start, end, _) =
                    super::temporal::resolve(period, context, field.field.data_type())?;
                Ok(RequestedTemporalWindow {
                    field: field.field.name().clone(),
                    grain: period.unit,
                    start,
                    end,
                })
            })
        })
        .collect::<Result<Vec<_>, CompileDiagnostic>>()?;
    if matches!(
        relation.definition().kind,
        semantic_catalog::RelationKind::View { .. }
    ) {
        let coverage = relation
            .definition()
            .semantics
            .as_ref()
            .and_then(|semantics| semantics.view_coverage.as_ref());
        if let Some(coverage) = coverage {
            if temporal_windows.len() != 1 {
                return Err(diagnostic(
                    "view_coverage_unproven",
                    "A coverage-restricted view requires one explicit calendar interval",
                ));
            }
            let window = &temporal_windows[0];
            coverage
                .contains(&window.field, window.grain, &window.start, &window.end)
                .map_err(|error| {
                    diagnostic(
                        "view_applicability",
                        &format!("Authored view coverage: {error}"),
                    )
                })?;
            definitions.push(
                relation
                    .definition_reference("view_coverage", &relation.definition().name)
                    .expect("published view coverage")
                    .clone(),
            );
        } else if !temporal_windows.is_empty() {
            return Err(diagnostic(
                "view_coverage_unproven",
                "Temporal applicability of this authored view is not declared",
            ));
        }
    }
    let mut output_slots = std::collections::BTreeMap::new();
    for (index, requirement) in query.requirements.iter().enumerate() {
        let output_type = in_requirement(&requirement.id, || {
            Ok(match &requirement.operation {
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
                RowOperation::Metric {
                    name,
                    applicability,
                    ..
                } => {
                    let selected = resolve_governed_metric(
                        snapshot,
                        relation,
                        name,
                        &requirement.source_text,
                        applicability,
                        &temporal_windows,
                        options,
                    )?;
                    let semantics = relation
                        .definition()
                        .semantics
                        .as_ref()
                        .expect("resolved metric belongs to selected relation");
                    match selected {
                        GovernedMetricCandidate::Ratio {
                            name: selected_name,
                            definition: ratio,
                            ..
                        } => {
                            resolved_metric_names.insert(requirement.id.clone(), None);
                            if ratio.id.is_empty() || semantics.metrics.contains_key(selected_name)
                            {
                                return Err(diagnostic(
                                    "metric_contract",
                                    "Governed metric identities must be explicit and unambiguous",
                                ));
                            }
                            validate_ratio_applicability(
                                &relation.definition().name,
                                semantics,
                                ratio,
                                applicability,
                                &temporal_windows,
                            )?;
                            let component_applicability = MetricApplicability {
                                required_unit: None,
                                required_source_grain: applicability.required_source_grain.clone(),
                            };
                            let numerator = binder.metric(
                                &ratio.numerator,
                                &grouping_fields,
                                &lookup_dimensions,
                                &component_applicability,
                                &temporal_windows,
                            )?;
                            let denominator = binder.metric(
                                &ratio.denominator,
                                &grouping_fields,
                                &lookup_dimensions,
                                &component_applicability,
                                &temporal_windows,
                            )?;
                            if numerator.1.0 != DataType::Int64
                                || denominator.1.0 != DataType::Int64
                            {
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
                            output_meanings.insert(
                                requirement.id.clone(),
                                authored_metric_meaning(
                                    &relation.definition().name,
                                    &ratio.id,
                                    &ratio.unit,
                                    &semantics.metrics[&ratio.numerator].source_grain,
                                    &ratio.source_refs,
                                ),
                            );
                            definitions.extend([
                                relation
                                    .definition_reference("ratio", selected_name)
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
                        }
                        GovernedMetricCandidate::Metric {
                            name: selected_name,
                            definition,
                            ..
                        } => {
                            resolved_metric_names
                                .insert(requirement.id.clone(), Some(selected_name.to_owned()));
                            let (input, ty, reference) = binder.metric(
                                selected_name,
                                &grouping_fields,
                                &lookup_dimensions,
                                applicability,
                                &temporal_windows,
                            )?;
                            output_meanings.insert(
                                requirement.id.clone(),
                                authored_metric_meaning(
                                    &relation.definition().name,
                                    &definition.id,
                                    &definition.unit,
                                    &definition.source_grain,
                                    &definition.source_refs,
                                ),
                            );
                            definitions.push(reference);
                            metrics.insert(requirement.id.clone(), input);
                            Some(ty)
                        }
                    }
                }
                RowOperation::Group { field, .. } => {
                    let field = binder.field(field)?;
                    Some((field.field.data_type().clone(), field.field.is_nullable()))
                }
                RowOperation::CalendarGroup {
                    field,
                    grain,
                    timezone,
                    ..
                } => {
                    let bound = binder.field(field)?;
                    validate_utc_month_group(&bound, *grain, timezone, binder.relation)?;
                    let filled = query.requirements.iter().any(|requirement| {
                        matches!(requirement.operation, RowOperation::CalendarFill { .. })
                    });
                    Some((bound.field.data_type().clone(), !filled))
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
            })
        })?;
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
            let (window, ty, nullable) = in_requirement(&requirement.id, || {
                validate_window_rollup(window, query, relation, &resolved_metric_names, options)?;
                validate_cumulative_frame(window, query)?;
                binder.window(window, aggregate_query, &output_slots)
            })?;
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
        let bound = in_requirement(&requirement.id, || {
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
                                || query.requirements.iter().any(|r| {
                                    matches!(r.operation, RowOperation::Window { .. })
                                }) =>
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
                RowOperation::PathLookup {
                    hops,
                    field,
                    alias,
                    missing,
                } => {
                    if aggregate_query {
                        return Err(diagnostic(
                            "path_lookup_grain",
                            "Path lookup currently requires a row query",
                        ));
                    }
                    for hop in hops {
                        if hop.instance.trim().is_empty()
                            || hop.instance.starts_with('$')
                            || !instances.insert(hop.instance.as_str())
                        {
                            return Err(diagnostic(
                                "invalid_scope",
                                "Path occurrences need distinct nonempty instance IDs",
                            ));
                        }
                    }
                    check_alias(alias, &mut aliases)?;
                    let bound = path::bind_path(
                        &mut binder,
                        snapshot,
                        hops,
                        field,
                        *missing,
                        requirements.len(),
                    )?;
                    for lookup in &bound.lookups {
                        definitions.push(lookup.relationship.definition.clone());
                        definitions.extend(lookup.relationship.policies.iter().cloned());
                    }
                    definitions.push(ObjectRef {
                        id: "functions/semantic_assert_single_v1".into(),
                        revision: "1".into(),
                    });
                    BoundOperation::PathLookup {
                        lookups: bound.lookups,
                        alias: alias.clone(),
                    }
                }
                RowOperation::Allocate {
                    allocation,
                    target_aliases,
                    amount_alias,
                } => {
                    if query.requirements.len() != 1 {
                        return Err(diagnostic(
                            "allocation_profile",
                            "The executable allocation profile requires one terminal allocation requirement",
                        ));
                    }
                    if target_aliases.is_empty() || target_aliases.len() > 2 {
                        return Err(diagnostic(
                            "allocation_profile",
                            "Allocation requires one or two typed target aliases",
                        ));
                    }
                    for alias in target_aliases {
                        check_alias(alias, &mut aliases)?;
                    }
                    check_alias(amount_alias, &mut aliases)?;
                    let allocation =
                        allocation::bind_allocation(&mut binder, snapshot, allocation)?;
                    if target_aliases.len() != allocation.targets.len() {
                        return Err(diagnostic(
                            "allocation_profile",
                            "Allocation target aliases must match the authored dimensions",
                        ));
                    }
                    definitions.push(allocation.definition.clone());
                    definitions.extend(allocation.policies.iter().cloned());
                    for function in [
                        "semantic_assert_single_v1",
                        "semantic_allocation_floor_v1",
                        "semantic_allocation_remainder_v1",
                        "semantic_assert_allocation_v1",
                        "semantic_allocation_share_v1",
                        "semantic_sum_v1",
                    ] {
                        definitions.push(ObjectRef {
                            id: format!("functions/{function}"),
                            revision: "1".into(),
                        });
                    }
                    BoundOperation::Allocate {
                        allocation: Box::new(allocation),
                        target_aliases: target_aliases.clone(),
                        amount_alias: amount_alias.clone(),
                    }
                }
                RowOperation::Convert { conversion, alias } => {
                    if aggregate_query {
                        return Err(diagnostic(
                            "conversion_grain",
                            "This conversion profile requires a row query",
                        ));
                    }
                    check_alias(alias, &mut aliases)?;
                    let semantics = relation.definition().semantics.as_ref().ok_or_else(|| {
                        diagnostic(
                            "unknown_conversion",
                            "No conversions are authored on this relation",
                        )
                    })?;
                    let mut matches = semantics.conversions.iter().filter(|(name, rule)| {
                        name.as_str() == conversion || rule.id == *conversion
                    });
                    let (name, rule) = matches.next().ok_or_else(|| {
                        diagnostic(
                            "unknown_conversion",
                            "The exact conversion is not authored on this relation",
                        )
                    })?;
                    if matches.next().is_some() {
                        return Err(diagnostic(
                            "ambiguous_conversion",
                            "This conversion name matches multiple authored definitions",
                        ));
                    }
                    rule.validate().map_err(|_| {
                        diagnostic(
                            "conversion_contract",
                            "Authored conversion contract is invalid",
                        )
                    })?;
                    let source = binder.field(&FieldRef {
                        instance: query.input.instance.clone(),
                        field: rule.field.clone(),
                    })?;
                    if semantics
                        .fields
                        .get(&rule.field)
                        .and_then(|field| field.unit.as_ref())
                        .is_some_and(|unit| unit != &rule.from_unit)
                    {
                        return Err(diagnostic(
                            "conversion_contract",
                            "Authored source field unit disagrees with conversion input unit",
                        ));
                    }
                    if source.field.data_type() != &DataType::Int64 {
                        return Err(diagnostic(
                            "conversion_type",
                            "Exact conversion requires an Int64 source field",
                        ));
                    }
                    let mut output_name = format!("__semantic_conversion_{}", requirements.len());
                    while relation.field(&output_name).is_some() {
                        output_name.push('_');
                    }
                    let output = BoundField {
                        instance: "$output".into(),
                        field: Field::new(output_name, DataType::Decimal128(38, 18), true),
                    };
                    definitions.push(
                        relation
                            .definition_reference("conversion", name)
                            .expect("published conversion")
                            .clone(),
                    );
                    BoundOperation::Convert {
                        conversion: BoundConversion {
                            source,
                            output,
                            numerator: rule.numerator,
                            denominator: rule.denominator,
                            half_even: rule.rounding
                                == semantic_catalog::ConversionRounding::HalfEven,
                            from_unit: rule.from_unit.clone(),
                            to_unit: rule.to_unit.clone(),
                        },
                        alias: alias.clone(),
                    }
                }
                RowOperation::ConvertRate {
                    rate,
                    amount,
                    result_type,
                    rounding,
                    alias,
                } => {
                    if aggregate_query {
                        return Err(diagnostic(
                            "conversion_grain",
                            "Exact rate conversion requires a row query",
                        ));
                    }
                    check_alias(alias, &mut aliases)?;
                    let semantics = relation.definition().semantics.as_ref().ok_or_else(|| {
                        diagnostic("unknown_conversion", "No exact rate profiles are authored")
                    })?;
                    let mut matches = semantics
                        .exact_decimal_rates
                        .iter()
                        .filter(|(name, rule)| name.as_str() == rate || rule.id == *rate);
                    let (name, rule) = matches.next().ok_or_else(|| {
                        diagnostic(
                            "unknown_conversion",
                            "Exact rate profile is not authored on this relation",
                        )
                    })?;
                    if matches.next().is_some() {
                        return Err(diagnostic(
                            "ambiguous_conversion",
                            "Exact rate profile has competing identities",
                        ));
                    }
                    rule.validate(snapshot, options.allowed_relations.as_ref())
                        .map_err(|_| {
                            diagnostic("conversion_contract", "Invalid exact decimal rate contract")
                        })?;
                    if rule.rate_relation != query.input.relation
                        || result_type.precision == 0
                        || result_type.precision > 38
                        || result_type.scale > result_type.precision
                    {
                        return Err(diagnostic(
                            "conversion_contract",
                            "Exact rate relation or result representation is invalid",
                        ));
                    }
                    let RateAmount::Literal { value, .. } = amount;
                    if !matches!(value, Literal::Decimal128 { .. }) {
                        return Err(diagnostic(
                            "conversion_type",
                            "Exact rate amount requires a decimal literal in major source currency units",
                        ));
                    }
                    super::literal::checked_scalar(value)?;
                    for field in [&rule.source_currency_field, &rule.date_field] {
                        binder.field(&FieldRef {
                            instance: query.input.instance.clone(),
                            field: field.clone(),
                        })?;
                    }
                    let source = binder.field(&FieldRef {
                        instance: query.input.instance.clone(),
                        field: rule.rate_field.clone(),
                    })?;
                    let mut output_name = format!("__semantic_rate_{}", requirements.len());
                    while relation.field(&output_name).is_some() {
                        output_name.push('_');
                    }
                    let output = BoundField {
                        instance: "$output".into(),
                        field: Field::new(
                            output_name,
                            DataType::Decimal128(result_type.precision, result_type.scale as i8),
                            true,
                        ),
                    };
                    definitions.push(
                        relation
                            .definition_reference("exact_decimal_rate", name)
                            .expect("published exact rate")
                            .clone(),
                    );
                    BoundOperation::ConvertRate {
                        conversion: BoundRateConversion {
                            source,
                            output,
                            amount: value.clone(),
                            result_type: *result_type,
                            rounding: *rounding,
                            target_currency: rule.target_currency.clone(),
                            profile: rule.reference(),
                            source_refs: rule.source_refs.clone(),
                        },
                        alias: alias.clone(),
                    }
                }
                RowOperation::CurrencyConvert { rate, alias } => {
                    if query.requirements.len() != 1 {
                        return Err(diagnostic(
                            "currency_rate_profile",
                            "The executable currency rate profile requires one terminal requirement",
                        ));
                    }
                    check_alias(alias, &mut aliases)?;
                    let rate = currency_rate::bind_currency_rate(&mut binder, snapshot, rate)?;
                    definitions.push(rate.definition.clone());
                    definitions.push(rate.rate_relation.clone());
                    definitions.extend(rate.policies.iter().cloned());
                    for function in ["semantic_assert_exactly_one_v1", "semantic_scale_i64_v1"] {
                        definitions.push(ObjectRef {
                            id: format!("functions/{function}"),
                            revision: "1".into(),
                        });
                    }
                    BoundOperation::CurrencyConvert {
                        rate: Box::new(rate),
                        alias: alias.clone(),
                    }
                }
                RowOperation::BusinessCalendar {
                    calendar,
                    field,
                    alias,
                } => {
                    if query.requirements.len() != 1 {
                        return Err(diagnostic(
                            "business_calendar_profile",
                            "The executable business calendar profile requires one terminal requirement",
                        ));
                    }
                    check_alias(alias, &mut aliases)?;
                    let calendar = business_calendar::bind_business_calendar(
                        &mut binder,
                        snapshot,
                        calendar,
                        *field,
                    )?;
                    definitions.push(calendar.definition.clone());
                    definitions.push(calendar.calendar_relation.clone());
                    definitions.extend(calendar.policies.iter().cloned());
                    definitions.push(ObjectRef {
                        id: "functions/semantic_assert_exactly_one_v1".into(),
                        revision: "1".into(),
                    });
                    if calendar.source_basis
                        == semantic_catalog::CalendarSourceBasis::UtcInstantMicros
                    {
                        definitions.push(ObjectRef {
                            id: "functions/semantic_local_date_us_v1".into(),
                            revision: "1-chrono-tz-0.10.4".into(),
                        });
                    }
                    BoundOperation::BusinessCalendar {
                        calendar: Box::new(calendar),
                        alias: alias.clone(),
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
                    target_requirements,
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
                    let mut relationship = binder.relationship(
                        snapshot,
                        relationship,
                        role,
                        instance,
                        *mode,
                        predicate.as_ref(),
                    )?;
                    if !target_requirements.is_empty() {
                        let mut target_options = options.clone();
                        target_options.max_depth =
                            target_options.max_depth.checked_sub(1).ok_or_else(|| {
                                diagnostic("work_limit", "Related depth budget exhausted")
                            })?;
                        let target = bind_scoped(
                            snapshot,
                            &RowQuery {
                                version: ROW_QUERY_VERSION,
                                input: RelationInput {
                                    relation: relationship.right.id.clone(),
                                    instance: instance.clone(),
                                },
                                requirements: target_requirements.clone(),
                                unresolved: vec![],
                            },
                            &target_options,
                            binder.work,
                            true,
                        )?;
                        definitions.push(target.input.clone());
                        definitions.extend(target.definitions.iter().cloned());
                        relationship.target = Some(Box::new(target));
                    }
                    definitions.push(relationship.definition.clone());
                    definitions.push(relationship.right.clone());
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
                            mean_state: input.mean_state,
                            weighted: input.weighted,
                            exact_distinct: input.exact_distinct,
                            snapshot: input.snapshot,
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
                RowOperation::CalendarGroup {
                    field,
                    grain,
                    timezone,
                    alias,
                } => {
                    check_alias(alias, &mut aliases)?;
                    let source = binder.field(field)?;
                    validate_utc_month_group(&source, *grain, timezone, binder.relation)?;
                    BoundOperation::CalendarGroup {
                        source,
                        output: output_slots[&requirement.id].clone(),
                        alias: alias.clone(),
                    }
                }
                RowOperation::CalendarFill {
                    month_slot,
                    count_slot,
                    start_us,
                    end_us,
                    fill,
                } => BoundOperation::CalendarFill {
                    month: output_slots[month_slot].clone(),
                    count: output_slots[count_slot].clone(),
                    months: super::calendar_spine::utc_months(
                        *start_us,
                        *end_us,
                        options.max_nodes,
                    )?,
                    fill: *fill,
                },
                RowOperation::Aggregate {
                    function,
                    field,
                    distinct,
                    alias,
                } => {
                    check_alias(alias, &mut aliases)?;
                    BoundOperation::Aggregate {
                        function: *function,
                        mean_state: false,
                        weighted: None,
                        exact_distinct: false,
                        snapshot: None,
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
                RowOperation::ConceptFilter { concept, arguments } => {
                    let semantics = relation.definition().semantics.as_ref().ok_or_else(|| {
                        diagnostic(
                            "unknown_concept",
                            "No concepts are authored on this relation",
                        )
                    })?;
                    // A durable ID is an explicit choice. Names and aliases
                    // expand the authored alternative set and require a
                    // single remaining definition; no predicate implication
                    // or equivalence is inferred.
                    let mut explicit = semantics
                        .concepts
                        .iter()
                        .filter(|(_, definition)| definition.id == *concept);
                    let selected = explicit.next();
                    if explicit.next().is_some() {
                        return Err(diagnostic(
                            "ambiguous_concept",
                            "This concept ID matches multiple authored definitions",
                        ));
                    }
                    let (name, definition) = if let Some(selected) = selected {
                        selected
                    } else {
                        let mut candidates = BTreeSet::new();
                        for (name, definition) in &semantics.concepts {
                            if name == concept
                                || definition.aliases.iter().any(|alias| alias == concept)
                            {
                                candidates.insert(name.as_str());
                                for alternative in &definition.alternatives {
                                    if !semantics.concepts.contains_key(alternative) {
                                        return Err(diagnostic(
                                            "ambiguous_concept",
                                            "An authored concept alternative is unavailable",
                                        ));
                                    }
                                    candidates.insert(alternative.as_str());
                                }
                            }
                        }
                        let name = candidates.iter().next().ok_or_else(|| {
                            diagnostic(
                                "unknown_concept",
                                "The exact concept is not authored on this relation",
                            )
                        })?;
                        if candidates.len() != 1 {
                            return Err(diagnostic(
                                "ambiguous_concept",
                                "This concept name has competing authored definitions",
                            ));
                        }
                        semantics
                            .concepts
                            .get_key_value(*name)
                            .expect("selected authored concept")
                    };
                    definitions.push(
                        relation
                            .definition_reference("concept", name)
                            .expect("indexed concept")
                            .clone(),
                    );
                    let authored = substitute_concept_arguments(&definition.predicate, arguments)?;
                    let predicate = binder.predicate_with(&authored, 1, &|binder, field| {
                        binder.field(&FieldRef {
                            instance: query.input.instance.clone(),
                            field: field.clone(),
                        })
                    })?;
                    BoundOperation::Filter { predicate }
                }
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
                            | DataType::Float64
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
                RowOperation::Page { offset, fetch } => {
                    if limited {
                        return Err(diagnostic(
                            "conflicting_limits",
                            "A query can have only one pagination requirement",
                        ));
                    }
                    if *offset > options.max_page_offset
                        || *fetch > options.max_page_fetch
                        || offset.checked_add(*fetch).is_none()
                    {
                        return Err(diagnostic(
                            "page_limit",
                            "Requested offset/fetch exceeds the bounded pagination profile",
                        ));
                    }
                    limited = true;
                    BoundOperation::Page {
                        offset: *offset,
                        fetch: *fetch,
                    }
                }
            };
            let authored_field = match &operation {
                BoundOperation::Project { field, .. } | BoundOperation::Group { field, .. } => {
                    field_meaning(relation, field.field.name())
                }
                BoundOperation::Lookup { lookup, .. } => snapshot
                    .relation(&lookup.relationship.right.id)
                    .and_then(|right| field_meaning(right, lookup.value.field.name())),
                BoundOperation::ConvertRate { conversion, .. } => Some(SlotMeaning {
                    unit: FactResolution::Known {
                        value: Presence::Value(Unit::Currency {
                            code: conversion.target_currency.clone(),
                        }),
                        contributors: vec![Fact {
                            id: conversion.profile.id.clone(),
                            scope: query.input.relation.clone(),
                            value: Presence::Value(Unit::Currency {
                                code: conversion.target_currency.clone(),
                            }),
                            authority: Authority::Authored,
                            origins: conversion.source_refs.clone(),
                            evidence: vec![],
                        }],
                    },
                    source_grain: FactResolution::Unknown,
                    entity: FactResolution::Unknown,
                }),
                _ => None,
            };
            if let Some(meaning) = authored_field {
                output_meanings.insert(requirement.id.clone(), meaning);
            }
            Ok(BoundRequirement {
                id: requirement.id.clone(),
                operation,
            })
        })?;
        requirements.push(bound);
    }
    if aliases.is_empty() && !target_scope {
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
        .any(|r| matches!(r.operation, BoundOperation::Convert { .. }))
    {
        definitions.push(ObjectRef {
            id: "functions/semantic_scale_i64_v1".into(),
            revision: "1".into(),
        });
    }
    if requirements
        .iter()
        .any(|r| matches!(r.operation, BoundOperation::ConvertRate { .. }))
    {
        definitions.push(ObjectRef {
            id: "functions/semantic_decimal_rate_v1".into(),
            revision: "2".into(),
        });
    }
    if requirements
        .iter()
        .any(|r| matches!(r.operation, BoundOperation::CalendarGroup { .. }))
    {
        definitions.push(ObjectRef {
            id: "functions/semantic_utc_month_us_v1".into(),
            revision: "1".into(),
        });
    }
    if requirements
        .iter()
        .any(|r| matches!(r.operation, BoundOperation::CalendarFill { .. }))
    {
        definitions.push(ObjectRef {
            id: "functions/calendar_fill_utc_month_count_v1".into(),
            revision: "1".into(),
        });
    }
    if requirements
        .iter()
        .any(|requirement| match &requirement.operation {
            BoundOperation::Aggregate {
                function,
                field,
                mean_state,
                weighted,
                exact_distinct,
                snapshot,
                ..
            } => {
                *function == AggregateFunction::Sum
                    && field
                        .as_ref()
                        .is_some_and(|field| field.field.data_type() != &DataType::Float64)
                    && !mean_state
                    && weighted.is_none()
                    && !exact_distinct
                    && snapshot.is_none()
            }
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
    if requirements.iter().any(|requirement| {
        matches!(
            &requirement.operation,
            BoundOperation::Aggregate {
                mean_state: true,
                ..
            }
        )
            || matches!(
                &requirement.operation,
                BoundOperation::Aggregate {
                    function: AggregateFunction::Avg,
                    field: Some(field),
                    ..
                } if matches!(field.field.data_type(), DataType::Int16 | DataType::Int32 | DataType::Int64)
            )
    }) {
        definitions.push(ObjectRef {
            id: "functions/semantic_mean_i64_v1".into(),
            revision: "1".into(),
        });
    }
    if requirements.iter().any(|requirement| {
        matches!(
            requirement.operation,
            BoundOperation::Aggregate {
                weighted: Some(_),
                ..
            }
        )
    }) {
        definitions.push(ObjectRef {
            id: "functions/semantic_weighted_mean_i64_v1".into(),
            revision: "1".into(),
        });
    }
    if requirements.iter().any(|requirement| {
        matches!(
            requirement.operation,
            BoundOperation::Aggregate {
                exact_distinct: true,
                ..
            }
        )
    }) {
        definitions.push(ObjectRef {
            id: "functions/semantic_exact_count_i64_v1".into(),
            revision: "1".into(),
        });
    }
    if requirements.iter().any(|requirement| {
        matches!(
            requirement.operation,
            BoundOperation::Aggregate {
                snapshot: Some(_),
                ..
            }
        )
    }) {
        definitions.push(ObjectRef {
            id: "functions/semantic_snapshot_balance_i64_v1".into(),
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
            BoundOperation::PathLookup { lookups, .. } => {
                for lookup in lookups {
                    predicates.extend(&lookup.relationship.predicate);
                }
            }
            BoundOperation::Allocate { allocation, .. } => {
                predicates.extend(&allocation.predicate);
            }
            BoundOperation::CurrencyConvert { rate, .. } => predicates.extend(&rate.predicate),
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
        output_meanings,
    })
}

fn in_requirement<T>(
    id: &str,
    bind: impl FnOnce() -> Result<T, CompileDiagnostic>,
) -> Result<T, CompileDiagnostic> {
    bind().map_err(|mut error| {
        // Preserve a more precise reference supplied by a nested binder. An
        // empty or duplicate requirement identity cannot safely name one
        // requested operation.
        if error.details.requirement_ref.is_none()
            && !id.trim().is_empty()
            && error.code != "invalid_requirement"
        {
            error.details.requirement_ref = Some(id.to_owned());
        }
        error
    })
}

fn field_meaning(relation: &SnapshotRelation, field: &str) -> Option<SlotMeaning> {
    let authored = relation
        .definition()
        .semantics
        .as_ref()?
        .fields
        .get(field)?;
    let unit = authored.unit.clone()?;
    let typed = Presence::Value(unit);
    Some(SlotMeaning {
        unit: FactResolution::Known {
            value: typed.clone(),
            contributors: vec![Fact {
                id: format!("field/{field}/unit"),
                scope: relation.definition().name.clone(),
                value: typed,
                authority: Authority::Authored,
                origins: authored.source_refs.clone(),
                evidence: vec![],
            }],
        },
        source_grain: FactResolution::Unknown,
        entity: FactResolution::Unknown,
    })
}

fn authored_metric_meaning(
    relation: &str,
    id: &str,
    unit: &Presence<Unit>,
    source_grain: &SourceGrain,
    source_refs: &[SourceRef],
) -> SlotMeaning {
    let unit = match unit {
        Presence::Missing => FactResolution::Unknown,
        Presence::Null | Presence::Value(_) => {
            let typed = unit.clone();
            FactResolution::Known {
                value: typed.clone(),
                contributors: vec![Fact {
                    id: format!("{id}/unit"),
                    scope: relation.into(),
                    value: typed,
                    authority: Authority::Authored,
                    origins: source_refs.to_vec(),
                    evidence: vec![],
                }],
            }
        }
    };
    let typed_grain = Presence::Value(source_grain.clone());
    SlotMeaning {
        unit,
        source_grain: FactResolution::Known {
            value: typed_grain.clone(),
            contributors: vec![Fact {
                id: format!("{id}/source_grain"),
                scope: relation.into(),
                value: typed_grain,
                authority: Authority::Authored,
                origins: source_refs.to_vec(),
                evidence: vec![],
            }],
        },
        entity: FactResolution::Unknown,
    }
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
            extra_value: None,
            missing,
            obligation: "same-query/grouped-right-key-count-at-most-one/v1",
            as_of: None,
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
                let mut error = diagnostic(
                    "unknown_relationship",
                    "Only an authored relationship can authorize related-row queries",
                );
                error.details.object_ref = Some(format!(
                    "relationship/{}/{}",
                    self.relation.definition().name,
                    relationship
                ));
                error
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
        if !super::allowed(&relationship.right_relation, self.options) {
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
            let left_system = self
                .relation
                .definition()
                .semantics
                .as_ref()
                .and_then(|semantics| semantics.fields.get(&key.left_field))
                .and_then(|field| field.reference_system.as_ref());
            let right_system = right_binder
                .relation
                .definition()
                .semantics
                .as_ref()
                .and_then(|semantics| semantics.fields.get(&key.right_field))
                .and_then(|field| field.reference_system.as_ref());
            if !semantic_catalog::reference_systems_compatible(left_system, right_system) {
                return Err(diagnostic(
                    "relationship_reference_system",
                    "Relationship keys have different or incomplete authored reference systems",
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
            target: None,
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
        if (rank || spec.frame != WindowFrame::EntirePartition) && order_by.is_empty() {
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
        applicability: &MetricApplicability,
        temporal_windows: &[RequestedTemporalWindow],
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
        if metric.id.is_empty() || metric.source_grain.keys.is_empty() {
            return Err(diagnostic(
                "metric_contract",
                "Metric requires a durable identity and explicit source grain",
            ));
        }
        validate_metric_applicability(
            &self.relation.definition().name,
            metric,
            applicability,
            temporal_windows,
        )?;
        if metric.source_grain.keys.len() > self.options.max_nodes
            || metric.row_filters.len() > self.options.max_nodes
        {
            return Err(diagnostic(
                "work_limit",
                "Metric contract exceeds the work budget",
            ));
        }
        for key in &metric.source_grain.keys {
            self.field(&FieldRef {
                instance: self.instance.into(),
                field: key.field.clone(),
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
        let mean_state = matches!(
            metric.state.as_ref().map(|state| &state.state),
            Some(semantic_catalog::MetricStateKind::SumCountAverage)
        );
        let weighted = match metric.state.as_ref().map(|state| &state.state) {
            Some(semantic_catalog::MetricStateKind::WeightedAverage { weight_field, zero }) => {
                Some(BoundWeightedState {
                    weight: self.field(&FieldRef {
                        instance: self.instance.into(),
                        field: weight_field.clone(),
                    })?,
                    zero: *zero,
                })
            }
            _ => None,
        };
        let exact_distinct = match metric.state.as_ref().map(|state| &state.state) {
            Some(semantic_catalog::MetricStateKind::ExactDistinct { identity_fields }) => {
                if identity_fields.len() != 1
                    || metric.field.as_deref() != Some(identity_fields[0].as_str())
                    || metric.function != AggregateFunction::Count
                    || metric.distinct
                    || field
                        .as_ref()
                        .is_none_or(|field| *field.field.data_type() != DataType::Int64)
                    || metric.result_type != DataType::Int64
                    || metric.empty_behavior != semantic_catalog::EmptyBehavior::Zero
                {
                    return Err(diagnostic(
                        "unsupported_metric_state",
                        "Exact distinct execution requires one authored Int64 identity and Count result",
                    ));
                }
                true
            }
            _ => false,
        };
        let snapshot = match metric.state.as_ref().map(|state| &state.state) {
            Some(semantic_catalog::MetricStateKind::SnapshotBalance {
                time_field,
                tie_break_fields,
            }) => {
                if tie_break_fields.len() != 1
                    || metric.function != AggregateFunction::Max
                    || metric.distinct
                    || field
                        .as_ref()
                        .is_none_or(|field| *field.field.data_type() != DataType::Int64)
                    || metric.result_type != DataType::Int64
                    || metric.empty_behavior != semantic_catalog::EmptyBehavior::Null
                    || metric.state.as_ref().is_some_and(|state| {
                        grouping_fields
                            .iter()
                            .any(|field| !state.merge_dimensions.contains(*field))
                    })
                {
                    return Err(diagnostic(
                        "unsupported_metric_state",
                        "Snapshot balance requires a non-additive Int64 latest-value profile",
                    ));
                }
                let time = self.field(&FieldRef {
                    instance: self.instance.into(),
                    field: time_field.clone(),
                })?;
                let tie = self.field(&FieldRef {
                    instance: self.instance.into(),
                    field: tie_break_fields[0].clone(),
                })?;
                if time.field.is_nullable()
                    || tie.field.is_nullable()
                    || *time.field.data_type()
                        != DataType::Timestamp(
                            datafusion::arrow::datatypes::TimeUnit::Microsecond,
                            Some("UTC".into()),
                        )
                    || *tie.field.data_type() != DataType::Int64
                {
                    return Err(diagnostic(
                        "unsupported_metric_state",
                        "Snapshot balance requires non-null UTC microsecond time and Int64 tie fields",
                    ));
                }
                Some(BoundSnapshotState { time, tie })
            }
            _ => None,
        };
        if mean_state
            && (metric.function != AggregateFunction::Sum
                || metric.distinct
                || field
                    .as_ref()
                    .is_none_or(|field| *field.field.data_type() != DataType::Int64)
                || metric.result_type != DataType::Decimal128(38, 18))
        {
            return Err(diagnostic(
                "metric_contract",
                "Exact mean requires a non-distinct Int64 input and Decimal128(38,18) result",
            ));
        }
        if weighted.as_ref().is_some_and(|weighted| {
            metric.function != AggregateFunction::Sum
                || metric.distinct
                || field
                    .as_ref()
                    .is_none_or(|field| *field.field.data_type() != DataType::Int64)
                || *weighted.weight.field.data_type() != DataType::Int64
                || metric.result_type != DataType::Decimal128(38, 18)
        }) {
            return Err(diagnostic(
                "metric_contract",
                "Exact weighted mean requires non-distinct Int64 value and weight fields",
            ));
        }
        let ty = if mean_state {
            (DataType::Decimal128(38, 18), true)
        } else if let Some(weighted) = &weighted {
            (
                DataType::Decimal128(38, 18),
                weighted.zero != semantic_catalog::ZeroWeight::Zero,
            )
        } else if exact_distinct || snapshot.is_some() {
            (DataType::Int64, true)
        } else {
            aggregate_type(metric.function, field.as_ref(), metric.distinct)?
        };
        let empty = if weighted
            .as_ref()
            .is_some_and(|weighted| weighted.zero == semantic_catalog::ZeroWeight::Zero)
            || (metric.function == AggregateFunction::Count && !mean_state)
        {
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
                mean_state,
                weighted,
                exact_distinct,
                snapshot,
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
            mean_state: false,
            weighted: None,
            exact_distinct: false,
            snapshot: None,
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
    fn check_comparison_profile(&self, field: &BoundField) -> Result<(), CompileDiagnostic> {
        if field.instance != self.instance {
            return Ok(());
        }
        match self
            .relation
            .definition()
            .semantics
            .as_ref()
            .and_then(|semantics| semantics.fields.get(field.field.name()))
            .and_then(|semantics| semantics.comparison_profile.as_ref())
        {
            None | Some(semantic_catalog::ComparisonProfile::BinaryExact) => Ok(()),
            Some(semantic_catalog::ComparisonProfile::Locale { .. }) => Err(diagnostic(
                "comparison_profile",
                "This field requires a checked locale comparison profile before it can be compared",
            )),
        }
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
                self.check_comparison_profile(&field)?;
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
                let field_domain = self
                    .relation
                    .definition()
                    .semantics
                    .as_ref()
                    .and_then(|semantics| semantics.fields.get(field.field.name()))
                    .and_then(|field| field.enum_domain.as_ref());
                if !semantic_catalog::enum_domains_compatible(
                    field_domain,
                    definition.enum_domain.as_ref(),
                ) {
                    return Err(diagnostic(
                        "enum_domain",
                        "The authored mapping and field have different or incomplete enum domains",
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
                self.check_comparison_profile(&field)?;
                if matches!(value, Literal::Utf8(_))
                    && field.instance == self.instance
                    && self
                        .relation
                        .definition()
                        .semantics
                        .as_ref()
                        .and_then(|semantics| semantics.fields.get(field.field.name()))
                        .is_some_and(|field| field.enum_domain.is_some())
                {
                    return Err(diagnostic(
                        "enum_literal",
                        "An authored enum field requires an exact domain-bound value mapping",
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
            RowPredicate::CompareParameter { .. } => {
                return Err(diagnostic(
                    "unbound_parameter",
                    "A comparison parameter must be bound before compilation",
                ));
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

fn resolve_governed_metric<'a>(
    snapshot: &'a CatalogSnapshot,
    selected_relation: &'a SnapshotRelation,
    selector: &str,
    source_text: &str,
    requested: &MetricApplicability,
    temporal_windows: &[RequestedTemporalWindow],
    options: &CompileOptions,
) -> Result<GovernedMetricCandidate<'a>, CompileDiagnostic> {
    if selector.trim().is_empty() {
        return Err(diagnostic(
            "unknown_metric",
            "A governed metric selector must be nonempty",
        ));
    }

    // Ground against the exact request evidence before trusting a model-selected
    // catalog handle. This deliberately narrow rule does not attempt fuzzy
    // phrase interpretation, but an exact authored ambiguity cannot disappear
    // merely because the proposal picked one canonical name or identity.
    let grounded = collect_metric_candidates(snapshot, source_text, false, options)?;
    if !grounded.is_empty() {
        let grounded = choose_metric_candidate(grounded, requested, temporal_windows)?;
        let selected = collect_metric_candidates(snapshot, selector, false, options)?;
        if !selected
            .iter()
            .copied()
            .any(|candidate| same_metric_candidate(candidate, grounded))
        {
            return Err(diagnostic(
                "metric_grounding",
                "The selected governed definition does not match the exact authored request label",
            ));
        }
        return ensure_metric_relation(grounded, selected_relation);
    }

    // A durable identity can bypass label ambiguity when the source phrase did
    // not itself exactly match an authored label, but the identity must be unique
    // throughout the allowed catalog scope.
    let identities = collect_metric_candidates(snapshot, selector, true, options)?;
    match identities.as_slice() {
        [candidate] => return ensure_metric_relation(*candidate, selected_relation),
        [] => {}
        _ => {
            return Err(diagnostic(
                "metric_identity",
                "A durable metric identity resolves to multiple governed definitions",
            ));
        }
    }

    // Canonical display names and aliases have equal authority. Search ranking
    // is deliberately absent from this decision.
    let candidates = collect_metric_candidates(snapshot, selector, false, options)?;
    ensure_metric_relation(
        choose_metric_candidate(candidates, requested, temporal_windows)?,
        selected_relation,
    )
}

fn choose_metric_candidate<'a>(
    candidates: Vec<GovernedMetricCandidate<'a>>,
    requested: &MetricApplicability,
    temporal_windows: &[RequestedTemporalWindow],
) -> Result<GovernedMetricCandidate<'a>, CompileDiagnostic> {
    if candidates.is_empty() {
        return Err(diagnostic(
            "unknown_metric",
            "No exact authored metric name, alias or identity matches the selector",
        ));
    }
    if candidates.len() == 1 {
        return Ok(candidates[0]);
    }
    let applicable = candidates
        .into_iter()
        .filter(|candidate| {
            candidate_applicability(*candidate, requested, temporal_windows).is_ok()
        })
        .collect::<Vec<_>>();
    match applicable.as_slice() {
        [candidate] => Ok(*candidate),
        [] => Err(diagnostic(
            "metric_applicability",
            "No competing governed definition satisfies the exact requested applicability",
        )),
        _ => Err(diagnostic(
            "ambiguous_metric",
            "Multiple governed definitions match the selector and exact applicability; use a durable metric identity",
        )),
    }
}

fn same_metric_candidate(
    left: GovernedMetricCandidate<'_>,
    right: GovernedMetricCandidate<'_>,
) -> bool {
    match (left, right) {
        (
            GovernedMetricCandidate::Metric {
                relation: left_relation,
                name: left_name,
                ..
            },
            GovernedMetricCandidate::Metric {
                relation: right_relation,
                name: right_name,
                ..
            },
        )
        | (
            GovernedMetricCandidate::Ratio {
                relation: left_relation,
                name: left_name,
                ..
            },
            GovernedMetricCandidate::Ratio {
                relation: right_relation,
                name: right_name,
                ..
            },
        ) => std::ptr::eq(left_relation, right_relation) && left_name == right_name,
        _ => false,
    }
}

fn collect_metric_candidates<'a>(
    snapshot: &'a CatalogSnapshot,
    selector: &str,
    identity_only: bool,
    options: &CompileOptions,
) -> Result<Vec<GovernedMetricCandidate<'a>>, CompileDiagnostic> {
    let mut candidates = Vec::new();
    let mut relations_visited = 0usize;
    for relation in snapshot.relations() {
        options.check()?;
        if !super::allowed(&relation.definition().name, options) {
            continue;
        }
        relations_visited = relations_visited.saturating_add(1);
        if relations_visited > options.max_index_objects {
            return Err(diagnostic(
                "work_limit",
                "Metric alternative resolution exceeded the relation budget",
            ));
        }
        candidates.extend(metric_candidates(relation, selector, identity_only));
        if candidates.len() > options.max_nodes {
            return Err(diagnostic(
                "work_limit",
                "Metric alternative resolution exceeded the candidate budget",
            ));
        }
    }
    Ok(candidates)
}

fn metric_candidates<'a>(
    relation: &'a SnapshotRelation,
    selector: &str,
    identity_only: bool,
) -> Vec<GovernedMetricCandidate<'a>> {
    let Some(semantics) = &relation.definition().semantics else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    for (name, definition) in &semantics.metrics {
        let matches = definition.id == selector
            || (!identity_only
                && (name == selector || definition.aliases.iter().any(|a| a == selector)));
        if matches {
            candidates.push(GovernedMetricCandidate::Metric {
                relation,
                name,
                definition,
            });
        }
    }
    for (name, definition) in &semantics.ratio_metrics {
        let matches = definition.id == selector
            || (!identity_only
                && (name == selector || definition.aliases.iter().any(|a| a == selector)));
        if matches {
            candidates.push(GovernedMetricCandidate::Ratio {
                relation,
                name,
                definition,
            });
        }
    }
    candidates
}

fn ensure_metric_relation<'a>(
    candidate: GovernedMetricCandidate<'a>,
    selected_relation: &SnapshotRelation,
) -> Result<GovernedMetricCandidate<'a>, CompileDiagnostic> {
    if std::ptr::eq(candidate.relation(), selected_relation) {
        Ok(candidate)
    } else {
        Err(diagnostic(
            "metric_scope",
            "The matching governed definition belongs to a different relation; select it explicitly",
        ))
    }
}

fn candidate_applicability(
    candidate: GovernedMetricCandidate<'_>,
    requested: &MetricApplicability,
    temporal_windows: &[RequestedTemporalWindow],
) -> Result<(), CompileDiagnostic> {
    match candidate {
        GovernedMetricCandidate::Metric {
            relation,
            definition,
            ..
        } => validate_metric_applicability(
            &relation.definition().name,
            definition,
            requested,
            temporal_windows,
        ),
        GovernedMetricCandidate::Ratio {
            relation,
            definition,
            ..
        } => validate_ratio_applicability(
            &relation.definition().name,
            relation
                .definition()
                .semantics
                .as_ref()
                .expect("ratio candidate has semantics"),
            definition,
            requested,
            temporal_windows,
        ),
    }
}

fn validate_ratio_applicability(
    relation: &str,
    semantics: &semantic_catalog::RelationSemantics,
    ratio: &semantic_catalog::RatioDefinition,
    requested: &MetricApplicability,
    temporal_windows: &[RequestedTemporalWindow],
) -> Result<(), CompileDiagnostic> {
    validate_required_unit(&ratio.unit, requested.required_unit.as_ref())?;
    let component_request = MetricApplicability {
        required_unit: None,
        required_source_grain: requested.required_source_grain.clone(),
    };
    for name in [&ratio.numerator, &ratio.denominator] {
        let component = semantics.metrics.get(name).ok_or_else(|| {
            diagnostic(
                "metric_contract",
                "Governed ratio depends on a missing aggregate metric",
            )
        })?;
        validate_metric_applicability(relation, component, &component_request, temporal_windows)?;
    }
    Ok(())
}

fn validate_metric_applicability(
    _relation: &str,
    metric: &semantic_catalog::MetricDefinition,
    requested: &MetricApplicability,
    temporal_windows: &[RequestedTemporalWindow],
) -> Result<(), CompileDiagnostic> {
    if metric
        .state
        .as_ref()
        .is_some_and(|state| state.validate().is_err())
    {
        return Err(diagnostic(
            "metric_contract",
            "Versioned metric state contract is invalid",
        ));
    }
    if metric.state.as_ref().is_some_and(|state| {
        !matches!(
            state.state,
            semantic_catalog::MetricStateKind::SumCountAverage
                | semantic_catalog::MetricStateKind::WeightedAverage { .. }
                | semantic_catalog::MetricStateKind::ExactDistinct { .. }
                | semantic_catalog::MetricStateKind::SnapshotBalance { .. }
        )
    }) {
        return Err(diagnostic(
            "unsupported_metric_state",
            "This metric state cannot execute until its checked merge and finalize profile is installed",
        ));
    }
    validate_required_unit(&metric.unit, requested.required_unit.as_ref())?;
    if requested
        .required_source_grain
        .as_ref()
        .is_some_and(|requested| requested != &metric.source_grain)
    {
        return Err(diagnostic(
            "metric_grain",
            "Requested source grain does not exactly match the governed metric",
        ));
    }
    match &metric.temporal {
        semantic_catalog::Presence::Null => Ok(()),
        semantic_catalog::Presence::Missing if temporal_windows.is_empty() => Ok(()),
        semantic_catalog::Presence::Missing => Err(diagnostic(
            "metric_applicability",
            "Metric temporal applicability is unknown for the requested calendar filter",
        )),
        semantic_catalog::Presence::Value(contract) => {
            let matching: Vec<_> = temporal_windows
                .iter()
                .filter(|window| window.field == contract.field)
                .collect();
            if matching.len() != 1 {
                return Err(diagnostic(
                    "metric_applicability",
                    "Restricted metrics require exactly one calendar filter on their governed time field",
                ));
            }
            let requested = matching[0];
            if requested.grain != contract.grain {
                return Err(diagnostic(
                    "metric_time_grain",
                    "Requested calendar grain does not match the governed metric",
                ));
            }
            if !coverage_contains(
                &contract.coverage_start,
                &contract.coverage_end,
                &requested.start,
                &requested.end,
            ) {
                return Err(diagnostic(
                    "metric_coverage",
                    "Requested calendar interval is outside the governed metric coverage",
                ));
            }
            Ok(())
        }
    }
}

fn validate_required_unit(
    actual: &semantic_catalog::Presence<Unit>,
    requested: Option<&Unit>,
) -> Result<(), CompileDiagnostic> {
    let Some(requested) = requested else {
        return Ok(());
    };
    if !semantic_catalog::valid_unit(requested) {
        return Err(diagnostic(
            "metric_unit",
            "Requested metric units must be valid exact typed values",
        ));
    }
    if !matches!(actual, semantic_catalog::Presence::Value(actual) if actual == requested) {
        return Err(diagnostic(
            "metric_unit",
            "Requested metric unit does not exactly match the governed definition",
        ));
    }
    Ok(())
}

fn coverage_contains(
    available_start: &Literal,
    available_end: &Literal,
    requested_start: &Literal,
    requested_end: &Literal,
) -> bool {
    match (
        available_start,
        available_end,
        requested_start,
        requested_end,
    ) {
        (
            Literal::Date32(available_start),
            Literal::Date32(available_end),
            Literal::Date32(requested_start),
            Literal::Date32(requested_end),
        ) => available_start <= requested_start && requested_end <= available_end,
        (
            Literal::Timestamp {
                ticks: available_start,
                unit: available_start_unit,
                timezone: available_start_zone,
            },
            Literal::Timestamp {
                ticks: available_end,
                unit: available_end_unit,
                timezone: available_end_zone,
            },
            Literal::Timestamp {
                ticks: requested_start,
                unit: requested_start_unit,
                timezone: requested_start_zone,
            },
            Literal::Timestamp {
                ticks: requested_end,
                unit: requested_end_unit,
                timezone: requested_end_zone,
            },
        ) => {
            available_start_unit == available_end_unit
                && available_start_unit == requested_start_unit
                && available_start_unit == requested_end_unit
                && available_start_zone == available_end_zone
                && available_start_zone == requested_start_zone
                && available_start_zone == requested_end_zone
                && available_start <= requested_start
                && requested_end <= available_end
        }
        _ => false,
    }
}

// A ROWS frame must have a strict order within each partition. Every grouping
// output in the partition or order key makes that proof from the GROUP BY
// itself, including nullable keys, without trusting a declared source key.
fn validate_cumulative_frame(
    window: &WindowSpec,
    query: &RowQuery,
) -> Result<(), CompileDiagnostic> {
    if window.frame != WindowFrame::RowsThroughCurrent {
        return Ok(());
    }
    let grouped: BTreeSet<_> = query
        .requirements
        .iter()
        .filter(|requirement| {
            matches!(
                requirement.operation,
                RowOperation::Group { .. }
                    | RowOperation::CalendarGroup { .. }
                    | RowOperation::Lookup {
                        usage: LookupUsage::Group,
                        ..
                    }
            )
        })
        .map(|requirement| requirement.id.as_str())
        .collect();
    let Some(WindowInput::Output { slot }) = &window.input else {
        return Err(diagnostic(
            "cumulative_frame",
            "Cumulative sums require a grouped aggregate output",
        ));
    };
    let additive = query.requirements.iter().any(|requirement| {
        requirement.id == *slot
            && matches!(
                requirement.operation,
                RowOperation::Metric { .. }
                    | RowOperation::Aggregate {
                        function: AggregateFunction::Sum | AggregateFunction::Count,
                        distinct: false,
                        ..
                    }
            )
    });
    if grouped.is_empty() || window.function != WindowFunction::Sum || !additive {
        return Err(diagnostic(
            "cumulative_frame",
            "Cumulative sums require an additive metric or aggregate at a grouped grain",
        ));
    }
    let covered: BTreeSet<_> = window
        .partition_by
        .iter()
        .chain(window.order_by.iter().map(|order| &order.input))
        .filter_map(|input| match input {
            WindowInput::Output { slot } => Some(slot.as_str()),
            WindowInput::Field { .. } => None,
        })
        .collect();
    if window.order_by.is_empty() || !grouped.is_subset(&covered) {
        return Err(diagnostic(
            "cumulative_order",
            "Cumulative order and partition keys must cover every grouping output",
        ));
    }
    Ok(())
}

// A scalar window sum is a merge of subgroup results. Grouping has made
// source rows disjoint, but that does not make distinct sets or finalized
// ratios additive. Named metrics additionally need authored dimensional scope.
fn validate_window_rollup(
    window: &WindowSpec,
    query: &RowQuery,
    relation: &SnapshotRelation,
    resolved_metric_names: &std::collections::BTreeMap<String, Option<String>>,
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
        RowOperation::Metric { .. } => {
            let Some(Some(name)) = resolved_metric_names.get(&input.id) else {
                return Err(unsupported());
            };
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
                if let RowOperation::Group { field, .. }
                | RowOperation::CalendarGroup { field, .. } = &group.operation
                {
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

fn validate_utc_month_group(
    field: &BoundField,
    grain: CalendarUnit,
    timezone: &str,
    relation: &SnapshotRelation,
) -> Result<(), CompileDiagnostic> {
    if let Some(reference) = relation
        .definition()
        .semantics
        .as_ref()
        .and_then(|semantics| semantics.fields.get(field.field.name()))
        .and_then(|field| field.calendar_reference.as_ref())
        && (!matches!(
            &reference.system,
            semantic_catalog::CalendarSystem::Gregorian
        ) || reference.timezone != "UTC")
    {
        return Err(diagnostic(
            "calendar_reference",
            "The authored calendar and timezone do not permit UTC Gregorian month grouping",
        ));
    }
    if grain != CalendarUnit::Month || timezone != "UTC" {
        return Err(diagnostic(
            "calendar_group_profile",
            "This executable calendar grouping profile requires an explicit UTC month",
        ));
    }
    if field.field.data_type()
        != &DataType::Timestamp(
            datafusion::arrow::datatypes::TimeUnit::Microsecond,
            Some("UTC".into()),
        )
    {
        return Err(diagnostic(
            "calendar_group_type",
            "UTC month grouping requires an exact UTC microsecond timestamp",
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
        (AggregateFunction::Sum | AggregateFunction::Avg, DataType::Float64) => DataType::Float64,
        (AggregateFunction::Avg, DataType::Int16 | DataType::Int32 | DataType::Int64) => {
            if distinct {
                return Err(diagnostic(
                    "aggregate_arguments",
                    "Exact signed integer average does not accept DISTINCT",
                ));
            }
            DataType::Decimal128(38, 18)
        }
        (
            AggregateFunction::Min | AggregateFunction::Max,
            DataType::Int16
            | DataType::Int32
            | DataType::Int64
            | DataType::UInt64
            | DataType::Float64
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

/// Replace only placeholders declared by an authored concept. The resulting
/// ordinary predicate still passes through the full field/type/profile binder.
fn substitute_concept_arguments(
    predicate: &RowPredicate<String>,
    arguments: &BTreeMap<String, Literal>,
) -> Result<RowPredicate<String>, CompileDiagnostic> {
    if arguments.len() > 16 {
        return Err(diagnostic(
            "parameter_contract",
            "Concept argument count exceeds the supported bound",
        ));
    }
    fn substitute(
        predicate: &RowPredicate<String>,
        arguments: &BTreeMap<String, Literal>,
        used: &mut BTreeSet<String>,
        depth: usize,
        remaining: &mut usize,
    ) -> Result<RowPredicate<String>, CompileDiagnostic> {
        if depth > 32 || *remaining == 0 {
            return Err(diagnostic(
                "parameter_contract",
                "Concept predicate exceeds the supported bound",
            ));
        }
        *remaining -= 1;
        match predicate {
            RowPredicate::CompareParameter {
                field,
                operator,
                parameter,
            } => {
                let value = arguments.get(parameter).ok_or_else(|| {
                    diagnostic("parameter_missing", "A concept argument is missing")
                })?;
                super::literal::checked_scalar(value)?;
                used.insert(parameter.clone());
                Ok(RowPredicate::Compare {
                    field: field.clone(),
                    operator: *operator,
                    value: value.clone(),
                })
            }
            RowPredicate::All { predicates } => Ok(RowPredicate::All {
                predicates: predicates
                    .iter()
                    .map(|child| substitute(child, arguments, used, depth + 1, remaining))
                    .collect::<Result<_, _>>()?,
            }),
            RowPredicate::Any { predicates } => Ok(RowPredicate::Any {
                predicates: predicates
                    .iter()
                    .map(|child| substitute(child, arguments, used, depth + 1, remaining))
                    .collect::<Result<_, _>>()?,
            }),
            RowPredicate::Not { predicate } => Ok(RowPredicate::Not {
                predicate: Box::new(substitute(
                    predicate,
                    arguments,
                    used,
                    depth + 1,
                    remaining,
                )?),
            }),
            other => Ok(other.clone()),
        }
    }
    let mut used = BTreeSet::new();
    let mut remaining = 256;
    let result = substitute(predicate, arguments, &mut used, 0, &mut remaining)?;
    if arguments.keys().any(|name| !used.contains(name)) {
        return Err(diagnostic(
            "parameter_extra",
            "A concept argument was not declared by the authored predicate",
        ));
    }
    Ok(result)
}
