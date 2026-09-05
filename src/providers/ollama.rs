use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;

use crate::config::Config;
use crate::providers::{FunctionCall, Message, Response, ToolCall};

pub const PRESET_MODEL: &str = "llama3.2";

/// Ollama, a local runner. Uses the native `/api/chat` endpoint (NDJSON
/// streaming, no API key required). Config URLs that pointed at Ollama's
/// OpenAI-compatible `/v1` shim are normalized away.
pub struct Provider {
    client: reqwest::Client,
    base_url: String,
    model: String,
}

impl Provider {
    pub fn new(config: &Config) -> Result<Self> {
        let mut base_url = config.base_url.trim_end_matches('/').to_string();
        if let Some(rest) = base_url.strip_suffix("/v1") {
            base_url = rest.to_string();
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            base_url,
            model: config.model.clone(),
        })
    }

    fn chat_url(&self) -> String {
        format!("{}/api/chat", self.base_url)
    }

    /// Translate OpenAI-shaped exchange messages into Ollama's chat format.
    fn wire_messages(&self, messages: &[Message]) -> Vec<serde_json::Value> {
        messages
            .iter()
            .map(|message| {
                let tool_calls = message.tool_calls.as_ref().map(|calls| {
                    calls
                        .iter()
                        .map(|call| {
                            json!({
                                "function": {
                                    "name": call.function.name,
                                    "arguments": serde_json::from_str::<serde_json::Value>(&call.function.arguments)
                                        .unwrap_or_else(|_| json!({ "raw": call.function.arguments })),
                                }
                            })
                        })
                        .collect::<Vec<_>>()
                });
                json!({
                    "role": message.role,
                    "content": message.content,
                    "tool_calls": tool_calls,
                })
            })
            .collect()
    }
}

#[async_trait]
impl super::Provider for Provider {
    async fn respond(
        &self,
        messages: &[Message],
        on_text: &mut (dyn FnMut(String) + Send),
    ) -> Result<Response> {
        let response = self
            .client
            .post(self.chat_url())
            .json(&json!({
                "model": self.model,
                "messages": self.wire_messages(messages),
                "tools": crate::tools::definitions(),
                "stream": true,
            }))
            .send()
            .await
            .map_err(|error| super::provider_transport_error(&self.base_url, error))?;
        let status = response.status();
        if !status.is_success() {
            let mut body = response.text().await.unwrap_or_default();
            body.truncate(2_000);
            let hint = super::error_hint(status.as_u16());
            anyhow::bail!("ollama returned {status} ({hint}): {body}");
        }
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut content = String::new();
        let mut tool_calls: Vec<PartialToolCall> = Vec::new();
        while let Some(chunk) = stream.next().await {
            buffer.push_str(&String::from_utf8_lossy(&chunk?));
            while let Some(position) = buffer.find('\n') {
                let line = buffer[..position].trim().to_owned();
                buffer.drain(..position + 1);
                if line.is_empty() {
                    continue;
                }
                let event: OllamaChunk =
                    serde_json::from_str(&line).context("invalid ollama stream chunk")?;
                if let Some(text) = event.message.content {
                    on_text(text.clone());
                    content.push_str(&text);
                }
                for (index, call) in event
                    .message
                    .tool_calls
                    .unwrap_or_default()
                    .into_iter()
                    .enumerate()
                {
                    while tool_calls.len() <= index {
                        tool_calls.push(PartialToolCall::default());
                    }
                    let current = &mut tool_calls[index];
                    current.name = call.function.name;
                    current.arguments = call
                        .function
                        .arguments
                        .map(|arguments| arguments.to_string())
                        .unwrap_or_default();
                }
            }
        }
        let tool_calls: Vec<ToolCall> = tool_calls
            .into_iter()
            .filter(|call| !call.name.is_empty())
            .enumerate()
            .map(|(index, call)| ToolCall {
                kind: "function".into(),
                id: format!("call_{index}"),
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

    async fn summarize(&self, messages: &[Message]) -> Result<String> {
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
            .post(self.chat_url())
            .json(&json!({
                "model": self.model,
                "messages": self.wire_messages(&summary_messages),
                "stream": false,
            }))
            .send()
            .await
            .map_err(|error| super::provider_transport_error(&self.base_url, error))?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("summary request returned {status}");
        }
        let body = response.text().await?;
        let parsed: OllamaChunk =
            serde_json::from_str(&body).context("invalid summary response")?;
        parsed
            .message
            .content
            .context("provider returned an empty summary")
    }

    async fn models(&self) -> Result<Vec<String>> {
        let response = self
            .client
            .get(format!("{}/api/tags", self.base_url))
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|error| super::provider_transport_error(&self.base_url, error))?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("ollama returned {status}");
        }
        let body = response.text().await?;
        let value: serde_json::Value =
            serde_json::from_str(&body).context("invalid models response")?;
        Ok(value
            .get("models")
            .and_then(|models| models.as_array())
            .into_iter()
            .flatten()
            .filter_map(|model| {
                model
                    .get("name")
                    .and_then(|name| name.as_str())
                    .map(String::from)
            })
            .collect())
    }
}

#[derive(Deserialize)]
struct OllamaChunk {
    message: OllamaMessage,
}
#[derive(Deserialize)]
struct OllamaMessage {
    content: Option<String>,
    tool_calls: Option<Vec<OllamaToolCall>>,
}
#[derive(Deserialize)]
struct OllamaToolCall {
    function: OllamaFunction,
}
#[derive(Deserialize)]
struct OllamaFunction {
    name: String,
    arguments: Option<serde_json::Value>,
}
#[derive(Default)]
struct PartialToolCall {
    name: String,
    arguments: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ToolCall;

    fn provider(base_url: &str) -> Provider {
        Provider::new(&crate::config::Config {
            provider: "ollama".into(),
            api_key: String::new(),
            base_url: base_url.into(),
            model: "llama3.2".into(),
            approval_mode: "always".into(),
        })
        .unwrap()
    }

    #[test]
    fn normalizes_legacy_v1_base_url() {
        assert_eq!(
            provider("http://localhost:11434/v1").base_url,
            "http://localhost:11434"
        );
        assert_eq!(
            provider("http://localhost:11434/").base_url,
            "http://localhost:11434"
        );
    }

    #[test]
    fn wire_messages_use_native_tool_call_shape() {
        let messages = vec![Message {
            role: "assistant".into(),
            content: None,
            name: None,
            tool_call_id: None,
            tool_calls: Some(vec![ToolCall {
                kind: "function".into(),
                id: "call_0".into(),
                function: FunctionCall {
                    name: "run_command".into(),
                    arguments: "{\"command\":\"ls\"}".into(),
                },
            }]),
        }];
        let wire = provider("http://localhost:11434").wire_messages(&messages);
        assert_eq!(
            wire[0]["tool_calls"][0]["function"]["name"].as_str(),
            Some("run_command")
        );
        assert_eq!(
            wire[0]["tool_calls"][0]["function"]["arguments"]["command"].as_str(),
            Some("ls")
        );
    }
}
