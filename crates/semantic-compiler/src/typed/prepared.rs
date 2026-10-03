//! Unbound row proposals have no execution path. Supplying values creates a
//! fresh typed compilation, so value-dependent catalog and temporal checks run
//! at every binding rather than being inherited from a prior execution.

use std::collections::{BTreeMap, BTreeSet};

use semantic_engine::Engine;
use semantic_plan::typed::{Literal, RowOperation, RowPredicate, RowQuery, TimestampUnit};

use super::{
    Calendar, CompileDiagnostic, CompileOptions, ContextOrigin, RequestContext, TypedCompilation,
    compile_rows, diagnostic, preflight,
};

const MAX_PARAMETERS: usize = 16;
const MAX_PREDICATE_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreparedType {
    Boolean,
    Int64,
    Float64,
    Utf8,
    Date32,
    TimestampMicrosUtc,
}
impl PreparedType {
    fn accepts(self, value: &Literal) -> bool {
        match (self, value) {
            (Self::Boolean, Literal::Boolean(_))
            | (Self::Int64, Literal::Int64(_))
            | (Self::Float64, Literal::Float64(_))
            | (Self::Utf8, Literal::Utf8(_))
            | (Self::Date32, Literal::Date32(_)) => true,
            (
                Self::TimestampMicrosUtc,
                Literal::Timestamp {
                    unit: TimestampUnit::Microsecond,
                    timezone: Some(zone),
                    ..
                },
            ) => zone == "UTC",
            _ => false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterDeclaration {
    pub name: String,
    pub value_type: PreparedType,
}

#[derive(Debug, Clone)]
pub struct PreparedReferenceContext {
    pub instant_parameter: String,
    pub timezone: String,
    pub calendar: Calendar,
    pub origin: ContextOrigin,
}

pub struct PreparedRows {
    query: RowQuery,
    declarations: BTreeMap<String, PreparedType>,
    reference: Option<PreparedReferenceContext>,
    options: CompileOptions,
    snapshot_id: String,
}

impl PreparedRows {
    pub fn prepare(
        engine: &Engine,
        query: RowQuery,
        declarations: Vec<ParameterDeclaration>,
        reference: Option<PreparedReferenceContext>,
        options: CompileOptions,
    ) -> Result<Self, CompileDiagnostic> {
        if declarations.is_empty() || declarations.len() > MAX_PARAMETERS {
            return Err(diagnostic(
                "parameter_contract",
                "Prepared rows require one to sixteen typed parameters",
            ));
        }
        if reference.is_some() && options.request_context.is_some() {
            return Err(diagnostic(
                "parameter_contract",
                "A prepared reference instant cannot replace an existing request context",
            ));
        }
        preflight(&query, &options)?;
        let mut declared = BTreeMap::new();
        for declaration in declarations {
            if !valid_name(&declaration.name)
                || declared
                    .insert(declaration.name, declaration.value_type)
                    .is_some()
            {
                return Err(diagnostic(
                    "parameter_contract",
                    "Parameter names must be unique bounded identifiers",
                ));
            }
        }
        let mut uses = BTreeMap::<String, usize>::new();
        for requirement in &query.requirements {
            match &requirement.operation {
                RowOperation::Filter { predicate }
                | RowOperation::Related {
                    predicate: Some(predicate),
                    ..
                } => count_uses(predicate, 0, &mut uses)?,
                RowOperation::FilterOutput { predicate, .. } => {
                    count_uses(predicate, 0, &mut uses)?
                }
                _ => {}
            }
        }
        if let Some(reference) = &reference {
            let context = RequestContext {
                reference_unix_millis: 0,
                timezone: reference.timezone.clone(),
                calendar: reference.calendar,
                origin: reference.origin.clone(),
            };
            context.validate()?;
            *uses.entry(reference.instant_parameter.clone()).or_default() += 1;
            if declared.get(&reference.instant_parameter) != Some(&PreparedType::Int64) {
                return Err(diagnostic(
                    "parameter_contract",
                    "The prepared reference instant requires an Int64 millisecond declaration",
                ));
            }
        }
        if uses.len() != declared.len() || uses.keys().any(|name| !declared.contains_key(name)) {
            return Err(diagnostic(
                "parameter_contract",
                "Every declared parameter must be used and every use declared",
            ));
        }
        Ok(Self {
            query,
            declarations: declared,
            reference,
            options,
            snapshot_id: engine.catalog().snapshot().id().to_owned(),
        })
    }

    pub fn declarations(&self) -> &BTreeMap<String, PreparedType> {
        &self.declarations
    }

    /// Current authorization may narrow the preparation scope, never expand it.
    pub async fn bind_values(
        &self,
        engine: &Engine,
        values: BTreeMap<String, Literal>,
        current_scope: Option<&BTreeSet<String>>,
    ) -> Result<TypedCompilation, CompileDiagnostic> {
        if engine.catalog().snapshot().id() != self.snapshot_id {
            return Err(diagnostic(
                "snapshot_mismatch",
                "Prepare again against the current catalog snapshot",
            ));
        }
        if self.options.allowed_relations.is_some() && current_scope.is_none() {
            return Err(diagnostic(
                "execution_scope",
                "A scoped preparation requires current execution authorization",
            ));
        }
        if values.len() != self.declarations.len() {
            return Err(diagnostic(
                "parameter_count",
                "Every required value must be supplied exactly once, with no extras",
            ));
        }
        for (name, declared_type) in &self.declarations {
            let value = values.get(name).ok_or_else(|| {
                diagnostic("parameter_missing", "A required parameter value is missing")
            })?;
            if !declared_type.accepts(value) {
                return Err(diagnostic(
                    "parameter_type",
                    "Parameter value has a different physical type than declared",
                ));
            }
        }
        if values
            .keys()
            .any(|name| !self.declarations.contains_key(name))
        {
            return Err(diagnostic(
                "parameter_extra",
                "An undeclared parameter was supplied",
            ));
        }
        let mut query = self.query.clone();
        for requirement in &mut query.requirements {
            match &mut requirement.operation {
                RowOperation::Filter { predicate }
                | RowOperation::Related {
                    predicate: Some(predicate),
                    ..
                } => substitute(predicate, &values, 0)?,
                RowOperation::FilterOutput { predicate, .. } => substitute(predicate, &values, 0)?,
                _ => {}
            }
        }
        let mut options = self.options.clone();
        options.allowed_relations = match (&self.options.allowed_relations, current_scope) {
            (Some(prepared), Some(current)) => {
                Some(prepared.intersection(current).cloned().collect())
            }
            (None, Some(current)) => Some(current.clone()),
            (Some(_), None) => unreachable!("checked scoped preparation"),
            (None, None) => None,
        };
        if let Some(reference) = &self.reference {
            let Some(Literal::Int64(instant)) = values.get(&reference.instant_parameter) else {
                unreachable!("validated reference instant type")
            };
            options.request_context = Some(RequestContext {
                reference_unix_millis: *instant,
                timezone: reference.timezone.clone(),
                calendar: reference.calendar,
                origin: reference.origin.clone(),
            });
        }
        Ok(compile_rows(engine, query, options).await)
    }
}

fn valid_name(name: &str) -> bool {
    name.len() <= 64
        && name
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

fn count_uses<F>(
    predicate: &RowPredicate<F>,
    depth: usize,
    uses: &mut BTreeMap<String, usize>,
) -> Result<(), CompileDiagnostic> {
    if depth > MAX_PREDICATE_DEPTH {
        return Err(diagnostic(
            "parameter_contract",
            "Prepared predicate is too deep",
        ));
    }
    match predicate {
        RowPredicate::CompareParameter { parameter, .. } => {
            *uses.entry(parameter.clone()).or_default() += 1;
        }
        RowPredicate::All { predicates } | RowPredicate::Any { predicates } => {
            for predicate in predicates {
                count_uses(predicate, depth + 1, uses)?;
            }
        }
        RowPredicate::Not { predicate } => count_uses(predicate, depth + 1, uses)?,
        _ => {}
    }
    Ok(())
}

fn substitute<F: Clone>(
    predicate: &mut RowPredicate<F>,
    values: &BTreeMap<String, Literal>,
    depth: usize,
) -> Result<(), CompileDiagnostic> {
    if depth > MAX_PREDICATE_DEPTH {
        return Err(diagnostic(
            "parameter_contract",
            "Prepared predicate is too deep",
        ));
    }
    match predicate {
        RowPredicate::CompareParameter {
            field,
            operator,
            parameter,
        } => {
            let value = values.get(parameter).ok_or_else(|| {
                diagnostic("parameter_missing", "A required parameter value is missing")
            })?;
            *predicate = RowPredicate::Compare {
                field: field.clone(),
                operator: *operator,
                value: value.clone(),
            };
        }
        RowPredicate::All { predicates } | RowPredicate::Any { predicates } => {
            for predicate in predicates {
                substitute(predicate, values, depth + 1)?;
            }
        }
        RowPredicate::Not { predicate } => substitute(predicate, values, depth + 1)?,
        _ => {}
    }
    Ok(())
}
