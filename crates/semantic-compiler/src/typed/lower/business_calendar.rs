use super::*;
use datafusion::{
    functions_aggregate::count,
    functions_window::row_number,
    logical_expr::{
        JoinType, WindowFrame,
        expr::{WindowFunction, WindowFunctionDefinition},
    },
};
use semantic_catalog::CalendarSourceBasis;

const ROW_ID: &str = "__semantic_calendar_row_id";
const VALUE: &str = "__semantic_calendar_value";
const PRESENT: &str = "__semantic_calendar_present";
const MATCH_COUNT: &str = "__semantic_calendar_match_count";

fn quoted(name: &str) -> String {
    ident(name).to_string()
}
fn field(alias: &str, name: &str) -> String {
    format!("{}.{}", quoted(alias), quoted(name))
}

fn sql_source_date(calendar: &BoundBusinessCalendar) -> String {
    let source = field("src", calendar.source_date.field.name());
    match calendar.source_basis {
        CalendarSourceBasis::Date32 => source,
        CalendarSourceBasis::UtcInstantMicros => format!(
            "semantic_local_date_us_v1({source}, '{}')",
            calendar.timezone.replace('\'', "''")
        ),
    }
}

fn df_source_date(calendar: &BoundBusinessCalendar) -> Expr {
    let source = qualified("src", calendar.source_date.field.name());
    match calendar.source_basis {
        CalendarSourceBasis::Date32 => source,
        CalendarSourceBasis::UtcInstantMicros => semantic_engine::semantic_local_date_us_v1()
            .call(vec![source, lit(calendar.timezone.clone())]),
    }
}

pub(super) fn emit(
    query: &mut ast::Query,
    calendar: &BoundBusinessCalendar,
    parameters: &mut Vec<Literal>,
) {
    let ast::SetExpr::Select(source) = query.body.as_mut() else {
        unreachable!()
    };
    source.projection = vec![ast::SelectItem::Wildcard(Default::default())];
    let source_sql = query.to_string();
    let predicate = calendar
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
         ), calendar_joined AS (\
           SELECT {src}.*, {selected} AS {value_alias}, {present} AS {present_alias} \
           FROM source_numbered AS {src} LEFT JOIN {mapping} AS {rhs} ON \
             {source_date} = {calendar_date}{predicate}\
         ), calendar_measured AS (\
           SELECT *, count({present_alias}) OVER (PARTITION BY {row_id}) AS {match_count} \
           FROM calendar_joined\
         ) \
         SELECT *, {value_alias} AS {output} FROM calendar_measured \
         WHERE semantic_assert_exactly_one_v1({match_count})",
        src = quoted("src"),
        rhs = quoted("rhs"),
        source_inner = quoted("source_inner"),
        mapping = quoted(&calendar.calendar_relation.id),
        row_id = quoted(ROW_ID),
        selected = field("rhs", calendar.calendar_value.field.name()),
        present = field("rhs", calendar.calendar_date.field.name()),
        value_alias = quoted(VALUE),
        present_alias = quoted(PRESENT),
        source_date = sql_source_date(calendar),
        calendar_date = field("rhs", calendar.calendar_date.field.name()),
        match_count = quoted(MATCH_COUNT),
        output = quoted(calendar.output.field.name()),
    );
    let mut parsed = Parser::parse_sql(&GenericDialect {}, &sql).expect("bound calendar SQL");
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

pub(super) async fn plan(
    input: DataFrame,
    engine: &Engine,
    calendar: &BoundBusinessCalendar,
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
    let mut mapping = engine
        .plan_generated_sql(&format!(
            "SELECT * FROM {}",
            quoted(&calendar.calendar_relation.id)
        ))
        .await
        .map_err(|_| {
            datafusion::error::DataFusionError::Plan("calendar relation planning failed".into())
        })?
        .alias("rhs")?;
    if let Some(predicate) = &calendar.predicate {
        mapping = mapping.filter(df_predicate_aliased(predicate, "rhs"))?;
    }
    let joined = source.join_on(
        mapping,
        JoinType::Left,
        [df_source_date(calendar).eq(qualified("rhs", calendar.calendar_date.field.name()))],
    )?;
    let mut columns = original
        .iter()
        .map(|name| qualified("src", name))
        .collect::<Vec<_>>();
    columns.extend([
        qualified("src", ROW_ID).alias(ROW_ID),
        qualified("rhs", calendar.calendar_value.field.name()).alias(VALUE),
        qualified("rhs", calendar.calendar_date.field.name()).alias(PRESENT),
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
    let rows = rows.with_column(calendar.output.field.name(), column(VALUE))?;
    let mut output = original.iter().map(|name| column(name)).collect::<Vec<_>>();
    output.push(column(calendar.output.field.name()));
    rows.select(output)
}
