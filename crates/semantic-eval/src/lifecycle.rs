use crate::{Dataset, Result};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::Command;
pub(crate) struct Lifecycle {
    pub project: String,
    files: Vec<PathBuf>,
    root: PathBuf,
    artifacts: PathBuf,
    pub bindings: BTreeMap<String, String>,
    pub keep: bool,
    pub attached: bool,
    finished: bool,
    executable: PathBuf,
}
impl Lifecycle {
    pub fn new(dataset: &Dataset, artifacts: &Path, keep: bool, attached: bool) -> Result<Self> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos();
        Ok(Self {
            project: format!("eval-{}-{now:x}", std::process::id()),
            files: dataset
                .manifest
                .environment
                .as_ref()
                .map(|e| e.compose_files.iter().map(|p| dataset.resolve(p)).collect())
                .transpose()?
                .unwrap_or_default(),
            root: dataset.root.clone(),
            artifacts: artifacts.to_owned(),
            bindings: BTreeMap::new(),
            keep,
            attached,
            finished: false,
            executable: "docker".into(),
        })
    }
    fn command(&self, args: &[String]) -> Command {
        let mut c = Command::new(&self.executable);
        c.args(["compose", "--project-name", &self.project]);
        for f in &self.files {
            c.arg("--file").arg(f);
        }
        c.args(args).current_dir(&self.root).kill_on_drop(true);
        c
    }
    async fn invoke(&self, label: &str, args: Vec<String>, seconds: u64) -> Result<String> {
        let output =
            tokio::time::timeout(Duration::from_secs(seconds), self.command(&args).output())
                .await
                .map_err(|_| format!("{label} timed out"))??;
        let mut log = output.stdout.clone();
        log.extend(&output.stderr);
        tokio::fs::write(self.artifacts.join(format!("compose-{label}.log")), log).await?;
        if !output.status.success() {
            return Err(format!(
                "Compose {label} failed with {} (see captured logs)",
                output.status
            )
            .into());
        }
        Ok(String::from_utf8(output.stdout)?)
    }
    pub async fn start(&mut self, d: &Dataset) -> Result<()> {
        let Some(e) = &d.manifest.environment else {
            return Ok(());
        };
        if self.attached {
            return Ok(());
        }
        let mut args = vec![
            "up".into(),
            "--detach".into(),
            "--wait".into(),
            "--wait-timeout".into(),
            e.startup_timeout_seconds.to_string(),
        ];
        args.extend(e.services.clone());
        self.invoke("startup", args, e.startup_timeout_seconds + 10)
            .await?;
        // Compose --wait also accepts running services without healthchecks; require health explicitly.
        let status = self
            .invoke(
                "health",
                vec!["ps".into(), "--format".into(), "json".into()],
                30,
            )
            .await?;
        let records: Vec<serde_json::Value> = if status.trim_start().starts_with('[') {
            serde_json::from_str(&status)?
        } else {
            status
                .lines()
                .filter(|s| !s.trim().is_empty())
                .map(serde_json::from_str)
                .collect::<std::result::Result<_, _>>()?
        };
        for service in &e.services {
            if !records.iter().any(|r| {
                r["Service"].as_str() == Some(service) && r["Health"].as_str() == Some("healthy")
            }) {
                return Err(
                    format!("service {service} lacks a healthy declared healthcheck").into(),
                );
            }
        }
        for (name, b) in &e.bindings {
            let address = self
                .invoke(
                    &format!("port-{name}"),
                    vec!["port".into(), b.service.clone(), b.port.to_string()],
                    30,
                )
                .await?;
            let (host, port) = parse_endpoint(address.trim())?;
            self.bindings.insert(
                name.clone(),
                b.template.replace("{host}", &host).replace("{port}", &port),
            );
        }
        if let Some(service) = &e.bootstrap_service {
            self.invoke(
                "bootstrap",
                vec![
                    "run".into(),
                    "--rm".into(),
                    "--no-deps".into(),
                    service.clone(),
                ],
                e.bootstrap_timeout_seconds,
            )
            .await?;
        }
        Ok(())
    }
    pub async fn finish(&mut self) -> Result<()> {
        if self.files.is_empty() || self.attached {
            self.finished = true;
            return Ok(());
        }
        let _ = self
            .invoke("logs", vec!["logs".into(), "--no-color".into()], 30)
            .await;
        if self.keep {
            self.finished = true;
            return Ok(());
        }
        self.invoke(
            "cleanup",
            vec![
                "down".into(),
                "--volumes".into(),
                "--remove-orphans".into(),
                "--timeout".into(),
                "10".into(),
            ],
            45,
        )
        .await?;
        self.finished = true;
        Ok(())
    }
    pub fn cleanup_command(&self) -> Vec<String> {
        let mut args = vec![
            "docker".into(),
            "compose".into(),
            "--project-name".into(),
            self.project.clone(),
        ];
        for p in &self.files {
            args.extend(["--file".into(), p.display().to_string()])
        }
        args.extend(["down".into(), "--volumes".into(), "--remove-orphans".into()]);
        args
    }
}
impl Drop for Lifecycle {
    fn drop(&mut self) {
        if !self.finished && !self.keep && !self.attached && !self.files.is_empty() {
            // Best effort fallback for cancellation or unwinding; normal finish is awaited.
            let mut c = std::process::Command::new(&self.executable);
            c.args(["compose", "--project-name", &self.project]);
            for f in &self.files {
                c.arg("--file").arg(f);
            }
            let _ = c
                .args(["down", "--volumes", "--remove-orphans", "--timeout", "1"])
                .current_dir(&self.root)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
    }
}
fn parse_endpoint(s: &str) -> Result<(String, String)> {
    let line = s.lines().next().ok_or("empty published endpoint")?;
    let (host, port) = line.rsplit_once(':').ok_or("invalid published endpoint")?;
    let _: u16 = port.parse()?;
    let host = host.trim_matches(['[', ']']);
    let host = if host == "0.0.0.0" || host == "::" {
        "127.0.0.1"
    } else {
        host
    };
    Ok((host.into(), port.into()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint() {
        assert_eq!(
            parse_endpoint("0.0.0.0:34567").unwrap(),
            ("127.0.0.1".into(), "34567".into())
        );
        assert!(parse_endpoint("bad").is_err());
    }
}

#[cfg(all(test, unix))]
mod failure_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    #[tokio::test]
    async fn bootstrap_failure_retains_logs_and_cleans_up() {
        let root = std::env::temp_dir().join(format!(
            "semantic-eval-lifecycle-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let script = root.join("fake-docker");
        std::fs::write(
            &script,
            r#"#!/bin/sh
case "$*" in
 *" up "*) echo started;;
 *" ps "*) echo '{"Service":"database","Health":"healthy"}';;
 *" port "*) echo '0.0.0.0:54321';;
 *" run "*) echo 'bootstrap intentionally failed' >&2; exit 7;;
 *" logs "*) echo 'service logs retained';;
 *" down "*) echo cleaned;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut lifecycle = Lifecycle {
            project: "test".into(),
            files: vec![root.join("compose.yaml")],
            root: root.clone(),
            artifacts: root.clone(),
            bindings: BTreeMap::new(),
            keep: false,
            attached: false,
            finished: false,
            executable: script,
        };
        let mut dataset = crate::Dataset::load(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/tiny/manifest.json"),
        )
        .unwrap();
        dataset.manifest.environment = Some(crate::Environment {
            compose_files: vec!["compose.yaml".into()],
            services: vec!["database".into()],
            bootstrap_service: Some("seed".into()),
            startup_timeout_seconds: 1,
            bootstrap_timeout_seconds: 1,
            bindings: BTreeMap::new(),
        });
        assert!(lifecycle.start(&dataset).await.is_err());
        assert!(
            std::fs::read_to_string(root.join("compose-bootstrap.log"))
                .unwrap()
                .contains("intentionally failed")
        );
        lifecycle.finish().await.unwrap();
        assert!(lifecycle.finished);
        assert!(root.join("compose-cleanup.log").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
