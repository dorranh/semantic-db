use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

use semantic_compiler::provider::{
    Message, ModelProvider, OpenAiConfig, OpenAiProvider, ProviderError, Role,
};
use serde_json::json;

fn server(status: u16, body: String, delay: Duration) -> (String, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/custom/v1/", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut data = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let read = socket.read(&mut buffer).unwrap();
            assert!(read > 0);
            data.extend_from_slice(&buffer[..read]);
            if let Some(header_end) = data.windows(4).position(|value| value == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&data[..header_end]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .map(str::to_owned)
                    })
                    .unwrap()
                    .parse()
                    .unwrap();
                if data.len() >= header_end + 4 + length {
                    break;
                }
            }
        }
        thread::sleep(delay);
        let response = format!(
            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = socket.write_all(response.as_bytes());
        String::from_utf8(data).unwrap()
    });
    (url, handle)
}

fn messages() -> Vec<Message> {
    vec![Message {
        role: Role::User,
        content: "Return JSON".into(),
    }]
}

#[tokio::test]
async fn posts_compatible_envelope_and_honors_json_mode_toggle() {
    for json_mode in [true, false] {
        let body = json!({"choices":[{"message":{"content":"{\"status\":\"unsupported\",\"reason\":\"test\"}"},"finish_reason":"stop"}]}).to_string();
        let (url, handle) = server(200, body, Duration::ZERO);
        let mut config = OpenAiConfig::new("test-key".into(), "test-model".into());
        config.base_url = url;
        config.json_mode = json_mode;
        let provider = OpenAiProvider::new(config).unwrap();
        assert!(
            provider
                .complete(&messages())
                .await
                .unwrap()
                .contains("unsupported")
        );
        let request = handle.join().unwrap();
        assert!(request.starts_with("POST /custom/v1/chat/completions HTTP/1.1\r\n"));
        assert!(
            request
                .to_lowercase()
                .contains("authorization: bearer test-key")
        );
        let body: serde_json::Value =
            serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body.get("response_format").is_some(), json_mode);
        if json_mode {
            assert_eq!(body["response_format"]["type"], "json_object");
        }
    }
}

#[tokio::test]
async fn rejects_http_errors_refusals_truncation_and_malformed_envelopes() {
    for (status, body, expected) in [
        (401, "secret-key".into(), "HTTP 401"),
        (429, "secret-key".into(), "HTTP 429"),
        (500, "secret-key".into(), "HTTP 500"),
        (302, "secret-key".into(), "HTTP 302"),
        (200, "not JSON".into(), "JSON envelope"),
        (200, json!({"choices":[]}).to_string(), "no choices"),
        (200, json!({"choices":[{"message":{"content":null,"refusal":"secret-key"},"finish_reason":"stop"}]}).to_string(), "refused"),
        (200, json!({"choices":[{"message":{"content":"{}"},"finish_reason":"length"}]}).to_string(), "did not finish"),
        (200, json!({"choices":[{"message":{"content":""},"finish_reason":"stop"}]}).to_string(), "empty text"),
        (200, "x".repeat(1024 * 1024 + 1), "exceeds 1 MiB"),
    ] {
        let (url, handle) = server(status, body, Duration::ZERO);
        let mut config = OpenAiConfig::new("secret-key".into(), "test".into());
        config.base_url = url;
        let error = OpenAiProvider::new(config).unwrap().complete(&messages()).await.unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
        assert!(!format!("{error:?}").contains("secret-key"));
        handle.join().unwrap();
    }
}

#[tokio::test]
async fn times_out_requests() {
    let (url, handle) = server(200, "{}".into(), Duration::from_millis(300));
    let mut config = OpenAiConfig::new("test".into(), "test".into());
    config.base_url = url;
    config.timeout = Duration::from_millis(100);
    assert!(matches!(
        OpenAiProvider::new(config)
            .unwrap()
            .complete(&messages())
            .await,
        Err(ProviderError::Timeout)
    ));
    handle.join().unwrap();
}

#[test]
fn rejects_invalid_configuration_without_exposing_credentials() {
    for url in [
        "not a URL",
        "file:///private/data",
        "https://secret@example.com/v1",
        "https://example.com/v1?key=secret",
    ] {
        let mut config = OpenAiConfig::new("secret".into(), "test".into());
        config.base_url = url.into();
        let error = OpenAiProvider::new(config).err().unwrap();
        assert!(!format!("{error:?}").contains("secret"));
    }
    assert!(OpenAiProvider::new(OpenAiConfig::new("".into(), "test".into())).is_err());
    assert!(OpenAiProvider::new(OpenAiConfig::new("test".into(), "".into())).is_err());
}
