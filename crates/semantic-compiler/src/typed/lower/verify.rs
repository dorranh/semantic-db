//! Fixed relational boundary checks for the supported row pipeline.
//! The verifier is independent of SQL and direct-plan adapters.

use super::*;

pub(super) const PASS_ID: &str = "semantic.row_lower";
pub(super) const PASS_VERSION: u32 = 8;
pub(super) const COMPARISON_PROFILE: &str = "datafusion-55/sql-null-logic/utf8-binary";

/// The row lowerer is one fixed pass. Its input is a bound, ordered unary
/// pipeline; its output must have a scan, a projection, and exactly one
/// disposition for every bound requirement. No optimizer may silently reuse a
/// fact that an operator invalidated on the way to that output.
struct RowPassContract {
    id: &'static str,
    version: u32,
    comparison_profile: &'static str,
}

const ROW_PASS: RowPassContract = RowPassContract {
    id: PASS_ID,
    version: PASS_VERSION,
    comparison_profile: COMPARISON_PROFILE,
};

#[derive(Clone, Copy)]
enum FactTransfer {
    Preserve,
    Extend,
    Replace,
    Invalidate,
}

#[derive(Clone, Copy)]
struct AnalysisTransfer {
    source_scope: FactTransfer,
    generated_fields: FactTransfer,
    grouped_key: FactTransfer,
}

impl AnalysisTransfer {
    fn for_operator(operator: &Operator) -> Self {
        use FactTransfer::{Extend, Invalidate, Preserve, Replace};
        match operator {
            Operator::Scan { .. } => Self {
                source_scope: Extend,
                generated_fields: Preserve,
                grouped_key: Preserve,
            },
            Operator::Lookup { lookup } => Self {
                source_scope: Preserve,
                generated_fields: Extend,
                grouped_key: if lookup.missing == semantic_plan::typed::MissingMatch::Null {
                    Invalidate
                } else {
                    Preserve
                },
            },
            Operator::Convert { .. }
            | Operator::CalendarGroup { .. }
            | Operator::Derive { .. }
            | Operator::Window { .. } => Self {
                source_scope: Preserve,
                generated_fields: Extend,
                grouped_key: Preserve,
            },
            Operator::CurrencyRate { .. } | Operator::BusinessCalendar { .. } => Self {
                source_scope: Preserve,
                generated_fields: Extend,
                grouped_key: Invalidate,
            },
            Operator::Aggregate { .. }
            | Operator::CalendarFill { .. }
            | Operator::Allocate { .. } => Self {
                source_scope: Invalidate,
                generated_fields: Replace,
                grouped_key: Replace,
            },
            Operator::Related { .. }
            | Operator::Filter { .. }
            | Operator::OutputFilter { .. }
            | Operator::Sort { .. }
            | Operator::Project { .. }
            | Operator::Fetch { .. } => Self {
                source_scope: Preserve,
                generated_fields: Preserve,
                grouped_key: Preserve,
            },
        }
    }

    fn verify(
        self,
        before: &AbstractSchema,
        after: &AbstractSchema,
    ) -> Result<(), CompileDiagnostic> {
        let source_valid = match self.source_scope {
            FactTransfer::Preserve => {
                before.source_instance == after.source_instance
                    && before.source_visible == after.source_visible
                    && before
                        .source_fields
                        .iter()
                        .all(|(name, field)| after.source_fields.get(name) == Some(field))
            }
            FactTransfer::Extend => {
                !before.source_visible && after.source_visible && after.source_instance.is_some()
            }
            FactTransfer::Invalidate => !after.source_visible,
            FactTransfer::Replace => false,
        };
        let outputs_valid = match self.generated_fields {
            FactTransfer::Preserve => before.outputs == after.outputs,
            FactTransfer::Extend => before
                .outputs
                .iter()
                .all(|(name, field)| after.outputs.get(name) == Some(field)),
            FactTransfer::Replace => true,
            FactTransfer::Invalidate => after.outputs.is_empty(),
        };
        let key_valid = match self.grouped_key {
            FactTransfer::Preserve => before.grouped_key == after.grouped_key,
            FactTransfer::Extend => before.grouped_key.is_none() && after.grouped_key.is_some(),
            FactTransfer::Replace => after.grouped_key.is_some(),
            FactTransfer::Invalidate => after.grouped_key.is_none(),
        };
        if source_valid && outputs_valid && key_valid {
            Ok(())
        } else {
            Err(diagnostic(
                "invalid_relational_analysis",
                "Relational operator did not preserve or invalidate analysis facts as declared",
            ))
        }
    }
}

pub(super) fn verify(
    plan: &RelationalPlan,
    expected_requirements: &BTreeSet<&str>,
) -> Result<(), CompileDiagnostic> {
    if plan.version != 1
        || plan.pass_id != ROW_PASS.id
        || plan.pass_version != ROW_PASS.version
        || plan.comparison_profile != ROW_PASS.comparison_profile
    {
        return Err(diagnostic(
            "invalid_relational_plan",
            "Relational pass identity is incompatible with this verifier",
        ));
    }
    let mut actual = BTreeSet::new();
    let mut scan = false;
    let mut allocate = false;
    let mut currency_rate = false;
    let mut business_calendar = false;
    let mut convert = false;
    let mut calendar_group = false;
    let mut aggregate = false;
    let mut calendar_fill = false;
    let mut derive = false;
    let mut window = false;
    let mut sort = false;
    let mut project = false;
    let mut fetch = false;
    let mut schema = AbstractSchema::default();
    for (index, node) in plan.nodes.iter().enumerate() {
        if node.id != index || node.input != index.checked_sub(1) {
            return Err(diagnostic(
                "invalid_relational_plan",
                "Invalid relational node linkage",
            ));
        }
        for id in &node.requirements {
            if !actual.insert(id.as_str()) {
                return Err(diagnostic(
                    "requirement_coverage",
                    "A requirement was lowered more than once",
                ));
            }
        }
        match &node.operator {
            Operator::Scan { .. } if index == 0 => scan = true,
            Operator::Scan { .. } => return Err(invalid_stage()),
            Operator::Allocate { .. }
                if scan
                    && !allocate
                    && !currency_rate
                    && !convert
                    && !calendar_group
                    && !aggregate
                    && !derive
                    && !window
                    && !sort
                    && !project
                    && !fetch =>
            {
                allocate = true;
            }
            Operator::CurrencyRate { .. }
                if scan
                    && !allocate
                    && !currency_rate
                    && !convert
                    && !calendar_group
                    && !aggregate
                    && !derive
                    && !window
                    && !sort
                    && !project
                    && !fetch =>
            {
                currency_rate = true;
            }
            Operator::BusinessCalendar { .. }
                if scan
                    && !allocate
                    && !currency_rate
                    && !business_calendar
                    && !convert
                    && !calendar_group
                    && !aggregate
                    && !derive
                    && !window
                    && !sort
                    && !project
                    && !fetch =>
            {
                business_calendar = true;
            }
            Operator::Lookup { .. } | Operator::Related { .. }
                if scan
                    && !allocate
                    && !currency_rate
                    && !convert
                    && !calendar_group
                    && !aggregate
                    && !derive
                    && !window
                    && !sort
                    && !project
                    && !fetch => {}
            Operator::Convert { conversions }
                if scan
                    && !allocate
                    && !currency_rate
                    && !convert
                    && !calendar_group
                    && !aggregate
                    && !derive
                    && !window
                    && !sort
                    && !project
                    && !fetch
                    && !conversions.is_empty() =>
            {
                convert = true;
            }
            Operator::CalendarGroup { buckets }
                if scan
                    && !allocate
                    && !currency_rate
                    && !calendar_group
                    && !aggregate
                    && !derive
                    && !window
                    && !sort
                    && !project
                    && !fetch
                    && !buckets.is_empty() =>
            {
                calendar_group = true;
            }
            Operator::Aggregate { .. }
                if scan
                    && !allocate
                    && !currency_rate
                    && !aggregate
                    && !derive
                    && !window
                    && !sort
                    && !project
                    && !fetch =>
            {
                aggregate = true;
            }
            Operator::CalendarFill { months, .. }
                if aggregate
                    && !calendar_fill
                    && !derive
                    && !window
                    && !sort
                    && !project
                    && !fetch
                    && !months.is_empty()
                    && months.len() <= super::super::calendar_spine::MAX_UTC_MONTHS =>
            {
                calendar_fill = true;
            }
            Operator::Derive { .. }
                if aggregate
                    && !allocate
                    && !currency_rate
                    && !derive
                    && !window
                    && !sort
                    && !project
                    && !fetch =>
            {
                derive = true;
            }
            Operator::Window { .. }
                if scan
                    && !allocate
                    && !currency_rate
                    && !window
                    && !sort
                    && !project
                    && !fetch =>
            {
                window = true;
            }
            Operator::Filter { .. }
                if scan && !allocate && !currency_rate && !sort && !project && !fetch => {}
            Operator::OutputFilter {
                stage: OutputFilterStage::AfterAggregate,
                ..
            } if aggregate && !window && !sort && !project && !fetch => {}
            Operator::OutputFilter {
                stage: OutputFilterStage::AfterWindow,
                ..
            } if window && !sort && !project && !fetch => {}
            Operator::Sort { keys }
                if scan && !currency_rate && !sort && !project && !fetch && !keys.is_empty() =>
            {
                sort = true;
            }
            Operator::Project { columns } if scan && !project && !fetch && !columns.is_empty() => {
                project = true;
            }
            Operator::Fetch { .. } if project && !fetch => fetch = true,
            _ => return Err(invalid_stage()),
        }
        let before = schema.clone();
        schema.apply(&node.operator)?;
        AnalysisTransfer::for_operator(&node.operator).verify(&before, &schema)?;
    }
    if !scan || !project || actual != *expected_requirements {
        return Err(diagnostic(
            "requirement_coverage",
            "Lowering did not preserve the required output and all requirements",
        ));
    }
    Ok(())
}

#[derive(Clone, Default)]
struct AbstractSchema {
    source_instance: Option<String>,
    source_visible: bool,
    source_fields: BTreeMap<String, semantic_catalog::Field>,
    outputs: BTreeMap<String, semantic_catalog::Field>,
    /// GROUP BY proves these output columns uniquely identify each row.
    grouped_key: Option<BTreeSet<String>>,
}

impl AbstractSchema {
    fn read(&mut self, field: &BoundField) -> Result<(), CompileDiagnostic> {
        if field.instance == "$output" {
            return match self.outputs.get(field.field.name()) {
                Some(existing) if existing == &field.field => Ok(()),
                _ => Err(invalid_schema()),
            };
        }
        if !self.source_visible || self.source_instance.as_deref() != Some(&field.instance) {
            return Err(invalid_schema());
        }
        match self.source_fields.get(field.field.name()) {
            Some(existing) if existing != &field.field => Err(invalid_schema()),
            Some(_) => Ok(()),
            None => {
                self.source_fields
                    .insert(field.field.name().into(), field.field.clone());
                Ok(())
            }
        }
    }

    fn write(&mut self, field: &BoundField) -> Result<(), CompileDiagnostic> {
        if field.instance != "$output"
            || field.field.name().is_empty()
            || self.outputs.contains_key(field.field.name())
            || self.source_fields.contains_key(field.field.name())
        {
            return Err(invalid_schema());
        }
        self.outputs
            .insert(field.field.name().into(), field.field.clone());
        Ok(())
    }

    fn predicate(&mut self, predicate: &BoundPredicate) -> Result<(), CompileDiagnostic> {
        match predicate {
            BoundPredicate::Mapped { predicate, .. } | BoundPredicate::Not { predicate } => {
                self.predicate(predicate)
            }
            BoundPredicate::Compare { field, .. } | BoundPredicate::IsNull { field, .. } => {
                self.read(field)
            }
            BoundPredicate::All { predicates } | BoundPredicate::Any { predicates } => {
                for predicate in predicates {
                    self.predicate(predicate)?;
                }
                Ok(())
            }
        }
    }

    fn related_target(relationship: &BoundRelationship) -> Result<(), CompileDiagnostic> {
        let Some(target) = &relationship.target else {
            return Ok(());
        };
        if target.input != relationship.right || target.instance != relationship.instance {
            return Err(invalid_schema());
        }
        for requirement in &target.requirements {
            match &requirement.operation {
                BoundOperation::Filter { predicate } => {
                    Self::right_predicate(predicate, &relationship.instance)?
                }
                BoundOperation::Related {
                    relationship: child,
                } => {
                    for (left, right) in &child.keys {
                        if left.instance != relationship.instance
                            || right.instance != child.instance
                            || left.field.data_type() != right.field.data_type()
                        {
                            return Err(invalid_schema());
                        }
                    }
                    if let Some(predicate) = &child.predicate {
                        Self::right_predicate(predicate, &child.instance)?;
                    }
                    Self::related_target(child)?;
                }
                _ => return Err(invalid_schema()),
            }
        }
        Ok(())
    }
    fn right_predicate(
        predicate: &BoundPredicate,
        right_instance: &str,
    ) -> Result<(), CompileDiagnostic> {
        match predicate {
            BoundPredicate::Mapped { predicate, .. } | BoundPredicate::Not { predicate } => {
                Self::right_predicate(predicate, right_instance)
            }
            BoundPredicate::Compare { field, .. } | BoundPredicate::IsNull { field, .. } => {
                if field.instance == right_instance {
                    Ok(())
                } else {
                    Err(invalid_schema())
                }
            }
            BoundPredicate::All { predicates } | BoundPredicate::Any { predicates } => {
                for predicate in predicates {
                    Self::right_predicate(predicate, right_instance)?;
                }
                Ok(())
            }
        }
    }

    fn output_predicate(&mut self, predicate: &CheckedPredicate) -> Result<(), CompileDiagnostic> {
        match predicate {
            CheckedPredicate::Mapped { predicate, .. }
            | CheckedPredicate::Not { inner: predicate } => self.output_predicate(predicate),
            CheckedPredicate::Compare { slot, .. } | CheckedPredicate::IsNull { slot, .. } => {
                if slot.scope != "$output" {
                    return Err(invalid_schema());
                }
                self.read(&BoundField {
                    instance: slot.scope.clone(),
                    field: slot.field.clone(),
                })
            }
            CheckedPredicate::All { children } | CheckedPredicate::Any { children } => {
                if children.is_empty() {
                    return Err(invalid_schema());
                }
                for child in children {
                    self.output_predicate(child)?;
                }
                Ok(())
            }
        }
    }

    fn apply(&mut self, operator: &Operator) -> Result<(), CompileDiagnostic> {
        match operator {
            Operator::Scan { instance, .. } => {
                if instance.is_empty() || self.source_instance.is_some() {
                    return Err(invalid_schema());
                }
                self.source_instance = Some(instance.clone());
                self.source_visible = true;
            }
            Operator::Lookup { lookup } => {
                if let Some(predicate) = &lookup.relationship.predicate {
                    Self::right_predicate(predicate, &lookup.relationship.instance)?;
                }
                for (left, right) in &lookup.relationship.keys {
                    self.read(left)?;
                    if right.instance != lookup.relationship.instance
                        || left.field.data_type() != right.field.data_type()
                    {
                        return Err(invalid_schema());
                    }
                }
                if lookup.value.instance != lookup.relationship.instance
                    || !lookup.output.field.is_nullable()
                    || lookup.output.field.data_type() != lookup.value.field.data_type()
                {
                    return Err(invalid_schema());
                }
                if let Some(as_of) = &lookup.as_of {
                    self.read(&as_of.fact_time)?;
                    if as_of.valid_from.instance != lookup.relationship.instance
                        || as_of.valid_to.instance != lookup.relationship.instance
                        || as_of.fact_time.field.data_type() != as_of.valid_from.field.data_type()
                        || as_of.fact_time.field.data_type() != as_of.valid_to.field.data_type()
                    {
                        return Err(invalid_schema());
                    }
                }
                self.write(&lookup.output)?;
                if let Some(extra) = &lookup.extra_value {
                    if extra.value.instance != lookup.relationship.instance
                        || !extra.output.field.is_nullable()
                        || extra.output.field.data_type() != extra.value.field.data_type()
                    {
                        return Err(invalid_schema());
                    }
                    self.write(&extra.output)?;
                }
                // The right side of a LEFT JOIN has no non-null or unique-key
                // certificate in the output, even when its source field had one.
                if lookup.missing == semantic_plan::typed::MissingMatch::Null {
                    self.grouped_key = None;
                }
            }
            Operator::Related { relationship } => {
                Self::related_target(relationship)?;
                if let Some(predicate) = &relationship.predicate {
                    Self::right_predicate(predicate, &relationship.instance)?;
                }
                for (left, right) in &relationship.keys {
                    self.read(left)?;
                    if right.instance != relationship.instance
                        || left.field.data_type() != right.field.data_type()
                    {
                        return Err(invalid_schema());
                    }
                }
            }
            Operator::Filter { predicate } => self.predicate(predicate)?,
            Operator::OutputFilter { predicate, .. } => self.output_predicate(predicate)?,
            Operator::Convert { conversions } => {
                for conversion in conversions {
                    self.read(&conversion.source)?;
                    if conversion.source.field.data_type() != &semantic_catalog::DataType::Int64
                        || conversion.output.field.data_type()
                            != &semantic_catalog::DataType::Decimal128(38, 18)
                    {
                        return Err(invalid_schema());
                    }
                    self.write(&conversion.output)?;
                }
            }
            Operator::CalendarGroup { buckets } => {
                for bucket in buckets {
                    self.read(&bucket.source)?;
                    if bucket.source.field.data_type() != bucket.output.field.data_type() {
                        return Err(invalid_schema());
                    }
                    self.write(&bucket.output)?;
                }
            }
            Operator::Aggregate { groups, aggregates } => {
                for group in groups {
                    self.read(&group.field)?;
                    if group.output.field.data_type() != group.field.field.data_type()
                        || group.output.field.is_nullable() != group.field.field.is_nullable()
                    {
                        return Err(invalid_schema());
                    }
                }
                for aggregate in aggregates {
                    if let Some(field) = &aggregate.field {
                        self.read(field)?;
                    }
                    if let Some(weighted) = &aggregate.weighted {
                        self.read(&weighted.weight)?;
                    }
                    let requires_zero = aggregate.weighted.as_ref().is_some_and(|weighted| {
                        weighted.zero == semantic_catalog::ZeroWeight::Zero
                    });
                    if requires_zero != aggregate.zero_finalizer.is_some()
                        || aggregate
                            .zero_finalizer
                            .as_ref()
                            .is_some_and(|finalizer| !finalizer.valid_for(&aggregate.output.field))
                    {
                        return Err(invalid_schema());
                    }
                    if let Some(snapshot) = &aggregate.snapshot {
                        self.read(&snapshot.time)?;
                        self.read(&snapshot.tie)?;
                    }
                    if let Some(predicate) = &aggregate.filter {
                        self.predicate(predicate)?;
                    }
                    let expected = if aggregate.mean_state || aggregate.weighted.is_some() {
                        Some(semantic_catalog::DataType::Decimal128(38, 18))
                    } else if aggregate.exact_distinct
                        || aggregate.snapshot.is_some()
                        || aggregate.function == AggregateFunction::Count
                    {
                        Some(semantic_catalog::DataType::Int64)
                    } else {
                        None
                    };
                    if expected
                        .as_ref()
                        .is_some_and(|ty| aggregate.output.field.data_type() != ty)
                    {
                        return Err(invalid_schema());
                    }
                }
                self.source_visible = false;
                self.outputs.clear();
                let mut key = BTreeSet::new();
                for group in groups {
                    self.write(&group.output)?;
                    key.insert(group.output.field.name().to_owned());
                }
                for aggregate in aggregates {
                    self.write(&aggregate.output)?;
                }
                self.grouped_key = Some(key);
            }
            Operator::CalendarFill { month, count, .. } => {
                self.read(month)?;
                self.read(count)?;
                self.source_visible = false;
                self.outputs
                    .retain(|name, _| name == month.field.name() || name == count.field.name());
                self.grouped_key = Some(BTreeSet::from([month.field.name().to_owned()]));
            }
            Operator::Derive { ratios } => {
                for ratio in ratios {
                    self.read(&ratio.numerator)?;
                    self.read(&ratio.denominator)?;
                    if ratio.output.field.data_type()
                        != &semantic_catalog::DataType::Decimal128(38, 18)
                    {
                        return Err(invalid_schema());
                    }
                    self.write(&ratio.output)?;
                }
            }
            Operator::Window { windows } => {
                for expression in windows {
                    let window = &expression.window;
                    if let Some(input) = &window.input {
                        self.read(input)?;
                    }
                    for field in &window.partition_by {
                        self.read(field)?;
                    }
                    for (field, _, _) in &window.order_by {
                        self.read(field)?;
                    }
                    if window.frame == semantic_plan::typed::WindowFrame::RowsThroughCurrent {
                        let Some(key) = &self.grouped_key else {
                            return Err(invalid_schema());
                        };
                        let covered: BTreeSet<_> = window
                            .partition_by
                            .iter()
                            .chain(window.order_by.iter().map(|(field, _, _)| field))
                            .map(|field| field.field.name().to_owned())
                            .collect();
                        if !key.is_subset(&covered) {
                            return Err(invalid_schema());
                        }
                    }
                    let expected = match window.function {
                        semantic_plan::typed::WindowFunction::Rank
                        | semantic_plan::typed::WindowFunction::DenseRank => {
                            Some(semantic_catalog::DataType::UInt64)
                        }
                        semantic_plan::typed::WindowFunction::Count => {
                            Some(semantic_catalog::DataType::Int64)
                        }
                        _ => None,
                    };
                    if expected
                        .as_ref()
                        .is_some_and(|ty| expression.output.field.data_type() != ty)
                    {
                        return Err(invalid_schema());
                    }
                    self.write(&expression.output)?;
                }
            }
            Operator::Sort { keys } => {
                for key in keys {
                    self.read(&key.field)?;
                }
            }
            Operator::Project { columns } => {
                let mut aliases = BTreeSet::new();
                for column in columns {
                    self.read(&column.field)?;
                    if column.alias.is_empty() || !aliases.insert(&column.alias) {
                        return Err(invalid_schema());
                    }
                }
            }
            Operator::Allocate { allocation } => {
                for field in allocation.source_key.iter().chain([
                    &allocation.source_amount,
                    &allocation.expected_count,
                    &allocation.expected_weight,
                ]) {
                    self.read(field)?;
                }
                if allocation.source_key.is_empty()
                    || allocation.source_key.len() > 4
                    || allocation.source_key.len() != allocation.bridge_key.len()
                    || allocation
                        .source_key
                        .iter()
                        .zip(&allocation.bridge_key)
                        .any(|(source, bridge)| {
                            source.field.data_type() != bridge.field.data_type()
                                || source.field.is_nullable()
                                || bridge.field.is_nullable()
                        })
                {
                    return Err(invalid_schema());
                }
                if allocation.amount_output.field.data_type() != &semantic_catalog::DataType::Int64
                    || allocation.targets.is_empty()
                    || allocation.targets.len() > 2
                    || allocation.targets.len() != allocation.target_outputs.len()
                    || allocation
                        .targets
                        .iter()
                        .zip(&allocation.target_outputs)
                        .any(|(target, output)| {
                            target.field.data_type() != output.field.data_type()
                                || target.field.is_nullable()
                                || !output.field.is_nullable()
                        })
                {
                    return Err(invalid_schema());
                }
                self.source_visible = false;
                self.outputs.clear();
                let mut key = BTreeSet::new();
                for target in &allocation.target_outputs {
                    self.write(target)?;
                    key.insert(target.field.name().to_owned());
                }
                self.write(&allocation.amount_output)?;
                self.grouped_key = Some(key);
            }
            Operator::CurrencyRate { rate } => {
                for field in [
                    &rate.source_amount,
                    &rate.source_currency,
                    &rate.source_time,
                ] {
                    self.read(field)?;
                }
                if rate.output.field.data_type() != &semantic_catalog::DataType::Decimal128(38, 18)
                {
                    return Err(invalid_schema());
                }
                // The guarded LEFT JOIN emits every original source column
                // alongside its derived value.
                self.write(&rate.output)?;
                self.grouped_key = None;
            }
            Operator::BusinessCalendar { calendar } => {
                self.read(&calendar.source_date)?;
                if calendar.output.field.data_type() != calendar.calendar_value.field.data_type()
                    || !calendar.output.field.is_nullable()
                {
                    return Err(invalid_schema());
                }
                // Calendar enrichment also preserves the source projection.
                self.write(&calendar.output)?;
                self.grouped_key = None;
            }
            Operator::Fetch { .. } => {}
        }
        Ok(())
    }
}

fn invalid_schema() -> CompileDiagnostic {
    diagnostic(
        "invalid_relational_schema",
        "Relational operator reads a field outside its checked scope or changes its type or nullability",
    )
}

fn invalid_stage() -> CompileDiagnostic {
    diagnostic(
        "invalid_relational_plan",
        "Relational operators are not in a valid checked stage order",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(
        instance: &str,
        name: &str,
        ty: semantic_catalog::DataType,
        nullable: bool,
    ) -> BoundField {
        BoundField {
            instance: instance.into(),
            field: semantic_catalog::Field::new(name, ty, nullable),
        }
    }

    fn scan_plan() -> RelationalPlan {
        let mut plan = RelationalPlan {
            version: 1,
            pass_id: PASS_ID,
            pass_version: PASS_VERSION,
            comparison_profile: COMPARISON_PROFILE,
            nodes: vec![],
        };
        plan.push(
            vec![],
            Operator::Scan {
                relation: ObjectRef {
                    id: "orders".into(),
                    revision: "r1".into(),
                },
                instance: "o".into(),
            },
        );
        plan
    }

    fn fixture() -> RelationalPlan {
        let mut plan = scan_plan();
        plan.push(
            vec!["projection".into()],
            Operator::Project {
                columns: vec![Projection {
                    field: BoundField {
                        instance: "o".into(),
                        field: semantic_catalog::Field::new(
                            "id",
                            semantic_catalog::DataType::Int64,
                            false,
                        ),
                    },
                    alias: "id".into(),
                }],
            },
        );
        plan.push(
            vec!["limit".into()],
            Operator::Fetch {
                offset: 0,
                count: 1,
            },
        );
        plan
    }

    #[test]
    fn checked_output_filter_rejects_forged_slot_type_and_stage() {
        use semantic_catalog::DataType;

        let source = field("o", "group_id", DataType::Int64, false);
        let output = field("$output", "__group", DataType::Int64, false);
        let mut plan = scan_plan();
        plan.push(
            vec!["group".into()],
            Operator::Aggregate {
                groups: vec![Grouping {
                    field: source,
                    output: output.clone(),
                }],
                aggregates: vec![],
            },
        );
        plan.push(
            vec!["keep".into()],
            Operator::OutputFilter {
                stage: OutputFilterStage::AfterAggregate,
                predicate: CheckedPredicate::compare(
                    super::super::super::scalar::CheckedSlot::new(
                        "$output",
                        "group",
                        &output.field,
                    ),
                    Comparison::Gt,
                    Literal::Int64(0),
                )
                .unwrap(),
            },
        );
        plan.push(
            vec!["project".into()],
            Operator::Project {
                columns: vec![Projection {
                    field: output,
                    alias: "group_id".into(),
                }],
            },
        );
        let expected = BTreeSet::from(["group", "keep", "project"]);
        verify(&plan, &expected).unwrap();

        let mut wrong_type = plan.clone();
        let Operator::OutputFilter {
            predicate: CheckedPredicate::Compare { slot, .. },
            ..
        } = &mut wrong_type.nodes[2].operator
        else {
            unreachable!()
        };
        slot.field = semantic_catalog::Field::new("__group", DataType::Utf8, false);
        assert_eq!(
            verify(&wrong_type, &expected).unwrap_err().code,
            "invalid_relational_schema"
        );

        let mut wrong_stage = plan;
        let Operator::OutputFilter { stage, .. } = &mut wrong_stage.nodes[2].operator else {
            unreachable!()
        };
        *stage = OutputFilterStage::AfterWindow;
        assert_eq!(
            verify(&wrong_stage, &expected).unwrap_err().code,
            "invalid_relational_plan"
        );
    }

    #[test]
    fn checked_weighted_zero_finalizer_rejects_forged_decimal_type() {
        use semantic_catalog::DataType;

        let region = field("o", "region", DataType::Utf8, false);
        let grouped = field("$output", "__region", DataType::Utf8, false);
        let mean = field("$output", "__mean", DataType::Decimal128(38, 18), false);
        let mut plan = scan_plan();
        plan.push(
            vec!["region".into(), "mean".into()],
            Operator::Aggregate {
                groups: vec![Grouping {
                    field: region,
                    output: grouped.clone(),
                }],
                aggregates: vec![Aggregation {
                    function: AggregateFunction::Sum,
                    mean_state: false,
                    weighted: Some(BoundWeightedState {
                        weight: field("o", "weight", DataType::Int64, true),
                        zero: semantic_catalog::ZeroWeight::Zero,
                    }),
                    zero_finalizer: Some(CheckedDecimalZeroFinalizer::new(&mean.field).unwrap()),
                    exact_distinct: false,
                    snapshot: None,
                    field: Some(field("o", "amount", DataType::Int64, true)),
                    distinct: false,
                    output: mean.clone(),
                    filter: None,
                }],
            },
        );
        plan.push(
            vec!["project".into()],
            Operator::Project {
                columns: vec![Projection {
                    field: mean,
                    alias: "mean".into(),
                }],
            },
        );
        let expected = BTreeSet::from(["region", "mean", "project"]);
        verify(&plan, &expected).unwrap();

        let mut forged = plan.clone();
        let Operator::Aggregate { aggregates, .. } = &mut forged.nodes[1].operator else {
            unreachable!()
        };
        aggregates[0].zero_finalizer.as_mut().unwrap().result_type = DataType::Int64;
        assert_eq!(
            verify(&forged, &expected).unwrap_err().code,
            "invalid_relational_schema"
        );

        let mut missing = plan;
        let Operator::Aggregate { aggregates, .. } = &mut missing.nodes[1].operator else {
            unreachable!()
        };
        aggregates[0].zero_finalizer = None;
        assert_eq!(
            verify(&missing, &expected).unwrap_err().code,
            "invalid_relational_schema"
        );
    }

    #[test]
    fn mutation_checks_reject_stage_movement_and_lost_requirements() {
        let expected = BTreeSet::from(["projection", "limit"]);
        let plan = fixture();
        verify(&plan, &expected).unwrap();

        let mut moved = plan.clone();
        let (before, after) = moved.nodes.split_at_mut(2);
        std::mem::swap(&mut before[1].operator, &mut after[0].operator);
        assert_eq!(
            verify(&moved, &expected).unwrap_err().code,
            "invalid_relational_plan"
        );

        let mut dropped = plan.clone();
        dropped.nodes[2].requirements.clear();
        assert_eq!(
            verify(&dropped, &expected).unwrap_err().code,
            "requirement_coverage"
        );

        let mut stale = plan.clone();
        stale.version = 0;
        assert_eq!(
            verify(&stale, &expected).unwrap_err().code,
            "invalid_relational_plan"
        );
        stale = plan.clone();
        stale.pass_version = PASS_VERSION - 1;
        assert_eq!(
            verify(&stale, &expected).unwrap_err().code,
            "invalid_relational_plan"
        );
        stale = plan.clone();
        stale.comparison_profile = "unknown-comparison";
        assert_eq!(
            verify(&stale, &expected).unwrap_err().code,
            "invalid_relational_plan"
        );
    }

    #[test]
    fn analysis_transfer_rejects_stale_key_leaked_scope_and_lost_provenance() {
        use semantic_catalog::DataType;

        let source = field("o", "amount", DataType::Int64, false);
        let generated = field("$output", "__amount", DataType::Int64, false);
        let mut before = AbstractSchema {
            source_instance: Some("o".into()),
            source_visible: true,
            source_fields: BTreeMap::from([(source.field.name().into(), source.field.clone())]),
            outputs: BTreeMap::from([(generated.field.name().into(), generated.field.clone())]),
            grouped_key: Some(BTreeSet::from(["__amount".into()])),
        };
        let mut after = before.clone();
        after.source_visible = false;
        after.outputs.clear();
        after.grouped_key = Some(BTreeSet::from(["__tenant".into()]));
        let replacement = AnalysisTransfer {
            source_scope: FactTransfer::Invalidate,
            generated_fields: FactTransfer::Replace,
            grouped_key: FactTransfer::Replace,
        };
        replacement.verify(&before, &after).unwrap();

        let mut leaked_source = after.clone();
        leaked_source.source_visible = true;
        assert_eq!(
            replacement
                .verify(&before, &leaked_source)
                .unwrap_err()
                .code,
            "invalid_relational_analysis"
        );

        let preservation = AnalysisTransfer {
            source_scope: FactTransfer::Preserve,
            generated_fields: FactTransfer::Preserve,
            grouped_key: FactTransfer::Invalidate,
        };
        before.grouped_key = Some(BTreeSet::from(["__amount".into()]));
        let mut invalidated = before.clone();
        invalidated.grouped_key = None;
        preservation.verify(&before, &invalidated).unwrap();
        let mut stale_key = invalidated.clone();
        stale_key.grouped_key = before.grouped_key.clone();
        assert_eq!(
            preservation.verify(&before, &stale_key).unwrap_err().code,
            "invalid_relational_analysis"
        );
        invalidated.outputs.clear();
        assert_eq!(
            preservation.verify(&before, &invalidated).unwrap_err().code,
            "invalid_relational_analysis"
        );
    }

    #[test]
    fn analysis_rejects_outer_join_nonnull_claim_and_changed_output_type() {
        use semantic_catalog::DataType;

        let source = field("o", "id", DataType::Int64, false);
        let right_key = field("r", "id", DataType::Int64, false);
        let right_value = field("r", "name", DataType::Utf8, false);
        let output = field("$output", "__lookup", DataType::Utf8, true);
        let mut plan = scan_plan();
        plan.push(
            vec!["lookup".into()],
            Operator::Lookup {
                lookup: Box::new(BoundLookup {
                    relationship: BoundRelationship {
                        definition: ObjectRef {
                            id: "relationship".into(),
                            revision: "r1".into(),
                        },
                        right: ObjectRef {
                            id: "names".into(),
                            revision: "r1".into(),
                        },
                        instance: "r".into(),
                        mode: ExistenceMode::Exists,
                        keys: vec![(source, right_key)],
                        null_keys_match: false,
                        predicate: None,
                        policies: vec![],
                        target: None,
                    },
                    value: right_value,
                    output: output.clone(),
                    extra_value: None,
                    missing: semantic_plan::typed::MissingMatch::Null,
                    obligation: "same-query/grouped-right-key-count-at-most-one/v1",
                    as_of: None,
                }),
            },
        );
        plan.push(
            vec!["project".into()],
            Operator::Project {
                columns: vec![Projection {
                    field: output,
                    alias: "name".into(),
                }],
            },
        );
        let expected = BTreeSet::from(["lookup", "project"]);
        verify(&plan, &expected).unwrap();

        let mut forged_nonnull = plan.clone();
        let Operator::Lookup { lookup } = &mut forged_nonnull.nodes[1].operator else {
            unreachable!()
        };
        lookup.output.field = semantic_catalog::Field::new("__lookup", DataType::Utf8, false);
        assert_eq!(
            verify(&forged_nonnull, &expected).unwrap_err().code,
            "invalid_relational_schema"
        );

        let mut changed_type = plan;
        let Operator::Project { columns } = &mut changed_type.nodes[2].operator else {
            unreachable!()
        };
        columns[0].field.field = semantic_catalog::Field::new("__lookup", DataType::Int64, true);
        assert_eq!(
            verify(&changed_type, &expected).unwrap_err().code,
            "invalid_relational_schema"
        );
    }

    #[test]
    fn right_predicate_cannot_rebind_left_lineage_to_right_alias() {
        use semantic_catalog::DataType;

        let source = field("o", "id", DataType::Int64, false);
        let right_key = field("r", "id", DataType::Int64, false);
        let right_value = field("r", "name", DataType::Utf8, false);
        let relationship = BoundRelationship {
            definition: ObjectRef {
                id: "relationship".into(),
                revision: "r1".into(),
            },
            right: ObjectRef {
                id: "names".into(),
                revision: "r1".into(),
            },
            instance: "r".into(),
            mode: ExistenceMode::Exists,
            keys: vec![(source.clone(), right_key)],
            null_keys_match: false,
            predicate: Some(BoundPredicate::All {
                predicates: vec![BoundPredicate::Mapped {
                    definition: ObjectRef {
                        id: "policy".into(),
                        revision: "r1".into(),
                    },
                    phrase: "eligible".into(),
                    predicate: Box::new(BoundPredicate::Compare {
                        field: field("r", "tenant", DataType::Int64, false),
                        operator: Comparison::Eq,
                        value: Literal::Int64(7),
                    }),
                }],
            }),
            policies: vec![],
            target: None,
        };

        let mut related = scan_plan();
        related.push(
            vec!["related".into()],
            Operator::Related {
                relationship: relationship.clone(),
            },
        );
        related.push(
            vec!["project".into()],
            Operator::Project {
                columns: vec![Projection {
                    field: source,
                    alias: "id".into(),
                }],
            },
        );
        let related_expected = BTreeSet::from(["related", "project"]);
        verify(&related, &related_expected).unwrap();
        let Operator::Related {
            relationship: related_relationship,
        } = &mut related.nodes[1].operator
        else {
            unreachable!()
        };
        let Some(BoundPredicate::All { predicates }) = &mut related_relationship.predicate else {
            unreachable!()
        };
        let BoundPredicate::Mapped { predicate, .. } = &mut predicates[0] else {
            unreachable!()
        };
        let BoundPredicate::Compare {
            field: predicate_field,
            ..
        } = predicate.as_mut()
        else {
            unreachable!()
        };
        predicate_field.instance = "o".into();
        assert_eq!(
            verify(&related, &related_expected).unwrap_err().code,
            "invalid_relational_schema"
        );

        let mut lookup = scan_plan();
        let output = field("$output", "__name", DataType::Utf8, true);
        lookup.push(
            vec!["lookup".into()],
            Operator::Lookup {
                lookup: Box::new(BoundLookup {
                    relationship,
                    value: right_value,
                    output: output.clone(),
                    extra_value: None,
                    missing: semantic_plan::typed::MissingMatch::Null,
                    obligation: "same-query/grouped-right-key-count-at-most-one/v1",
                    as_of: None,
                }),
            },
        );
        lookup.push(
            vec!["project".into()],
            Operator::Project {
                columns: vec![Projection {
                    field: output,
                    alias: "name".into(),
                }],
            },
        );
        let lookup_expected = BTreeSet::from(["lookup", "project"]);
        verify(&lookup, &lookup_expected).unwrap();
        let Operator::Lookup { lookup: bound } = &mut lookup.nodes[1].operator else {
            unreachable!()
        };
        let Some(BoundPredicate::All { predicates }) = &mut bound.relationship.predicate else {
            unreachable!()
        };
        let BoundPredicate::Mapped { predicate, .. } = &mut predicates[0] else {
            unreachable!()
        };
        let BoundPredicate::Compare {
            field: predicate_field,
            ..
        } = predicate.as_mut()
        else {
            unreachable!()
        };
        predicate_field.instance = "$output".into();
        assert_eq!(
            verify(&lookup, &lookup_expected).unwrap_err().code,
            "invalid_relational_schema"
        );
    }

    #[test]
    fn analysis_rejects_source_scope_after_group_and_lost_strict_key() {
        use semantic_catalog::DataType;
        use semantic_plan::typed::{Direction, NullOrder, WindowFrame, WindowFunction};

        let source_tenant = field("o", "tenant", DataType::Utf8, false);
        let source_period = field("o", "period", DataType::Int64, false);
        let tenant = field("$output", "__tenant", DataType::Utf8, false);
        let period = field("$output", "__period", DataType::Int64, false);
        let amount = field("$output", "__amount", DataType::Int64, true);
        let running = field("$output", "__running", DataType::Int64, true);
        let mut plan = scan_plan();
        plan.push(
            vec!["tenant".into(), "period".into(), "amount".into()],
            Operator::Aggregate {
                groups: vec![
                    Grouping {
                        field: source_tenant.clone(),
                        output: tenant.clone(),
                    },
                    Grouping {
                        field: source_period,
                        output: period.clone(),
                    },
                ],
                aggregates: vec![Aggregation {
                    function: AggregateFunction::Sum,
                    mean_state: false,
                    weighted: None,
                    zero_finalizer: None,
                    exact_distinct: false,
                    snapshot: None,
                    field: Some(field("o", "amount", DataType::Int64, true)),
                    distinct: false,
                    output: amount.clone(),
                    filter: None,
                }],
            },
        );
        plan.push(
            vec!["running".into()],
            Operator::Window {
                windows: vec![WindowExpression {
                    window: BoundWindow {
                        function: WindowFunction::Sum,
                        input: Some(amount),
                        partition_by: vec![tenant.clone()],
                        order_by: vec![(period.clone(), Direction::Asc, NullOrder::Last)],
                        frame: WindowFrame::RowsThroughCurrent,
                    },
                    output: running.clone(),
                }],
            },
        );
        plan.push(
            vec!["project".into()],
            Operator::Project {
                columns: vec![Projection {
                    field: running,
                    alias: "running".into(),
                }],
            },
        );
        let expected = BTreeSet::from(["tenant", "period", "amount", "running", "project"]);
        verify(&plan, &expected).unwrap();

        let mut leaked_source = plan.clone();
        let Operator::Project { columns } = &mut leaked_source.nodes[3].operator else {
            unreachable!()
        };
        columns[0].field = source_tenant;
        assert_eq!(
            verify(&leaked_source, &expected).unwrap_err().code,
            "invalid_relational_schema"
        );

        let mut lost_key = plan;
        let Operator::Window { windows } = &mut lost_key.nodes[2].operator else {
            unreachable!()
        };
        windows[0].window.order_by.clear();
        assert_eq!(
            verify(&lost_key, &expected).unwrap_err().code,
            "invalid_relational_schema"
        );
    }

    #[test]
    fn grouping_cannot_forge_output_type_or_nullability() {
        use semantic_catalog::DataType;

        let source = field("o", "tenant", DataType::Utf8, false);
        let output = field("$output", "__tenant", DataType::Utf8, false);
        let mut plan = scan_plan();
        plan.push(
            vec!["group".into()],
            Operator::Aggregate {
                groups: vec![Grouping {
                    field: source,
                    output: output.clone(),
                }],
                aggregates: vec![],
            },
        );
        plan.push(
            vec!["project".into()],
            Operator::Project {
                columns: vec![Projection {
                    field: output,
                    alias: "tenant".into(),
                }],
            },
        );
        let expected = BTreeSet::from(["group", "project"]);
        verify(&plan, &expected).unwrap();

        let mut wrong_type = plan.clone();
        let Operator::Aggregate { groups, .. } = &mut wrong_type.nodes[1].operator else {
            unreachable!()
        };
        groups[0].output.field = semantic_catalog::Field::new("__tenant", DataType::Int64, false);
        let changed_field = groups[0].output.field.clone();
        let Operator::Project { columns } = &mut wrong_type.nodes[2].operator else {
            unreachable!()
        };
        columns[0].field.field = changed_field;
        assert_eq!(
            verify(&wrong_type, &expected).unwrap_err().code,
            "invalid_relational_schema"
        );

        let mut wrong_nullability = plan;
        let Operator::Aggregate { groups, .. } = &mut wrong_nullability.nodes[1].operator else {
            unreachable!()
        };
        groups[0].output.field = semantic_catalog::Field::new("__tenant", DataType::Utf8, true);
        let changed_field = groups[0].output.field.clone();
        let Operator::Project { columns } = &mut wrong_nullability.nodes[2].operator else {
            unreachable!()
        };
        columns[0].field.field = changed_field;
        assert_eq!(
            verify(&wrong_nullability, &expected).unwrap_err().code,
            "invalid_relational_schema"
        );
    }

    #[test]
    fn calendar_enrichment_preserves_source_field_and_checks_derived_type() {
        use semantic_catalog::{CalendarSourceBasis, DataType};

        let date = field("o", "business_date", DataType::Date32, false);
        let value = field("$calendar", "fiscal_year", DataType::Int64, false);
        let output = field("$output", "__fiscal_year", DataType::Int64, true);
        let mut plan = scan_plan();
        plan.push(
            vec!["calendar".into()],
            Operator::BusinessCalendar {
                calendar: Box::new(BoundBusinessCalendar {
                    definition: ObjectRef {
                        id: "business_calendar".into(),
                        revision: "r1".into(),
                    },
                    calendar_relation: ObjectRef {
                        id: "dates".into(),
                        revision: "r1".into(),
                    },
                    source_date: date.clone(),
                    source_basis: CalendarSourceBasis::Date32,
                    timezone: "UTC".into(),
                    calendar_date: field("$calendar", "date", DataType::Date32, false),
                    calendar_value: value,
                    predicate: None,
                    policies: vec![],
                    output: output.clone(),
                    obligation: "same-query/policy-visible-business-calendar-exactly-one/v1",
                }),
            },
        );
        plan.push(
            vec!["project".into()],
            Operator::Project {
                columns: vec![
                    Projection {
                        field: date,
                        alias: "business_date".into(),
                    },
                    Projection {
                        field: output,
                        alias: "fiscal_year".into(),
                    },
                ],
            },
        );
        let expected = BTreeSet::from(["calendar", "project"]);
        verify(&plan, &expected).unwrap();

        let mut changed = plan;
        let Operator::Project { columns } = &mut changed.nodes[2].operator else {
            unreachable!()
        };
        columns[1].field.field =
            semantic_catalog::Field::new("__fiscal_year", DataType::Utf8, true);
        assert_eq!(
            verify(&changed, &expected).unwrap_err().code,
            "invalid_relational_schema"
        );
    }
}
