mod agent_loop;
mod hooks;
pub(crate) mod selector;
pub(crate) mod tools;

use std::sync::Arc;
use tokio::sync::{Mutex, mpsc};

use ai_partner_shared::{
    AgentEvent, AppConfig, Message, ModelKind, Skill, Storage, ToolDefinition,
    WorkspaceConfig, load_system_prompt,
};

use crate::workspace::Workspace;

use crate::agent::Agent;
use crate::conversation::ConversationManager;
use crate::provider::{EmbeddingAdapter, ProviderBalancer, RateLimiter, create_embedding_adapter};
use crate::hook::AgentHook;
use crate::mcp::{McpError, manager::McpManager};
use selector::{HeuristicSelector, SkillSelector};
use crate::state::{AgentState, Conversation};
use crate::subagent::{SubAgent, SubAgentContext, SubAgentResult, SubAgentRegistry};
use crate::subagent::impls::compactor::{CompactionAgent, CompactionHook};
use crate::subagent::impls::skill_selector::SkillSelectorAgent;
use crate::subagent::registry::SubAgentError;
use crate::tools::{ToolRegistry, ProcessManager, register_builtins, register_memory_manage};

pub struct Runtime {
    pub(crate) agent: Agent,
    pub(crate) app_config: AppConfig,
    pub(crate) system_prompt: Option<String>,
    pub(crate) conversation: Conversation,
    pub(crate) session_id: String,
    pub(crate) conversation_id: String,
    pub(crate) conversation_manager: ConversationManager,
    pub(crate) tool_registry: ToolRegistry,
    pub(crate) mcp_manager: McpManager,
    pub(crate) process_manager: Arc<ProcessManager>,
    pub(crate) embedding_adapter: Option<Arc<dyn EmbeddingAdapter>>,
    pub(crate) workspace: Option<Workspace>,
    pub(crate) available_skills: Vec<Skill>,
    pub(crate) active_skills: Vec<Skill>,
    pub(crate) skill_selector: Box<dyn SkillSelector>,
    pub(crate) hooks: Vec<Box<dyn AgentHook>>,
    pub(crate) state: AgentState,
    pub(crate) event_tx: mpsc::UnboundedSender<AgentEvent>,
    pub(crate) balancer: ProviderBalancer,
    pub(crate) rate_limiter: RateLimiter,
    pub(crate) storage: Arc<Storage>,
    pub(crate) subagent_registry: SubAgentRegistry,
    pub(crate) diary_path: Arc<Mutex<std::path::PathBuf>>,
    /// Shared workspace root for tools to resolve relative paths.
    pub(crate) workspace_root: Arc<Mutex<Option<std::path::PathBuf>>>,
}

impl Runtime {
    pub fn new(
        agent: Agent,
        app_config: AppConfig,
        storage: Storage,
        event_tx: mpsc::UnboundedSender<AgentEvent>,
    ) -> Self {
        let chat_group = app_config.group(ModelKind::Chat);
        let balancer = ProviderBalancer::new(chat_group.providers.clone());
        let rate_limiter = RateLimiter::new();
        rate_limiter.configure_all(&chat_group.providers);

        let storage = Arc::new(storage);

        // 尝试恢复最近的 session，而不是每次都创建新的
        let (session_id, conversation_id) = match storage.list_sessions() {
            Ok(sessions) if !sessions.is_empty() => {
                // 使用最近更新的 session
                let recent = &sessions[0];
                let conv_id = uuid::Uuid::new_v4().to_string();
                let _ = storage.create_conversation(&conv_id, &recent.id);
                (recent.id.clone(), conv_id)
            }
            _ => {
                // 没有 session，暂不创建，等用户发送消息时再创建
                let sid = uuid::Uuid::new_v4().to_string();
                let cid = uuid::Uuid::new_v4().to_string();
                (sid, cid)
            }
        };

        let system_prompt = load_system_prompt();

        let process_manager = Arc::new(ProcessManager::new());

        // Initialize embedding adapter from config
        let embedding_group = app_config.group(ModelKind::Embedding);
        let embedding_adapter = embedding_group.providers.iter()
            .find(|p| p.enabled)
            .map(|p| create_embedding_adapter(p));

        // Default diary path: {CWD}/.ai-partner/memory/diary
        let default_diary = std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(".ai-partner")
            .join("memory")
            .join("diary");
        let diary_path = Arc::new(Mutex::new(default_diary));
        let workspace_root: Arc<Mutex<Option<std::path::PathBuf>>> = Arc::new(Mutex::new(None));

        let mut runtime = Self {
            agent,
            app_config,
            system_prompt,
            conversation: Conversation::new(),
            session_id,
            conversation_id,
            conversation_manager: ConversationManager::new(),
            tool_registry: ToolRegistry::new(),
            mcp_manager: McpManager::new(),
            process_manager,
            embedding_adapter,
            workspace: None,
            available_skills: Vec::new(),
            active_skills: Vec::new(),
            skill_selector: Box::new(HeuristicSelector),
            hooks: Vec::new(),
            state: AgentState::Idle,
            event_tx: event_tx.clone(),
            balancer,
            rate_limiter,
            storage,
            subagent_registry: SubAgentRegistry::new(),
            diary_path: diary_path.clone(),
            workspace_root: workspace_root.clone(),
        };

        register_builtins(&mut runtime.tool_registry, event_tx, workspace_root);
        register_memory_manage(&mut runtime.tool_registry, runtime.storage.clone(), runtime.embedding_adapter.clone());

        // Register built-in sub-agents
        runtime.subagent_registry.register(SkillSelectorAgent::new());

        // Register compaction sub-agent (LLM-powered, also used by CompactionHook)
        let compaction_agent: Arc<dyn SubAgent> = Arc::new(
            CompactionAgent::new(runtime.agent.adapter()),
        );
        runtime.subagent_registry.register_arc(compaction_agent.clone());

        runtime.hooks.push(Box::new(CompactionHook::new(
            ConversationManager::new(),
            compaction_agent,
            Arc::new(runtime.app_config.clone()),
            runtime.diary_path.clone(),
        )));

        runtime
    }

    pub fn register_tool(
        &mut self,
        definition: ToolDefinition,
        handler: impl Fn(serde_json::Value) -> Result<String, String> + Send + Sync + 'static,
    ) {
        self.tool_registry.register(definition, handler);
    }

    pub fn reload_system_prompt(&mut self) {
        let global = load_system_prompt();
        self.system_prompt = match self.workspace {
            Some(ref ws) => Some(ws.build_system_prompt(global.as_deref())),
            None => global,
        };
    }

    pub fn add_hook(&mut self, hook: impl AgentHook + 'static) {
        self.hooks.push(Box::new(hook));
    }

    pub async fn connect_mcp(
        &mut self,
        name: &str,
        command: &str,
        args: &[String],
    ) -> Result<(), McpError> {
        self.mcp_manager.connect(name, command, args).await
    }

    pub fn mcp_manager(&mut self) -> &mut McpManager {
        &mut self.mcp_manager
    }

    pub async fn set_workspace(&mut self, config: WorkspaceConfig) {
        let used_default = config.is_none();
        match Workspace::from_config(config) {
            Ok(ws) => {
                // Update diary path to workspace memory directory
                let new_diary = ws.memory_path().join("diary");
                *self.diary_path.lock().await = new_diary;
                // Update shared workspace root for tools
                *self.workspace_root.lock().await = Some(ws.root().clone());

                // If using default path, persist it to config so the agent knows the location
                if used_default {
                    let resolved = ws.root().display().to_string();
                    self.app_config.workspace = Some(resolved);
                    if let Err(e) = self.app_config.save() {
                        log::warn!("failed to save workspace path to config: {e}");
                    }
                }

                self.available_skills = ws.load_skills();
                log::info!("loaded {} skills from workspace", self.available_skills.len());
                // Rebuild system prompt with workspace instruction files
                let global = load_system_prompt();
                self.system_prompt = Some(ws.build_system_prompt(global.as_deref()));
                self.workspace = Some(ws);
            }
            Err(e) => {
                log::warn!("failed to set workspace: {e}");
            }
        }
    }

    pub fn set_skill_selector(&mut self, selector: impl SkillSelector + 'static) {
        self.skill_selector = Box::new(selector);
    }

    pub fn workspace(&self) -> Option<&Workspace> {
        self.workspace.as_ref()
    }

    pub fn available_skills(&self) -> &[Skill] {
        &self.available_skills
    }

    pub fn active_skills(&self) -> &[Skill] {
        &self.active_skills
    }

    pub fn state(&self) -> &AgentState {
        &self.state
    }

    pub fn conversation(&self) -> &Conversation {
        &self.conversation
    }

    pub fn clear_conversation(&mut self) {
        self.conversation.clear();
        self.conversation_id = uuid::Uuid::new_v4().to_string();
        let _ = self.storage.create_conversation(&self.conversation_id, &self.session_id);
    }

    pub fn app_config(&self) -> &AppConfig {
        &self.app_config
    }

    pub fn reload_config(&mut self, config: AppConfig) {
        let chat_group = config.group(ModelKind::Chat);
        self.balancer = ProviderBalancer::new(chat_group.providers.clone());
        self.rate_limiter.configure_all(&chat_group.providers);

        // Update embedding adapter
        let embedding_group = config.group(ModelKind::Embedding);
        self.embedding_adapter = embedding_group.providers.iter()
            .find(|p| p.enabled)
            .map(|p| create_embedding_adapter(p));

        // Re-register memory_manage to pick up the new embedding adapter
        register_memory_manage(&mut self.tool_registry, self.storage.clone(), self.embedding_adapter.clone());

        self.app_config = config;
    }

    /// Register a sub-agent.
    pub fn register_subagent(&mut self, agent: impl SubAgent + 'static) {
        self.subagent_registry.register(agent);
    }

    /// Get the sub-agent registry.
    pub fn subagents(&self) -> &SubAgentRegistry {
        &self.subagent_registry
    }

    /// Call a sub-agent by name with the current conversation context.
    pub async fn call_subagent(
        &self,
        name: &str,
        input: &str,
    ) -> Result<SubAgentResult, SubAgentError> {
        let ctx = SubAgentContext {
            session_id: &self.session_id,
            message_history: &self.conversation.messages,
            available_skills: &self.available_skills,
            app_config: &self.app_config,
        };
        self.subagent_registry.execute(name, input, ctx).await
    }

    /// Get the process manager for querying/controlling spawned subprocesses.
    pub fn process_manager(&self) -> &Arc<ProcessManager> {
        &self.process_manager
    }

    /// Shut down all managed processes and MCP connections.
    pub async fn shutdown(&mut self) {
        self.process_manager.kill_all().await;
        self.mcp_manager.disconnect_all().await;
    }

    // ── Session management ──

    /// Ensure the current conversation exists in the database
    pub fn ensure_session_exists(&mut self, first_message: Option<&str>) {
        // 如果 session 已存在，只更新 first_message（如果为空）
        if let Some(msg) = first_message {
            let _ = self.storage.update_session_first_message(&self.session_id, msg);
        }
        // 如果 session 不存在，创建它
        let _ = self.storage.create_session(&self.session_id, None, first_message);
        let _ = self.storage.create_conversation(&self.conversation_id, &self.session_id);
    }

    /// Ensure a new conversation exists for the current session
    pub fn ensure_new_conversation(&mut self) {
        self.conversation_id = uuid::Uuid::new_v4().to_string();
        let _ = self.storage.create_conversation(&self.conversation_id, &self.session_id);
    }

    /// List all sessions
    pub fn list_sessions(&self) {
        match self.storage.list_sessions() {
            Ok(list) => {
                let _ = self.event_tx.send(AgentEvent::SessionsLoaded(list));
            }
            Err(e) => {
                log::warn!("failed to list sessions: {e}");
                let _ = self.event_tx.send(AgentEvent::Error(format!("加载会话列表失败: {e}")));
            }
        }
    }

    /// Create a new session (doesn't save to DB until first message)
    pub fn new_session(&mut self) {
        // 创建新的 session_id
        self.session_id = uuid::Uuid::new_v4().to_string();
        self.conversation_id = uuid::Uuid::new_v4().to_string();
        self.conversation.clear();
        
        // 在数据库中创建 session 记录
        let _ = self.storage.create_session(&self.session_id, None, None);
        let _ = self.storage.create_conversation(&self.conversation_id, &self.session_id);
        
        // 通知 UI
        let _ = self.event_tx.send(AgentEvent::SessionCreated(self.session_id.clone()));
        
        // 刷新 session 列表
        self.list_sessions();
    }

    /// Switch to a specific session
    pub fn switch_session(&mut self, session_id: &str) {
        self.conversation.clear();
        self.conversation_id = session_id.to_string();

        match self.storage.load_messages_recent(session_id, self.conversation_manager.max_messages) {
            Ok(messages) => {
                let compressed: Vec<Message> = messages.iter()
                    .map(|m| self.conversation_manager.compress_message(m))
                    .collect();
                for msg in &compressed {
                    self.conversation.push(msg.clone());
                }
                let _ = self.event_tx.send(AgentEvent::SessionSwitched(session_id.to_string()));
                let _ = self.event_tx.send(AgentEvent::MessagesLoaded {
                    session_id: session_id.to_string(),
                    messages: compressed,
                });
            }
            Err(e) => {
                log::warn!("failed to load session {session_id}: {e}");
                let _ = self.event_tx.send(AgentEvent::Error(format!("加载会话失败: {e}")));
            }
        }
    }

    /// Delete a session and refresh the list
    pub fn delete_session(&mut self, session_id: &str) {
        match self.storage.delete_session(session_id) {
            Ok(()) => {
                log::info!("deleted session {session_id}");
                // If we deleted the current session, clear the conversation
                if self.session_id == session_id {
                    self.conversation.clear();
                    self.session_id = uuid::Uuid::new_v4().to_string();
                    self.conversation_id = uuid::Uuid::new_v4().to_string();
                }
                // Refresh the session list
                self.list_sessions();
            }
            Err(e) => {
                log::warn!("failed to delete session {session_id}: {e}");
                let _ = self.event_tx.send(AgentEvent::Error(format!("删除会话失败: {e}")));
            }
        }
    }

    /// Toggle pin status of a session
    pub fn toggle_pin_session(&mut self, session_id: &str) {
        match self.storage.toggle_session_pinned(session_id) {
            Ok(pinned) => {
                log::info!("session {session_id} pinned={pinned}");
                self.list_sessions();
            }
            Err(e) => {
                log::warn!("failed to toggle pin {session_id}: {e}");
                let _ = self.event_tx.send(AgentEvent::Error(format!("置顶失败: {e}")));
            }
        }
    }

    /// Toggle archive status of a session
    pub fn toggle_archive_session(&mut self, session_id: &str) {
        match self.storage.toggle_session_archived(session_id) {
            Ok(archived) => {
                log::info!("session {session_id} archived={archived}");
                self.list_sessions();
            }
            Err(e) => {
                log::warn!("failed to toggle archive {session_id}: {e}");
                let _ = self.event_tx.send(AgentEvent::Error(format!("归档失败: {e}")));
            }
        }
    }
}
