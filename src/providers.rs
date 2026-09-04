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

#[async_trait]
pub trait Provider: Send + Sync {
    async fn respond(
        &self,
        messages: &[Message],
        on_text: &mut (dyn FnMut(String) + Send),
    ) -> Result<Response>;
    async fn summarize(&self, messages: &[Message]) -> Result<String>;
}

pub struct OpenAiCompatibleProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
}
impl OpenAiCompatibleProvider {
    pub fn new(config: crate::config::Config) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: config.base_url.trim_end_matches('/').into(),
            api_key: config.api_key,
            model: config.model,
        }
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
#[async_trait]
impl Provider for OpenAiCompatibleProvider {
    async fn respond(
        &self,
        messages: &[Message],
        on_text: &mut (dyn FnMut(String) + Send),
    ) -> Result<Response> {
        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&ChatRequest {
                model: &self.model,
                messages,
                tools: crate::tools::definitions(),
                stream: true,
            })
            .send()
            .await
            .context("provider request failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await?;
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
            Message { role: "system".into(), content: Some("Summarize this terminal-assistant conversation for future context. Preserve user goals, decisions, file paths, commands, command outcomes, errors, and unfinished work. Be concise and factual. Do not suggest new actions.".into()), name: None, tool_call_id: None, tool_calls: None },
            Message { role: "user".into(), content: Some(transcript), name: None, tool_call_id: None, tool_calls: None },
        ];
        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&SummaryRequest {
                model: &self.model,
                messages: &summary_messages,
                stream: false,
            })
            .send()
            .await
            .context("summary request failed")?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            anyhow::bail!("summary request returned {status}");
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
}

fn error_hint(status: u16) -> &'static str {
    match status {
        401 | 403 => "check your API key",
        429 => "rate limit reached; try again shortly",
        500..=599 => "provider is unavailable",
        _ => "check the provider URL and request",
    }
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
}
