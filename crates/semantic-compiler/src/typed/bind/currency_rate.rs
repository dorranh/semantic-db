use super::*;
use semantic_catalog::{CurrencyRateError, RateTimeBasis};

pub(super) fn bind_currency_rate(
    binder: &mut Binder<'_>,
    snapshot: &CatalogSnapshot,
    selector: &str,
) -> Result<BoundCurrencyRate, CompileDiagnostic> {
    let semantics = binder
        .relation
        .definition()
        .semantics
        .as_ref()
        .ok_or_else(|| diagnostic("unknown_currency_rate", "No currency rates are authored"))?;
    let mut matches = semantics
        .currency_rates
        .iter()
        .filter(|(name, rule)| name.as_str() == selector || rule.id == selector);
    let (name, rule) = matches.next().ok_or_else(|| {
        diagnostic(
            "unknown_currency_rate",
            "The selected currency rate rule is not authored",
        )
    })?;
    if matches.next().is_some() {
        return Err(diagnostic(
            "ambiguous_currency_rate",
            "The selector matches multiple currency rate rules",
        ));
    }
    if rule.source_relation != binder.relation.definition().name {
        return Err(diagnostic(
            "currency_rate_contract",
            "Currency rate source differs from the selected relation",
        ));
    }
    rule.validate(snapshot, binder.options.allowed_relations.as_ref())
        .map_err(|error| {
            diagnostic(
                if error == CurrencyRateError::Scope {
                    "access_scope"
                } else {
                    "currency_rate_contract"
                },
                &format!("Authored currency rate is not applicable: {error}"),
            )
        })?;
    if !matches!(rule.time_basis, RateTimeBasis::UtcInstantMicros) {
        return Err(diagnostic(
            "unsupported_rate_time_basis",
            "Business-date rate matching requires an authored calendar execution profile",
        ));
    }
    binder.work.relations_looked_up += 1;
    let rate_relation = snapshot
        .relation(&rule.rate_relation)
        .expect("validated rate relation");
    if rate_relation
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
            "Currency rate relation is not executable",
        ));
    }
    let source_field = |binder: &mut Binder<'_>, field: &str| {
        binder.field(&FieldRef {
            instance: binder.instance.to_owned(),
            field: field.into(),
        })
    };
    let source_amount = source_field(binder, &rule.source_amount_field)?;
    let source_currency = source_field(binder, &rule.source_currency_field)?;
    let source_time = source_field(binder, &rule.source_time_field)?;
    let mut rate_binder = Binder {
        relation: rate_relation,
        instance: "$rate",
        options: binder.options,
        work: binder.work,
    };
    let rate_field = |binder: &mut Binder<'_>, field: &str| {
        binder.field(&FieldRef {
            instance: "$rate".into(),
            field: field.into(),
        })
    };
    let rate_from_currency = rate_field(&mut rate_binder, &rule.rate_from_currency_field)?;
    let rate_to_currency = rate_field(&mut rate_binder, &rule.rate_to_currency_field)?;
    let valid_from = rate_field(&mut rate_binder, &rule.rate_valid_from_field)?;
    let valid_to = rate_field(&mut rate_binder, &rule.rate_valid_to_field)?;
    let numerator = rate_field(&mut rate_binder, &rule.rate_numerator_field)?;
    let denominator = rate_field(&mut rate_binder, &rule.rate_denominator_field)?;
    let mut predicates = Vec::new();
    let mut policies = Vec::new();
    if let Some(semantics) = &rate_relation.definition().semantics {
        if semantics.row_policies.len() > binder.options.max_nodes {
            return Err(diagnostic(
                "work_limit",
                "Currency rate policy budget exhausted",
            ));
        }
        let mut ids = BTreeSet::new();
        for (index, policy) in semantics.row_policies.iter().enumerate() {
            if policy.id.trim().is_empty() || !ids.insert(&policy.id) {
                return Err(diagnostic(
                    "policy_contract",
                    "Policies require unique nonempty identities",
                ));
            }
            predicates.push(
                rate_binder
                    .governed_filters(&policy.filters)?
                    .ok_or_else(|| {
                        diagnostic(
                            "policy_contract",
                            "An executable policy requires a predicate",
                        )
                    })?,
            );
            policies.push(
                rate_relation
                    .definition_reference("policy", &index.to_string())
                    .expect("indexed rate policy")
                    .clone(),
            );
        }
    }
    let mut output_name = "__semantic_currency_rate_output".to_owned();
    while binder.relation.field(&output_name).is_some() {
        output_name.push('_');
    }
    Ok(BoundCurrencyRate {
        definition: binder
            .relation
            .definition_reference("currency_rate", name)
            .expect("indexed currency rate rule")
            .clone(),
        rate_relation: rate_relation.reference().clone(),
        source_amount,
        source_currency,
        source_time,
        rate_from_currency,
        rate_to_currency,
        valid_from,
        valid_to,
        numerator,
        denominator,
        to_currency: rule.to_currency.clone(),
        half_even: rule.rounding == semantic_catalog::ConversionRounding::HalfEven,
        predicate: (!predicates.is_empty()).then_some(BoundPredicate::All { predicates }),
        policies,
        output: BoundField {
            instance: "$output".into(),
            field: Field::new(output_name, DataType::Decimal128(38, 18), true),
        },
        obligation: "same-query/policy-visible-currency-rate-exactly-one/v1",
    })
}
