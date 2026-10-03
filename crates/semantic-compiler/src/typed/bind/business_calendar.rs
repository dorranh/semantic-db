use super::*;
use semantic_catalog::BusinessCalendarError;
use semantic_plan::typed::BusinessCalendarField;

pub(super) fn bind_business_calendar(
    binder: &mut Binder<'_>,
    snapshot: &CatalogSnapshot,
    selector: &str,
    selected: BusinessCalendarField,
) -> Result<BoundBusinessCalendar, CompileDiagnostic> {
    let semantics = binder
        .relation
        .definition()
        .semantics
        .as_ref()
        .ok_or_else(|| {
            diagnostic(
                "unknown_business_calendar",
                "No business calendars are authored",
            )
        })?;
    let mut matches = semantics
        .business_calendars
        .iter()
        .filter(|(name, rule)| name.as_str() == selector || rule.id == selector);
    let (name, rule) = matches.next().ok_or_else(|| {
        diagnostic(
            "unknown_business_calendar",
            "The selected business calendar is not authored",
        )
    })?;
    if matches.next().is_some() {
        return Err(diagnostic(
            "ambiguous_business_calendar",
            "The selector matches multiple calendar rules",
        ));
    }
    rule.validate(snapshot, binder.options.allowed_relations.as_ref())
        .map_err(|error| {
            diagnostic(
                if error == BusinessCalendarError::Scope {
                    "access_scope"
                } else {
                    "business_calendar_contract"
                },
                &format!("Authored business calendar is not applicable: {error}"),
            )
        })?;
    if rule.source_relation != binder.relation.definition().name {
        return Err(diagnostic(
            "business_calendar_contract",
            "Calendar source differs from the selected relation",
        ));
    }
    let mapping = snapshot
        .relation(&rule.calendar_relation)
        .expect("validated calendar relation");
    if mapping
        .definition()
        .semantics
        .as_ref()
        .and_then(|semantics| semantics.capability.as_ref())
        .is_some_and(|capability| {
            !matches!(capability, semantic_catalog::Capability::Executable { .. })
        })
    {
        return Err(diagnostic(
            "catalog_capability",
            "Calendar mapping relation is not executable",
        ));
    }
    binder.work.relations_looked_up += 1;
    let source_date = binder.field(&FieldRef {
        instance: binder.instance.to_owned(),
        field: rule.source_date_field.clone(),
    })?;
    let mut right = Binder {
        relation: mapping,
        instance: "$calendar",
        options: binder.options,
        work: binder.work,
    };
    let mapping_field = |binder: &mut Binder<'_>, field: &str| {
        binder.field(&FieldRef {
            instance: "$calendar".into(),
            field: field.into(),
        })
    };
    let calendar_date = mapping_field(&mut right, &rule.calendar_date_field)?;
    let selected_field = match selected {
        BusinessCalendarField::FiscalYear => &rule.fiscal_year_field,
        BusinessCalendarField::FiscalPeriod => &rule.fiscal_period_field,
        BusinessCalendarField::BusinessDay => &rule.business_day_field,
    };
    let calendar_value = mapping_field(&mut right, selected_field)?;
    let mut predicates = Vec::new();
    let mut policies = Vec::new();
    if let Some(semantics) = &mapping.definition().semantics {
        if semantics.row_policies.len() > binder.options.max_nodes {
            return Err(diagnostic("work_limit", "Calendar policy budget exhausted"));
        }
        let mut ids = BTreeSet::new();
        for (index, policy) in semantics.row_policies.iter().enumerate() {
            if policy.id.trim().is_empty() || !ids.insert(&policy.id) {
                return Err(diagnostic(
                    "policy_contract",
                    "Policies require unique nonempty identities",
                ));
            }
            predicates.push(right.governed_filters(&policy.filters)?.ok_or_else(|| {
                diagnostic(
                    "policy_contract",
                    "An executable policy requires a predicate",
                )
            })?);
            policies.push(
                mapping
                    .definition_reference("policy", &index.to_string())
                    .expect("indexed calendar policy")
                    .clone(),
            );
        }
    }
    let mut output_name = "__semantic_business_calendar_output".to_owned();
    while binder.relation.field(&output_name).is_some() {
        output_name.push('_');
    }
    Ok(BoundBusinessCalendar {
        definition: binder
            .relation
            .definition_reference("business_calendar", name)
            .expect("indexed calendar rule")
            .clone(),
        calendar_relation: mapping.reference().clone(),
        source_date,
        source_basis: rule.source_basis,
        timezone: rule.timezone.clone(),
        calendar_date,
        calendar_value: calendar_value.clone(),
        predicate: (!predicates.is_empty()).then_some(BoundPredicate::All { predicates }),
        policies,
        output: BoundField {
            instance: "$output".into(),
            field: Field::new(output_name, calendar_value.field.data_type().clone(), true),
        },
        obligation: "same-query/policy-visible-business-calendar-exactly-one/v1",
    })
}
