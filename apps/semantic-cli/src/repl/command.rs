use super::*;

pub(super) async fn run(
    engine: &mut Engine,
    compiler: &mut Option<Compiler<OpenAiProvider>>,
    line: &str,
    color: bool,
) -> Result<()> {
    let (command, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
    let rest = rest.trim();
    match command {
        ".help" => println!(
            ".tables                 List registered relations\n\
             .schema NAME            Show schema and definition\n\
             .view NAME=SELECT ...   Register an in-memory view (one line)\n\
             .ask REQUEST           Compile and execute natural language\n\
             .plan REQUEST          Compile and show SQL without execution\n\
             .ask-views REQUEST     Select a view and execute a bounded query\n\
             .plan-views REQUEST    Select a view and show SQL without execution\n\
             .quit                   Exit\n\
             End SQL with ; (trailing comments are allowed). Edit earlier lines with arrows.\n\
             Tab completes; Tab again lists alternatives. Ctrl-R searches history.\n\
             Ctrl-C clears input. Ctrl-D exits when the buffer is empty.\n\
             History stores SQL and natural-language requests locally across projects.\n\
             --no-history keeps history in memory; --history-file PATH overrides storage.\n\
             --no-color or NO_COLOR disables styling."
        ),
        ".tables" => {
            for relation in engine.catalog().relations() {
                println!("{}", relation.name);
            }
        }
        ".schema" => {
            let relation = engine
                .catalog()
                .relation(rest)
                .ok_or_else(|| format!("unknown relation: {rest}"))?;
            println!("{}\n{:?}", relation.name, relation.kind);
            for field in relation.schema.fields() {
                println!(
                    "  {}: {}{}",
                    field.name(),
                    field.data_type(),
                    if field.is_nullable() {
                        " (nullable)"
                    } else {
                        ""
                    }
                );
            }
        }
        ".view" => {
            let (name, sql) = parse_assignment(rest)?;
            engine.create_view(&name, &sql).await?;
            println!("{} {name}", paint("Registered view", "32", color));
        }
        ".ask" | ".plan" | ".ask-views" | ".plan-views" => {
            if rest.is_empty() {
                return Err("provide a natural-language request".into());
            }
            if compiler.is_none() {
                *compiler = Some(Compiler::new(config::provider()?));
            }
            run_ask(
                engine,
                compiler.as_ref().expect("compiler initialized"),
                rest,
                matches!(command, ".plan" | ".plan-views"),
                matches!(command, ".ask-views" | ".plan-views"),
            )
            .await?;
        }
        _ => return Err(format!("unknown command: {command}; use .help").into()),
    }
    Ok(())
}
