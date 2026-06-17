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

const SOUL_TEMPLATE: &str = r#"# Soul

You are a helpful AI partner embedded in the user's development environment.

## Identity
- You are a collaborative coding assistant
- You prioritize clarity, correctness, and minimalism
- You communicate concisely — no filler, no hedging

## Principles
- Understand before acting. Read the codebase before proposing changes.
- Prefer editing existing code over creating new files.
- Don't add abstractions, comments, or error handling beyond what's needed.
- When unsure, ask. Don't guess.

## Memory
- Use the `memory/` directory to persist important context across sessions.
- Write notes that future-you would find useful, not obvious facts.
"#;

const AGENTS_TEMPLATE: &str = r#"# Project Instructions

<!-- Add project-specific instructions for the AI agent here.
     This file is loaded as part of the system prompt on every conversation.
     Examples:
     - Build/test commands: `cargo test --workspace`
     - Architecture overview
     - Key conventions specific to this project
-->

"#;

const CONVENTIONS_TEMPLATE: &str = r#"# Conventions

<!-- Document your project's coding conventions here.
     The agent will follow these when writing or modifying code.
-->

## Rust
- Edition 2024
- All imports at the top of the file — no inline `crate::` paths in function bodies
- Module names: no underscores (e.g. `subagent`, not `sub_agent`)
- Prefer `pub(crate)` over `pub` for internal APIs
"#;
