mod agent_loop;
mod hooks;
pub(crate) mod selector;
pub(crate) mod tools;

use std::sync::Arc;
use tokio::sync::mpsc;

use ai_partner_shared::{
    AgentEvent, AppConfig, ModelKind, Skill, Storage, ToolDefinition,
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
        let conversation_id = uuid::Uuid::new_v4().to_string();
        let _ = storage.create_conversation(&conversation_id, None);

        let system_prompt = load_system_prompt();

        let process_manager = Arc::new(ProcessManager::new());

        // Initialize embedding adapter from config
        let embedding_group = app_config.group(ModelKind::Embedding);
        let embedding_adapter = embedding_group.providers.iter()
            .find(|p| p.enabled)
            .map(|p| create_embedding_adapter(p));

        let mut runtime = Self {
            agent,
            app_config,
            system_prompt,
            conversation: Conversation::new(),
            conversation_id,
            conversation_manager: ConversationManager::new(storage.clone()),
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
        };

        register_builtins(&mut runtime.tool_registry, event_tx);
        register_memory_manage(&mut runtime.tool_registry, runtime.storage.clone(), runtime.embedding_adapter.clone());

        // Register built-in sub-agents
        runtime.subagent_registry.register(SkillSelectorAgent::new());

        // Register compaction sub-agent (LLM-powered, also used by CompactionHook)
        let compaction_agent: Arc<dyn SubAgent> = Arc::new(
            CompactionAgent::new(runtime.agent.adapter()),
        );
        runtime.subagent_registry.register_arc(compaction_agent.clone());
        runtime.hooks.push(Box::new(CompactionHook::new(
            ConversationManager::new(runtime.storage.clone()),
            compaction_agent,
            Arc::new(runtime.app_config.clone()),
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

    pub fn set_workspace(&mut self, config: WorkspaceConfig) {
        match Workspace::from_config(config) {
            Ok(ws) => {
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
        let _ = self.storage.create_conversation(&self.conversation_id, None);
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
            conversation_id: &self.conversation_id,
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
}
