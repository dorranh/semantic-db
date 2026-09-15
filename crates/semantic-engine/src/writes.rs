use crate::*;
use datafusion::sql::sqlparser::ast::Statement;
use datafusion::{
    arrow::datatypes::DataType,
    common::{
        ScalarValue,
        tree_node::{TreeNode, TreeNodeRecursion},
    },
    logical_expr::{Expr as DfExpr, LogicalPlan},
    sql::sqlparser::{
        ast::*,
        dialect::GenericDialect,
        parser::Parser,
        tokenizer::{Token, Tokenizer},
    },
};
use semantic_runtime::{failure, staging::StagedInput};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

#[derive(Debug, Clone)]
pub struct WriteDescription {
    pub parameters: Vec<DataType>,
    pub command: String,
}
pub struct PreparedWrite<'a> {
    engine: &'a Engine,
    explain_only: bool,
    generation: u64,
    pub(crate) plan: MutationPlan,
    binding: WriteBinding,
    source: Option<String>,
    description_sql: String,
    explanation: WriteExplanation,
}
fn reject<T>(message: &str) -> Result<T> {
    Err(failure(message).into())
}

/// Classifies using tokens, never substring matching or splitting on semicolons.
pub fn is_write_statement(sql: &str) -> bool {
    dispatch(sql).is_some()
}
pub fn is_write_explanation(sql: &str) -> bool {
    dispatch(sql) == Some(true)
}
fn dispatch(sql: &str) -> Option<bool> {
    let tokens = Tokenizer::new(&GenericDialect {}, sql).tokenize().ok()?;
    let mut tokens = tokens.iter().filter(|t| !matches!(t, Token::Whitespace(_)));
    let Token::Word(mut word) = tokens.next()?.clone() else {
        return None;
    };
    if word.quote_style.is_some() {
        return None;
    }
    let explain = word.value.eq_ignore_ascii_case("EXPLAIN");
    if explain {
        let Token::Word(next) = tokens.next()? else {
            return None;
        };
        word = next.clone();
    }
    (word.quote_style.is_none()
        && ["REQUIRE", "INSERT", "UPDATE", "DELETE", "MERGE"]
            .contains(&word.value.to_ascii_uppercase().as_str()))
    .then_some(explain)
}

fn statement(sql: &str) -> Result<(bool, Statement)> {
    let tokens = Tokenizer::new(&GenericDialect {}, sql)
        .tokenize()
        .map_err(|_| failure("invalid SQL tokens"))?;
    let mut tokens: Vec<_> = tokens
        .into_iter()
        .filter(|t| !matches!(t, Token::Whitespace(_)))
        .collect();
    let word = |t: Option<&Token>, value: &str| matches!(t,Some(Token::Word(w)) if w.quote_style.is_none() && w.value.eq_ignore_ascii_case(value));
    if word(tokens.first(), "EXPLAIN") {
        tokens.remove(0);
    }
    let checked = word(tokens.first(), "REQUIRE");
    if checked {
        if !word(tokens.get(1), "IDEMPOTENT") {
            return reject("expected REQUIRE IDEMPOTENT");
        }
        tokens.drain(..2);
    }
    let mut statements = Parser::new(&GenericDialect {})
        .with_tokens(tokens)
        .parse_statements()
        .map_err(|_| failure("invalid write syntax"))?;
    if statements.len() != 1 {
        return reject(
            "exactly one statement is required; transaction control uses the library API",
        );
    }
    let stmt = statements.remove(0);
    if checked && !matches!(stmt, Statement::Merge(_)) {
        return reject("REQUIRE IDEMPOTENT supports keyed MERGE only");
    }
    Ok((checked, stmt))
}
fn ident(i: &Ident) -> String {
    if i.quote_style.is_some() {
        i.value.clone()
    } else {
        i.value.to_ascii_lowercase()
    }
}
fn name(n: &ObjectName) -> Result<String> {
    if n.0.len() != 1 {
        return reject("only unqualified registered names are supported");
    }
    n.0[0]
        .as_ident()
        .map(ident)
        .ok_or_else(|| failure("identifier required").into())
}
fn table(t: &TableFactor) -> Result<(String, String)> {
    match t {
        TableFactor::Table {
            name: n,
            alias,
            args: None,
            with_hints,
            version: None,
            partitions,
            ..
        } if with_hints.is_empty() && partitions.is_empty() => {
            let n = name(n)?;
            if alias.as_ref().is_some_and(|a| !a.columns.is_empty()) {
                return reject("target column aliases unsupported");
            }
            Ok((
                n.clone(),
                alias.as_ref().map(|a| ident(&a.name)).unwrap_or(n),
            ))
        }
        _ => reject("one registered base target is required"),
    }
}
fn joined(t: &TableWithJoins) -> Result<(String, String)> {
    if !t.joins.is_empty() {
        return reject("joined write targets unsupported");
    }
    table(&t.relation)
}
fn column(e: &Expr, alias: &str) -> Result<String> {
    match e {
        Expr::Identifier(i) => Ok(ident(i)),
        Expr::CompoundIdentifier(parts) if parts.len() == 2 && ident(&parts[0]) == alias => {
            Ok(ident(&parts[1]))
        }
        Expr::Nested(e) => column(e, alias),
        _ => reject("expected a column of the declared relation"),
    }
}
fn assigned(a: &Assignment) -> Result<String> {
    match &a.target {
        AssignmentTarget::ColumnName(n) => name(n),
        _ => reject("tuple assignments unsupported"),
    }
}
fn mapped(columns: &BTreeMap<String, String>, n: &str) -> Result<String> {
    columns
        .get(n)
        .cloned()
        .ok_or_else(|| failure("unknown or unmapped target column").into())
}
pub(crate) fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
fn data_type(t: &datafusion::sql::sqlparser::ast::DataType) -> Result<DataType> {
    use datafusion::sql::sqlparser::ast::DataType as T;
    Ok(match t {
        T::Text | T::Varchar(_) => DataType::Utf8,
        T::Boolean | T::Bool => DataType::Boolean,
        T::SmallInt(_) => DataType::Int16,
        T::Int(_) | T::Integer(_) => DataType::Int32,
        T::BigInt(_) => DataType::Int64,
        T::Real => DataType::Float32,
        T::Double(_) | T::DoublePrecision => DataType::Float64,
        T::Date => DataType::Date32,
        T::Timestamp(_, tz) => DataType::Timestamp(
            datafusion::arrow::datatypes::TimeUnit::Microsecond,
            if matches!(tz, TimezoneInfo::WithTimeZone | TimezoneInfo::Tz) {
                Some("UTC".into())
            } else {
                None
            },
        ),
        _ => return reject("unsupported mutation cast"),
    })
}
fn expression(e: &Expr, alias: &str, columns: &BTreeMap<String, String>) -> Result<ValueExpr> {
    Ok(match e {
        Expr::Identifier(_) | Expr::CompoundIdentifier(_) => {
            ValueExpr::Column(mapped(columns, &column(e, alias)?)?)
        }
        Expr::Nested(e) => expression(e, alias, columns)?,
        Expr::Value(v) => match &v.value {
            Value::Placeholder(p) => ValueExpr::Parameter(crate::parameters::position(p)? - 1),
            Value::SingleQuotedString(s) => ValueExpr::Literal(ScalarValue::Utf8(Some(s.clone()))),
            Value::Boolean(b) => ValueExpr::Literal(ScalarValue::Boolean(Some(*b))),
            Value::Null => ValueExpr::Literal(ScalarValue::Null),
            Value::Number(s, _) => ValueExpr::Literal(if let Ok(n) = s.parse::<i64>() {
                ScalarValue::Int64(Some(n))
            } else {
                ScalarValue::Float64(Some(
                    s.parse()
                        .map_err(|_| failure("unsupported numeric literal"))?,
                ))
            }),
            _ => return reject("unsupported mutation literal"),
        },
        Expr::BinaryOp { left, op, right }
            if matches!(
                op,
                BinaryOperator::Eq
                    | BinaryOperator::NotEq
                    | BinaryOperator::Lt
                    | BinaryOperator::LtEq
                    | BinaryOperator::Gt
                    | BinaryOperator::GtEq
                    | BinaryOperator::And
                    | BinaryOperator::Or
                    | BinaryOperator::Plus
                    | BinaryOperator::Minus
                    | BinaryOperator::Multiply
                    | BinaryOperator::Divide
            ) =>
        {
            ValueExpr::Binary(
                Box::new(expression(left, alias, columns)?),
                op.clone(),
                Box::new(expression(right, alias, columns)?),
            )
        }
        Expr::UnaryOp { op, expr }
            if matches!(
                op,
                UnaryOperator::Plus | UnaryOperator::Minus | UnaryOperator::Not
            ) =>
        {
            ValueExpr::Unary(*op, Box::new(expression(expr, alias, columns)?))
        }
        Expr::Cast {
            expr,
            data_type: t,
            format: None,
            ..
        } => ValueExpr::Cast(Box::new(expression(expr, alias, columns)?), data_type(t)?),
        Expr::IsNull(e) => ValueExpr::IsNull(Box::new(expression(e, alias, columns)?), false),
        Expr::IsNotNull(e) => ValueExpr::IsNull(Box::new(expression(e, alias, columns)?), true),
        _ => {
            return reject(
                "unsupported mutation expression; use deterministic columns, values, parameters, casts and scalar operators",
            );
        }
    })
}
fn keys(e: &Expr, t: &str, s: &str, out: &mut Vec<(String, String)>) -> Result<()> {
    match e {
        Expr::Nested(e) => keys(e, t, s, out),
        Expr::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        } => {
            keys(left, t, s, out)?;
            keys(right, t, s, out)
        }
        Expr::BinaryOp {
            left,
            op: BinaryOperator::Eq,
            right,
        } => {
            // Both sides must be qualified so an unqualified column cannot be mistaken for the other input.
            if !matches!(left.as_ref(), Expr::CompoundIdentifier(_))
                || !matches!(right.as_ref(), Expr::CompoundIdentifier(_))
            {
                return reject("MERGE keys must be qualified target/source columns");
            }
            let pair = column(left, t)
                .and_then(|a| Ok((a, column(right, s)?)))
                .or_else(|_| column(right, t).and_then(|a| Ok((a, column(left, s)?))))?;
            out.push(pair);
            Ok(())
        }
        _ => reject("MERGE ON must be a conjunction of key equalities"),
    }
}
impl Engine {
    pub async fn attach_write_binding(
        &mut self,
        relation: &str,
        binding: WriteBinding,
    ) -> Result<()> {
        let info = binding.connection.inspect_target(&binding.target).await?;
        verify_write_mapping(
            self.catalog
                .relation(relation)
                .ok_or_else(|| failure("unknown binding relation"))?,
            &binding,
            &info,
        )?;
        self.install_write_binding(relation, binding, info).await
    }
    async fn install_write_binding(
        &mut self,
        relation: &str,
        binding: WriteBinding,
        info: TargetInspection,
    ) -> Result<()> {
        self.resource_identities.insert(
            relation.into(),
            ResourceIdentity {
                namespace: info.physical_namespace.clone(),
                domain: Some(info.domain.clone()),
                resource: Some(info.resource.clone()),
            },
        );
        self.write_domains.insert(relation.into(), info.domain);
        self.write_bindings.insert(relation.into(), binding);
        self.binding_generation += 1;
        // Exclude dependent generations immediately, including those loaded by later executions.
        self.invalidate_writable_caches().await?;
        Ok(())
    }
    pub fn attach_read_binding(&mut self, relation: &str, binding: ReadBinding) -> Result<()> {
        self.validate_mapping(relation, &binding.columns, false)?;
        if let Some(namespace) = binding.connection.physical_namespace() {
            self.resource_identities.insert(
                relation.into(),
                ResourceIdentity {
                    namespace: namespace.into(),
                    domain: Some(binding.connection.domain()),
                    resource: Some(binding.resource.clone()),
                },
            );
        }
        self.read_bindings.insert(relation.into(), binding);
        self.binding_generation += 1;
        Ok(())
    }
    pub fn attach_resource_identity(
        &mut self,
        relation: &str,
        identity: ResourceIdentity,
    ) -> Result<()> {
        if !self
            .catalog
            .relation(relation)
            .is_some_and(|r| matches!(r.kind, RelationKind::Base { .. }))
            || identity.namespace.is_empty()
            || identity.domain.as_ref().is_some_and(String::is_empty)
            || identity.resource.as_ref().is_some_and(String::is_empty)
        {
            return reject(
                "resource identity requires a registered base relation and connector-issued identifiers",
            );
        }
        self.resource_identities.insert(relation.into(), identity);
        self.binding_generation += 1;
        Ok(())
    }
    fn validate_mapping(
        &self,
        relation: &str,
        columns: &BTreeMap<String, String>,
        writable: bool,
    ) -> Result<()> {
        let r = self
            .catalog
            .relation(relation)
            .ok_or_else(|| failure("unknown binding relation"))?;
        if !matches!(r.kind, RelationKind::Base { .. })
            || columns.len() != r.schema.fields().len()
            || columns.keys().any(|c| r.schema.field_with_name(c).is_err())
            || writable && columns.values().collect::<BTreeSet<_>>().len() != columns.len()
        {
            return reject("binding requires a reversible base-table column mapping");
        }
        Ok(())
    }
    pub async fn register_writable_table(
        &mut self,
        relation: semantic_catalog::Relation,
        provider: Arc<dyn TableProvider>,
        binding: WriteBinding,
    ) -> Result<()> {
        self.check_new_name(&relation.name)?;
        let info = binding.connection.inspect_target(&binding.target).await?;
        verify_write_mapping(&relation, &binding, &info)?;
        let n = relation.name.clone();
        self.register_table(relation, provider)?;
        self.install_write_binding(&n, binding, info).await
    }
    pub async fn create_table(
        &mut self,
        connection: Arc<dyn WriteConnection>,
        definition: TableDefinition,
    ) -> Result<()> {
        self.check_new_name(&definition.name)?;
        let created = connection.create_table(&definition).await?;
        let target = created.write.target.clone();
        let registered = async {
            self.register_table(
                semantic_catalog::Relation::base(
                    &definition.name,
                    created.provider.schema(),
                    "application",
                ),
                created.provider,
            )?;
            self.attach_read_binding(&definition.name, created.read)?;
            self.attach_write_binding(&definition.name, created.write)
                .await
        }
        .await;
        registered.map_err(|_| {
            failure(&format!(
                "physical table created but registration failed; recover target {target:?}"
            ))
            .into()
        })
    }
    pub async fn prepare_write(&self, sql: &str) -> Result<PreparedWrite<'_>> {
        self.prepare_write_hints(sql, &[]).await
    }
    pub async fn describe_write(
        &self,
        sql: &str,
        hints: &[Option<DataType>],
    ) -> Result<WriteDescription> {
        let p = self.prepare_write_hints(sql, hints).await?;
        let d = self.describe_read(&p.description_sql, hints).await?;
        Ok(WriteDescription {
            parameters: d.parameters,
            command: p.plan.mutation.command().into(),
        })
    }
    async fn prepare_write_hints(
        &self,
        sql: &str,
        hints: &[Option<DataType>],
    ) -> Result<PreparedWrite<'_>> {
        let explain_only = Tokenizer::new(&GenericDialect {}, sql).tokenize().map_err(|_|failure("invalid SQL"))?.iter().find(|t|!matches!(t,Token::Whitespace(_))).is_some_and(|t|matches!(t,Token::Word(w) if w.quote_style.is_none() && w.value.eq_ignore_ascii_case("EXPLAIN")));
        let (checked, stmt) = statement(sql)?;
        let sql = self.parameter_sql(&stmt.to_string(), hints)?;
        let (_, stmt) = statement(&sql)?;
        let (target, alias) = match &stmt {
            Statement::Insert(i) => match &i.table {
                TableObject::TableName(n) => {
                    let n = name(n)?;
                    (n.clone(), n)
                }
                _ => return reject("registered INSERT target required"),
            },
            Statement::Update(u) => joined(&u.table)?,
            Statement::Delete(d) => {
                let (FromTable::WithFromKeyword(v) | FromTable::WithoutKeyword(v)) = &d.from;
                if v.len() != 1 {
                    return reject("one DELETE target required");
                }
                joined(&v[0])?
            }
            Statement::Merge(m) => table(&m.table)?,
            _ => {
                return reject(
                    "expected INSERT, UPDATE, DELETE or MERGE; transaction control uses the library API",
                );
            }
        };
        let binding = self
            .write_bindings
            .get(&target)
            .cloned()
            .ok_or_else(|| failure("target is read-only; attach an explicit write binding"))?;
        let info = binding.connection.inspect_target(&binding.target).await?;
        let mut source = None;
        let description_sql;
        let mutation = match stmt {
            Statement::Update(u) => {
                if u.from.is_some()
                    || u.returning.is_some()
                    || u.output.is_some()
                    || u.or.is_some()
                    || u.limit.is_some()
                    || !u.order_by.is_empty()
                    || !u.optimizer_hints.is_empty()
                {
                    return reject("unsupported UPDATE clauses");
                }
                let mut assignments = vec![];
                let mut projection = vec![];
                for a in u.assignments {
                    let n = assigned(&a)?;
                    let ty = self
                        .catalog
                        .relation(&target)
                        .unwrap()
                        .schema
                        .field_with_name(&n)
                        .map_err(datafusion::error::DataFusionError::from)?
                        .data_type()
                        .clone();
                    projection.push(format!(
                        "CAST({} AS {})",
                        a.value,
                        crate::parameters::sql_type(&ty)?
                    ));
                    assignments.push((
                        mapped(&binding.columns, &n)?,
                        expression(&a.value, &alias, &binding.columns)?,
                    ));
                }
                unique(assignments.iter().map(|(c, _)| c.as_str()))?;
                let predicate = u
                    .selection
                    .as_ref()
                    .map(|e| expression(e, &alias, &binding.columns))
                    .transpose()?;
                description_sql = format!(
                    "SELECT {} FROM {} AS {} {}",
                    projection.join(", "),
                    quote(&target),
                    quote(&alias),
                    u.selection
                        .map(|e| format!("WHERE {e}"))
                        .unwrap_or_default()
                );
                Mutation::Update {
                    assignments,
                    predicate,
                }
            }
            Statement::Delete(d) => {
                if d.using.is_some()
                    || d.returning.is_some()
                    || d.output.is_some()
                    || d.limit.is_some()
                    || !d.order_by.is_empty()
                    || !d.tables.is_empty()
                    || !d.optimizer_hints.is_empty()
                {
                    return reject("unsupported DELETE clauses");
                }
                let predicate = d
                    .selection
                    .as_ref()
                    .map(|e| expression(e, &alias, &binding.columns))
                    .transpose()?;
                description_sql = format!(
                    "SELECT 1 FROM {} AS {} {}",
                    quote(&target),
                    quote(&alias),
                    d.selection
                        .map(|e| format!("WHERE {e}"))
                        .unwrap_or_default()
                );
                Mutation::Delete { predicate }
            }
            Statement::Insert(i) => {
                if i.on.is_some()
                    || i.returning.is_some()
                    || i.output.is_some()
                    || i.overwrite
                    || i.ignore
                    || i.or.is_some()
                    || i.partitioned.is_some()
                    || i.replace_into
                    || i.priority.is_some()
                    || i.insert_alias.is_some()
                    || i.settings.is_some()
                    || i.format_clause.is_some()
                    || i.multi_table_insert_type.is_some()
                    || !i.assignments.is_empty()
                    || !i.after_columns.is_empty()
                    || !i.multi_table_into_clauses.is_empty()
                    || !i.multi_table_when_clauses.is_empty()
                    || i.multi_table_else_clause.is_some()
                    || !i.optimizer_hints.is_empty()
                {
                    return reject("unsupported INSERT clauses");
                }
                let columns = if i.columns.is_empty() {
                    self.catalog
                        .relation(&target)
                        .unwrap()
                        .schema
                        .fields()
                        .iter()
                        .map(|f| f.name().clone())
                        .collect::<Vec<_>>()
                } else {
                    i.columns.iter().map(name).collect::<Result<Vec<_>>>()?
                };
                unique(columns.iter().map(String::as_str))?;
                let query = i
                    .source
                    .ok_or_else(|| failure("INSERT requires VALUES or SELECT"))?
                    .to_string();
                // Supply destination types to standalone VALUES placeholders.
                let aliases = (0..columns.len())
                    .map(|i| quote(&format!("c{i}")))
                    .collect::<Vec<_>>();
                let selects = columns
                    .iter()
                    .enumerate()
                    .map(|(j, c)| {
                        let ty = self
                            .catalog
                            .relation(&target)
                            .unwrap()
                            .schema
                            .field_with_name(c)
                            .map_err(datafusion::error::DataFusionError::from)?
                            .data_type();
                        Ok(format!(
                            "CAST({} AS {})",
                            aliases[j],
                            crate::parameters::sql_type(ty)?
                        ))
                    })
                    .collect::<Result<Vec<_>>>()?;
                let query = format!(
                    "SELECT {} FROM ({query}) AS input ({})",
                    selects.join(","),
                    aliases.join(",")
                );
                description_sql = query.clone();
                source = Some(query);
                Mutation::Insert {
                    columns: columns
                        .iter()
                        .map(|c| mapped(&binding.columns, c))
                        .collect::<Result<_>>()?,
                }
            }
            Statement::Merge(m) => {
                if m.output.is_some() || !m.optimizer_hints.is_empty() {
                    return reject("MERGE output/hints unsupported");
                }
                let (query, source_alias) = match m.source {
                    TableFactor::Derived {
                        lateral: false,
                        subquery,
                        alias: Some(a),
                        ..
                    } if a.columns.is_empty() => (subquery.to_string(), ident(&a.name)),
                    _ => return reject("MERGE USING requires an aliased SELECT"),
                };
                if source_alias == alias {
                    return reject("source and target aliases must differ");
                }
                let mut key_pairs = vec![];
                keys(&m.on, &alias, &source_alias, &mut key_pairs)?;
                unique(key_pairs.iter().map(|(k, _)| k.as_str()))?;
                let physical_keys = key_pairs
                    .iter()
                    .map(|(k, _)| mapped(&binding.columns, k))
                    .collect::<Result<Vec<_>>>()?;
                if !info.unique_keys.iter().any(|k| {
                    k.iter().collect::<BTreeSet<_>>()
                        == physical_keys.iter().collect::<BTreeSet<_>>()
                }) {
                    return reject("MERGE requires a complete enforced non-null unique key");
                }
                let schema = self.describe_read(&query, hints).await?.schema;
                let source_columns = schema
                    .fields()
                    .iter()
                    .map(|f| (f.name().clone(), f.name().clone()))
                    .collect::<BTreeMap<_, _>>();
                let mut projections = vec![];
                let mut normalized_keys = vec![];
                let mut updates = vec![];
                let mut inserts = vec![];
                for (j, ((_, s), p)) in key_pairs.iter().zip(&physical_keys).enumerate() {
                    if !source_columns.contains_key(s) {
                        return reject("unknown source key");
                    }
                    let a = format!("k{j}");
                    projections.push(format!(
                        "{}.{} AS {}",
                        quote(&source_alias),
                        quote(s),
                        quote(&a)
                    ));
                    normalized_keys.push((p.clone(), a));
                }
                let mut saw_update = false;
                let mut saw_insert = false;
                for clause in m.clauses {
                    if clause.predicate.is_some() {
                        return reject("conditional MERGE clauses unsupported");
                    }
                    match (clause.clause_kind, clause.action) {
                        (MergeClauseKind::Matched, MergeAction::Update(u))
                            if !saw_update
                                && u.update_predicate.is_none()
                                && u.delete_predicate.is_none() =>
                        {
                            saw_update = true;
                            for a in u.assignments {
                                let n = assigned(&a)?;
                                let p = mapped(&binding.columns, &n)?;
                                if physical_keys.contains(&p) {
                                    return reject("MERGE cannot update key columns");
                                }
                                expression(&a.value, &source_alias, &source_columns)?;
                                let staged = format!("u{}", updates.len());
                                projections.push(format!("{} AS {}", a.value, quote(&staged)));
                                updates.push((p, staged));
                            }
                        }
                        (MergeClauseKind::NotMatched, MergeAction::Insert(i))
                            if !saw_insert && i.insert_predicate.is_none() =>
                        {
                            saw_insert = true;
                            let MergeInsertKind::Values(v) = i.kind else {
                                return reject("MERGE insert requires explicit VALUES");
                            };
                            if v.rows.len() != 1 || v.rows[0].len() != i.columns.len() {
                                return reject("MERGE insert requires one value per named column");
                            }
                            for (n, e) in i.columns.iter().zip(v.rows[0].iter()) {
                                let n = name(n)?;
                                let p = mapped(&binding.columns, &n)?;
                                expression(e, &source_alias, &source_columns)?;
                                if let Some(j) = physical_keys.iter().position(|k| k == &p)
                                    && column(e, &source_alias)? != key_pairs[j].1
                                {
                                    return reject("inserted keys must equal matched source keys");
                                }
                                let staged = format!("i{}", inserts.len());
                                projections.push(format!("{e} AS {}", quote(&staged)));
                                inserts.push((p, staged));
                            }
                            if physical_keys
                                .iter()
                                .any(|k| !inserts.iter().any(|(c, _)| c == k))
                            {
                                return reject("MERGE insert must assign every key");
                            }
                        }
                        _ => {
                            return reject(
                                "only one unconditional matched UPDATE and unmatched INSERT are supported",
                            );
                        }
                    }
                }
                if !saw_update && !saw_insert {
                    return reject("MERGE requires an action");
                }
                unique(updates.iter().map(|(n, _)| n.as_str()))?;
                unique(inserts.iter().map(|(n, _)| n.as_str()))?;
                let query = format!(
                    "SELECT {} FROM ({query}) AS {}",
                    projections.join(","),
                    quote(&source_alias)
                );
                description_sql = query.clone();
                source = Some(query);
                Mutation::Merge {
                    keys: normalized_keys,
                    updates,
                    inserts,
                }
            }
            _ => unreachable!(),
        };
        let mut observations = vec![];
        if let Some(sql) = &source {
            observations = self.base_dependencies(sql)?;
            for n in &observations {
                if checked {
                    let source_identity = self.resource_identities.get(n).ok_or_else(|| {
                        failure("checked input requires verified physical resource identity")
                    })?;
                    if source_identity.namespace == info.physical_namespace
                        && (source_identity.domain.as_ref() != Some(&info.domain)
                            || source_identity.resource.is_none()
                            || source_identity.resource.as_ref() == Some(&info.resource))
                    {
                        return reject(
                            "checked source may overlap the destination physical resource; use verified disjoint bindings",
                        );
                    }
                }
                if n == &target
                    || self
                        .read_bindings
                        .get(n)
                        .is_some_and(|r| r.resource == info.resource)
                    || self.write_bindings.get(n).is_some_and(|b| {
                        Arc::ptr_eq(&b.connection, &binding.connection)
                            && b.target == binding.target
                    })
                {
                    return reject(
                        "write input cannot read its destination, including aliases and views",
                    );
                }
            }
            let frame = self.plan_sql(sql).await?;
            if checked {
                validate_source(frame.logical_plan())?;
            }
        }
        // Resolve assignments and parameters even when no source is needed.
        self.plan_sql(&description_sql).await?;
        let plan = MutationPlan {
            target: binding.target.clone(),
            revision: info.revision,
            mutation,
            checked,
        };
        if !info
            .supported_operations
            .iter()
            .any(|op| op == plan.mutation.command())
        {
            return reject("connector does not support this mutation");
        }
        if checked && !info.atomic_writes {
            return reject("checked merge requires atomic writes");
        }
        binding.connection.validate_operation(&plan).await?;
        let explanation = WriteExplanation {
            operation: plan.mutation.command().into(),
            target,
            boundary: info.domain,
            require_idempotent: checked,
            mapped_keys: match &plan.mutation {
                Mutation::Merge { keys, .. } => keys.clone(),
                _ => vec![],
            },
            atomic_supported: info.atomic_writes,
            source_observations: observations,
            static_checks: vec![
                "registered target and reversible mapping".into(),
                "normalized expressions and source lineage".into(),
            ],
            runtime_checks: vec![
                "complete staged input within budgets".into(),
                "live schema, key and side-effect validation protected through commit".into(),
                "destination-native source key equality and uniqueness".into(),
            ],
        };
        Ok(PreparedWrite {
            engine: self,
            explain_only,
            generation: self.binding_generation,
            plan,
            binding,
            source,
            description_sql,
            explanation,
        })
    }
    pub(crate) fn base_dependencies(&self, sql: &str) -> Result<Vec<String>> {
        let mut pending = self.view_dependencies(sql)?;
        let mut bases = BTreeSet::new();
        while let Some(n) = pending.pop() {
            match self.catalog.relation(&n).map(|r| &r.kind) {
                Some(RelationKind::View { dependencies, .. }) => {
                    pending.extend(dependencies.clone())
                }
                Some(RelationKind::Base { .. }) => {
                    bases.insert(n);
                }
                None => return reject("unregistered read dependency"),
            }
        }
        Ok(bases.into_iter().collect())
    }
}
fn verify_write_mapping(
    relation: &Relation,
    binding: &WriteBinding,
    info: &TargetInspection,
) -> Result<()> {
    let columns = &binding.columns;
    if !matches!(relation.kind, RelationKind::Base { .. })
        || columns.len() != relation.schema.fields().len()
        || columns.values().collect::<BTreeSet<_>>().len() != columns.len()
    {
        return reject("write binding requires a reversible base-table column mapping");
    }
    for (logical, physical) in columns {
        let logical = relation
            .schema
            .field_with_name(logical)
            .map_err(|_| failure("unknown logical write column"))?;
        let physical = info
            .schema
            .field_with_name(physical)
            .map_err(|_| failure("unknown physical write column"))?;
        if logical.data_type() != physical.data_type() {
            return reject("write mapping must preserve physical column types");
        }
    }
    if !info.unique_keys.is_empty()
        && !info
            .unique_keys
            .iter()
            .any(|key| key.iter().all(|k| columns.values().any(|c| c == k)))
    {
        return reject("writable projections must expose a complete enforced key");
    }
    Ok(())
}
fn unique<'a>(names: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut set = BTreeSet::new();
    for n in names {
        if !set.insert(n) {
            return reject("duplicate mutation column");
        }
    }
    Ok(())
}
fn validate_source(plan: &LogicalPlan) -> Result<()> {
    plan.apply_with_subqueries(|p| {
        match p {
            LogicalPlan::Projection(_)
            | LogicalPlan::Filter(_)
            | LogicalPlan::TableScan(_)
            | LogicalPlan::SubqueryAlias(_)
            | LogicalPlan::EmptyRelation(_) => (),
            LogicalPlan::Join(j)
                if matches!(
                    j.join_type,
                    datafusion::logical_expr::JoinType::Inner
                        | datafusion::logical_expr::JoinType::Left
                ) && !j.on.is_empty()
                    && j.filter.is_none() => {}
            _ => return Err(failure("unsupported checked source operator")),
        }
        p.apply_expressions(|e| {
            e.apply(|e| {
                if !matches!(
                    e,
                    DfExpr::Column(_)
                        | DfExpr::Alias(_)
                        | DfExpr::Literal(_, _)
                        | DfExpr::Placeholder(_)
                        | DfExpr::BinaryExpr(_)
                        | DfExpr::Cast(_)
                        | DfExpr::Not(_)
                        | DfExpr::Negative(_)
                        | DfExpr::IsNull(_)
                        | DfExpr::IsNotNull(_)
                ) {
                    return Err(failure("unverified checked source expression"));
                }
                Ok(TreeNodeRecursion::Continue)
            })
        })?;
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok(())
}
impl PreparedWrite<'_> {
    pub async fn explain(&self) -> Result<WriteExplanation> {
        Ok(self.explanation.clone())
    }
    pub async fn execute(
        &self,
        parameters: Vec<ScalarValue>,
        options: WriteOptions,
    ) -> Result<WriteResult> {
        self.execute_on(parameters, options, None).await
    }
    pub(crate) async fn execute_on(
        &self,
        parameters: Vec<ScalarValue>,
        options: WriteOptions,
        session: Option<Arc<dyn ConnectorSession>>,
    ) -> Result<WriteResult> {
        if self.explain_only {
            return reject("EXPLAIN is non-executing; call explain()");
        }
        if self.generation != self.engine.binding_generation {
            return reject("stale prepared write; prepare again");
        }
        if options.atomicity == Atomicity::Required && !self.explanation.atomic_supported {
            return reject("connector cannot establish required atomicity");
        }
        if options.atomicity == Atomicity::BestEffort && (self.plan.checked || session.is_some()) {
            return reject("checked writes and transactions require atomicity");
        }
        let hints = parameters
            .iter()
            .map(|v| Some(v.data_type()))
            .collect::<Vec<_>>();
        let description = self
            .engine
            .describe_read(&self.description_sql, &hints)
            .await?;
        if description.parameters.len() != parameters.len() {
            return reject("parameter count does not match SQL");
        }
        let mut query = options.query;
        query.bypass_materialization = true;
        let context = QueryContext::new(query.clone())?;
        let input = if let Some(sql) = &self.source {
            let execution = async {
                if self.plan.checked {
                    // Use local scalar evaluation for the checked source; federation
                    // optimizers must not replace audited expressions with opaque SQL.
                    self.engine
                        .bound_engine(BTreeMap::new())
                        .await?
                        .execute_parameters_context(sql, parameters.clone(), context.clone())
                        .await
                } else {
                    self.engine
                        .execute_parameters_context(sql, parameters.clone(), context.clone())
                        .await
                }
            }
            .await
            .map_err(|_: EngineError| {
                failure("write input planning failed; no destination changes")
            })?;
            StagedInput::collect(execution.stream, &options.staging, &context)
                .await
                .map_err(|e| {
                    let message = e.to_string();
                    failure(if message.contains("budget") {
                        "write input budget exhausted; no destination changes"
                    } else if message.contains("cancel") {
                        "write input cancelled; no destination changes"
                    } else {
                        "write input failed; no destination changes"
                    })
                })?
        } else {
            StagedInput::empty()
        };
        context.check()?;
        let external_observations = self
            .explanation
            .source_observations
            .iter()
            .filter(|name| {
                session.as_ref().is_none_or(|s| {
                    self.engine
                        .read_bindings
                        .get(*name)
                        .is_none_or(|b| b.connection.domain() != s.domain())
                })
            })
            .cloned()
            .collect();
        let mut result = if let Some(s) = session {
            s.apply(&self.plan, &input, &parameters, context).await?
        } else {
            self.binding
                .connection
                .apply(&self.plan, &input, &parameters, context)
                .await?
        };
        result.external_observations = external_observations;
        Ok(result)
    }
}
