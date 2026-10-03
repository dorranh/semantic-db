use super::*;
use semantic_catalog::{AllocationEligibility, AllocationError};

pub(super) fn bind_allocation(
    binder: &mut Binder<'_>,
    snapshot: &CatalogSnapshot,
    name: &str,
) -> Result<BoundAllocation, CompileDiagnostic> {
    let semantics = binder
        .relation
        .definition()
        .semantics
        .as_ref()
        .ok_or_else(|| {
            diagnostic(
                "unknown_allocation",
                "The selected relation has no authored allocation",
            )
        })?;
    let contract = semantics.allocations.get(name).ok_or_else(|| {
        diagnostic(
            "unknown_allocation",
            "Only an authored allocation can authorize fan-out",
        )
    })?;
    if contract.source_relation != binder.relation.definition().name {
        return Err(diagnostic(
            "allocation_contract",
            "Allocation source differs from the selected relation",
        ));
    }
    contract
        .validate(snapshot, binder.options.allowed_relations.as_ref())
        .map_err(|error| {
            let code = if error == AllocationError::Scope {
                "access_scope"
            } else {
                "allocation_contract"
            };
            diagnostic(
                code,
                &format!("Authored allocation is not applicable: {error}"),
            )
        })?;
    if contract.source_entity_fields.len() > 4 || contract.target_dimensions.len() > 2 {
        return Err(diagnostic(
            "allocation_profile",
            "Executable allocation supports up to four exact source-key fields and two target dimensions",
        ));
    }
    binder.work.relations_looked_up += 1;
    let bridge = snapshot
        .relation(&contract.bridge_relation)
        .expect("validated bridge");
    if bridge
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
            "Allocation bridge is not executable",
        ));
    }
    let source_field = |binder: &mut Binder<'_>, name: &str| {
        let instance = binder.instance.to_owned();
        binder.field(&FieldRef {
            instance,
            field: name.into(),
        })
    };
    let source_key = contract
        .source_entity_fields
        .iter()
        .map(|name| source_field(binder, name))
        .collect::<Result<Vec<_>, _>>()?;
    let source_amount = source_field(binder, &contract.source_amount_field)?;
    let expected_count = source_field(binder, &contract.expected_membership_count_field)?;
    let expected_weight = source_field(binder, &contract.expected_weight_total_field)?;
    let mut right_binder = Binder {
        relation: bridge,
        instance: "$bridge",
        options: binder.options,
        work: binder.work,
    };
    let right_field = |binder: &mut Binder<'_>, name: &str| {
        binder.field(&FieldRef {
            instance: "$bridge".into(),
            field: name.into(),
        })
    };
    let bridge_key = contract
        .bridge_source_fields
        .iter()
        .map(|name| right_field(&mut right_binder, name))
        .collect::<Result<Vec<_>, _>>()?;
    let targets = contract
        .target_dimensions
        .iter()
        .map(|name| right_field(&mut right_binder, name))
        .collect::<Result<Vec<_>, _>>()?;
    let weight = right_field(&mut right_binder, &contract.weight_field)?;
    let mut predicates = Vec::new();
    if let AllocationEligibility::Utf8Equals { field, value } = &contract.eligible_population {
        predicates.push(BoundPredicate::Compare {
            field: right_field(&mut right_binder, field)?,
            operator: Comparison::Eq,
            value: Literal::Utf8(value.clone()),
        });
    }
    let mut policies = Vec::new();
    if let Some(semantics) = &bridge.definition().semantics {
        if semantics.row_policies.len() > binder.options.max_nodes {
            return Err(diagnostic(
                "work_limit",
                "Allocation policy budget exhausted",
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
                right_binder
                    .governed_filters(&policy.filters)?
                    .ok_or_else(|| {
                        diagnostic(
                            "policy_contract",
                            "An executable policy requires a predicate",
                        )
                    })?,
            );
            policies.push(
                bridge
                    .definition_reference("policy", &index.to_string())
                    .expect("indexed policy")
                    .clone(),
            );
        }
    }
    let predicate = (!predicates.is_empty()).then_some(BoundPredicate::All { predicates });
    let mut target_names = Vec::new();
    for index in 0..targets.len() {
        let mut name = format!("__semantic_alloc_target_{index}");
        while binder.relation.field(&name).is_some() || target_names.contains(&name) {
            name.push('_');
        }
        target_names.push(name);
    }
    let mut amount_name = "__semantic_alloc_amount".to_owned();
    while binder.relation.field(&amount_name).is_some() || target_names.contains(&amount_name) {
        amount_name.push('_');
    }
    let target_outputs = targets
        .iter()
        .zip(target_names)
        .map(|(target, name)| BoundField {
            instance: "$output".into(),
            // A checked LEFT JOIN retains backend nullability. The runtime
            // population assertion proves the target exists but does not
            // narrow the planner's static schema.
            field: Field::new(name, target.field.data_type().clone(), true),
        })
        .collect();
    Ok(BoundAllocation {
        definition: binder
            .relation
            .definition_reference("allocation", name)
            .expect("indexed allocation")
            .clone(),
        bridge: bridge.reference().clone(),
        source_key,
        bridge_key,
        source_amount,
        expected_count,
        expected_weight,
        targets,
        weight,
        predicate,
        policies,
        target_outputs,
        amount_output: BoundField {
            instance: "$output".into(),
            field: Field::new(amount_name, DataType::Int64, true),
        },
        obligation: "same-query/allocation-population-uniqueness-conservation/v1",
    })
}
