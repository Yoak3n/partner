use std::sync::Arc;
use tokio::sync::mpsc;

use ai_partner_shared::{AgentEvent, Message, ModelProvider, ToolDefinition};

use crate::adapter::{AgentResponse, LlmAdapter};

pub struct Agent {
    adapter: Arc<dyn LlmAdapter>,
    max_tool_rounds: usize,
}

impl Agent {
    pub fn new(adapter: impl LlmAdapter + 'static) -> Self {
        Self {
            adapter: Arc::new(adapter),
            max_tool_rounds: 200,
        }
    }

    pub fn with_max_rounds(mut self, max: usize) -> Self {
        self.max_tool_rounds = max;
        self
    }

    pub async fn run(
        &self,
        provider: &ModelProvider,
        messages: &[Message],
        tools: &[ToolDefinition],
        event_tx: &mpsc::UnboundedSender<AgentEvent>,
    ) -> Result<AgentResponse, AgentError> {
        self.adapter
            .chat(provider, messages, tools, event_tx)
            .await
    }

    pub fn max_tool_rounds(&self) -> usize {
        self.max_tool_rounds
    }

    /// Get a reference-counted handle to the underlying LLM adapter.
    pub fn adapter(&self) -> Arc<dyn LlmAdapter> {
        self.adapter.clone()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Tool error: {0}")]
    Tool(String),

    #[error("rate limited, retry after {retry_after_secs:.1}s")]
    RateLimited { retry_after_secs: f64 },

    #[error("{0}")]
    Other(String),
}
