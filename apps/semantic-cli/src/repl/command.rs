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
             .cache-status           List published cache generations\n\
             .cache-refresh NAME     Refresh a materialized relation\n\
             .cache-invalidate KEY   Invalidate a cache generation key\n\
             .cache-bypass on|off    Toggle materialization use\n\
             .quit                   Exit\n\
             End SQL with ; (trailing comments are allowed). Edit earlier lines with arrows.\n\
             Tab completes; Tab again lists alternatives. Ctrl-R searches history.\n\
             Ctrl-C clears input. Ctrl-D exits when the buffer is empty.\n\
             History stores SQL and natural-language requests locally across projects.\n\
             --no-history keeps history in memory; --history-file PATH overrides storage.\n\
             --no-color or NO_COLOR disables styling."
        ),
        ".cache-status" => {
            let manager = engine
                .materialization_manager()
                .ok_or("project has no cache configuration")?;
            for manifest in manager.status()? {
                println!(
                    "{} generation={} rows={} disk_bytes={}",
                    manifest.key, manifest.generation, manifest.rows, manifest.disk_bytes
                );
            }
        }
        ".cache-refresh" => {
            engine.refresh_materialization(rest).await?;
        }
        ".cache-invalidate" => {
            let manager = engine
                .materialization_manager()
                .ok_or("project has no cache configuration")?;
            manager
                .invalidate(
                    rest,
                    &semantic_engine::QueryContext::new(engine.query_options().clone())?,
                )
                .await?;
        }
        ".cache-bypass" => {
            let mut options = engine.query_options().clone();
            options.bypass_materialization = match rest {
                "on" => true,
                "off" => false,
                _ => return Err("use .cache-bypass on|off".into()),
            };
            engine.set_query_options(options)?;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cache_commands_use_the_active_repl_engine() {
        let mut engine = Engine::new();
        engine
            .set_query_options(semantic_engine::QueryOptions {
                timeout_seconds: 60,
                ..Default::default()
            })
            .unwrap();
        let mut compiler = None;
        run(&mut engine, &mut compiler, ".cache-bypass on", false)
            .await
            .unwrap();
        assert!(engine.query_options().bypass_materialization);
        assert_eq!(engine.query_options().timeout_seconds, 60);
        assert!(
            run(&mut engine, &mut compiler, ".cache-bypass invalid", false)
                .await
                .is_err()
        );
        assert!(engine.query_options().bypass_materialization);
        run(&mut engine, &mut compiler, ".cache-bypass off", false)
            .await
            .unwrap();
        assert!(!engine.query_options().bypass_materialization);
        let error = run(&mut engine, &mut compiler, ".cache-status", false)
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "project has no cache configuration");
    }
}
