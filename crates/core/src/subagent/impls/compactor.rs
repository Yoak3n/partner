use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc;

use ai_partner_shared::{AgentEvent, AppConfig, Message, ModelKind};

use crate::adapter::{AgentResponse, LlmAdapter};
use crate::conversation::ConversationManager;
use crate::hook::{AgentHook, HookContext, HookResult};
use crate::subagent::{SubAgent, SubAgentContext, SubAgentResult};

/// Sub-agent that uses an LLM to compress a set of evicted conversation messages
/// into a concise summary, preserving key context.
pub struct CompactionAgent {
    adapter: Arc<dyn LlmAdapter>,
}

impl CompactionAgent {
    pub fn new(adapter: Arc<dyn LlmAdapter>) -> Self {
        Self { adapter }
    }
}

#[async_trait]
impl SubAgent for CompactionAgent {
    fn name(&self) -> &str {
        "compactor"
    }

    fn description(&self) -> &str {
        "Compress evicted conversation rounds into a concise summary using an LLM"
    }

    async fn execute(&self, input: &str, ctx: SubAgentContext<'_>) -> SubAgentResult {
        // Select the first enabled chat provider
        let chat_group = ctx.app_config.group(ModelKind::Chat);
        let provider = match chat_group.providers.iter().find(|p| p.enabled) {
            Some(p) => p,
            None => {
                return SubAgentResult {
                    output: "[compaction skipped: no enabled provider]".to_string(),
                    metadata: None,
                };
            }
        };

        let messages = vec![
            Message::system(
                "You are a conversation compaction assistant. \
                 Your task is to compress conversation history into a concise summary. \
                 Preserve all key information: decisions, facts, tasks, code changes, \
                 errors, and important context. Output ONLY the summary, nothing else.",
            ),
            Message::user(input),
        ];

        let (dummy_tx, _rx) = mpsc::unbounded_channel::<AgentEvent>();

        let response = match self.adapter.chat(provider, &messages, &[], &dummy_tx).await {
            Ok(resp) => resp,
            Err(e) => {
                return SubAgentResult {
                    output: format!("[compaction failed: {e}]"),
                    metadata: None,
                };
            }
        };

        let output = match response {
            AgentResponse::MessageComplete(msg) => msg.content,
            AgentResponse::ToolCalls(_) => {
                "[compaction produced tool calls instead of summary]".to_string()
            }
        };

        SubAgentResult {
            output,
            metadata: None,
        }
    }
}

/// Hook that triggers conversation compaction before LLM calls.
///
/// When the message list exceeds `max_messages`, the hook:
/// 1. Evicts the oldest messages (keeping the most recent `min_recent_messages`)
/// 2. Sends evicted messages to the compaction sub-agent for summarization
/// 3. Replaces evicted messages with a single assistant summary message
/// 4. Persists the changes (deletes old messages from DB)
pub struct CompactionHook {
    manager: ConversationManager,
    compactor: Arc<dyn SubAgent>,
    app_config: Arc<AppConfig>,
}

impl CompactionHook {
    pub fn new(
        manager: ConversationManager,
        compactor: Arc<dyn SubAgent>,
        app_config: Arc<AppConfig>,
    ) -> Self {
        Self {
            manager,
            compactor,
            app_config,
        }
    }
}

#[async_trait]
impl AgentHook for CompactionHook {
    async fn before_llm_call(
        &self,
        ctx: &HookContext<'_>,
        messages: &mut Vec<Message>,
    ) -> HookResult {
        if !self.manager.should_compact(messages) {
            return HookResult::Continue;
        }

        let (evicted, kept) =
            self.manager.evict_old_messages(messages, self.manager.min_recent_messages);

        if evicted.is_empty() {
            return HookResult::Continue;
        }

        log::info!(
            "compacting {} evicted messages (keeping {})",
            evicted.len(),
            kept.len()
        );

        let prompt = self.manager.build_compaction_prompt(&evicted);
        let sub_ctx = SubAgentContext {
            session_id: ctx.session_id,
            message_history: &[],
            available_skills: &[],
            app_config: &self.app_config,
        };

        let result = self.compactor.execute(&prompt, sub_ctx).await;

        let summary_msg = Message::assistant(&format!(
            "[Compacted history summary]\n{}",
            result.output
        ));

        let mut new_messages = Vec::with_capacity(kept.len() + 1);
        new_messages.push(summary_msg);
        new_messages.extend(kept);
        *messages = new_messages;

        let boundary = evicted.len() as i64;
        if let Err(e) = self
            .manager
            .storage_ref()
            .delete_messages_before(ctx.session_id, boundary)
        {
            log::warn!("failed to delete compacted messages: {e}");
        }

        log::info!(
            "compaction complete: {} messages replaced with summary",
            evicted.len()
        );

        HookResult::Continue
    }
}
