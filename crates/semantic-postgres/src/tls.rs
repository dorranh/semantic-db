use datafusion::error::Result;
use deadpool_postgres::{Connect, Manager, ManagerConfig};
use rustls::{
    DigitallySignedStruct, RootCertStore, SignatureScheme,
    client::{
        danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
        verify_server_cert_signed_by_trust_anchor,
    },
    crypto::{WebPkiSupportedAlgorithms, verify_tls12_signature, verify_tls13_signature},
    pki_types::{CertificateDer, ServerName, UnixTime},
    server::ParsedCertificate,
};
use semantic_runtime::failure;
use std::{future::Future, io::Cursor, pin::Pin, sync::Arc};
use tokio::task::JoinHandle;
use tokio_postgres::{Client, Config, Error, NoTls, config::SslMode as NativeSslMode};
use tokio_postgres_rustls::MakeRustlsConnect;

/// Optional PEM material for server trust and mutual TLS.
#[derive(Clone, Default)]
pub struct PostgresTlsConfig {
    pub ca_pem: Option<String>,
    pub client_cert_pem: Option<String>,
    pub client_key_pem: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SslMode {
    Disable,
    Allow,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl SslMode {
    fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "disable" => Self::Disable,
            "allow" => Self::Allow,
            "prefer" => Self::Prefer,
            "require" => Self::Require,
            "verify-ca" => Self::VerifyCa,
            "verify-full" => Self::VerifyFull,
            _ => return Err(failure("invalid Postgres sslmode")),
        })
    }
}

pub(crate) fn normalize_connection_string(input: &str) -> Result<(String, SslMode)> {
    let (value, span) = if input.starts_with("postgres://") || input.starts_with("postgresql://") {
        url_mode(input)?
    } else {
        keyword_mode(input)?
    };
    let mode = SslMode::parse(&value)?;
    let native = match mode {
        SslMode::Disable => "disable",
        _ => "require",
    };
    let mut normalized = input.to_owned();
    normalized.replace_range(span, &format!("sslmode={native}"));
    Ok((normalized, mode))
}

fn url_mode(input: &str) -> Result<(String, std::ops::Range<usize>)> {
    let start = input
        .find('?')
        .ok_or_else(|| failure("Postgres connection string requires explicit sslmode"))?
        + 1;
    let mut found = None;
    let mut offset = start;
    for segment in input[start..].split('&') {
        if let Some((key, value)) = url::form_urlencoded::parse(segment.as_bytes()).next()
            && key == "sslmode"
        {
            if found.is_some() {
                return Err(failure("duplicate Postgres sslmode"));
            }
            found = Some((value.into_owned(), offset..offset + segment.len()));
        }
        offset += segment.len() + 1;
    }
    found.ok_or_else(|| failure("Postgres connection string requires explicit sslmode"))
}

fn keyword_mode(input: &str) -> Result<(String, std::ops::Range<usize>)> {
    let mut offset = 0;
    let mut found = None;
    while offset < input.len() {
        while next(input, offset).is_some_and(|(c, _)| c.is_whitespace()) {
            offset += next(input, offset).unwrap().1;
        }
        if offset == input.len() {
            break;
        }
        let key_start = offset;
        while next(input, offset).is_some_and(|(c, _)| !c.is_whitespace() && c != '=') {
            offset += next(input, offset).unwrap().1;
        }
        let key = &input[key_start..offset];
        while next(input, offset).is_some_and(|(c, _)| c.is_whitespace()) {
            offset += next(input, offset).unwrap().1;
        }
        if key.is_empty() || next(input, offset).map(|(c, _)| c) != Some('=') {
            return Err(failure("invalid Postgres connection string"));
        }
        offset += 1;
        while next(input, offset).is_some_and(|(c, _)| c.is_whitespace()) {
            offset += next(input, offset).unwrap().1;
        }
        let value_start = offset;
        let quoted = next(input, offset).map(|(c, _)| c) == Some('\'');
        if quoted {
            offset += 1;
        }
        let mut value = String::new();
        let mut closed = !quoted;
        while let Some((c, width)) = next(input, offset) {
            if quoted && c == '\'' {
                offset += width;
                closed = true;
                break;
            }
            if !quoted && c.is_whitespace() {
                break;
            }
            offset += width;
            if c == '\\' {
                if let Some((escaped, width)) = next(input, offset) {
                    value.push(escaped);
                    offset += width;
                }
            } else {
                value.push(c);
            }
        }
        if !closed || (!quoted && value.is_empty()) {
            return Err(failure("invalid Postgres connection string"));
        }
        if key == "sslmode" {
            if found.is_some() {
                return Err(failure("duplicate Postgres sslmode"));
            }
            found = Some((value, key_start..offset));
        }
        if offset == value_start {
            return Err(failure("invalid Postgres connection string"));
        }
    }
    found.ok_or_else(|| failure("Postgres connection string requires explicit sslmode"))
}

fn next(input: &str, offset: usize) -> Option<(char, usize)> {
    input[offset..].chars().next().map(|c| (c, c.len_utf8()))
}

pub(crate) fn manager(
    config: Config,
    mode: SslMode,
    secrets: &PostgresTlsConfig,
) -> Result<Manager> {
    let tls = build_tls(mode, secrets)?;
    Ok(Manager::from_connect(
        config,
        PgConnect { mode, tls },
        ManagerConfig::default(),
    ))
}

#[derive(Clone)]
struct PgConnect {
    mode: SslMode,
    tls: Option<MakeRustlsConnect>,
}

impl Connect for PgConnect {
    fn connect(
        &self,
        pg_config: &Config,
    ) -> Pin<
        Box<dyn Future<Output = std::result::Result<(Client, JoinHandle<()>), Error>> + Send + '_>,
    > {
        let config = pg_config.clone();
        let mode = self.mode;
        let tls = self.tls.clone();
        Box::pin(async move {
            match mode {
                SslMode::Disable => plain(&config).await,
                SslMode::Allow => match plain(&config).await {
                    Ok(connection) => Ok(connection),
                    Err(_) => encrypted(&config, tls.expect("TLS configured")).await,
                },
                SslMode::Prefer => {
                    match encrypted(&config, tls.clone().expect("TLS configured")).await {
                        Ok(connection) => Ok(connection),
                        Err(_) => plain(&config).await,
                    }
                }
                SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => {
                    encrypted(&config, tls.expect("TLS configured")).await
                }
            }
        })
    }
}

async fn plain(config: &Config) -> std::result::Result<(Client, JoinHandle<()>), Error> {
    let mut config = config.clone();
    config.ssl_mode(NativeSslMode::Disable);
    let (client, connection) = config.connect(NoTls).await?;
    Ok((
        client,
        tokio::spawn(async move {
            let _ = connection.await;
        }),
    ))
}

async fn encrypted(
    config: &Config,
    tls: MakeRustlsConnect,
) -> std::result::Result<(Client, JoinHandle<()>), Error> {
    let mut config = config.clone();
    config.ssl_mode(NativeSslMode::Require);
    let (client, connection) = config.connect(tls).await?;
    Ok((
        client,
        tokio::spawn(async move {
            let _ = connection.await;
        }),
    ))
}

// Libpq's encryption-only modes do not authenticate the server. The same
// verifier also implements verify-ca: it checks the chain but not the name.
// TLS handshake signatures remain checked by rustls in both cases.
#[derive(Debug)]
struct PolicyVerifier {
    roots: Option<RootCertStore>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ServerCertVerifier for PolicyVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, rustls::Error> {
        if let Some(roots) = &self.roots {
            let cert = ParsedCertificate::try_from(end_entity)?;
            verify_server_cert_signed_by_trust_anchor(
                &cert,
                roots,
                intermediates,
                now,
                self.algorithms.all,
            )?;
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

fn build_tls(mode: SslMode, secrets: &PostgresTlsConfig) -> Result<Option<MakeRustlsConnect>> {
    if secrets.client_cert_pem.is_some() != secrets.client_key_pem.is_some() {
        return Err(failure(
            "Postgres client certificate and key must be supplied together",
        ));
    }
    if mode == SslMode::Disable {
        if secrets.ca_pem.is_some() || secrets.client_cert_pem.is_some() {
            return Err(failure("Postgres TLS certificates require an SSL mode"));
        }
        return Ok(None);
    }
    if matches!(mode, SslMode::Allow | SslMode::Prefer) && secrets.ca_pem.is_some() {
        return Err(failure(
            "Postgres custom CA requires sslmode=require or a verified mode",
        ));
    }

    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verification = match mode {
        SslMode::VerifyFull => 2,
        SslMode::VerifyCa => 1,
        SslMode::Require if secrets.ca_pem.is_some() => 1,
        _ => 0,
    };
    let roots = if verification == 0 {
        None
    } else {
        Some(load_roots(secrets.ca_pem.as_deref())?)
    };
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|_| failure("could not configure Postgres TLS"))?;
    let builder = if verification == 2 {
        builder.with_root_certificates(roots.expect("verified roots"))
    } else {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PolicyVerifier {
                roots,
                algorithms: provider.signature_verification_algorithms,
            }))
    };
    let client = match (&secrets.client_cert_pem, &secrets.client_key_pem) {
        (Some(cert), Some(key)) => {
            let certs = rustls_pemfile::certs(&mut Cursor::new(cert.as_bytes()))
                .collect::<std::io::Result<Vec<_>>>()
                .map_err(|_| failure("invalid Postgres client certificate"))?;
            if certs.is_empty() {
                return Err(failure("invalid Postgres client certificate"));
            }
            let key = rustls_pemfile::private_key(&mut Cursor::new(key.as_bytes()))
                .map_err(|_| failure("invalid Postgres client key"))?
                .ok_or_else(|| failure("invalid Postgres client key"))?;
            builder
                .with_client_auth_cert(certs, key)
                .map_err(|_| failure("invalid Postgres client identity"))?
        }
        _ => builder.with_no_client_auth(),
    };
    Ok(Some(MakeRustlsConnect::new(client)))
}

fn load_roots(pem: Option<&str>) -> Result<RootCertStore> {
    let mut roots = RootCertStore::empty();
    if let Some(pem) = pem {
        let certs = rustls_pemfile::certs(&mut Cursor::new(pem.as_bytes()))
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|_| failure("invalid Postgres CA certificate"))?;
        if certs.is_empty() {
            return Err(failure("invalid Postgres CA certificate"));
        }
        for cert in certs {
            roots
                .add(cert)
                .map_err(|_| failure("invalid Postgres CA certificate"))?;
        }
    } else {
        let native = rustls_native_certs::load_native_certs();
        roots.add_parsable_certificates(native.certs);
        if roots.is_empty() {
            return Err(failure("no Postgres TLS trust roots available"));
        }
    }
    Ok(roots)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_ssl_modes_are_explicit_and_normalized() {
        for (name, mode) in [
            ("disable", SslMode::Disable),
            ("allow", SslMode::Allow),
            ("prefer", SslMode::Prefer),
            ("require", SslMode::Require),
            ("verify-ca", SslMode::VerifyCa),
            ("verify-full", SslMode::VerifyFull),
        ] {
            let (url, actual) = normalize_connection_string(&format!(
                "postgresql://user:secret@localhost/db?application_name=semantic&sslmode={name}"
            ))
            .unwrap();
            assert_eq!(actual, mode);
            assert!(url.contains("application_name=semantic"));
            assert!(url.contains("user:secret@localhost"));
            assert!(url.contains(if mode == SslMode::Disable {
                "sslmode=disable"
            } else {
                "sslmode=require"
            }));
            let (keywords, actual) = normalize_connection_string(&format!(
                "user='a b' password='sec ret' sslmode='{name}' host=localhost"
            ))
            .unwrap();
            assert_eq!(actual, mode);
            assert!(keywords.contains("password='sec ret'"));
            assert!(keywords.contains("user='a b'"));
            assert!(keywords.parse::<Config>().is_ok());
        }
        assert!(normalize_connection_string("postgresql://localhost/db").is_err());
        assert!(normalize_connection_string("host=localhost user=postgres").is_err());
        assert!(normalize_connection_string("host=localhost sslmode=invalid").is_err());
        assert!(
            normalize_connection_string("host=localhost sslmode=require sslmode=disable").is_err()
        );
        assert!(
            normalize_connection_string(
                "postgresql://localhost/db?sslmode=require&sslmode=disable"
            )
            .is_err()
        );
    }

    #[test]
    fn certificate_options_fail_without_exposing_material() {
        let secret = "THIS-IS-SENSITIVE";
        let config = PostgresTlsConfig {
            ca_pem: Some(secret.into()),
            ..Default::default()
        };
        let error = match build_tls(SslMode::VerifyFull, &config) {
            Ok(_) => panic!("invalid CA unexpectedly accepted"),
            Err(error) => error,
        };
        assert!(!error.to_string().contains(secret));
        assert!(build_tls(SslMode::Disable, &config).is_err());
        assert!(build_tls(SslMode::Prefer, &config).is_err());
        let config = PostgresTlsConfig {
            client_cert_pem: Some(secret.into()),
            ..Default::default()
        };
        assert!(build_tls(SslMode::Require, &config).is_err());
    }
}
