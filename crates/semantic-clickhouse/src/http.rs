//! Bounded HTTP transport. Query text and remote error bodies never appear in errors.
use crate::{Connection, error};
use datafusion::error::Result;
use reqwest::{Client, Response};
use semantic_runtime::QueryContext;
use std::{sync::Arc, time::Duration};

pub(crate) fn client(config: &crate::ClickHouseConfig) -> Result<Client> {
    let mut builder = Client::builder()
        .user_agent("semantic-db-clickhouse")
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(config.connect_timeout)
        .read_timeout(config.read_timeout)
        .pool_idle_timeout(config.pool_idle_timeout)
        .no_proxy();
    if let Some(pem) = &config.ca_pem {
        builder = builder.add_root_certificate(
            reqwest::Certificate::from_pem(pem.as_bytes())
                .map_err(|_| error("invalid CA certificate"))?,
        );
    }
    if let Some(pem) = &config.identity_pem {
        builder = builder.identity(
            reqwest::Identity::from_pem(pem.as_bytes())
                .map_err(|_| error("invalid client identity"))?,
        );
    }
    if let Some(proxy) = &config.proxy {
        builder = builder.proxy(reqwest::Proxy::all(proxy).map_err(|_| error("invalid proxy"))?);
    }
    builder
        .build()
        .map_err(|_| error("HTTP client construction failed"))
}

pub(crate) async fn request(
    connection: &Connection,
    sql: &str,
    format: &str,
    query: &Arc<QueryContext>,
    request_id: &str,
) -> Result<Response> {
    let config = &connection.config;
    let mut params = vec![
        ("output_format_json_quote_64bit_integers".into(), "0".into()),
        ("database".to_owned(), config.database.clone()),
        ("query_id".into(), request_id.into()),
        ("readonly".into(), "1".into()),
        (
            "cancel_http_readonly_queries_on_client_close".into(),
            "1".into(),
        ),
        ("join_use_nulls".into(), "1".into()),
        ("cast_keep_nullable".into(), "1".into()),
        ("join_default_strictness".into(), "ALL".into()),
        (
            "output_format_arrow_string_as_string".into(),
            if config.string_as_binary { "0" } else { "1" }.into(),
        ),
        (
            "output_format_arrow_low_cardinality_as_dictionary".into(),
            if config.dictionary_output { "1" } else { "0" }.into(),
        ),
        (
            "output_format_arrow_compression_method".into(),
            config.codec.setting().into(),
        ),
        (
            "max_execution_time".into(),
            config.query_timeout.as_secs_f64().to_string(),
        ),
        ("max_block_size".into(), config.max_block_size.to_string()),
        ("log_comment".into(), connection.context.clone()),
        ("read_overflow_mode".into(), "throw".into()),
        ("result_overflow_mode".into(), "throw".into()),
        ("timeout_overflow_mode".into(), "throw".into()),
        ("group_by_overflow_mode".into(), "throw".into()),
        ("sort_overflow_mode".into(), "throw".into()),
        ("use_query_cache".into(), "0".into()),
    ];
    params.extend(
        config
            .server
            .settings()
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_string())),
    );
    params.extend(config.roles.iter().map(|v| ("role".to_owned(), v.clone())));
    let endpoints: Vec<_> = std::iter::once(&config.endpoint)
        .chain(&config.failover_endpoints)
        .collect();
    for attempt in 0..config.max_attempts {
        query.check()?;
        let mut request = connection
            .client
            .post(endpoints[attempt % endpoints.len()])
            .query(&params)
            .header("Content-Type", "text/plain")
            .body(format!("{sql} FORMAT {format}"));
        request = if let Some(token) = &config.bearer_token {
            request.bearer_auth(token)
        } else {
            request.basic_auth(&config.user, Some(&config.password))
        };
        query.request_started()?;
        let response = query
            .run(async {
                request
                    .send()
                    .await
                    .map_err(|_| error(&format!("transport failure; query_id={request_id}")))
            })
            .await;
        let mut delay = Duration::from_millis(100 * (1 << attempt));
        match response {
            Ok(response)
                if response.status().is_success()
                    && !response
                        .headers()
                        .contains_key("x-clickhouse-exception-code") =>
            {
                return Ok(response);
            }
            Ok(mut response) => {
                let status = response.status().as_u16();
                if let Some(seconds) = response
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                {
                    delay = Duration::from_secs(seconds.min(60));
                }
                let code = response
                    .headers()
                    .get("x-clickhouse-exception-code")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u32>().ok());
                // Drop after a bounded prefix, including responses with no Content-Length.
                let mut received = 0;
                while received < 65536 {
                    match query
                        .run(async {
                            response
                                .chunk()
                                .await
                                .map_err(|_| error("error response transport failure"))
                        })
                        .await?
                    {
                        Some(chunk) => {
                            query.charge_remote(chunk.len())?;
                            received += chunk.len();
                        }
                        None => break,
                    }
                }
                if ![429, 502, 503, 504].contains(&status) || attempt + 1 == config.max_attempts {
                    return Err(error(&format!(
                        "HTTP {status}; code={code:?}; query_id={request_id}"
                    )));
                }
            }
            Err(error) if attempt + 1 == config.max_attempts => return Err(error),
            Err(_) => {
                query.check()?;
            }
        }
        query
            .run(async {
                tokio::time::sleep(delay).await;
                Ok(())
            })
            .await?;
    }
    Err(error("request attempts exhausted"))
}
