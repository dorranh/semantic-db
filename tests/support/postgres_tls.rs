use std::{fs, path::Path, process::Command, time::Duration};
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
};

#[allow(dead_code)] // Shared by connector test binaries with different TLS scenarios.
pub struct Certificates {
    pub ca_pem: String,
    pub other_ca_pem: String,
    pub client_cert_pem: String,
    pub client_key_pem: String,
    server_cert_pem: String,
    server_key_pem: String,
}

impl Certificates {
    fn generate() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "semantic-postgres-tls-{}",
            semantic_runtime::unique_id()
        ));
        fs::create_dir(&dir).unwrap();
        fs::write(
            dir.join("ca.cnf"),
            "[req]\nprompt=no\ndistinguished_name=dn\nx509_extensions=v3_ca\n[dn]\nCN=Semantic DB Test CA\n[v3_ca]\nbasicConstraints=critical,CA:TRUE\nkeyUsage=critical,keyCertSign,cRLSign\n",
        )
        .unwrap();
        fs::write(
            dir.join("leaf.cnf"),
            "[server_cert]\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:localhost\n[client_cert]\nbasicConstraints=CA:FALSE\nkeyUsage=digitalSignature\nextendedKeyUsage=clientAuth\n",
        )
        .unwrap();
        openssl(
            &dir,
            &[
                "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-sha256", "-days", "3650",
                "-keyout", "ca.key", "-out", "ca.crt", "-config", "ca.cnf",
            ],
        );
        openssl(
            &dir,
            &[
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-sha256",
                "-days",
                "3650",
                "-keyout",
                "other-ca.key",
                "-out",
                "other-ca.crt",
                "-config",
                "ca.cnf",
                "-subj",
                "/CN=Other Test CA",
            ],
        );
        openssl(
            &dir,
            &[
                "req",
                "-new",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-sha256",
                "-keyout",
                "server.key",
                "-out",
                "server.csr",
                "-subj",
                "/CN=localhost",
            ],
        );
        openssl(
            &dir,
            &[
                "x509",
                "-req",
                "-in",
                "server.csr",
                "-CA",
                "ca.crt",
                "-CAkey",
                "ca.key",
                "-CAcreateserial",
                "-days",
                "3650",
                "-sha256",
                "-out",
                "server.crt",
                "-extfile",
                "leaf.cnf",
                "-extensions",
                "server_cert",
            ],
        );
        openssl(
            &dir,
            &[
                "req",
                "-new",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-sha256",
                "-keyout",
                "client.key",
                "-out",
                "client.csr",
                "-subj",
                "/CN=postgres",
            ],
        );
        openssl(
            &dir,
            &[
                "x509",
                "-req",
                "-in",
                "client.csr",
                "-CA",
                "ca.crt",
                "-CAkey",
                "ca.key",
                "-CAserial",
                "ca.srl",
                "-days",
                "3650",
                "-sha256",
                "-out",
                "client.crt",
                "-extfile",
                "leaf.cnf",
                "-extensions",
                "client_cert",
            ],
        );
        let certs = Self {
            ca_pem: fs::read_to_string(dir.join("ca.crt")).unwrap(),
            other_ca_pem: fs::read_to_string(dir.join("other-ca.crt")).unwrap(),
            client_cert_pem: fs::read_to_string(dir.join("client.crt")).unwrap(),
            client_key_pem: fs::read_to_string(dir.join("client.key")).unwrap(),
            server_cert_pem: fs::read_to_string(dir.join("server.crt")).unwrap(),
            server_key_pem: fs::read_to_string(dir.join("server.key")).unwrap(),
        };
        fs::remove_dir_all(dir).unwrap();
        certs
    }
}

fn openssl(dir: &Path, args: &[&str]) {
    let output = Command::new("openssl")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("OpenSSL is required for the Docker TLS test");
    assert!(
        output.status.success(),
        "OpenSSL certificate generation failed"
    );
}

pub struct Database {
    pub certs: Certificates,
    pub plain_client: Option<tokio_postgres::Client>,
    port: u16,
    _container: ContainerAsync<GenericImage>,
}

impl Database {
    pub async fn start(client_auth: bool) -> Self {
        let certs = Certificates::generate();
        let hba = "local all all trust\nhostssl all all all cert clientcert=verify-full\nhost all all all reject\n";
        let mut script = "cp /tmp/tls/server.key /tmp/semantic-server.key && chown postgres:postgres /tmp/semantic-server.key && chmod 600 /tmp/semantic-server.key && exec docker-entrypoint.sh postgres -c ssl=on -c ssl_cert_file=/tmp/tls/server.crt -c ssl_key_file=/tmp/semantic-server.key -c ssl_ca_file=/tmp/tls/ca.crt".to_string();
        if client_auth {
            script.push_str(" -c hba_file=/tmp/tls/pg_hba.conf");
        }
        let image = GenericImage::new("postgres", "18.3-alpine")
            .with_entrypoint("/bin/sh")
            .with_exposed_port(5432.tcp())
            .with_wait_for(WaitFor::message_on_stderr(
                "database system is ready to accept connections",
            ))
            .with_env_var("POSTGRES_PASSWORD", "fixture")
            .with_copy_to("/tmp/tls/ca.crt", certs.ca_pem.as_bytes().to_vec())
            .with_copy_to(
                "/tmp/tls/server.crt",
                certs.server_cert_pem.as_bytes().to_vec(),
            )
            .with_copy_to(
                "/tmp/tls/server.key",
                certs.server_key_pem.as_bytes().to_vec(),
            )
            .with_copy_to("/tmp/tls/pg_hba.conf", hba.as_bytes().to_vec())
            .with_copy_to(
                "/docker-entrypoint-initdb.d/001-tls-probe.sql",
                b"CREATE TABLE tls_probe (id bigint);".to_vec(),
            )
            .with_cmd(["-c", &script]);
        let container = image.start().await.unwrap();
        let port = container.get_host_port_ipv4(5432).await.unwrap();
        let mut plain_client = None;
        let mut ready = false;
        for _ in 0..60 {
            let url =
                format!("postgresql://postgres:fixture@127.0.0.1:{port}/postgres?sslmode=disable");
            if client_auth {
                let tls = semantic_postgres::PostgresTlsConfig {
                    ca_pem: Some(certs.ca_pem.clone()),
                    client_cert_pem: Some(certs.client_cert_pem.clone()),
                    client_key_pem: Some(certs.client_key_pem.clone()),
                };
                let pg = semantic_postgres::Postgres::new_with_tls(
                    &format!("host=localhost hostaddr=127.0.0.1 port={port} user=postgres dbname=postgres sslmode=verify-full"),
                    1,
                    1,
                    tls,
                )
                .unwrap();
                if pg.table("public", "tls_probe").await.is_ok() {
                    ready = true;
                    break;
                }
            } else if let Ok((client, connection)) =
                tokio_postgres::connect(&url, tokio_postgres::NoTls).await
            {
                tokio::spawn(async move {
                    let _ = connection.await;
                });
                plain_client = Some(client);
                ready = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert!(ready, "TLS Postgres not ready");
        Self {
            certs,
            plain_client,
            port,
            _container: container,
        }
    }

    pub fn connection_string(&self, mode: &str, host: &str) -> String {
        format!(
            "host={host} hostaddr=127.0.0.1 port={} user=postgres password=fixture dbname=postgres sslmode={mode}",
            self.port
        )
    }

    pub fn connection_url(&self, mode: &str, host: &str) -> String {
        format!(
            "postgresql://postgres:fixture@{host}:{}/postgres?hostaddr=127.0.0.1&sslmode={mode}",
            self.port
        )
    }
}
