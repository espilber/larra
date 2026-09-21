//! Probing an external OpenAI-compatible endpoint.
//!
//! The Settings UI asks an endpoint which models it serves (`GET /models`,
//! the OpenAI list endpoint that llama.cpp, LM Studio, Ollama and friends all
//! speak) and shows them in a picker. The probe never sends documents.

use anyhow::{Context, Result};
use std::time::Duration;

use crate::settings::EndpointConfig;

/// One model the endpoint reports.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EndpointModel {
    pub id: String,
}

/// Ask the endpoint for its model list. `client` is the engine's plain HTTP
/// client (no https-only restriction: a LAN box usually speaks plain HTTP).
pub async fn probe_models(
    client: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<EndpointModel>> {
    let url = format!("{base_url}/models");
    let mut request = client.get(&url).timeout(Duration::from_secs(8));
    if let Some(key) = api_key {
        request = request.bearer_auth(key);
    }
    let response = request.send().await.context("GET {base_url}/models")?;
    let status = response.status();
    if !status.is_success() {
        anyhow::bail!("HTTP {status} from {base_url}/models");
    }
    let value: serde_json::Value = response
        .json()
        .await
        .context("endpoint /models body is not JSON")?;
    let data = value
        .get("data")
        .and_then(|data| data.as_array())
        .context("endpoint /models has no data array")?;
    let mut models: Vec<EndpointModel> = data
        .iter()
        .filter_map(|item| item.get("id"))
        .filter_map(|id| id.as_str())
        .filter(|id| !id.is_empty())
        .map(|id| EndpointModel { id: id.to_string() })
        .collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup();
    Ok(models)
}

/// True when the parsed config matches a machine that runs llama.cpp
/// (LLaVA-style multimodal GGUFs carry a separate mmproj file).
pub fn endpoint_base_url(config: &EndpointConfig) -> &str {
    &config.base_url
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::EndpointConfig;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Serve one canned HTTP response, then report the bound port.
    async fn serve_one(response: &'static str) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 4096];
            let _ = socket.read(&mut buffer).await;
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        (format!("http://127.0.0.1:{port}"), task)
    }

    async fn test_client() -> reqwest::Client {
        reqwest::Client::builder().build().unwrap()
    }

    #[tokio::test]
    async fn probe_models_parses_the_data_array() {
        let payload =
            b"{\"data\":[{\"id\":\"gemma-4-12b\"},{\"id\":\"llama3\"},{\"id\":\"gemma-4-12b\"}]}";
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                payload.len(),
                std::str::from_utf8(payload).unwrap()
            )
            .into_boxed_str(),
        );
        let (base_url, task) = serve_one(response).await;
        let models = probe_models(&test_client().await, &base_url, None)
            .await
            .unwrap();
        task.await.unwrap();
        assert_eq!(
            models,
            vec![
                EndpointModel {
                    id: "gemma-4-12b".into()
                },
                EndpointModel {
                    id: "llama3".into()
                }
            ]
        );
    }

    #[tokio::test]
    async fn probe_models_sends_bearer_auth() {
        // A 401 without a header proves the request reached us; the point of
        // the test is the Authorization header, read from the request bytes.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0u8; 4096];
            let read = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let body = if request.contains("Bearer sk-test") {
                "{\"data\":[{\"id\":\"seen\"}]}"
            } else {
                "{\"data\":[]}"
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = response;
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        let models = probe_models(
            &test_client().await,
            &format!("http://127.0.0.1:{port}"),
            Some("sk-test"),
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "seen");
    }

    #[tokio::test]
    async fn probe_models_surfaces_http_errors() {
        let canned: &'static str =
            "HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n";
        let (base_url, task) = serve_one(canned).await;
        let error = probe_models(&test_client().await, &base_url, None)
            .await
            .unwrap_err();
        task.await.unwrap();
        assert!(error.to_string().contains("404"), "{error}");
    }

    #[tokio::test]
    async fn probe_models_reports_connection_failures() {
        // Nothing listens on port 1 on loopback; the probe must fail with a
        // message the Settings UI can show.
        let client = test_client().await;
        let error = probe_models(&client, "http://127.0.0.1:1", None)
            .await
            .unwrap_err();
        assert!(!error.to_string().is_empty());
    }

    #[test]
    fn endpoint_base_url_returns_the_configured_url() {
        let config = EndpointConfig::parse("http://127.0.0.1:1234/v1", "m", None).unwrap();
        assert_eq!(endpoint_base_url(&config), "http://127.0.0.1:1234/v1");
        let _ = Arc::new(()); // keep the Arc import used in style with crate tests
    }
}
