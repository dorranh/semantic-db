use std::{collections::HashMap, time::Duration};

use semantic_compiler::provider::{OpenAiConfig, OpenAiProvider};

/// Read .env from the working directory only when natural-language mode is used.
/// Existing process variables take precedence. Do not mutate global environment
/// state (unsafe once Tokio threads exist in Rust 2024).
pub fn provider() -> super::Result<OpenAiProvider> {
    let env = environment()?;
    let config = parse_config(env)?;
    Ok(OpenAiProvider::new(config)?)
}

/// The CLI resolves secrets; the shared loader never reads process state.
pub fn environment() -> super::Result<impl Fn(&str) -> Option<String> + Send + Sync> {
    let file = match dotenvy::from_path_iter(".env") {
        Ok(values) => values
            .collect::<std::result::Result<HashMap<_, _>, _>>()
            .map_err(|_| "invalid .env syntax; check the file without printing credentials")?,
        Err(dotenvy::Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            HashMap::new()
        }
        Err(_) => return Err("could not read .env in the current directory".into()),
    };
    Ok(move |name: &str| std::env::var(name).ok().or_else(|| file.get(name).cloned()))
}

fn parse_config(get: impl Fn(&str) -> Option<String>) -> super::Result<OpenAiConfig> {
    let api_key = get("OPENAI_API_KEY")
        .filter(|key| !key.trim().is_empty())
        .ok_or("set OPENAI_API_KEY in the environment or .env to use --ask/.ask")?;
    let model = get("OPENAI_MODEL").unwrap_or_else(|| "gpt-4.1-mini".into());
    let mut config = OpenAiConfig::new(api_key, model);
    if let Some(url) = get("OPENAI_BASE_URL") {
        config.base_url = url;
    }
    if let Some(value) = get("OPENAI_TIMEOUT_SECONDS") {
        let seconds: u64 = value
            .parse()
            .ok()
            .filter(|seconds| *seconds > 0)
            .ok_or("OPENAI_TIMEOUT_SECONDS must be a positive integer")?;
        config.timeout = Duration::from_secs(seconds);
    }
    if let Some(value) = get("OPENAI_JSON_MODE") {
        config.json_mode = value
            .parse()
            .map_err(|_| "OPENAI_JSON_MODE must be true or false")?;
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_defaults_and_overrides_without_global_environment_mutation() {
        let config =
            parse_config(|name| (name == "OPENAI_API_KEY").then(|| "test-key".into())).unwrap();
        assert_eq!(config.model, "gpt-4.1-mini");
        assert_eq!(config.base_url, "https://api.openai.com/v1");
        assert!(config.json_mode);
        let config = parse_config(|name| match name {
            "OPENAI_API_KEY" => Some("test-key".into()),
            "OPENAI_MODEL" => Some("local-model".into()),
            "OPENAI_BASE_URL" => Some("http://localhost:8080/v1".into()),
            "OPENAI_TIMEOUT_SECONDS" => Some("15".into()),
            "OPENAI_JSON_MODE" => Some("false".into()),
            _ => None,
        })
        .unwrap();
        assert_eq!(config.model, "local-model");
        assert_eq!(config.timeout, Duration::from_secs(15));
        assert!(!config.json_mode);
    }

    #[test]
    fn rejects_missing_credentials_and_bad_options_without_echoing_values() {
        assert!(parse_config(|_| None).is_err());
        for (name, value) in [
            ("OPENAI_TIMEOUT_SECONDS", "0"),
            ("OPENAI_TIMEOUT_SECONDS", "secret"),
            ("OPENAI_JSON_MODE", "secret"),
        ] {
            let error = parse_config(|key| {
                if key == "OPENAI_API_KEY" {
                    Some("test-key".into())
                } else if key == name {
                    Some(value.into())
                } else {
                    None
                }
            })
            .err()
            .unwrap()
            .to_string();
            assert!(error.contains(name));
            assert!(!error.contains("secret"));
        }
    }
}
