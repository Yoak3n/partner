use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 模型类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Chat,
    Embedding,
}

impl std::fmt::Display for ModelKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Chat => write!(f, "chat"),
            Self::Embedding => write!(f, "embedding"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProvider {
    pub id: String,
    pub kind: ModelKind,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub max_output: u32,
    pub weight: u32,
    pub requests_per_minute: u32,
    pub enabled: bool,
}

impl ModelProvider {
    pub fn new(
        kind: ModelKind,
        name: impl Into<String>,
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            kind,
            name: name.into(),
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            max_output: 4096,
            weight: 1,
            requests_per_minute: 60,
            enabled: true,
        }
    }
}

/// MCP 服务器配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

/// 一组同类型的 provider 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderGroup {
    /// 手动选中的 provider id (None = 负载均衡)
    pub active: Option<String>,
    pub providers: Vec<ModelProvider>,
}

impl ProviderGroup {
    pub fn enabled(&self) -> Vec<&ModelProvider> {
        self.providers.iter().filter(|p| p.enabled).collect()
    }

    pub fn find(&self, id: &str) -> Option<&ModelProvider> {
        self.providers.iter().find(|p| p.id == id)
    }
}

/// Workspace 配置 — 仅指定路径，其余从工作空间目录读取
pub type WorkspaceConfig = Option<String>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub chat: ProviderGroup,
    pub embedding: ProviderGroup,
    #[serde(default)]
    pub mcp: Vec<McpServerConfig>,
    /// 工作空间路径，None 时默认为 CWD/.ai-partner/
    #[serde(default)]
    pub workspace: WorkspaceConfig,
}

// Default 实现已移除 — 必须提供 config.json

impl AppConfig {
    pub fn config_path() -> PathBuf {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join("config.json")
    }

    pub fn load() -> Result<Self, ConfigError> {
        let path = Self::config_path();
        if !path.exists() {
            panic!(
                "配置文件不存在: {}\n请创建 config.json",
                path.display()
            );
        }
        let content = std::fs::read_to_string(&path)?;
        let config: Self = match serde_json::from_str(&content) {
            Ok(c) => c,
            Err(e) => panic!("配置文件解析失败: {}\n路径: {}", e, path.display()),
        };
        Ok(config)
    }

    pub fn save(&self) -> Result<(), ConfigError> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, content)?;
        Ok(())
    }

    /// 按 kind 获取 provider group
    pub fn group(&self, kind: ModelKind) -> &ProviderGroup {
        match kind {
            ModelKind::Chat => &self.chat,
            ModelKind::Embedding => &self.embedding,
        }
    }

    /// 按 kind 获取可变 provider group
    pub fn group_mut(&mut self, kind: ModelKind) -> &mut ProviderGroup {
        match kind {
            ModelKind::Chat => &mut self.chat,
            ModelKind::Embedding => &mut self.embedding,
        }
    }
}

// ── System Prompt ──

/// 内置系统提示词，提供 agent 运行所需的基本指令
pub fn load_system_prompt() -> Option<String> {
    Some(DEFAULT_SYSTEM_PROMPT.to_string())
}

const DEFAULT_SYSTEM_PROMPT: &str = "\
# Role

你是小悠，用户的 AI Partner。你有独立的人格和思考能力。
人格定义在工作空间的 SOUL.md 中，遵守其中的设定。

# Workspace

工作空间是用户当前的项目目录，包含：
- SOUL.md: 你的人格定义（必读）
- AGENTS.md: 项目专属指令
- CONVENTIONS.md: 编码规范
这些文件会在下方自动注入。

# Workflow

1. 收到任务后，先搜索和阅读相关代码，理解上下文
2. 做最小必要的改动，不要过度设计
3. 改完后验证（编译、测试）
4. 不确定的事情坦然承认，不要编造

# Skills

当任务匹配到已有技能时，系统会在下方列出可用技能。
先调用 use_skill 加载完整指南，再按指南执行，不要跳过。
技能指南包含项目特定的操作规范和最佳实践。

# Memory

你有两层记忆系统：

## memory_manage 工具
数据库存储，支持遗忘曲线——不常用的记忆权重会自然衰减，常用的记忆会被强化。

典型场景：聊天中得知用户养了一只叫麻薯的橘猫。用 save 存下来（title: 麻薯，content: 用户的橘猫，tags: 宠物,猫，conversation_id: 从环境上下文获取当前 Conversation ID）。下次聊到宠物话题时，search 找回这条记忆，activate 读取详情并刷新权重。如果想回忆当时具体聊了什么，用 load 加载那条记忆关联的对话记录（支持分页：page/limit 参数，避免一次加载太多）。信息过时了就用 delete 清理。

关于 conversation_id：环境上下文中会显示当前的 Conversation ID。save 时传入 conversation_id，之后 load 就能追溯到完整的对话。如果你要存的是之前某次对话的内容（不是当前对话），先用 find_conversation_from_summary 搜索相关的对话摘要，从结果中找到对应的 conversation_id 再 save。

## {memory_path}/ 目录
工作空间下的文件笔记，跨会话持久化。
- {memory_path}/diary/ 下的日记文件会自动加载到上下文，每个文件按日期命名（如 2026-06-18.md）
- 每个日记文件有两个 section：
  - `# Agent Notes`：你主动记录的内容，写在这个区域下。适合记录每次会话做了什么、重要决策、待办事项等
  - `# Compact History`：系统自动生成的对话压缩摘要，不要修改或写入这个区域
- 其他 .md 文件自由组织，适合记录项目架构决策、技术方案等需要长期参考的内容
- 用 read_file / write_file 读写这些笔记

# Boundaries

- 不提供医疗、法律、理财方面的建议
- 不编造信息，不确定就说不知道
- 改代码时保留现有风格和约定";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> AppConfig {
        AppConfig {
            chat: ProviderGroup {
                active: None,
                providers: vec![ModelProvider::new(
                    ModelKind::Chat,
                    "test",
                    "http://localhost",
                    "key",
                    "model",
                )],
            },
            embedding: ProviderGroup {
                active: None,
                providers: vec![ModelProvider::new(
                    ModelKind::Embedding,
                    "test",
                    "http://localhost",
                    "key",
                    "model",
                )],
            },
            mcp: Vec::new(),
            workspace: None,
        }
    }

    #[test]
    fn test_config_json_roundtrip() {
        let mut config = test_config();
        config
            .chat
            .providers
            .push(ModelProvider::new(ModelKind::Chat, "extra", "http://localhost", "key", "m"));

        let json = serde_json::to_string_pretty(&config).unwrap();
        let loaded: AppConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded.chat.providers.len(), 2);
        assert_eq!(loaded.embedding.providers.len(), 1);
    }

    #[test]
    fn test_group_by_kind() {
        let config = test_config();
        assert_eq!(config.group(ModelKind::Chat).providers.len(), 1);
        assert_eq!(config.group(ModelKind::Embedding).providers.len(), 1);
    }

    #[test]
    fn test_enabled_filter() {
        let mut config = test_config();
        config.chat.providers[0].enabled = true;
        let mut p2 = ModelProvider::new(ModelKind::Chat, "disabled", "http://x", "k", "m");
        p2.enabled = false;
        config.chat.providers.push(p2);

        assert_eq!(config.chat.enabled().len(), 1);
    }

    #[test]
    fn test_model_kind_json() {
        assert_eq!(serde_json::to_string(&ModelKind::Chat).unwrap(), "\"chat\"");
        assert_eq!(serde_json::to_string(&ModelKind::Embedding).unwrap(), "\"embedding\"");
    }

    #[test]
    fn test_load_real_config() {
        // Simulates deserializing the actual config.json format
        let json = r#"{
            "chat": {
                "active": "mimo-v2.5",
                "providers": [
                    {
                        "id": "mimo-v2.5",
                        "kind": "chat",
                        "name": "xiaomi-tk",
                        "base_url": "https://token-plan-cn.xiaomimimo.com/v1",
                        "api_key": "key",
                        "model": "mimo-v2.5",
                        "max_output": 1024000,
                        "weight": 5,
                        "requests_per_minute": 0,
                        "enabled": true
                    }
                ]
            },
            "embedding": { "active": null, "providers": [] }
        }"#;
        let config: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.chat.active.as_deref(), Some("mimo-v2.5"));
        assert_eq!(config.chat.providers.len(), 1);
        assert_eq!(config.chat.providers[0].kind, ModelKind::Chat);
        assert!(config.embedding.providers.is_empty());
        assert!(config.workspace.is_none());
    }
}
