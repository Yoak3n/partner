use ai_partner_shared::{
    AgentEvent, Message, ModelKind, ModelProvider, Role, ToolDefinition,
    rag::{chunk_text, estimate_tokens},
};

use crate::adapter::AgentResponse;
use crate::hook::{HookContext, HookResult};
use crate::provider::RateLimitError;
use crate::state::AgentState;

use super::Runtime;
use super::tools::ToolExecutionResult;

impl Runtime {
    pub async fn send_message(&mut self, content: impl Into<String>) {
        // Load conversation history from DB if this is a fresh in-memory state
        if self.conversation.messages.is_empty() {
            match self.conversation_manager.load_recent(&self.session_id) {
                Ok(history) if !history.is_empty() => {
                    log::info!("loaded {} messages from session history", history.len());
                    for msg in history {
                        self.conversation.push(msg);
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    log::warn!("failed to load session history: {e}");
                }
            }
        }

        let content = content.into();
        let user_msg = Message::user(&content);

        // Start a new conversation for each agent loop
        self.ensure_new_conversation();

        // Ensure session exists in DB, save first message
        self.ensure_session_exists(Some(&content));

        self.conversation.push(user_msg.clone());
        let order = self.conversation.messages.len() as i64 - 1;
        let _ = self.storage.save_message(&self.session_id, &self.conversation_id, &user_msg, order);

        self.active_skills = self
            .skill_selector
            .select(&content, &self.available_skills);

        let mut tool_defs = self.tool_registry.definitions();
        tool_defs.extend(self.mcp_manager.all_tool_definitions());

        if !self.available_skills.is_empty() {
            tool_defs.push(ToolDefinition {
                name: "use_skill".into(),
                description: "Load the full guide for a skill. Call this when you need detailed instructions for a specific task.".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Name of the skill to load"
                        }
                    },
                    "required": ["name"]
                }),
            });
        }

        // Expose sub-agents as a callable tool
        let subagent_list = self.subagent_registry.all();
        if !subagent_list.is_empty() {
            let agent_names: Vec<&str> = subagent_list.iter().map(|(n, _)| *n).collect();
            let agent_descs: Vec<String> = subagent_list.iter()
                .map(|(n, d)| format!("- {}: {}", n, d))
                .collect();
            tool_defs.push(ToolDefinition {
                name: "call_subagent".into(),
                description: format!(
                    "Call a sub-agent to perform a specialized task.\nAvailable sub-agents:\n{}",
                    agent_descs.join("\n")
                ),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {
                            "type": "string",
                            "description": "Sub-agent name to call",
                            "enum": agent_names
                        },
                        "input": {
                            "type": "string",
                            "description": "Input text or task description for the sub-agent"
                        }
                    },
                    "required": ["name", "input"]
                }),
            });
        }

        let max_rounds = self.agent.max_tool_rounds();

        for round in 0..max_rounds {
            let provider = match self.select_provider() {
                Some(p) => p.clone(),
                None => {
                    let err = "No available provider (all disabled or rate-limited)".to_string();
                    self.state = AgentState::Error { message: err.clone() };
                    self.run_hooks_on_error(&err).await;
                    let _ = self.event_tx.send(AgentEvent::Error(err));
                    return;
                }
            };

            if let Err(RateLimitError::Limited { retry_after }) =
                self.rate_limiter.check(&provider)
            {
                if self.rate_limiter.wait(&provider).await.is_err() {
                    let err = format!("Rate limited on {}, retry after {:?}", provider.name, retry_after);
                    self.state = AgentState::Error { message: err.clone() };
                    self.run_hooks_on_error(&err).await;
                    let _ = self.event_tx.send(AgentEvent::Error(err));
                    return;
                }
            }

            self.state = AgentState::Thinking;
            let mut messages = self.build_llm_messages();

            let sess_id = self.session_id.clone();
            let ctx = HookContext {
                provider: &provider,
                round,
                session_id: &sess_id,
            };

            match self.run_hooks_before_llm(&ctx, &mut messages).await {
                HookResult::Abort(reason) => {
                    self.state = AgentState::Error { message: reason.clone() };
                    self.run_hooks_on_error(&reason).await;
                    let _ = self.event_tx.send(AgentEvent::Error(reason));
                    return;
                }
                HookResult::Skip => continue,
                HookResult::Continue => {}
            }

            let response = self.agent.run(&provider, &messages, &tool_defs, &self.event_tx).await;

            let mut response = match response {
                Ok(r) => r,
                Err(e) => {
                    let err = e.to_string();
                    self.state = AgentState::Error { message: err.clone() };
                    self.run_hooks_on_error(&err).await;
                    let _ = self.event_tx.send(AgentEvent::Error(err));
                    return;
                }
            };

            match self.run_hooks_after_llm(&ctx, &mut response).await {
                HookResult::Abort(reason) => {
                    self.state = AgentState::Error { message: reason.clone() };
                    self.run_hooks_on_error(&reason).await;
                    let _ = self.event_tx.send(AgentEvent::Error(reason));
                    return;
                }
                HookResult::Skip => continue,
                HookResult::Continue => {}
            }

            match response {
                AgentResponse::MessageComplete(msg) => {
                    self.conversation.push(msg.clone());
                    let order = self.conversation.messages.len() as i64 - 1;
                    let _ = self.storage.save_message(&self.session_id, &self.conversation_id, &msg, order);
                    self.generate_and_store_summary();
                    self.state = AgentState::Idle;
                    return;
                }
                AgentResponse::ToolCalls(calls) => {
                    let assistant_msg = Message::assistant_tool_calls(calls.clone());
                    self.conversation.push(assistant_msg.clone());
                    let order = self.conversation.messages.len() as i64 - 1;
                    let _ = self.storage.save_message(&self.session_id, &self.conversation_id, &assistant_msg, order);

                    let result = self.execute_tool_calls(&ctx, &calls, round, max_rounds).await;
                    match result {
                        ToolExecutionResult::Continue => continue,
                        ToolExecutionResult::Abort => return,
                    }
                }
            }
        }
    }

    fn build_llm_messages(&self) -> Vec<Message> {
        let mut msgs = Vec::new();

        let mut prompt_parts = Vec::new();

        if let Some(ref sys_prompt) = self.system_prompt {
            prompt_parts.push(sys_prompt.clone());
        }

        // 环境信息后置，优先级低于角色和工作空间定义
        prompt_parts.push(self.build_environment_context());

        if !self.active_skills.is_empty() {
            let summaries: Vec<String> = self.active_skills.iter().map(|s| s.summary()).collect();
            prompt_parts.push(format!(
                "Available skills (call use_skill to load full guide):\n{}",
                summaries.join("\n")
            ));
        }

        if !prompt_parts.is_empty() {
            msgs.push(Message::system(&prompt_parts.join("\n\n")));
        }
        msgs.extend(self.conversation.messages.clone());
        msgs
    }

    fn build_environment_context(&self) -> String {
        let now = chrono::Local::now();
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        let workspace = self.workspace.as_ref()
            .map(|ws| ws.root().display().to_string())
            .unwrap_or_else(|| "not initialized".to_string());

        format!(
            "# Runtime Environment\n\
             - OS: {} ({})\n\
             - Working directory: {}\n\
             - Workspace: {}\n\
             - Current time: {}\n\
             - Timezone: UTC{}\n\
             - Conversation ID: {}",
            os,
            arch,
            cwd,
            workspace,
            now.format("%Y-%m-%d %H:%M:%S"),
            now.format("%:z"),
            self.conversation_id,
        )
    }

    fn select_provider(&self) -> Option<&ModelProvider> {
        let group = self.app_config.group(ModelKind::Chat);
        eprintln!("[provider] active: {:?}, providers: {}", group.active, group.providers.len());
        if let Some(ref active_id) = group.active {
            if let Some(p) = group.providers.iter().find(|p| &p.id == active_id) {
                eprintln!("[provider] found active: {} ({})", p.name, p.base_url);
                if p.enabled {
                    return Some(p);
                }
            }
        }

        for _ in 0..self.balancer.providers().len() {
            if let Some(provider) = self.balancer.select() {
                if self.rate_limiter.is_available(provider) {
                    eprintln!("[provider] using balancer: {} ({})", provider.name, provider.base_url);
                    return Some(provider);
                }
            }
        }
        eprintln!("[provider] no provider available!");
        None
    }

    fn generate_and_store_summary(&mut self) {
        let messages = &self.conversation.messages;
        if messages.is_empty() {
            return;
        }

        let summary_content = generate_summary(messages);
        let msg_range = format!("0-{}", messages.len().saturating_sub(1));

        let summary_id = match self.storage.save_summary(
            &self.conversation_id,
            &summary_content,
            &msg_range,
        ) {
            Ok(id) => id,
            Err(e) => {
                log::warn!("failed to save summary: {e}");
                return;
            }
        };

        let chunks = chunk_text(&summary_content, 2000, 400);
        let mut doc_ids = Vec::new();
        for (i, chunk) in chunks.iter().enumerate() {
            let token_count = estimate_tokens(chunk);
            match self.storage.save_document(
                &summary_id,
                &self.session_id,
                chunk,
                i as i32,
                token_count,
            ) {
                Ok(id) => doc_ids.push(id),
                Err(e) => log::warn!("failed to save document chunk {i}: {e}"),
            }
        }

        log::info!(
            "stored summary + {} chunks for session {}",
            chunks.len(),
            &self.session_id
        );

        // Generate embeddings in background if adapter is available
        if let Some(ref adapter) = self.embedding_adapter {
            let storage = self.storage.clone();
            let adapter = adapter.clone();
            let sess_id = self.session_id.clone();
            tokio::spawn(async move {
                let mut count = 0;
                for doc_id in &doc_ids {
                    let docs = match storage.get_documents_by_session(&sess_id) {
                        Ok(d) => d,
                        Err(e) => {
                            log::warn!("failed to get documents for embedding: {e}");
                            break;
                        }
                    };
                    let doc = match docs.iter().find(|d| d.id == *doc_id && d.embedding.is_none()) {
                        Some(d) => d,
                        None => continue,
                    };
                    match adapter.embed(&doc.content).await {
                        Ok(embedding) => {
                            if let Err(e) = storage.save_document_embedding(doc_id, &embedding) {
                                log::warn!("failed to save embedding for {doc_id}: {e}");
                            } else {
                                count += 1;
                            }
                        }
                        Err(e) => {
                            log::warn!("embedding generation failed for {doc_id}: {e}");
                        }
                    }
                }
                if count > 0 {
                    log::info!("generated {count} embeddings for session {sess_id}");
                }
            });
        }
    }
}

fn generate_summary(messages: &[Message]) -> String {
    let user_msgs: Vec<&str> = messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.content.as_str())
        .collect();

    let assistant_msgs: Vec<&str> = messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .map(|m| m.content.as_str())
        .collect();

    let mut summary = String::new();

    if !user_msgs.is_empty() {
        summary.push_str("User asked about: ");
        summary.push_str(&user_msgs.join("; "));
        summary.push_str(". ");
    }

    if !assistant_msgs.is_empty() {
        summary.push_str("Assistant responded about: ");
        let last = assistant_msgs.last().unwrap();
        let truncated: String = last.chars().take(200).collect();
        if truncated.len() < last.len() {
            summary.push_str(&truncated);
            summary.push_str("...");
        } else {
            summary.push_str(&truncated);
        }
    }

    if summary.is_empty() {
        summary = "Empty conversation".to_string();
    }

    summary
}
