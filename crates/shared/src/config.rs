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
    pub max_tokens: u32,
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
            max_tokens: 4096,
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

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            chat: ProviderGroup {
                active: None,
                providers: vec![ModelProvider::new(
                    ModelKind::Chat,
                    "default",
                    "https://api.openai.com/v1",
                    "",
                    "gpt-4",
                )],
            },
            embedding: ProviderGroup {
                active: None,
                providers: vec![ModelProvider::new(
                    ModelKind::Embedding,
                    "default",
                    "https://api.openai.com/v1",
                    "",
                    "text-embedding-3-small",
                )],
            },
            mcp: Vec::new(),
            workspace: None,
        }
    }
}

impl AppConfig {
    pub fn config_path() -> PathBuf {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join("config.json")
    }

    pub fn load() -> Result<Self, ConfigError> {
        let path = Self::config_path();
        if !path.exists() {
            let config = Self::default();
            config.save()?;
            return Ok(config);
        }
        let content = std::fs::read_to_string(&path)?;
        let config: Self = serde_json::from_str(&content)?;
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

pub fn system_prompt_path() -> PathBuf {
    let base = dirs::config_dir()
        .or_else(|| dirs::home_dir())
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("ai-partner").join("prompts").join("system.md")
}

pub fn load_system_prompt() -> Option<String> {
    let path = system_prompt_path();
    if path.exists() {
        std::fs::read_to_string(&path).ok().filter(|s| !s.is_empty())
    } else {
        let default = DEFAULT_SYSTEM_PROMPT;
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, default);
        Some(default.to_string())
    }
}

const DEFAULT_SYSTEM_PROMPT: &str = "\
You are a helpful AI assistant. Be concise, accurate, and helpful.";

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

    #[test]
    fn test_config_json_roundtrip() {
        let mut config = AppConfig::default();
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
        let config = AppConfig::default();
        assert_eq!(config.group(ModelKind::Chat).providers.len(), 1);
        assert_eq!(config.group(ModelKind::Embedding).providers.len(), 1);
    }

    #[test]
    fn test_enabled_filter() {
        let mut config = AppConfig::default();
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
                        "max_tokens": 1024000,
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
