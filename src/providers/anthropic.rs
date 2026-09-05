use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::Config;
use crate::providers::{FunctionCall, Message, Response, ToolCall};

pub const PRESET_MODEL: &str = "claude-sonnet-4-5";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 4096;

/// Anthropic's Messages API (SSE streaming, `tool_use`/`tool_result` blocks).
pub struct Provider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}

impl Provider {
    pub fn new(config: &Config) -> Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(15))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            base_url: config.base_url.trim_end_matches('/').into(),
            api_key: config.api_key.clone(),
            model: config.model.clone(),
        })
    }

    fn endpoint(&self, stream: bool) -> reqwest::RequestBuilder {
        self.client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .query(&[("stream", stream)])
    }

    fn tools(&self) -> Vec<Value> {
        super::tool_defs()
            .into_iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.parameters,
                })
            })
            .collect()
    }

    /// Translate OpenAI-shaped exchange messages into Anthropic "messages"
    /// (system prompt extracted, tool results grouped as user `tool_result`
    /// blocks, assistant `tool_calls` expressed as `tool_use` blocks).
    fn wire_messages(&self, messages: &[Message]) -> (String, Vec<Value>) {
        let system = messages
            .iter()
            .filter(|message| message.role == "system")
            .filter_map(|message| message.content.clone())
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut wire: Vec<Value> = Vec::new();
        for message in messages.iter().filter(|message| message.role != "system") {
            if message.role == "tool" {
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": message.tool_call_id.clone().unwrap_or_default(),
                    "content": message.content.clone().unwrap_or_default(),
                });
                let appended_to_tool_group = wire.last_mut().map(|last| {
                    let has_tool_result = last
                        .get("content")
                        .and_then(|content| content.as_array())
                        .map(|blocks| {
                            blocks.iter().any(|block| {
                                block.get("type") == Some(&Value::String("tool_result".into()))
                            })
                        })
                        .unwrap_or(false);
                    if has_tool_result {
                        last.get_mut("content")
                            .and_then(|content| content.as_array_mut())
                            .unwrap()
                            .push(block.clone());
                        true
                    } else {
                        false
                    }
                });
                if appended_to_tool_group != Some(true) {
                    wire.push(json!({ "role": "user", "content": [block] }));
                }
                continue;
            }
            let mut blocks: Vec<Value> = Vec::new();
            if let Some(content) = &message.content {
                if !content.trim().is_empty() {
                    blocks.push(json!({ "type": "text", "text": content }));
                }
            }
            if let Some(calls) = &message.tool_calls {
                for call in calls {
                    let input = serde_json::from_str::<Value>(&call.function.arguments)
                        .unwrap_or_else(|_| json!({ "raw": call.function.arguments }));
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.function.name,
                        "input": input,
                    }));
                }
            }
            let role = if message.role == "assistant" {
                "assistant"
            } else {
                "user"
            };
            wire.push(json!({ "role": role, "content": blocks }));
        }
        (system, wire)
    }

    async fn stream_response(
        &self,
        body: reqwest::Response,
        on_text: &mut (dyn FnMut(String) + Send),
    ) -> Result<Response> {
        let status = body.status();
        if !status.is_success() {
            let error_body = body.text().await.unwrap_or_default();
            let hint = super::error_hint(status.as_u16());
            anyhow::bail!(
                "anthropic returned {status} ({hint}): {}",
                error_body.truncate_body()
            );
        }
        let mut stream = body.bytes_stream();
        let mut buffer = String::new();
        let mut content = String::new();
        let mut tool_calls: Vec<PartialToolCall> = Vec::new();
        while let Some(chunk) = stream.next().await {
            buffer.push_str(&String::from_utf8_lossy(&chunk?));
            while let Some(position) = buffer.find("\n\n") {
                let event = buffer[..position].to_owned();
                buffer.drain(..position + 2);
                let Some(data) = event.lines().find_map(|line| line.strip_prefix("data: ")) else {
                    continue;
                };
                let event: AnthropicEvent = match serde_json::from_str(data) {
                    Ok(event) => event,
                    Err(_) => continue,
                };
                match event.event_type.as_str() {
                    "content_block_start" => {
                        if let Some(block) = event.content_block {
                            if block.block_type == "tool_use" {
                                let index = event.index.unwrap_or(0) as usize;
                                while tool_calls.len() <= index {
                                    tool_calls.push(PartialToolCall::default());
                                }
                                let current = &mut tool_calls[index];
                                current.id = block.id.unwrap_or_default();
                                current.name = block.name.unwrap_or_default();
                            }
                        }
                    }
                    "content_block_delta" => match event.delta.block_type.as_str() {
                        "text_delta" => {
                            if let Some(text) = event.delta.text {
                                on_text(text.clone());
                                content.push_str(&text);
                            }
                        }
                        "input_json_delta" => {
                            if let Some(partial) = event.delta.partial_json {
                                let index = event.index.unwrap_or(0) as usize;
                                while tool_calls.len() <= index {
                                    tool_calls.push(PartialToolCall::default());
                                }
                                tool_calls[index].arguments.push_str(&partial);
                            }
                        }
                        _ => {}
                    },
                    "error" => {
                        let message = event
                            .error
                            .and_then(|error| error.message)
                            .unwrap_or_else(|| "unknown anthropic error".into());
                        anyhow::bail!("anthropic stream error: {message}");
                    }
                    _ => {}
                }
            }
        }
        let tool_calls: Vec<ToolCall> = tool_calls
            .into_iter()
            .filter(|call| !call.name.is_empty())
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
}

trait TruncateBody {
    fn truncate_body(self) -> String;
}
impl TruncateBody for String {
    fn truncate_body(mut self) -> String {
        self.truncate(2_000);
        self
    }
}

#[async_trait]
impl super::Provider for Provider {
    async fn respond(
        &self,
        messages: &[Message],
        on_text: &mut (dyn FnMut(String) + Send),
    ) -> Result<Response> {
        let (system, wire) = self.wire_messages(messages);
        let response = self
            .endpoint(true)
            .json(&json!({
                "model": self.model,
                "max_tokens": MAX_TOKENS,
                "system": system,
                "messages": wire,
                "tools": self.tools(),
                "tool_choice": { "type": "auto" },
                "stream": true,
            }))
            .send()
            .await
            .map_err(|error| super::provider_transport_error(&self.base_url, error))?;
        self.stream_response(response, on_text).await
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
        let response = self
            .endpoint(false)
            .json(&json!({
                "model": self.model,
                "max_tokens": 1024,
                "system": "Summarize this terminal-assistant conversation for future context. Preserve user goals, decisions, file paths, commands, command outcomes, errors, and unfinished work. Be concise and factual. Do not suggest new actions.",
                "messages": [{
                    "role": "user",
                    "content": [{ "type": "text", "text": transcript }],
                }],
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
        let parsed: AnthropicNonStream =
            serde_json::from_str(&body).context("invalid summary response")?;
        let text = parsed
            .content
            .into_iter()
            .filter_map(|block| block.text)
            .collect::<Vec<_>>()
            .join("");
        if text.trim().is_empty() {
            anyhow::bail!("provider returned an empty summary");
        }
        Ok(text)
    }

    async fn models(&self) -> Result<Vec<String>> {
        let response = self
            .client
            .get(format!("{}/v1/models", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|error| super::provider_transport_error(&self.base_url, error))?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("anthropic returned {status}");
        }
        let body = response.text().await?;
        let value: Value = serde_json::from_str(&body).context("invalid models response")?;
        Ok(value
            .get("data")
            .and_then(|data| data.as_array())
            .into_iter()
            .flatten()
            .filter_map(|model| model.get("id").and_then(|id| id.as_str()).map(String::from))
            .collect())
    }
}

#[derive(Deserialize)]
struct AnthropicEvent {
    #[serde(rename = "type")]
    event_type: String,
    index: Option<u32>,
    content_block: Option<EventContentBlock>,
    delta: EventDelta,
    error: Option<EventError>,
}
#[derive(Deserialize)]
struct EventContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    id: Option<String>,
    name: Option<String>,
}
#[derive(Deserialize)]
struct EventDelta {
    #[serde(rename = "type", default)]
    block_type: String,
    text: Option<String>,
    partial_json: Option<String>,
}
#[derive(Deserialize)]
struct EventError {
    message: Option<String>,
}
#[derive(Deserialize)]
struct AnthropicNonStream {
    content: Vec<NonStreamBlock>,
}
#[derive(Deserialize)]
struct NonStreamBlock {
    text: Option<String>,
}
#[derive(Default)]
struct PartialToolCall {
    id: String,
    name: String,
    arguments: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::ToolCall;

    fn provider() -> Provider {
        Provider::new(&crate::config::Config {
            provider: "anthropic".into(),
            api_key: "k".into(),
            base_url: "https://api.anthropic.com".into(),
            model: "claude-sonnet-4-5".into(),
            approval_mode: "always".into(),
        })
        .unwrap()
    }

    fn tool_call(id: &str, name: &str, arguments: &str) -> ToolCall {
        ToolCall {
            kind: "function".into(),
            id: id.into(),
            function: FunctionCall {
                name: name.into(),
                arguments: arguments.into(),
            },
        }
    }

    #[test]
    fn translates_tool_call_and_result_into_tool_use_and_tool_result() {
        let messages = vec![
            Message {
                role: "assistant".into(),
                content: Some("checking".into()),
                name: None,
                tool_call_id: None,
                tool_calls: Some(vec![tool_call(
                    "toolu_1",
                    "run_command",
                    "{\"command\":\"ls\"}",
                )]),
            },
            Message {
                role: "tool".into(),
                content: Some("ok".into()),
                name: Some("run_command".into()),
                tool_call_id: Some("toolu_1".into()),
                tool_calls: None,
            },
        ];
        let (system, wire) = provider().wire_messages(&messages);
        assert_eq!(system, "");
        assert_eq!(wire.len(), 2);
        assert_eq!(wire[0]["role"].as_str(), Some("assistant"));
        assert_eq!(wire[0]["content"][0]["type"].as_str(), Some("text"));
        assert_eq!(wire[0]["content"][1]["type"].as_str(), Some("tool_use"));
        assert_eq!(wire[0]["content"][1]["id"].as_str(), Some("toolu_1"));
        assert_eq!(
            wire[0]["content"][1]["input"]["command"].as_str(),
            Some("ls")
        );
        assert_eq!(wire[1]["role"].as_str(), Some("user"));
        assert_eq!(wire[1]["content"][0]["type"].as_str(), Some("tool_result"));
        assert_eq!(
            wire[1]["content"][0]["tool_use_id"].as_str(),
            Some("toolu_1")
        );
    }

    #[test]
    fn groups_consecutive_tool_results_into_one_user_message() {
        let messages = vec![
            Message {
                role: "assistant".into(),
                content: None,
                name: None,
                tool_call_id: None,
                tool_calls: Some(vec![
                    tool_call("toolu_1", "run_command", "{\"command\":\"a\"}"),
                    tool_call("toolu_2", "run_command", "{\"command\":\"b\"}"),
                ]),
            },
            Message {
                role: "tool".into(),
                content: Some("first".into()),
                name: Some("run_command".into()),
                tool_call_id: Some("toolu_1".into()),
                tool_calls: None,
            },
            Message {
                role: "tool".into(),
                content: Some("second".into()),
                name: Some("run_command".into()),
                tool_call_id: Some("toolu_2".into()),
                tool_calls: None,
            },
        ];
        let (_, wire) = provider().wire_messages(&messages);
        let last = wire.last().unwrap();
        assert_eq!(last["role"].as_str(), Some("user"));
        assert_eq!(last["content"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn extracts_system_prompt_separately() {
        let messages = vec![
            Message {
                role: "system".into(),
                content: Some("you are hi".into()),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
            Message {
                role: "user".into(),
                content: Some("hello".into()),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
        ];
        let (system, wire) = provider().wire_messages(&messages);
        assert_eq!(system, "you are hi");
        assert_eq!(wire.len(), 1);
        assert_eq!(wire[0]["content"][0]["text"].as_str(), Some("hello"));
    }
}
