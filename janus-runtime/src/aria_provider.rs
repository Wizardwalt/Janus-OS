//! Server-side OpenAI-compatible chat-completions adapter for Aria.
//!
//! Provider credentials remain in protected runtime configuration and are
//! never sent to browser clients or written to the conversation database.

use anyhow::{bail, ensure, Context, Result};
use reqwest::{redirect::Policy, Client, Url};
use serde::{Deserialize, Serialize};
use std::{future::Future, path::Path, pin::Pin, sync::Arc, time::Duration};

const MAX_PROVIDER_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_ASSISTANT_MESSAGE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct AriaProviderMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct AriaProviderCompletion {
    pub content: String,
    pub provider: String,
    pub model: String,
}

#[derive(Clone)]
pub struct AriaModelProvider {
    endpoint: Url,
    provider_name: String,
    model: String,
    api_key: Option<String>,
    client: Client,
}

#[derive(Serialize)]
struct ChatCompletionRequest<'a> {
    model: &'a str,
    messages: &'a [AriaProviderMessage],
    temperature: f32,
    max_tokens: u16,
}

#[derive(Deserialize)]
struct ChatCompletionResponse {
    model: Option<String>,
    choices: Vec<ChatCompletionChoice>,
}

#[derive(Deserialize)]
struct ChatCompletionChoice {
    message: ChatCompletionMessage,
}

#[derive(Deserialize)]
struct ChatCompletionMessage {
    content: Option<String>,
}

/// Provider-neutral asynchronous chat contract used by the authenticated runtime.
/// Implementations own their transport and secret handling; the API layer depends
/// only on this contract, not on a vendor SDK or wire format.
pub trait AriaChatProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn model(&self) -> &str;

    fn complete<'a>(
        &'a self,
        messages: &'a [AriaProviderMessage],
    ) -> Pin<Box<dyn Future<Output = Result<AriaProviderCompletion>> + Send + 'a>>;
}

impl AriaChatProvider for AriaModelProvider {
    fn provider_name(&self) -> &str {
        AriaModelProvider::provider_name(self)
    }

    fn model(&self) -> &str {
        AriaModelProvider::model(self)
    }

    fn complete<'a>(
        &'a self,
        messages: &'a [AriaProviderMessage],
    ) -> Pin<Box<dyn Future<Output = Result<AriaProviderCompletion>> + Send + 'a>> {
        Box::pin(AriaModelProvider::complete(self, messages))
    }
}

impl AriaModelProvider {
    /// Load a provider only when both endpoint and model are configured.
    /// A partial configuration is an error rather than a silent fallback.
    pub fn from_environment() -> Result<Option<Self>> {
        let endpoint = std::env::var("JANUS_ARIA_PROVIDER_CHAT_COMPLETIONS_URL").ok();
        let model = std::env::var("JANUS_ARIA_PROVIDER_MODEL").ok();
        match (endpoint, model) {
            (None, None) => Ok(None),
            (Some(endpoint), Some(model)) => {
                let provider_name = std::env::var("JANUS_ARIA_PROVIDER_NAME")
                    .unwrap_or_else(|_| "openai-compatible".to_string());
                let api_key = match std::env::var_os("JANUS_ARIA_PROVIDER_API_KEY_FILE") {
                    Some(path) => Some(read_private_api_key(Path::new(&path))?),
                    None => None,
                };
                Self::new(endpoint, provider_name, model, api_key).map(Some)
            }
            _ => bail!(
                "JANUS_ARIA_PROVIDER_CHAT_COMPLETIONS_URL and JANUS_ARIA_PROVIDER_MODEL must be configured together"
            ),
        }
    }

    fn new(
        endpoint: impl AsRef<str>,
        provider_name: String,
        model: String,
        api_key: Option<String>,
    ) -> Result<Self> {
        ensure!(!provider_name.trim().is_empty() && provider_name.len() <= 64,
            "Aria provider name must contain 1..=64 bytes");
        ensure!(!model.trim().is_empty() && model.len() <= 128,
            "Aria provider model must contain 1..=128 bytes");
        let endpoint = Url::parse(endpoint.as_ref()).context("parsing Aria provider endpoint")?;
        validate_endpoint(&endpoint)?;
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .redirect(Policy::none())
            .build()
            .context("building Aria provider HTTP client")?;
        Ok(Self { endpoint, provider_name, model, api_key, client })
    }

    pub fn provider_name(&self) -> &str {
        &self.provider_name
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub async fn complete(
        &self,
        messages: &[AriaProviderMessage],
    ) -> Result<AriaProviderCompletion> {
        ensure!(!messages.is_empty() && messages.len() <= 32,
            "Aria provider request must contain 1..=32 messages");
        let request = ChatCompletionRequest {
            model: &self.model,
            messages,
            temperature: 0.7,
            max_tokens: 2048,
        };
        let body = serde_json::to_vec(&request).context("encoding Aria provider request")?;
        let mut builder = self.client
            .post(self.endpoint.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body);
        if let Some(api_key) = &self.api_key {
            builder = builder.bearer_auth(api_key);
        }

        let mut response = builder.send().await.context("sending Aria provider request")?;
        let status = response.status();
        ensure!(status.is_success(), "Aria provider returned HTTP {status}");
        if response.content_length().is_some_and(|length| length > MAX_PROVIDER_RESPONSE_BYTES as u64) {
            bail!("Aria provider response exceeds the 2 MiB limit");
        }

        let mut response_bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.context("reading Aria provider response")? {
            ensure!(
                response_bytes.len().saturating_add(chunk.len()) <= MAX_PROVIDER_RESPONSE_BYTES,
                "Aria provider response exceeds the 2 MiB limit"
            );
            response_bytes.extend_from_slice(&chunk);
        }
        let decoded: ChatCompletionResponse = serde_json::from_slice(&response_bytes)
            .context("decoding OpenAI-compatible Aria provider response")?;
        let content = decoded.choices.into_iter().next()
            .and_then(|choice| choice.message.content)
            .unwrap_or_default();
        let content = content.trim().to_string();
        ensure!(!content.is_empty(), "Aria provider returned an empty assistant message");
        ensure!(content.len() <= MAX_ASSISTANT_MESSAGE_BYTES,
            "Aria assistant response exceeds the 64 KiB storage limit");

        let model = decoded.model
            .filter(|model| !model.trim().is_empty() && model.len() <= 128)
            .unwrap_or_else(|| self.model.clone());
        Ok(AriaProviderCompletion {
            content,
            provider: self.provider_name.clone(),
            model,
        })
    }
}


/// Native Anthropic Messages API adapter. Credentials remain server-side.
#[derive(Clone)]
pub struct AriaAnthropicProvider {
    endpoint: Url,
    model: String,
    api_key: String,
    client: Client,
}

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u16,
    messages: Vec<AnthropicInputMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
}

#[derive(Serialize)]
struct AnthropicInputMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    model: Option<String>,
    content: Vec<AnthropicContentBlock>,
}

#[derive(Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    kind: String,
    text: Option<String>,
}

impl AriaAnthropicProvider {
    pub fn from_environment() -> Result<Option<Self>> {
        let model = std::env::var("JANUS_ARIA_ANTHROPIC_MODEL").ok();
        let key_path = std::env::var_os("JANUS_ARIA_ANTHROPIC_API_KEY_FILE");
        match (model, key_path) {
            (None, None) => Ok(None),
            (Some(model), Some(path)) => {
                let endpoint = std::env::var("JANUS_ARIA_ANTHROPIC_MESSAGES_URL")
                    .unwrap_or_else(|_| "https://api.anthropic.com/v1/messages".to_string());
                let api_key = read_private_api_key(Path::new(&path))?;
                Self::new(endpoint, model, api_key).map(Some)
            }
            _ => bail!("JANUS_ARIA_ANTHROPIC_MODEL and JANUS_ARIA_ANTHROPIC_API_KEY_FILE must be configured together"),
        }
    }

    fn new(endpoint: impl AsRef<str>, model: String, api_key: String) -> Result<Self> {
        ensure!(!model.trim().is_empty() && model.len() <= 128,
            "Anthropic model must contain 1..=128 bytes");
        ensure!(!api_key.trim().is_empty() && api_key.len() <= 4096,
            "Anthropic API key must contain 1..=4096 bytes");
        let endpoint = Url::parse(endpoint.as_ref()).context("parsing Anthropic endpoint")?;
        validate_endpoint(&endpoint)?;
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .redirect(Policy::none())
            .build()
            .context("building Anthropic HTTP client")?;
        Ok(Self { endpoint, model, api_key, client })
    }

    async fn complete(&self, messages: &[AriaProviderMessage]) -> Result<AriaProviderCompletion> {
        ensure!(!messages.is_empty() && messages.len() <= 32,
            "Aria provider request must contain 1..=32 messages");
        let mut system_parts = Vec::new();
        let mut input = Vec::new();
        for message in messages {
            match message.role.as_str() {
                "system" => system_parts.push(message.content.as_str()),
                "user" | "assistant" => input.push(AnthropicInputMessage {
                    role: &message.role,
                    content: &message.content,
                }),
                _ => bail!("Anthropic chat supports only system, user, and assistant roles"),
            }
        }
        ensure!(!input.is_empty(), "Anthropic request must contain a user or assistant message");
        let request = AnthropicRequest {
            model: &self.model,
            max_tokens: 2048,
            messages: input,
            system: if system_parts.is_empty() { None } else {
                Some(system_parts.join("\n\n"))
            },
        };
        let body = serde_json::to_vec(&request).context("encoding Anthropic request")?;
        let mut response = self.client.post(self.endpoint.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .body(body)
            .send().await.context("sending Anthropic request")?;
        let status = response.status();
        ensure!(status.is_success(), "Anthropic provider returned HTTP {status}");
        if response.content_length().is_some_and(|length| length > MAX_PROVIDER_RESPONSE_BYTES as u64) {
            bail!("Anthropic response exceeds the 2 MiB limit");
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.context("reading Anthropic response")? {
            ensure!(bytes.len().saturating_add(chunk.len()) <= MAX_PROVIDER_RESPONSE_BYTES,
                "Anthropic response exceeds the 2 MiB limit");
            bytes.extend_from_slice(&chunk);
        }
        let decoded: AnthropicResponse = serde_json::from_slice(&bytes)
            .context("decoding Anthropic response")?;
        let content = decoded.content.into_iter()
            .filter(|block| block.kind == "text")
            .filter_map(|block| block.text)
            .collect::<Vec<_>>()
            .join("\n")
            .trim()
            .to_string();
        ensure!(!content.is_empty(), "Anthropic returned no text content");
        ensure!(content.len() <= MAX_ASSISTANT_MESSAGE_BYTES,
            "Aria assistant response exceeds the 64 KiB storage limit");
        let model = decoded.model
            .filter(|value| !value.trim().is_empty() && value.len() <= 128)
            .unwrap_or_else(|| self.model.clone());
        Ok(AriaProviderCompletion {
            content,
            provider: "anthropic".into(),
            model,
        })
    }
}

impl AriaChatProvider for AriaAnthropicProvider {
    fn provider_name(&self) -> &str { "anthropic" }
    fn model(&self) -> &str { &self.model }
    fn complete<'a>(&'a self, messages: &'a [AriaProviderMessage])
        -> Pin<Box<dyn Future<Output = Result<AriaProviderCompletion>> + Send + 'a>> {
        Box::pin(AriaAnthropicProvider::complete(self, messages))
    }
}

/// Select a concrete protocol without changing the runtime chat contract.
pub fn provider_from_environment() -> Result<Option<Arc<dyn AriaChatProvider>>> {
    match std::env::var("JANUS_ARIA_PROVIDER_KIND")
        .unwrap_or_else(|_| "openai-compatible".into()).as_str() {
        "openai-compatible" => Ok(AriaModelProvider::from_environment()?
            .map(|provider| Arc::new(provider) as Arc<dyn AriaChatProvider>)),
        "anthropic" => Ok(AriaAnthropicProvider::from_environment()?
            .map(|provider| Arc::new(provider) as Arc<dyn AriaChatProvider>)),
        other => bail!("unsupported JANUS_ARIA_PROVIDER_KIND value: {other}"),
    }
}

fn validate_endpoint(endpoint: &Url) -> Result<()> {
    ensure!(endpoint.username().is_empty() && endpoint.password().is_none(),
        "Aria provider endpoint must not embed credentials");
    ensure!(endpoint.query().is_none() && endpoint.fragment().is_none(),
        "Aria provider endpoint must not contain a query or fragment");
    match endpoint.scheme() {
        "https" => Ok(()),
        "http" => {
            let host = endpoint.host_str().unwrap_or_default();
            ensure!(
                matches!(host, "localhost" | "127.0.0.1" | "::1"),
                "unencrypted HTTP is allowed only for a loopback Aria provider"
            );
            Ok(())
        }
        _ => bail!("Aria provider endpoint must use HTTPS or loopback HTTP"),
    }
}

fn read_private_api_key(path: &Path) -> Result<String> {
    let metadata = std::fs::symlink_metadata(path)
        .with_context(|| format!("reading Aria provider key metadata at {}", path.display()))?;
    ensure!(metadata.file_type().is_file(),
        "Aria provider key path must be a regular, non-symlink file");
    ensure!(metadata.len() > 0 && metadata.len() <= 4096,
        "Aria provider key file must contain 1..=4096 bytes");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(metadata.permissions().mode() & 0o077 == 0,
            "Aria provider key file permissions must be owner-only (0600)");
    }
    let key = std::fs::read_to_string(path)
        .with_context(|| format!("reading Aria provider key file at {}", path.display()))?;
    let key = key.trim();
    ensure!(!key.is_empty() && !key.chars().any(char::is_whitespace),
        "Aria provider key file must contain one non-empty token");
    Ok(key.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};
    use serde_json::{json, Value};

    #[test]
    fn rejects_remote_plain_http_and_embedded_credentials() {
        assert!(AriaModelProvider::new(
            "http://example.com/v1/chat/completions", "test".into(), "model".into(), None
        ).is_err());
        assert!(AriaModelProvider::new(
            "https://user:secret@example.com/v1/chat/completions", "test".into(), "model".into(), None
        ).is_err());
    }

    #[test]
    fn allows_https_and_loopback_http_endpoints() {
        assert!(AriaModelProvider::new(
            "https://api.example.com/v1/chat/completions", "test".into(), "model".into(), None
        ).is_ok());
        assert!(AriaModelProvider::new(
            "http://127.0.0.1:11434/v1/chat/completions", "local".into(), "model".into(), None
        ).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn provider_api_key_file_must_be_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(
            "janus-aria-provider-key-{}-{}.txt",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::write(&path, "test-secret-token\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_private_api_key(&path).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(read_private_api_key(&path).unwrap(), "test-secret-token");
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn provider_client_does_not_follow_redirects() {
        async fn redirect() -> axum::http::StatusCode {
            axum::http::StatusCode::TEMPORARY_REDIRECT
        }
        async fn target() -> Json<Value> {
            Json(json!({
                "model": "redirected",
                "choices": [{"message": {"content": "should not be reached"}}]
            }))
        }
        let app = Router::new()
            .route("/v1/chat/completions", post(redirect))
            .route("/target", post(target));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        let provider = AriaModelProvider::new(
            format!("http://{address}/v1/chat/completions"),
            "local-test".into(),
            "test-model".into(),
            None,
        ).unwrap();
        assert!(provider.complete(&[
            AriaProviderMessage { role: "user".into(), content: "Hello".into() },
        ]).await.is_err());
    }

    #[tokio::test]
    async fn sends_openai_compatible_messages_and_parses_assistant_reply() {
        async fn complete(Json(payload): Json<Value>) -> Json<Value> {
            assert_eq!(payload["model"], "test-model");
            assert_eq!(payload["messages"][0]["role"], "system");
            assert_eq!(payload["messages"][1]["content"], "Hello");
            Json(json!({
                "model": "test-model-returned",
                "choices": [{"message": {"content": "Hello from Aria"}}]
            }))
        }

        let app = Router::new().route("/v1/chat/completions", post(complete));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let provider: Box<dyn AriaChatProvider> = Box::new(AriaModelProvider::new(
            format!("http://{address}/v1/chat/completions"),
            "local-test".into(),
            "test-model".into(),
            None,
        ).unwrap());
        let completion = provider.complete(&[
            AriaProviderMessage { role: "system".into(), content: "You are Aria.".into() },
            AriaProviderMessage { role: "user".into(), content: "Hello".into() },
        ]).await.unwrap();

        assert_eq!(completion.content, "Hello from Aria");
        assert_eq!(completion.provider, "local-test");
        assert_eq!(completion.model, "test-model-returned");
    }
}
