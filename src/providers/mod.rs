pub mod anthropic;
pub mod gemini;
pub mod ollama;
pub mod openai;
pub mod openrouter;

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: Option<String>,
    pub name: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(rename = "type", default = "default_tool_type")]
    pub kind: String,
    pub id: String,
    pub function: FunctionCall,
}

fn default_tool_type() -> String {
    "function".into()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
}
#[derive(Debug, Clone)]
pub struct Response {
    pub message: Message,
}

/// Every provider exposes the same contract to the agent. The agent knows
/// nothing about concrete providers or their models; it only knows there is a
/// configured provider and asks it to respond.
#[async_trait]
pub trait Provider: Send + Sync {
    async fn respond(
        &self,
        messages: &[Message],
        on_text: &mut (dyn FnMut(String) + Send),
    ) -> Result<Response>;
    async fn summarize(&self, messages: &[Message]) -> Result<String>;
    /// List the models this provider offers, for `hi models` / `hi doctor`.
    async fn models(&self) -> Result<Vec<String>> {
        anyhow::bail!("this provider does not expose a model catalog")
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// The tool schema the agent exposes, extracted from `tools::definitions`.
/// Each provider translates these into its native wire format.
pub fn tool_defs() -> Vec<ToolDef> {
    crate::tools::definitions()
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let function = entry.get("function")?;
            Some(ToolDef {
                name: function.get("name")?.as_str()?.to_string(),
                description: function
                    .get("description")
                    .and_then(|description| description.as_str())
                    .unwrap_or("")
                    .to_string(),
                parameters: function
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({})),
            })
        })
        .collect()
}

const REGISTRY: &[(&str, &str, &str, bool)] = &[
    (
        "openai",
        "https://api.openai.com/v1",
        openai::PRESET_MODEL,
        true,
    ),
    (
        "anthropic",
        "https://api.anthropic.com",
        anthropic::PRESET_MODEL,
        true,
    ),
    (
        "gemini",
        "https://generativelanguage.googleapis.com",
        gemini::PRESET_MODEL,
        true,
    ),
    (
        "openrouter",
        "https://openrouter.ai/api/v1",
        openrouter::PRESET_MODEL,
        true,
    ),
    (
        "ollama",
        "http://localhost:11434",
        ollama::PRESET_MODEL,
        false,
    ),
];

/// Build the provider selected in the config. This is the only place that
/// knows which concrete providers exist; callers just get a `Box<dyn Provider>`.
pub fn create(config: &crate::config::Config) -> Result<Box<dyn Provider>> {
    match config.provider.as_str() {
        "openai" => Ok(Box::new(openai::Provider::new(config)?)),
        "openrouter" => Ok(Box::new(openrouter::Provider::new(config)?)),
        "anthropic" => Ok(Box::new(anthropic::Provider::new(config)?)),
        "gemini" => Ok(Box::new(gemini::Provider::new(config)?)),
        "ollama" => Ok(Box::new(ollama::Provider::new(config)?)),
        other => anyhow::bail!(
            "unknown provider `{other}`; available: {}",
            REGISTRY
                .iter()
                .map(|entry| entry.0)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Defaults (base URL, model, whether an API key is required) for a provider.
/// Config uses these to migrate old setups and to render setup prompts.
pub fn preset(provider: &str) -> Option<(&'static str, &'static str, bool)> {
    REGISTRY
        .iter()
        .find(|entry| entry.0 == provider)
        .map(|entry| (entry.1, entry.2, entry.3))
}

pub fn provider_names() -> Vec<&'static str> {
    REGISTRY.iter().map(|entry| entry.0).collect()
}

/// The OpenAI wire protocol, shared by providers that speak it (OpenAI,
/// OpenRouter, and any other OpenAI-compatible endpoint). Chat, summarization
/// and model listing all live here once.
pub struct OpenAiWire {
    pub client: reqwest::Client,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub extra_headers: Vec<(String, String)>,
}

impl OpenAiWire {
    pub fn new(config: &crate::config::Config) -> Self {
        Self {
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            base_url: config.base_url.trim_end_matches('/').into(),
            api_key: config.api_key.clone(),
            model: config.model.clone(),
            extra_headers: Vec::new(),
        }
    }

    pub fn with_headers(mut self, headers: &[(&str, &str)]) -> Self {
        self.extra_headers = headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        self
    }

    pub async fn chat(
        &self,
        messages: &[Message],
        on_text: &mut (dyn FnMut(String) + Send),
    ) -> Result<Response> {
        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .header_static_headers(&self.extra_headers)
            .json(&ChatRequest {
                model: &self.model,
                messages,
                tools: crate::tools::definitions(),
                stream: true,
            })
            .send()
            .await
            .map_err(|error| provider_transport_error(&self.base_url, error))?;
        let status = response.status();
        if !status.is_success() {
            let mut body = response.text().await.unwrap_or_default();
            body.truncate(2_000);
            let hint = error_hint(status.as_u16());
            anyhow::bail!("provider returned {status} ({hint}): {body}");
        }
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut content = String::new();
        let mut tool_calls: Vec<PartialToolCall> = Vec::new();
        while let Some(chunk) = stream.next().await {
            buffer.push_str(&String::from_utf8_lossy(&chunk?));
            while let Some(position) = buffer.find("\n\n") {
                let event = buffer[..position].to_owned();
                buffer.drain(..position + 2);
                if let Some(data) = event.lines().find_map(|line| line.strip_prefix("data: ")) {
                    if data.trim() == "[DONE]" {
                        continue;
                    }
                    let event: StreamEvent =
                        serde_json::from_str(data).context("invalid streaming response")?;
                    let Some(delta) = event.choices.into_iter().next().map(|choice| choice.delta)
                    else {
                        continue;
                    };
                    if let Some(text) = delta.content {
                        on_text(text.clone());
                        content.push_str(&text);
                    }
                    for call in delta.tool_calls.unwrap_or_default() {
                        let index = call.index as usize;
                        while tool_calls.len() <= index {
                            tool_calls.push(PartialToolCall::default());
                        }
                        let current = &mut tool_calls[index];
                        if let Some(id) = call.id {
                            current.id = id;
                        }
                        if let Some(name) = call.function.name {
                            current.name = name;
                        }
                        if let Some(arguments) = call.function.arguments {
                            current.arguments.push_str(&arguments);
                        }
                    }
                }
            }
        }
        let tool_calls: Vec<ToolCall> = tool_calls
            .into_iter()
            .filter(|call| !call.id.is_empty() || !call.name.is_empty())
            .map(|call| ToolCall {
                kind: "function".into(),
                id: call.id,
                function: FunctionCall {
                    name: call.name,
                    arguments: call.arguments,
                },
            })
            .collect();
        Ok(Response {
            message: Message {
                role: "assistant".into(),
                content: Some(content).filter(|text| !text.is_empty()),
                name: None,
                tool_call_id: None,
                tool_calls: if tool_calls.is_empty() {
                    None
                } else {
                    Some(tool_calls)
                },
            },
        })
    }

    pub async fn summarize(&self, messages: &[Message]) -> Result<String> {
        let transcript = messages
            .iter()
            .filter_map(|message| {
                message
                    .content
                    .as_ref()
                    .map(|content| format!("{}: {content}", message.role))
            })
            .collect::<Vec<_>>()
            .join("\n");
        let summary_messages = [
            Message {
                role: "system".into(),
                content: Some(
                    "Summarize this terminal-assistant conversation for future context. Preserve user goals, decisions, file paths, commands, command outcomes, errors, and unfinished work. Be concise and factual. Do not suggest new actions.".into(),
                ),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
            Message {
                role: "user".into(),
                content: Some(transcript),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
        ];
        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .header_static_headers(&self.extra_headers)
            .json(&SummaryRequest {
                model: &self.model,
                messages: &summary_messages,
                stream: false,
            })
            .send()
            .await
            .map_err(|error| provider_transport_error(&self.base_url, error))?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            let hint = error_hint(status.as_u16());
            anyhow::bail!("summary request returned {status} ({hint})");
        }
        let parsed: BasicChatResponse =
            serde_json::from_str(&body).context("invalid summary response")?;
        parsed
            .choices
            .into_iter()
            .next()
            .and_then(|choice| choice.message.content)
            .context("provider returned an empty summary")
    }

    pub async fn models(&self) -> Result<Vec<String>> {
        let response = self
            .client
            .get(format!("{}/models", self.base_url))
            .bearer_auth(&self.api_key)
            .header_static_headers(&self.extra_headers)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|error| provider_transport_error(&self.base_url, error))?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("provider returned {status}");
        }
        let body = response.text().await?;
        let value: serde_json::Value =
            serde_json::from_str(&body).context("invalid models response")?;
        Ok(value
            .get("data")
            .and_then(|data| data.as_array())
            .into_iter()
            .flatten()
            .filter_map(|model| model.get("id").and_then(|id| id.as_str()).map(String::from))
            .collect())
    }
}

/// Apply a list of static headers to a request without touching the auth split.
trait HeaderStaticHeaders {
    fn header_static_headers(self, headers: &[(String, String)]) -> reqwest::RequestBuilder;
}

impl HeaderStaticHeaders for reqwest::RequestBuilder {
    fn header_static_headers(self, headers: &[(String, String)]) -> reqwest::RequestBuilder {
        headers
            .iter()
            .fold(self, |request, (name, value)| request.header(name, value))
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    tools: serde_json::Value,
    stream: bool,
}

#[derive(Serialize)]
struct SummaryRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    stream: bool,
}
#[derive(Deserialize)]
struct BasicChatResponse {
    choices: Vec<BasicChoice>,
}
#[derive(Deserialize)]
struct BasicChoice {
    message: Message,
}

pub(crate) fn error_hint(status: u16) -> &'static str {
    match status {
        401 | 403 => "check your API key",
        429 => "rate limit reached; try again shortly",
        500..=599 => "provider is unavailable",
        _ => "check the provider URL and request",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TransportKind {
    Connect,
    Timeout,
    Other,
}

impl TransportKind {
    fn classify(error: &reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::Timeout
        } else if error.is_connect() {
            Self::Connect
        } else {
            Self::Other
        }
    }
}

/// Turn a raw network failure into a message the user can act on. For local
/// endpoints the most likely cause is that the model server is simply not
/// running.
pub(crate) fn transport_hint(base_url: &str, kind: TransportKind) -> String {
    let action = match kind {
        TransportKind::Connect => format!("could not connect to the provider at {base_url}"),
        TransportKind::Timeout => format!("the provider at {base_url} did not respond in time"),
        TransportKind::Other => format!("failed to communicate with the provider at {base_url}"),
    };
    let local = ["localhost", "127.0.0.1", "0.0.0.0", "::1"]
        .iter()
        .any(|host| base_url.contains(host));
    if local {
        format!("{action}; make sure the local model server is running (e.g. `ollama serve`)")
    } else {
        format!("{action}; check your network connection and that the service is accessible")
    }
}

pub(crate) fn provider_transport_error(base_url: &str, error: reqwest::Error) -> anyhow::Error {
    let kind = TransportKind::classify(&error);
    anyhow::Error::new(error).context(transport_hint(base_url, kind))
}

#[derive(Deserialize)]
struct StreamEvent {
    choices: Vec<StreamChoice>,
}
#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}
#[derive(Deserialize)]
struct StreamDelta {
    content: Option<String>,
    tool_calls: Option<Vec<StreamToolCall>>,
}
#[derive(Deserialize)]
struct StreamToolCall {
    index: u32,
    id: Option<String>,
    function: StreamFunction,
}
#[derive(Deserialize)]
struct StreamFunction {
    name: Option<String>,
    arguments: Option<String>,
}
#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[cfg(test)]
mod tests {
    use super::{error_hint, StreamEvent, ToolCall};

    #[test]
    fn parses_streamed_text_delta() {
        let event: StreamEvent = serde_json::from_str(
            r#"{"choices":[{"delta":{"content":"hello","tool_calls":null}}]}"#,
        )
        .unwrap();
        assert_eq!(event.choices[0].delta.content.as_deref(), Some("hello"));
    }

    #[test]
    fn preserves_tool_call_type_when_stored() {
        let call: ToolCall = serde_json::from_str(r#"{"id":"call_1","type":"function","function":{"name":"run_command","arguments":"{}"}}"#).unwrap();
        let json = serde_json::to_string(&call).unwrap();
        assert!(json.contains("\"type\":\"function\""));
    }

    #[test]
    fn classifies_provider_errors() {
        assert_eq!(error_hint(401), "check your API key");
        assert_eq!(error_hint(429), "rate limit reached; try again shortly");
        assert_eq!(error_hint(503), "provider is unavailable");
    }

    #[test]
    fn suggests_local_server_startup_for_local_endpoints() {
        let connect =
            super::transport_hint("http://localhost:11434/v1", super::TransportKind::Connect);
        assert!(connect.contains("localhost:11434"));
        assert!(connect.contains("make sure"));
        assert!(connect.contains("ollama serve"));
        let remote =
            super::transport_hint("https://api.openai.com/v1", super::TransportKind::Timeout);
        assert!(!remote.contains("ollama"));
        assert!(remote.contains("did not respond in time"));
    }
}
