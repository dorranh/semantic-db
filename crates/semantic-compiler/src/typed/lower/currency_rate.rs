use super::*;
use datafusion::{
    functions_aggregate::count,
    functions_window::row_number,
    logical_expr::{
        JoinType, WindowFrame,
        expr::{WindowFunction, WindowFunctionDefinition},
    },
};

const ROW_ID: &str = "__semantic_rate_row_id";
const NUMERATOR: &str = "__semantic_rate_numerator";
const DENOMINATOR: &str = "__semantic_rate_denominator";
const PRESENT: &str = "__semantic_rate_present";
const MATCH_COUNT: &str = "__semantic_rate_match_count";

fn quoted(name: &str) -> String {
    ident(name).to_string()
}
fn field(alias: &str, name: &str) -> String {
    format!("{}.{}", quoted(alias), quoted(name))
}
fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(super) fn emit_currency_rate(
    query: &mut ast::Query,
    rate: &BoundCurrencyRate,
    parameters: &mut Vec<Literal>,
) {
    let ast::SetExpr::Select(source) = query.body.as_mut() else {
        unreachable!()
    };
    source.projection = vec![ast::SelectItem::Wildcard(Default::default())];
    let source_sql = query.to_string();
    let predicate = rate
        .predicate
        .as_ref()
        .map(|predicate| {
            format!(
                " AND ({})",
                sql_predicate_aliased(predicate, parameters, "rhs")
            )
        })
        .unwrap_or_default();
    let sql = format!(
        "WITH source_numbered AS (\
           SELECT *, row_number() OVER () AS {row_id} FROM ({source_sql}) AS {source_inner}\
         ), rate_joined AS (\
           SELECT {src}.*, {numerator} AS {num_alias}, {denominator} AS {den_alias}, \
             {present} AS {present_alias} \
           FROM source_numbered AS {src} LEFT JOIN {rate_relation} AS {rhs} ON \
             {source_currency} = {rate_currency} AND {target_currency} = {target_literal} AND \
             {source_time} >= {valid_from} AND {source_time} < {valid_to}{predicate}\
         ), rate_measured AS (\
           SELECT *, count({present_alias}) OVER (PARTITION BY {row_id}) AS {match_count} \
           FROM rate_joined\
         ) \
         SELECT *, semantic_scale_i64_v1({amount},{num_alias},{den_alias},{half_even}) AS {output} \
         FROM rate_measured WHERE semantic_assert_exactly_one_v1({match_count})",
        src = quoted("src"),
        rhs = quoted("rhs"),
        source_inner = quoted("source_inner"),
        rate_relation = quoted(&rate.rate_relation.id),
        row_id = quoted(ROW_ID),
        numerator = field("rhs", rate.numerator.field.name()),
        denominator = field("rhs", rate.denominator.field.name()),
        present = field("rhs", rate.rate_from_currency.field.name()),
        num_alias = quoted(NUMERATOR),
        den_alias = quoted(DENOMINATOR),
        present_alias = quoted(PRESENT),
        source_currency = field("src", rate.source_currency.field.name()),
        rate_currency = field("rhs", rate.rate_from_currency.field.name()),
        target_currency = field("rhs", rate.rate_to_currency.field.name()),
        target_literal = literal(&rate.to_currency),
        source_time = field("src", rate.source_time.field.name()),
        valid_from = field("rhs", rate.valid_from.field.name()),
        valid_to = field("rhs", rate.valid_to.field.name()),
        match_count = quoted(MATCH_COUNT),
        amount = quoted(rate.source_amount.field.name()),
        half_even = if rate.half_even { "true" } else { "false" },
        output = quoted(rate.output.field.name()),
    );
    let mut parsed = Parser::parse_sql(&GenericDialect {}, &sql).expect("bound currency rate SQL");
    let ast::Statement::Query(replacement) = parsed.remove(0) else {
        unreachable!()
    };
    *query = *replacement;
    wrap_query(query);
}

fn column(name: &str) -> Expr {
    Expr::Column(Column::from_name(name))
}
fn qualified(alias: &str, name: &str) -> Expr {
    Expr::Column(Column::new(Some(alias), name))
}
fn window(function: WindowFunctionDefinition, args: Vec<Expr>, partition: Vec<Expr>) -> Expr {
    let mut expression = WindowFunction::new(function, args);
    expression.params.partition_by = partition;
    expression.params.window_frame = WindowFrame::new(None);
    Expr::WindowFunction(Box::new(expression))
}

pub(super) async fn plan_currency_rate(
    input: DataFrame,
    engine: &Engine,
    rate: &BoundCurrencyRate,
) -> datafusion::error::Result<DataFrame> {
    let original = input
        .schema()
        .fields()
        .iter()
        .map(|field| field.name().clone())
        .collect::<Vec<_>>();
    let source = input
        .alias("src")?
        .with_column(
            ROW_ID,
            window(
                WindowFunctionDefinition::WindowUDF(row_number::row_number_udwf()),
                vec![],
                vec![],
            ),
        )?
        .alias("src")?;
    let mut rates = engine
        .plan_generated_sql(&format!("SELECT * FROM {}", quoted(&rate.rate_relation.id)))
        .await
        .map_err(|_| {
            datafusion::error::DataFusionError::Plan(
                "currency rate relation planning failed".into(),
            )
        })?
        .alias("rhs")?;
    if let Some(predicate) = &rate.predicate {
        rates = rates.filter(df_predicate_aliased(predicate, "rhs"))?;
    }
    let joined = source.join_on(
        rates,
        JoinType::Left,
        [
            qualified("src", rate.source_currency.field.name())
                .eq(qualified("rhs", rate.rate_from_currency.field.name())),
            qualified("rhs", rate.rate_to_currency.field.name()).eq(lit(rate.to_currency.clone())),
            qualified("src", rate.source_time.field.name())
                .gt_eq(qualified("rhs", rate.valid_from.field.name())),
            qualified("src", rate.source_time.field.name())
                .lt(qualified("rhs", rate.valid_to.field.name())),
        ],
    )?;
    let mut columns = original
        .iter()
        .map(|name| qualified("src", name))
        .collect::<Vec<_>>();
    columns.extend([
        qualified("src", ROW_ID).alias(ROW_ID),
        qualified("rhs", rate.numerator.field.name()).alias(NUMERATOR),
        qualified("rhs", rate.denominator.field.name()).alias(DENOMINATOR),
        qualified("rhs", rate.rate_from_currency.field.name()).alias(PRESENT),
    ]);
    let rows = joined.select(columns)?;
    let rows = rows.with_column(
        MATCH_COUNT,
        window(
            WindowFunctionDefinition::AggregateUDF(count::count_udaf()),
            vec![column(PRESENT)],
            vec![column(ROW_ID)],
        ),
    )?;
    let rows = rows.filter(
        semantic_engine::semantic_assert_exactly_one_v1().call(vec![column(MATCH_COUNT)]),
    )?;
    let rows = rows.with_column(
        rate.output.field.name(),
        semantic_engine::semantic_scale_i64_v1().call(vec![
            column(rate.source_amount.field.name()),
            column(NUMERATOR),
            column(DENOMINATOR),
            lit(rate.half_even),
        ]),
    )?;
    let mut output = original.iter().map(|name| column(name)).collect::<Vec<_>>();
    output.push(column(rate.output.field.name()));
    rows.select(output)
}
