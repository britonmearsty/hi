use anyhow::{Context, Result};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::config::Config;
use crate::providers::{FunctionCall, Message, Response, ToolCall};

pub const PRESET_MODEL: &str = "gemini-2.5-flash";

/// Google Gemini (`:streamGenerateContent` SSE with `functionCall` /
/// `functionResponse` parts; auth via `?key=` query parameter).
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

    fn model_url(&self, endpoint: &str) -> String {
        format!(
            "{}/v1beta/models/{}:{}?alt=sse&key={}",
            self.base_url, self.model, endpoint, self.api_key
        )
    }

    fn tools(&self) -> Vec<Value> {
        vec![
            json!({ "functionDeclarations": super::tool_defs().into_iter().map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters,
            })
        }).collect::<Vec<_>>() }),
        ]
    }

    /// Translate OpenAI-shaped exchange messages into Gemini "contents"
    /// (system prompt extracted, tool results expressed as `functionResponse`
    /// parts grouped per user turn, assistant `tool_calls` as `functionCall`
    /// parts). Gemini's parts carry text and function calls together.
    fn wire_messages(&self, messages: &[Message]) -> (Value, Vec<Value>) {
        let system = messages
            .iter()
            .filter(|message| message.role == "system")
            .filter_map(|message| message.content.clone())
            .collect::<Vec<_>>()
            .join("\n\n");
        let mut contents: Vec<Value> = Vec::new();
        for message in messages.iter().filter(|message| message.role != "system") {
            if message.role == "tool" {
                let response =
                    serde_json::from_str::<Value>(message.content.as_deref().unwrap_or_default())
                        .unwrap_or_else(|_| json!({ "content": message.content }));
                let part = json!({
                    "functionResponse": {
                        "name": message.name.clone().unwrap_or_default(),
                        "response": response,
                    }
                });
                let appended = contents.last_mut().map(|last| {
                    let has_function_response = last
                        .get("parts")
                        .and_then(|parts| parts.as_array())
                        .map(|parts| {
                            parts
                                .iter()
                                .any(|part| part.get("functionResponse").is_some())
                        })
                        .unwrap_or(false);
                    if has_function_response {
                        last.get_mut("parts")
                            .and_then(|parts| parts.as_array_mut())
                            .unwrap()
                            .push(part.clone());
                        true
                    } else {
                        false
                    }
                });
                if appended != Some(true) {
                    contents.push(json!({ "role": "user", "parts": [part] }));
                }
                continue;
            }
            let role = if message.role == "assistant" {
                "model"
            } else {
                "user"
            };
            let mut parts: Vec<Value> = Vec::new();
            if let Some(content) = &message.content {
                if !content.trim().is_empty() {
                    parts.push(json!({ "text": content }));
                }
            }
            if let Some(calls) = &message.tool_calls {
                for call in calls {
                    let args = serde_json::from_str::<Value>(&call.function.arguments)
                        .unwrap_or_else(|_| json!({}));
                    parts.push(json!({
                        "functionCall": {
                            "name": call.function.name,
                            "args": args,
                        }
                    }));
                }
            }
            contents.push(json!({ "role": role, "parts": parts }));
        }
        (json!({ "parts": [ { "text": system } ] }), contents)
    }

    async fn stream_response(
        &self,
        body: reqwest::Response,
        on_text: &mut (dyn FnMut(String) + Send),
    ) -> Result<Response> {
        let status = body.status();
        if !status.is_success() {
            let mut error_body = body.text().await.unwrap_or_default();
            error_body.truncate(2_000);
            let hint = super::error_hint(status.as_u16());
            anyhow::bail!("gemini returned {status} ({hint}): {error_body}");
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
                let payload: GeminiEvent = match serde_json::from_str(data) {
                    Ok(payload) => payload,
                    Err(_) => continue,
                };
                for part in payload.candidates.into_iter().flat_map(|candidate| {
                    candidate
                        .content
                        .and_then(|content| content.parts)
                        .into_iter()
                        .flatten()
                }) {
                    if let Some(text) = part.text {
                        on_text(text.clone());
                        content.push_str(&text);
                    }
                    if let Some(function_call) = part.function_call {
                        tool_calls.push(PartialToolCall {
                            name: function_call.name,
                            arguments: function_call.args.to_string(),
                        });
                    }
                }
            }
        }
        let tool_calls: Vec<ToolCall> = tool_calls
            .into_iter()
            .enumerate()
            .filter(|(_, call)| !call.name.is_empty())
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
}

#[async_trait]
impl super::Provider for Provider {
    async fn respond(
        &self,
        messages: &[Message],
        on_text: &mut (dyn FnMut(String) + Send),
    ) -> Result<Response> {
        let (system_instruction, contents) = self.wire_messages(messages);
        let response = self
            .client
            .post(self.model_url("streamGenerateContent"))
            .json(&json!({
                "contents": contents,
                "systemInstruction": system_instruction,
                "tools": self.tools(),
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
            .client
            .post(self.model_url("generateContent"))
            .json(&json!({
                "contents": [{
                    "role": "user",
                    "parts": [{ "text": transcript }],
                }],
                "systemInstruction": {
                    "parts": [{ "text": "Summarize this terminal-assistant conversation for future context. Preserve user goals, decisions, file paths, commands, command outcomes, errors, and unfinished work. Be concise and factual. Do not suggest new actions." }],
                },
            }))
            .send()
            .await
            .map_err(|error| super::provider_transport_error(&self.base_url, error))?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("summary request returned {status}");
        }
        let body = response.text().await?;
        let payload: GeminiEvent =
            serde_json::from_str(&body).context("invalid summary response")?;
        let text = payload
            .candidates
            .into_iter()
            .flat_map(|candidate| {
                candidate
                    .content
                    .and_then(|content| content.parts)
                    .into_iter()
                    .flatten()
            })
            .filter_map(|part| part.text)
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
            .get(format!(
                "{}/v1beta/models?key={}",
                self.base_url, self.api_key
            ))
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|error| super::provider_transport_error(&self.base_url, error))?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("gemini returned {status}");
        }
        let body = response.text().await?;
        let value: Value = serde_json::from_str(&body).context("invalid models response")?;
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
                    .map(|name| name.strip_prefix("models/").unwrap_or(&name).to_string())
            })
            .collect())
    }
}

#[derive(Deserialize)]
struct GeminiEvent {
    candidates: Vec<GeminiCandidate>,
}
#[derive(Deserialize)]
struct GeminiCandidate {
    content: Option<GeminiContent>,
}
#[derive(Deserialize)]
struct GeminiContent {
    parts: Option<Vec<GeminiPart>>,
}
#[derive(Deserialize)]
struct GeminiPart {
    text: Option<String>,
    function_call: Option<GeminiFunctionCall>,
}
#[derive(Deserialize)]
struct GeminiFunctionCall {
    name: String,
    #[serde(default)]
    args: Value,
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

    fn provider() -> Provider {
        Provider::new(&crate::config::Config {
            provider: "gemini".into(),
            api_key: "k".into(),
            base_url: "https://generativelanguage.googleapis.com".into(),
            model: "gemini-2.5-flash".into(),
            approval_mode: "always".into(),
        })
        .unwrap()
    }

    #[test]
    fn translates_tool_call_to_function_call_and_result_to_function_response() {
        let messages = vec![
            Message {
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
            },
            Message {
                role: "tool".into(),
                content: Some("{\"output\":\"ok\"}".into()),
                name: Some("run_command".into()),
                tool_call_id: Some("call_0".into()),
                tool_calls: None,
            },
        ];
        let (_, contents) = provider().wire_messages(&messages);
        assert_eq!(contents.len(), 2);
        assert_eq!(contents[0]["role"].as_str(), Some("model"));
        assert_eq!(
            contents[0]["parts"][0]["functionCall"]["name"].as_str(),
            Some("run_command")
        );
        assert_eq!(
            contents[0]["parts"][0]["functionCall"]["args"]["command"].as_str(),
            Some("ls")
        );
        assert_eq!(contents[1]["role"].as_str(), Some("user"));
        assert_eq!(
            contents[1]["parts"][0]["functionResponse"]["name"].as_str(),
            Some("run_command")
        );
        assert_eq!(
            contents[1]["parts"][0]["functionResponse"]["response"]["output"].as_str(),
            Some("ok")
        );
    }

    #[test]
    fn groups_consecutive_tool_results_into_one_user_turn() {
        let messages = vec![
            Message {
                role: "assistant".into(),
                content: None,
                name: None,
                tool_call_id: None,
                tool_calls: Some(vec![
                    ToolCall {
                        kind: "function".into(),
                        id: "call_0".into(),
                        function: FunctionCall {
                            name: "run_command".into(),
                            arguments: "{}".into(),
                        },
                    },
                    ToolCall {
                        kind: "function".into(),
                        id: "call_1".into(),
                        function: FunctionCall {
                            name: "run_command".into(),
                            arguments: "{}".into(),
                        },
                    },
                ]),
            },
            Message {
                role: "tool".into(),
                content: Some("{}".into()),
                name: Some("run_command".into()),
                tool_call_id: Some("call_0".into()),
                tool_calls: None,
            },
            Message {
                role: "tool".into(),
                content: Some("{}".into()),
                name: Some("run_command".into()),
                tool_call_id: Some("call_1".into()),
                tool_calls: None,
            },
        ];
        let (_, contents) = provider().wire_messages(&messages);
        let last = contents.last().unwrap();
        assert_eq!(last["role"].as_str(), Some("user"));
        assert_eq!(last["parts"].as_array().unwrap().len(), 2);
    }
}
