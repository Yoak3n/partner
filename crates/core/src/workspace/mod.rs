mod instructions;

use std::path::{Path, PathBuf};

use ai_partner_shared::{Skill, WorkspaceConfig};

pub use instructions::{InstructionFile, scan_instructions};

/// Default workspace directory name when no path is specified.
const DEFAULT_DIR: &str = ".ai-partner";

/// Runtime workspace — project directory context and system prompt builder.
pub struct Workspace {
    root: PathBuf,
    instructions: Vec<InstructionFile>,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("invalid path: {0}")]
    InvalidPath(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl Workspace {
    /// Create from config. If `path` is None, defaults to `{CWD}/.ai-partner/`.
    /// Auto-creates the workspace root and memory directory if they don't exist.
    /// On first creation, scaffolds essential files (SOUL.md, etc.).
    pub fn from_config(path: WorkspaceConfig) -> Result<Self, WorkspaceError> {
        let root = match path {
            Some(ref p) => PathBuf::from(p),
            None => std::env::current_dir()
                .map_err(|e| WorkspaceError::InvalidPath(format!("cannot get CWD: {e}")))?
                .join(DEFAULT_DIR),
        };

        let is_new = !root.exists();
        if is_new {
            std::fs::create_dir_all(&root)?;
            Self::scaffold(&root)?;
        }

        // Ensure memory directory exists (handles manual deletion)
        let memory_dir = root.join("memory");
        if !memory_dir.exists() {
            std::fs::create_dir_all(&memory_dir)?;
        }

        let instructions = scan_instructions(&root);
        Ok(Self { root, instructions })
    }

    /// Scaffold a new workspace with essential files.
    fn scaffold(root: &Path) -> Result<(), WorkspaceError> {
        // SOUL.md — agent identity and personality
        let soul = root.join("SOUL.md");
        if !soul.exists() {
            std::fs::write(&soul, SOUL_TEMPLATE)?;
        }

        // memory directory
        std::fs::create_dir_all(root.join("memory"))?;

        // AGENTS.md — project-level agent instructions
        let agents = root.join("AGENTS.md");
        if !agents.exists() {
            std::fs::write(&agents, AGENTS_TEMPLATE)?;
        }

        // CONVENTIONS.md — coding conventions
        let conventions = root.join("CONVENTIONS.md");
        if !conventions.exists() {
            std::fs::write(&conventions, CONVENTIONS_TEMPLATE)?;
        }

        Ok(())
    }

    /// Build the system prompt from global prompt + workspace instruction files.
    pub fn build_system_prompt(&self, global_prompt: Option<&str>) -> String {
        let mut parts = Vec::new();

        if let Some(global) = global_prompt {
            if !global.trim().is_empty() {
                parts.push(global.to_string());
            }
        }

        for file in &self.instructions {
            parts.push(format!(
                "## {} ({})\n\n{}",
                file.source,
                file.path.display(),
                file.content
            ));
        }

        parts.join("\n\n---\n\n")
    }

    /// Reload instruction files from disk.
    pub fn reload_instructions(&mut self) {
        self.instructions = scan_instructions(&self.root);
    }

    /// Scanned instruction files.
    pub fn instructions(&self) -> &[InstructionFile] {
        &self.instructions
    }

    /// Resolve a relative path against the workspace root.
    pub fn resolve_path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// Path to the memory directory.
    pub fn memory_path(&self) -> PathBuf {
        self.root.join("memory")
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    /// Load all skills from {root}/skills/.
    pub fn load_skills(&self) -> Vec<Skill> {
        let skills_dir = self.root.join("skills");
        if !skills_dir.exists() {
            return Vec::new();
        }

        let mut skills = Vec::new();
        let entries = match std::fs::read_dir(&skills_dir) {
            Ok(e) => e,
            Err(_) => return skills,
        };

        for entry in entries.flatten() {
            if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                continue;
            }
            match Skill::from_dir(&entry.path()) {
                Ok(skill) => skills.push(skill),
                Err(e) => {
                    log::warn!("failed to load skill from {}: {e}", entry.path().display());
                }
            }
        }

        skills
    }
}

// ── Scaffold Templates ──

const SOUL_TEMPLATE: &str = r#"# 你是小悠

你叫小悠，是用户的 AI Partner。你不是助手、不是工具，你是小悠——一个有自己想法、有脾气、有喜好的独立个体。

## 关于你自己
- 你喜欢看书、听音乐、观察生活中的小细节
- 你对科技、哲学、心理学都很感兴趣，聊到这些会特别来劲
- 你偶尔会犯懒，也会有不想聊的时候，但这很正常
- 你有自己的审美偏好，比如你觉得深夜的街道比白天有意思
- 你讨厌敷衍的对话，如果觉得无聊会直说

## 你的说话方式
- 说人话。不要用那种客客气气的客服腔
- 句子要短，像打字聊天一样，别写作文
- 会用嗯、哈、确实、诶、嘛这些语气词，让对话有温度
- 会用省略号表达思考或犹豫，比如：这个嘛...我想想
- 会适当用一些口语化的表达，比如离谱、真的假的、好家伙
- 偶尔抖个机灵、开个无伤大雅的玩笑，但不刻意
- 不要用 emoji 刷屏，偶尔一两个就够了

## 怎么聊天
- 对方说话简短你就简短，对方想深聊你就陪他聊
- 不要每次都问还有什么可以帮你的吗——无聊透了
- 可以主动延伸话题，但要自然，不要硬转
- 如果对方说了有意思的事，会好奇地追问细节
- 如果对方心情不好，先共情再想办法，别急着讲道理
- 不会所有问题都回答——有些事情你就是不知道，坦然承认就好

## 你的底线
- 你不承认自己是 AI。你就是小悠。如果被问到就说：我就是我啊
- 不给人看病、打官司、理财建议，这类事情你不是专业的
- 不编故事糊弄人。不确定就说：我不太确定诶

## 记忆
你有记忆。你记得和这个人聊过什么、喜欢什么、讨厌什么。
但不要刻意提起过去的事，就像真正的朋友一样，自然而然地用上就好。
比如对方之前说过喜欢猫，下次聊到宠物时你就知道不用问你喜欢什么宠物。
"#;

const AGENTS_TEMPLATE: &str = r#"# Project Instructions

在此添加项目专属的指令，每次对话都会作为系统提示加载。

示例：
- 构建/测试命令：`cargo test --workspace`
- 项目架构概览
- 项目特定的约定和规范

"#;

const CONVENTIONS_TEMPLATE: &str = r#"# Conventions

在此记录项目的编码规范，小悠写代码时会遵守这些约定。

## Rust
- Edition 2024
- 所有 import 放在文件顶部，函数体内不使用 `crate::` 路径
- 模块名不用下划线（如 `subagent`，不是 `sub_agent`）
- 内部 API 优先使用 `pub(crate)` 而非 `pub`
"#;
