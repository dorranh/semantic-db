use super::*;
use semantic_catalog::{
    AsOfJoin, PathMissing, PathStep, PathUsage, RELATIONSHIP_PATH_VERSION, RelationshipPath,
};

pub(super) struct BoundPath {
    pub lookups: Vec<BoundLookup>,
}

pub(super) fn bind_path(
    binder: &mut Binder<'_>,
    snapshot: &CatalogSnapshot,
    hops: &[PathHop],
    field: &str,
    missing: MissingMatch,
    index: usize,
) -> Result<BoundPath, CompileDiagnostic> {
    if hops.len() != 2 || hops.len() > binder.options.max_depth {
        return Err(diagnostic(
            "path_lookup_limit",
            "The executable path profile requires exactly two bounded hops",
        ));
    }
    let first_relation = binder.relation;
    let first_definition = first_relation
        .definition()
        .semantics
        .as_ref()
        .and_then(|semantics| semantics.relationships.get(&hops[0].relationship))
        .ok_or_else(|| diagnostic("unknown_relationship", "Path hop is not authored"))?;
    let middle = snapshot
        .relation(&first_definition.right_relation)
        .ok_or_else(|| diagnostic("unknown_relation", "Path endpoint is missing"))?;
    let second_definition = middle
        .definition()
        .semantics
        .as_ref()
        .and_then(|semantics| semantics.relationships.get(&hops[1].relationship))
        .ok_or_else(|| diagnostic("unknown_relationship", "Path hop is not authored"))?;
    let authored_path = RelationshipPath {
        version: RELATIONSHIP_PATH_VERSION,
        start_relation: first_relation.definition().name.clone(),
        start_occurrence: binder.instance.to_owned(),
        usage: PathUsage::LookupUnique,
        steps: vec![
            PathStep {
                from_occurrence: binder.instance.to_owned(),
                to_occurrence: hops[0].instance.clone(),
                right_relation: first_definition.right_relation.clone(),
                relationship: hops[0].relationship.clone(),
                role: hops[0].role.clone(),
                as_of: hops[0].as_of.as_ref().map(|as_of| AsOfJoin {
                    fact_time: as_of.fact_time.clone(),
                    valid_from: as_of.valid_from.clone(),
                    valid_to: as_of.valid_to.clone(),
                    timezone: as_of.timezone.clone(),
                    missing: PathMissing::Null,
                    same_query_uniqueness: true,
                }),
            },
            PathStep {
                from_occurrence: hops[0].instance.clone(),
                to_occurrence: hops[1].instance.clone(),
                right_relation: second_definition.right_relation.clone(),
                relationship: hops[1].relationship.clone(),
                role: hops[1].role.clone(),
                as_of: hops[1].as_of.as_ref().map(|as_of| AsOfJoin {
                    fact_time: as_of.fact_time.clone(),
                    valid_from: as_of.valid_from.clone(),
                    valid_to: as_of.valid_to.clone(),
                    timezone: as_of.timezone.clone(),
                    missing: match missing {
                        MissingMatch::Null => PathMissing::Null,
                        MissingMatch::Exclude => PathMissing::Exclude,
                    },
                    same_query_uniqueness: true,
                }),
            },
        ],
    };
    authored_path
        .validate(
            snapshot,
            binder.options.allowed_relations.as_ref(),
            binder.options.max_depth,
        )
        .map_err(|error| diagnostic("path_lookup_contract", &error.to_string()))?;
    if first_definition.key_pairs.len() != 1 || second_definition.key_pairs.len() != 1 {
        return Err(diagnostic(
            "path_lookup_keys",
            "The two-hop executable path profile requires one key per hop",
        ));
    }
    let middle_key = &second_definition.key_pairs[0].left_field;
    let mut first = binder.lookup(
        snapshot,
        &hops[0].relationship,
        &hops[0].role,
        &hops[0].instance,
        middle_key,
        MissingMatch::Null,
        index,
    )?;
    first.output.field = Field::new(
        format!("__semantic_path_{index}_0"),
        first.output.field.data_type().clone(),
        true,
    );
    if let Some(as_of) = &hops[0].as_of {
        let fact_time = binder.field(&FieldRef {
            instance: binder.instance.into(),
            field: as_of.fact_time.clone(),
        })?;
        let right = snapshot
            .relation(&first.relationship.right.id)
            .expect("bound endpoint");
        let mut right_binder = Binder {
            relation: right,
            instance: &hops[0].instance,
            options: binder.options,
            work: binder.work,
        };
        let valid_from = right_binder.field(&FieldRef {
            instance: hops[0].instance.clone(),
            field: as_of.valid_from.clone(),
        })?;
        let valid_to = right_binder.field(&FieldRef {
            instance: hops[0].instance.clone(),
            field: as_of.valid_to.clone(),
        })?;
        if valid_from.field.is_nullable() || valid_to.field.is_nullable() {
            return Err(diagnostic(
                "path_lookup_contract",
                "As-of interval endpoints must be non-null",
            ));
        }
        first.as_of = Some(BoundAsOf {
            fact_time,
            valid_from,
            valid_to,
        });
        first.obligation = "same-query/matched-as-of-interval-count-at-most-one/v1";
    }
    let mut middle_binder = Binder {
        relation: middle,
        instance: &hops[0].instance,
        options: binder.options,
        work: binder.work,
    };
    let projected_time = if let Some(as_of) = &hops[1].as_of {
        let value = middle_binder.field(&FieldRef {
            instance: hops[0].instance.clone(),
            field: as_of.fact_time.clone(),
        })?;
        if value.field.is_nullable() {
            return Err(diagnostic(
                "path_lookup_contract",
                "Intermediate as-of time must be non-null",
            ));
        }
        let mut name = format!("__semantic_path_{index}_time");
        while first_relation.field(&name).is_some() || name == *first.output.field.name() {
            name.push('_');
        }
        let output = BoundField {
            instance: "$output".into(),
            field: Field::new(name, value.field.data_type().clone(), true),
        };
        first.extra_value = Some(BoundLookupExtra {
            value,
            output: output.clone(),
        });
        Some(output)
    } else {
        None
    };
    let mut second = middle_binder.lookup(
        snapshot,
        &hops[1].relationship,
        &hops[1].role,
        &hops[1].instance,
        field,
        missing,
        index,
    )?;
    if second.relationship.keys[0].0.field.data_type() != first.output.field.data_type() {
        return Err(diagnostic(
            "path_lookup_key_type",
            "Intermediate path key types must match exactly",
        ));
    }
    second.relationship.keys[0].0 = first.output.clone();
    if let (Some(as_of), Some(fact_time)) = (&hops[1].as_of, projected_time) {
        let right = snapshot
            .relation(&second.relationship.right.id)
            .expect("bound second endpoint");
        let mut right_binder = Binder {
            relation: right,
            instance: &hops[1].instance,
            options: binder.options,
            work: binder.work,
        };
        let valid_from = right_binder.field(&FieldRef {
            instance: hops[1].instance.clone(),
            field: as_of.valid_from.clone(),
        })?;
        let valid_to = right_binder.field(&FieldRef {
            instance: hops[1].instance.clone(),
            field: as_of.valid_to.clone(),
        })?;
        if valid_from.field.is_nullable() || valid_to.field.is_nullable() {
            return Err(diagnostic(
                "path_lookup_contract",
                "As-of interval endpoints must be non-null",
            ));
        }
        second.as_of = Some(BoundAsOf {
            fact_time,
            valid_from,
            valid_to,
        });
        second.obligation = "same-query/matched-as-of-interval-count-at-most-one/v1";
    }
    second.output.field = Field::new(
        format!("__semantic_path_{index}_1"),
        second.output.field.data_type().clone(),
        true,
    );
    Ok(BoundPath {
        lookups: vec![first, second],
    })
}
