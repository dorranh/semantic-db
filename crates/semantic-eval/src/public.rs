//! Public command and HTTP/PostgreSQL checks use normal product entrypoints.
use crate::{Case, Column, Dataset, Expected, Result, RunOptions, TypedResult, compare};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, time::Duration};
use tokio::process::Command;
/// One recorded comparison through a normal product interface.
#[derive(Debug, Serialize, Deserialize)]
pub struct PublicCheck {
    pub interface: String,
    pub case_id: String,
    pub passed: bool,
    pub diagnostic: Option<String>,
    pub evidence: Vec<PathBuf>,
}
fn eligible(e: &Expected) -> bool {
    matches!(e,Expected::Result{columns,..} if columns.iter().all(|c|matches!(c.kind.as_str(),"int16"|"int32"|"int64"|"utf8"|"boolean"|"float32"|"float64"|"date32"|"time64(us)"|"timestamp(us)"|"timestamp(us,UTC)")))
}
fn command(binary: &PathBuf, env: &BTreeMap<String, String>) -> Command {
    let mut command = Command::new(binary);
    command.envs(env).kill_on_drop(true);
    command
}
fn evidence_path(o: &RunOptions, c: &Case, interface: &str, suffix: &str) -> PathBuf {
    o.artifacts
        .join(format!("public-{}-{interface}.{suffix}", c.id))
}
fn write_sanitized(path: &PathBuf, data: &[u8], env: &BTreeMap<String, String>) -> Result<()> {
    let mut text = String::from_utf8_lossy(data).into_owned();
    for (name, value) in env {
        if !value.is_empty()
            && (name.contains("KEY")
                || name.contains("TOKEN")
                || name.contains("PASSWORD")
                || name.contains("DATABASE_URL"))
        {
            text = text.replace(value, "[redacted]")
        }
    }
    std::fs::write(path, text)?;
    Ok(())
}
async fn cli(
    binary: &PathBuf,
    env: &BTreeMap<String, String>,
    d: &Dataset,
    c: &Case,
    o: &RunOptions,
    ask: bool,
) -> Result<()> {
    let interface = if ask { "cli_typed_ask" } else { "cli_sql" };
    let mut command = command(binary, env);
    command
        .arg(if ask { "ask" } else { "sql" })
        .arg("--project-config")
        .arg(d.resolve(&d.manifest.project)?)
        .arg("--output-json")
        .arg("--query-timeout-seconds")
        .arg(o.timeout_seconds.to_string());
    if ask {
        let context = c.context.as_ref().unwrap_or(&d.manifest.context);
        command.args([
            "--compiler-mode",
            "typed-auto",
            "--reference-time",
            &context.reference_time,
            "--timezone",
            &context.timezone,
        ]);
        command.arg(&c.question);
    } else {
        command.arg(c.sql.as_deref().ok_or("missing SQL")?);
    }
    let output = tokio::time::timeout(
        Duration::from_secs(o.timeout_seconds + 10),
        command.output(),
    )
    .await??;
    write_sanitized(
        &evidence_path(o, c, interface, "stdout.log"),
        &output.stdout,
        env,
    )?;
    write_sanitized(
        &evidence_path(o, c, interface, "stderr.log"),
        &output.stderr,
        env,
    )?;
    if !output.status.success() {
        return Err(format!(
            "public CLI failed with {} (see retained logs)",
            output.status
        )
        .into());
    }
    if ask {
        let value: Value = serde_json::from_slice(&output.stdout)?;
        let context = c.context.as_ref().unwrap_or(&d.manifest.context);
        let expected =
            chrono::DateTime::parse_from_rfc3339(&context.reference_time)?.timestamp_millis();
        let observed = &value["compilation"]["outcome"]["query"]["request_context"];
        if observed["reference_unix_millis"].as_i64() != Some(expected)
            || observed["timezone"].as_str() != Some(context.timezone.as_str())
        {
            return Err("CLI host request context was not retained".into());
        }
    }
    let actual: TypedResult = serde_json::from_slice(&output.stdout)?;
    let differences = compare(&d.expectations[&c.id], &actual, &c.comparison)?;
    if !differences.is_empty() {
        return Err(format!("public CLI mismatch: {differences:?}").into());
    }
    Ok(())
}
fn free_port() -> Result<u16> {
    Ok(std::net::TcpListener::bind(("127.0.0.1", 0))?
        .local_addr()?
        .port())
}
fn parameter(p: &Value) -> Result<Box<dyn tokio_postgres::types::ToSql + Send + Sync>> {
    let value = &p["value"];
    Ok(match p["type"].as_str() {
        Some("int16") => Box::new(i16::try_from(
            value.as_i64().ok_or("invalid int16 parameter")?,
        )?),
        Some("int32") => Box::new(i32::try_from(
            value.as_i64().ok_or("invalid int32 parameter")?,
        )?),
        Some("int64") => Box::new(value.as_i64().ok_or("invalid int64 parameter")?),
        Some("utf8") => Box::new(value.as_str().ok_or("invalid utf8 parameter")?.to_owned()),
        Some("boolean") => Box::new(value.as_bool().ok_or("invalid Boolean parameter")?),
        Some("float64") => {
            let number = value.as_f64().ok_or("invalid float parameter")?;
            if !number.is_finite() {
                return Err("nonfinite float parameter".into());
            }
            Box::new(number)
        }
        Some("date32") => Box::new(
            chrono::NaiveDate::from_ymd_opt(1970, 1, 1)
                .ok_or("invalid epoch")?
                .checked_add_signed(chrono::Duration::days(
                    value.as_i64().ok_or("invalid date parameter")?,
                ))
                .ok_or("date parameter out of range")?,
        ),
        Some("timestamp") => {
            let ticks = value["ticks"].as_i64().ok_or("invalid timestamp ticks")?;
            let factor = match value["unit"].as_str() {
                Some("second") => 1,
                Some("millisecond") => 1000,
                Some("microsecond") => 1_000_000,
                Some("nanosecond") => 1_000_000_000,
                _ => return Err("invalid timestamp parameter unit".into()),
            };
            let instant = chrono::DateTime::from_timestamp(
                ticks.div_euclid(factor),
                (ticks.rem_euclid(factor) * (1_000_000_000 / factor)) as u32,
            )
            .ok_or("timestamp parameter out of range")?;
            if value["timezone"].is_null() {
                Box::new(instant.naive_utc())
            } else {
                Box::new(instant)
            }
        }
        _ => return Err("public smoke parameter type requires adapter support".into()),
    })
}
fn pg_result(
    statement: &tokio_postgres::Statement,
    rows: Vec<tokio_postgres::Row>,
) -> Result<TypedResult> {
    use tokio_postgres::types::Type;
    let mut columns = vec![];
    for field in statement.columns() {
        let kind = match *field.type_() {
            Type::INT2 => "int16",
            Type::INT4 => "int32",
            Type::INT8 => "int64",
            Type::TEXT | Type::VARCHAR => "utf8",
            Type::BOOL => "boolean",
            Type::FLOAT4 => "float32",
            Type::FLOAT8 => "float64",
            Type::DATE => "date32",
            Type::TIME => "time64(us)",
            Type::TIMESTAMP => "timestamp(us)",
            Type::TIMESTAMPTZ => "timestamp(us,UTC)",
            _ => return Err("unsupported public smoke output type".into()),
        };
        columns.push(Column {
            name: field.name().into(),
            kind: kind.into(),
            nullable: true,
            physical_nullable: None,
            tolerance: None,
        });
    }
    let mut typed = vec![];
    for row in rows {
        let mut values = vec![];
        for (i, column) in columns.iter().enumerate() {
            macro_rules! get {
                ($type:ty,$convert:expr) => {
                    row.try_get::<_, Option<$type>>(i)?
                        .map($convert)
                        .unwrap_or(Value::Null)
                };
            }
            let value = match column.kind.as_str() {
                "int16" => get!(i16, |v: i16| json!(v.to_string())),
                "int32" => get!(i32, |v: i32| json!(v.to_string())),
                "int64" => get!(i64, |v: i64| json!(v.to_string())),
                "utf8" => get!(String, |v: String| json!(v)),
                "boolean" => get!(bool, |v: bool| json!(v)),
                "float32" => {
                    let value: Option<f32> = row.try_get(i)?;
                    if value.is_some_and(|v| !v.is_finite()) {
                        return Err("nonfinite PG float".into());
                    }
                    value.map(|v| json!(v)).unwrap_or(Value::Null)
                }
                "float64" => {
                    let value: Option<f64> = row.try_get(i)?;
                    if value.is_some_and(|v| !v.is_finite()) {
                        return Err("nonfinite PG float".into());
                    }
                    value.map(|v| json!(v)).unwrap_or(Value::Null)
                }
                "date32" => get!(chrono::NaiveDate, |v: chrono::NaiveDate| json!(
                    v.to_string()
                )),
                "time64(us)" => get!(chrono::NaiveTime, |v: chrono::NaiveTime| json!(
                    v.format("%H:%M:%S%.f").to_string()
                )),
                "timestamp(us)" => get!(chrono::NaiveDateTime, |v: chrono::NaiveDateTime| json!(
                    v.format("%Y-%m-%dT%H:%M:%S%.f").to_string()
                )),
                "timestamp(us,UTC)" => {
                    get!(chrono::DateTime<chrono::Utc>, |v: chrono::DateTime<
                        chrono::Utc,
                    >| json!(
                        v.to_rfc3339()
                    ))
                }
                _ => return Err("unsupported output type".into()),
            };
            values.push(value);
        }
        typed.push(values);
    }
    Ok(TypedResult {
        columns,
        rows: typed,
    })
}
async fn http_pg(
    binary: &PathBuf,
    env: &BTreeMap<String, String>,
    d: &Dataset,
    c: &Case,
    o: &RunOptions,
) -> Result<()> {
    let port = free_port()?;
    let http = free_port()?;
    let mut child = command(binary, env)
        .arg("server")
        .arg("--project-config")
        .arg(d.resolve(&d.manifest.project)?)
        .arg("--port")
        .arg(port.to_string())
        .arg("--http-port")
        .arg(http.to_string())
        .stdout(std::fs::File::create(evidence_path(
            o,
            c,
            "http_typed_pg",
            "server.stdout.log",
        ))?)
        .stderr(std::fs::File::create(evidence_path(
            o,
            c,
            "http_typed_pg",
            "server.stderr.log",
        ))?)
        .spawn()?;
    let result=async {
  let client=reqwest::Client::builder().timeout(Duration::from_secs(o.timeout_seconds)).build()?;let base=format!("http://127.0.0.1:{http}");
  tokio::time::timeout(Duration::from_secs(o.timeout_seconds),async {
   loop {
    if client.get(format!("{base}/health")).send().await.is_ok(){return Ok::<(),crate::Error>(())}
    if child.try_wait()?.is_some(){return Err("public server exited before readiness".into())}
    tokio::time::sleep(Duration::from_millis(100)).await;
   }
  }).await??;
  let context=c.context.as_ref().unwrap_or(&d.manifest.context);let reference=chrono::DateTime::parse_from_rfc3339(&context.reference_time)?.timestamp_millis();
  let response:Value=client.post(format!("{base}/v1/compile/semantic")).json(&json!({"question":c.question,"context":"auto","request_context":{"reference_unix_millis":reference,"timezone":context.timezone,"calendar":"gregorian","origin":{"kind":"caller"}}})).send().await?.error_for_status()?.json().await?;
  std::fs::write(evidence_path(o,c,"http_typed_pg","compilation.json"),serde_json::to_vec_pretty(&response)?)?;
  let outcome=&response["outcome"];if !matches!(outcome["status"].as_str(),Some("compiled"|"compiled_graph")){return Err("HTTP typed compilation did not produce an artifact".into())}
  let query=&outcome["query"];if query["request_context"]["reference_unix_millis"].as_i64()!=Some(reference) || query["request_context"]["timezone"].as_str()!=Some(context.timezone.as_str()){return Err("HTTP host request context was not retained".into())}
  let sql=query["sql"]["statement"].as_str().ok_or("HTTP artifact lacks SQL")?;let parameters=query["sql"]["parameters"].as_array().ok_or("HTTP artifact lacks parameters")?;
  let (pg,connection)=tokio_postgres::connect(&format!("host=127.0.0.1 port={port} user=acceptance dbname=acceptance"),tokio_postgres::NoTls).await?;let task=tokio::spawn(connection);
  let execution=async {
   let owned=parameters.iter().map(parameter).collect::<Result<Vec<_>>>()?;
   let statement=pg.prepare(sql).await?;
   let values:Vec<&(dyn tokio_postgres::types::ToSql+Sync)>=owned.iter().map(|value|value.as_ref() as &(dyn tokio_postgres::types::ToSql+Sync)).collect();
   let actual=pg_result(&statement,pg.query(&statement,&values).await?)?;
   std::fs::write(evidence_path(o,c,"http_typed_pg","actual.json"),serde_json::to_vec_pretty(&actual)?)?;
   let differences=compare(&d.expectations[&c.id],&actual,&c.comparison)?;
   if !differences.is_empty(){return Err(format!("HTTP compilation/server execution mismatch: {differences:?}").into())}Ok::<(),crate::Error>(())
  }.await;
  drop(pg);task.abort();execution
 }.await;
    let _ = child.kill().await;
    let _ = child.wait().await;
    for suffix in ["server.stdout.log", "server.stderr.log"] {
        let path = evidence_path(o, c, "http_typed_pg", suffix);
        if let Ok(data) = std::fs::read(&path) {
            let _ = write_sanitized(&path, &data, env);
        }
    }
    result
}
pub(crate) async fn checks(
    d: &Dataset,
    o: &RunOptions,
    env: &BTreeMap<String, String>,
) -> Vec<PublicCheck> {
    let selected: Vec<&Case> = if d.manifest.public_cases.is_empty() {
        d.cases
            .iter()
            .filter(|c| c.sql.is_some() && eligible(&d.expectations[&c.id]))
            .take(1)
            .collect()
    } else {
        d.manifest
            .public_cases
            .iter()
            .filter_map(|id| d.cases.iter().find(|c| &c.id == id))
            .collect()
    };
    if selected.is_empty() {
        return vec![PublicCheck {
            interface: "public".into(),
            case_id: String::new(),
            passed: false,
            diagnostic: Some("no representative public result case available".into()),
            evidence: vec![],
        }];
    }
    let binary = o.cli_binary.clone().unwrap_or_else(|| "sdb".into());
    let mut reports = vec![];
    for case in selected {
        for interface in ["cli_sql", "cli_typed_ask", "http_typed_pg"] {
            let attempt = async {
                if case
                    .context
                    .as_ref()
                    .unwrap_or(&d.manifest.context)
                    .allowed_relations
                    .is_some()
                {
                    return Err(
                        "public scope forwarding is not supported by these interfaces".into(),
                    );
                }
                match interface {
                    "cli_sql" => cli(&binary, env, d, case, o, false).await,
                    "cli_typed_ask" => cli(&binary, env, d, case, o, true).await,
                    _ => http_pg(&binary, env, d, case, o).await,
                }
            };
            let result = match tokio::time::timeout(
                Duration::from_secs(o.timeout_seconds * 3 + 15),
                attempt,
            )
            .await
            {
                Ok(result) => result,
                Err(_) => Err("public interface check timed out".into()),
            };
            let evidence = std::fs::read_dir(&o.artifacts)
                .ok()
                .into_iter()
                .flatten()
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter(|path| {
                    path.file_name().is_some_and(|name| {
                        name.to_string_lossy()
                            .starts_with(&format!("public-{}-{interface}.", case.id))
                    })
                })
                .collect();
            reports.push(PublicCheck {
                interface: interface.into(),
                case_id: case.id.clone(),
                passed: result.is_ok(),
                diagnostic: result.err().map(|e| e.to_string()),
                evidence,
            });
        }
    }
    reports
}
