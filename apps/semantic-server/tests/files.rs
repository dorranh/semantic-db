#[path = "../../../tests/support/file_sources.rs"]
mod fixtures;
use fixtures::Files;
use serde_json::json;
use std::{
    process::{Child, Command, Stdio},
    time::Duration,
};
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
#[tokio::test]
async fn release_server_loads_every_file_format_and_exposes_catalog() {
    let files = Files::new();
    for name in Files::FORMATS {
        let config = files.config(json!({"path":name}));
        let pg = port();
        let mut http = port();
        while http == pg {
            http = port();
        }
        let mut process = Process(
            Command::new(env!("CARGO_BIN_EXE_semantic-server"))
                .current_dir(&files.0)
                .args([
                    "--config",
                    config.to_str().unwrap(),
                    "--port",
                    &pg.to_string(),
                    "--http-port",
                    &http.to_string(),
                ])
                .env_remove("OPENAI_API_KEY")
                .stdout(Stdio::null())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap(),
        );
        let url = format!("http://127.0.0.1:{http}");
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap();
        let ready = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                assert!(
                    process.0.try_wait().unwrap().is_none(),
                    "server exited for {name}"
                );
                if client
                    .get(format!("{url}/health"))
                    .send()
                    .await
                    .is_ok_and(|r| r.status().is_success())
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await;
        assert!(ready.is_ok(), "server did not become ready for {name}");
        let catalog = client
            .get(format!("{url}/catalog"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(catalog.contains("selected_products"));
        let (client, connection) = tokio_postgres::connect(
            &format!("host=127.0.0.1 port={pg} user=test dbname=semantic"),
            tokio_postgres::NoTls,
        )
        .await
        .unwrap();
        let task = tokio::spawn(connection);
        let rows = client
            .query("SELECT code FROM selected_products ORDER BY code", &[])
            .await
            .unwrap();
        assert_eq!(
            rows.iter()
                .map(|r| r.get::<_, String>(0))
                .collect::<Vec<_>>(),
            ["00123", "00456"],
            "{name}"
        );
        drop(client);
        task.abort();
    }
}
