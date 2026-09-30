use super::*;
use datafusion::{
    functions_aggregate::{count, min_max},
    functions_window::row_number,
    logical_expr::{
        ExprSchemable, JoinType, SortExpr, WindowFrame,
        expr::{WindowFunction, WindowFunctionDefinition},
    },
};

const KEY: &str = "__semantic_allocation_key";
const SOURCE_COUNT: &str = "__semantic_allocation_source_count";
const AMOUNT: &str = "__semantic_allocation_source_amount";
const EXPECTED_COUNT: &str = "__semantic_allocation_expected_count";
const EXPECTED_WEIGHT: &str = "__semantic_allocation_expected_weight";
const TARGET: &str = "__semantic_allocation_target";
const WEIGHT: &str = "__semantic_allocation_weight";
const ACTUAL_COUNT: &str = "__semantic_allocation_actual_count";
const ACTUAL_WEIGHT: &str = "__semantic_allocation_actual_weight";
const TARGET_COUNT: &str = "__semantic_allocation_target_count";
const FLOOR: &str = "__semantic_allocation_floor";
const REMAINDER: &str = "__semantic_allocation_remainder";
const MAX_TARGET_COUNT: &str = "__semantic_allocation_max_target_count";
const FLOOR_TOTAL: &str = "__semantic_allocation_floor_total";
const RANK: &str = "__semantic_allocation_rank";
const SHARE: &str = "__semantic_allocation_share";

fn quoted(name: &str) -> String {
    ident(name).to_string()
}

pub(super) fn emit_allocation(
    query: &mut ast::Query,
    allocation: &BoundAllocation,
    parameters: &mut Vec<Literal>,
) {
    let ast::SetExpr::Select(source) = query.body.as_mut() else {
        unreachable!()
    };
    source.projection = vec![ast::SelectItem::Wildcard(Default::default())];
    let source_sql = query.to_string();
    let predicate = allocation
        .predicate
        .as_ref()
        .map(|predicate| {
            format!(
                " AND ({})",
                sql_predicate_aliased(predicate, parameters, "rhs")
            )
        })
        .unwrap_or_default();
    let field = |alias: &str, name: &str| format!("{}.{}", quoted(alias), quoted(name));
    let key_aliases = (0..allocation.source_key.len())
        .map(|index| quoted(&format!("{KEY}_{index}")))
        .collect::<Vec<_>>();
    let source_partition = allocation
        .source_key
        .iter()
        .map(|key| field("source_inner", key.field.name()))
        .collect::<Vec<_>>()
        .join(", ");
    let key_select = allocation
        .source_key
        .iter()
        .zip(&key_aliases)
        .map(|(key, alias)| format!("{} AS {alias}", field("src", key.field.name())))
        .collect::<Vec<_>>()
        .join(", ");
    let key_join = allocation
        .source_key
        .iter()
        .zip(&allocation.bridge_key)
        .map(|(source, bridge)| {
            format!(
                "{} = {}",
                field("src", source.field.name()),
                field("rhs", bridge.field.name())
            )
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    let key_partition = key_aliases.join(", ");
    let target_aliases = (0..allocation.targets.len())
        .map(|index| quoted(&format!("{TARGET}_{index}")))
        .collect::<Vec<_>>();
    let targets = target_aliases.join(", ");
    let target_partition = format!("{key_partition}, {targets}");
    let target_select = allocation
        .targets
        .iter()
        .zip(&target_aliases)
        .map(|(target, alias)| format!("{} AS {alias}", field("rhs", target.field.name())))
        .collect::<Vec<_>>()
        .join(", ");
    let target_order = target_aliases
        .iter()
        .map(|alias| format!("{alias} ASC"))
        .collect::<Vec<_>>()
        .join(", ");
    let target_outputs = target_aliases
        .iter()
        .zip(&allocation.target_outputs)
        .map(|(alias, output)| format!("{alias} AS {}", quoted(output.field.name())))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "WITH source_checked AS (\
           SELECT *, count(1) OVER (PARTITION BY {source_partition}) AS {source_count_alias} \
           FROM ({source_sql}) AS {source_inner}\
         ), joined AS (\
           SELECT {key_select}, {source_count} AS {source_count_alias}, {amount} AS {amount_alias}, \
                  {expected_count} AS {expected_count_alias}, {expected_weight} AS {expected_weight_alias}, \
                  {target_select}, {weight} AS {weight_alias} \
           FROM source_checked AS {src} \
           LEFT JOIN {bridge} AS {rhs} ON {key_join}{predicate}\
         ), measured AS (\
           SELECT *, \
             count({first_target}) OVER (PARTITION BY {key_partition}) AS {actual_count_alias}, \
             semantic_sum_v1({weight_alias}) OVER (PARTITION BY {key_partition}) AS {actual_weight_alias}, \
             count(1) OVER (PARTITION BY {target_partition}) AS {target_count_alias}, \
             semantic_allocation_floor_v1({amount_alias},{weight_alias},{expected_weight_alias}) AS {floor_alias}, \
             semantic_allocation_remainder_v1({amount_alias},{weight_alias},{expected_weight_alias}) AS {remainder_alias} \
           FROM joined\
         ), ranked AS (\
           SELECT *, \
             max({target_count_alias}) OVER (PARTITION BY {key_partition}) AS {max_target_count_alias}, \
             semantic_sum_v1({floor_alias}) OVER (PARTITION BY {key_partition}) AS {floor_total_alias}, \
             CAST(row_number() OVER (PARTITION BY {key_partition} ORDER BY {remainder_alias} DESC, {target_order}) AS BIGINT) AS {rank_alias} \
           FROM measured\
         ), shares AS (\
           SELECT {targets}, \
             semantic_allocation_share_v1({amount_alias},{weight_alias},{expected_weight_alias},{rank_alias}, \
               {amount_alias}-{floor_total_alias},{actual_count_alias}) AS {share_alias} \
           FROM ranked \
           WHERE semantic_assert_single_v1({source_count_alias}) AND \
             semantic_assert_allocation_v1({actual_count_alias},{expected_count_alias}, \
             {actual_weight_alias},{expected_weight_alias},{max_target_count_alias})\
         ) \
         SELECT {target_outputs}, semantic_sum_v1({share_alias}) AS {amount_output} \
         FROM shares GROUP BY {targets}",
        src = quoted("src"),
        source_inner = quoted("source_inner"),
        rhs = quoted("rhs"),
        bridge = quoted(&allocation.bridge.id),
        amount = field("src", allocation.source_amount.field.name()),
        expected_count = field("src", allocation.expected_count.field.name()),
        expected_weight = field("src", allocation.expected_weight.field.name()),
        weight = field("rhs", allocation.weight.field.name()),
        source_partition = source_partition,
        key_select = key_select,
        key_join = key_join,
        key_partition = key_partition,
        target_partition = target_partition,
        target_select = target_select,
        target_order = target_order,
        targets = targets,
        target_outputs = target_outputs,
        first_target = target_aliases[0],
        source_count = field("src", SOURCE_COUNT),
        source_count_alias = quoted(SOURCE_COUNT),
        amount_alias = quoted(AMOUNT),
        expected_count_alias = quoted(EXPECTED_COUNT),
        expected_weight_alias = quoted(EXPECTED_WEIGHT),
        weight_alias = quoted(WEIGHT),
        actual_count_alias = quoted(ACTUAL_COUNT),
        actual_weight_alias = quoted(ACTUAL_WEIGHT),
        target_count_alias = quoted(TARGET_COUNT),
        floor_alias = quoted(FLOOR),
        remainder_alias = quoted(REMAINDER),
        max_target_count_alias = quoted(MAX_TARGET_COUNT),
        floor_total_alias = quoted(FLOOR_TOTAL),
        rank_alias = quoted(RANK),
        share_alias = quoted(SHARE),
        amount_output = quoted(allocation.amount_output.field.name()),
    );
    let mut parsed = Parser::parse_sql(&GenericDialect {}, &sql).expect("bound allocation SQL");
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
fn window(
    function: WindowFunctionDefinition,
    args: Vec<Expr>,
    partition: Vec<Expr>,
    order: Vec<SortExpr>,
) -> Expr {
    let mut expression = WindowFunction::new(function, args);
    expression.params.partition_by = partition;
    expression.params.order_by = order;
    expression.params.window_frame = WindowFrame::new(None);
    Expr::WindowFunction(Box::new(expression))
}

pub(super) async fn plan_allocation(
    input: DataFrame,
    engine: &Engine,
    allocation: &BoundAllocation,
) -> datafusion::error::Result<DataFrame> {
    let key_aliases = (0..allocation.source_key.len())
        .map(|index| format!("{KEY}_{index}"))
        .collect::<Vec<_>>();
    let target_aliases = (0..allocation.targets.len())
        .map(|index| format!("{TARGET}_{index}"))
        .collect::<Vec<_>>();
    let source = input.alias("src")?;
    let source = source
        .with_column(
            SOURCE_COUNT,
            window(
                WindowFunctionDefinition::AggregateUDF(count::count_udaf()),
                vec![lit(1i64)],
                allocation
                    .source_key
                    .iter()
                    .map(|key| qualified("src", key.field.name()))
                    .collect(),
                vec![],
            ),
        )?
        .alias("src")?;
    let mut bridge = engine
        .plan_generated_sql(&format!("SELECT * FROM {}", quoted(&allocation.bridge.id)))
        .await
        .map_err(|_| {
            datafusion::error::DataFusionError::Plan("allocation bridge planning failed".into())
        })?;
    bridge = bridge.alias("rhs")?;
    if let Some(predicate) = &allocation.predicate {
        bridge = bridge.filter(df_predicate_aliased(predicate, "rhs"))?;
    }
    let joined = source.join_on(
        bridge,
        JoinType::Left,
        allocation
            .source_key
            .iter()
            .zip(&allocation.bridge_key)
            .map(|(source, bridge)| {
                qualified("src", source.field.name()).eq(qualified("rhs", bridge.field.name()))
            })
            .collect::<Vec<_>>(),
    )?;
    let mut selected = allocation
        .source_key
        .iter()
        .zip(&key_aliases)
        .map(|(key, alias)| qualified("src", key.field.name()).alias(alias.as_str()))
        .collect::<Vec<_>>();
    selected.extend([
        qualified("src", SOURCE_COUNT).alias(SOURCE_COUNT),
        qualified("src", allocation.source_amount.field.name()).alias(AMOUNT),
        qualified("src", allocation.expected_count.field.name()).alias(EXPECTED_COUNT),
        qualified("src", allocation.expected_weight.field.name()).alias(EXPECTED_WEIGHT),
        qualified("rhs", allocation.weight.field.name()).alias(WEIGHT),
    ]);
    selected.extend(
        allocation
            .targets
            .iter()
            .zip(&target_aliases)
            .map(|(target, alias)| qualified("rhs", target.field.name()).alias(alias.as_str())),
    );
    let mut rows = joined.select(selected)?;
    let part = || {
        key_aliases
            .iter()
            .map(|name| column(name))
            .collect::<Vec<_>>()
    };
    rows = rows.with_column(
        ACTUAL_COUNT,
        window(
            WindowFunctionDefinition::AggregateUDF(count::count_udaf()),
            vec![column(&target_aliases[0])],
            part(),
            vec![],
        ),
    )?;
    rows = rows.with_column(
        ACTUAL_WEIGHT,
        window(
            WindowFunctionDefinition::AggregateUDF(semantic_engine::semantic_sum_v1()),
            vec![column(WEIGHT)],
            part(),
            vec![],
        ),
    )?;
    rows = rows.with_column(
        TARGET_COUNT,
        window(
            WindowFunctionDefinition::AggregateUDF(count::count_udaf()),
            vec![lit(1i64)],
            part()
                .into_iter()
                .chain(target_aliases.iter().map(|alias| column(alias)))
                .collect(),
            vec![],
        ),
    )?;
    rows = rows.with_column(
        FLOOR,
        semantic_engine::semantic_allocation_floor_v1().call(vec![
            column(AMOUNT),
            column(WEIGHT),
            column(EXPECTED_WEIGHT),
        ]),
    )?;
    rows = rows.with_column(
        REMAINDER,
        semantic_engine::semantic_allocation_remainder_v1().call(vec![
            column(AMOUNT),
            column(WEIGHT),
            column(EXPECTED_WEIGHT),
        ]),
    )?;
    rows = rows.with_column(
        MAX_TARGET_COUNT,
        window(
            WindowFunctionDefinition::AggregateUDF(min_max::max_udaf()),
            vec![column(TARGET_COUNT)],
            part(),
            vec![],
        ),
    )?;
    rows = rows.with_column(
        FLOOR_TOTAL,
        window(
            WindowFunctionDefinition::AggregateUDF(semantic_engine::semantic_sum_v1()),
            vec![column(FLOOR)],
            part(),
            vec![],
        ),
    )?;
    let mut rank_order = vec![column(REMAINDER).sort(false, false)];
    rank_order.extend(
        target_aliases
            .iter()
            .map(|alias| column(alias).sort(true, false)),
    );
    let rank = window(
        WindowFunctionDefinition::WindowUDF(row_number::row_number_udwf()),
        vec![],
        part(),
        rank_order,
    )
    .cast_to(&semantic_catalog::DataType::Int64, rows.schema())?;
    rows = rows.with_column(RANK, rank)?;
    rows =
        rows.filter(semantic_engine::semantic_assert_single_v1().call(vec![column(SOURCE_COUNT)]))?;
    rows = rows.filter(semantic_engine::semantic_assert_allocation_v1().call(vec![
        column(ACTUAL_COUNT),
        column(EXPECTED_COUNT),
        column(ACTUAL_WEIGHT),
        column(EXPECTED_WEIGHT),
        column(MAX_TARGET_COUNT),
    ]))?;
    rows = rows.with_column(
        SHARE,
        semantic_engine::semantic_allocation_share_v1().call(vec![
            column(AMOUNT),
            column(WEIGHT),
            column(EXPECTED_WEIGHT),
            column(RANK),
            column(AMOUNT) - column(FLOOR_TOTAL),
            column(ACTUAL_COUNT),
        ]),
    )?;
    let rows = rows.select(
        target_aliases
            .iter()
            .map(|alias| column(alias))
            .chain([column(SHARE)])
            .collect::<Vec<_>>(),
    )?;
    rows.aggregate(
        target_aliases
            .iter()
            .zip(&allocation.target_outputs)
            .map(|(alias, output)| column(alias).alias(output.field.name()))
            .collect(),
        vec![
            semantic_engine::semantic_sum_v1()
                .call(vec![column(SHARE)])
                .alias(allocation.amount_output.field.name()),
        ],
    )
}
