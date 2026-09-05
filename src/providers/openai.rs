use anyhow::Result;
use async_trait::async_trait;

use crate::config::Config;

pub const PRESET_MODEL: &str = "gpt-4o-mini";

/// OpenAI. Speaks the OpenAI wire protocol.
pub struct Provider {
    wire: super::OpenAiWire,
}

impl Provider {
    pub fn new(config: &Config) -> Result<Self> {
        Ok(Self {
            wire: super::OpenAiWire::new(config),
        })
    }
}

#[async_trait]
impl super::Provider for Provider {
    async fn respond(
        &self,
        messages: &[super::Message],
        on_text: &mut (dyn FnMut(String) + Send),
    ) -> Result<super::Response> {
        self.wire.chat(messages, on_text).await
    }

    async fn summarize(&self, messages: &[super::Message]) -> Result<String> {
        self.wire.summarize(messages).await
    }

    async fn models(&self) -> Result<Vec<String>> {
        self.wire.models().await
    }
}
