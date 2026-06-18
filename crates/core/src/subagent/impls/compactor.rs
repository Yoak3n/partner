use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::{Mutex, mpsc};

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
/// 3. Persists the summary to the diary file (injected as system context)
/// 4. Keeps only the recent messages in the message list
pub struct CompactionHook {
    manager: ConversationManager,
    compactor: Arc<dyn SubAgent>,
    app_config: Arc<AppConfig>,
    diary_path: Arc<Mutex<PathBuf>>,
}

impl CompactionHook {
    pub fn new(
        manager: ConversationManager,
        compactor: Arc<dyn SubAgent>,
        app_config: Arc<AppConfig>,
        diary_path: Arc<Mutex<PathBuf>>,
    ) -> Self {
        Self {
            manager,
            compactor,
            app_config,
            diary_path,
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

        // Persist summary to diary file
        self.append_to_diary(&result.output, evicted.len()).await;

        // Keep only recent messages — summary is in the diary, not in the message list
        *messages = kept;

        log::info!(
            "compaction complete: {} evicted, diary updated",
            evicted.len()
        );

        HookResult::Continue
    }
}

impl CompactionHook {
    async fn append_to_diary(&self, summary: &str, message_count: usize) {
        let diary_path = self.diary_path.lock().await.clone();

        if let Err(e) = std::fs::create_dir_all(&diary_path) {
            log::warn!("failed to create diary directory: {e}");
            return;
        }

        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        let time = chrono::Local::now().format("%H:%M").to_string();

        let entry = format!(
            "### [{}] Compacted {} messages\n\n{}\n\n",
            time, message_count, summary
        );

        let path = diary_path.join(format!("{date}.md"));

        // Ensure file has section headers on first write
        if !path.exists() {
            let init = "# Agent Notes\n\n\n\n---\n\n# Compact History\n\n";
            if let Err(e) = std::fs::write(&path, init) {
                log::warn!("failed to init diary file: {e}");
                return;
            }
        }

        // Insert entry after "# Compact History" header
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("failed to read diary file: {e}");
                return;
            }
        };

        let marker = "# Compact History\n";
        let new_content = if let Some(pos) = content.find(marker) {
            let insert_at = pos + marker.len();
            let mut result = String::with_capacity(content.len() + entry.len());
            result.push_str(&content[..insert_at]);
            result.push('\n');
            result.push_str(&entry);
            result.push_str(&content[insert_at..]);
            result
        } else {
            // Fallback: prepend
            format!("{}\n{}", entry, content)
        };

        if let Err(e) = std::fs::write(&path, &new_content) {
            log::warn!("failed to write diary entry: {e}");
        }
    }
}
