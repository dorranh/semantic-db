use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
};
pub struct Database {
    pub client: tokio_postgres::Client,
    pub url: String,
    _container: ContainerAsync<GenericImage>,
}
impl Database {
    pub async fn start() -> Self {
        let container = GenericImage::new("postgres", "18.3-alpine")
            .with_exposed_port(5432.tcp())
            .with_wait_for(WaitFor::message_on_stderr(
                "database system is ready to accept connections",
            ))
            .with_env_var("POSTGRES_PASSWORD", "fixture")
            .start()
            .await
            .unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let url =
            format!("postgresql://postgres:fixture@127.0.0.1:{port}/postgres?sslmode=disable");
        // Postgres restarts once after initializing its data directory.
        let mut result = None;
        for _ in 0..60 {
            match tokio_postgres::connect(&url, tokio_postgres::NoTls).await {
                Ok((client, connection)) => {
                    tokio::spawn(async move {
                        let _ = connection.await;
                    });
                    result = Some(client);
                    break;
                }
                Err(_) => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
            }
        }
        Self {
            client: result.expect("Postgres ready"),
            url,
            _container: container,
        }
    }
}
