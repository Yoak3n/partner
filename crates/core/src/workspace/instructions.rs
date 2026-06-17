use std::path::{Path, PathBuf};

/// Well-known instruction file names that workspace projects may contain.
const INSTRUCTION_FILES: &[(&str, &str)] = &[
    ("SOUL.md", "Soul"),
    ("AGENTS.md", "Project Agents"),
    ("CLAUDE.md", "Project Claude"),
    (".cursorrules", "Cursor Rules"),
    (".clinerules", "Cline Rules"),
    (".windsurfrules", "Windsurf Rules"),
    ("CONVENTIONS.md", "Project Conventions"),
    ("CONTRIBUTING.md", "Contributing Guide"),
];

/// A scanned instruction file from the workspace.
#[derive(Debug, Clone)]
pub struct InstructionFile {
    /// Display name (e.g. "Project Agents").
    pub source: String,
    /// Absolute path to the file.
    pub path: PathBuf,
    /// File content.
    pub content: String,
}

/// Scan the workspace root for well-known instruction files.
/// Returns all files that exist and are non-empty.
pub fn scan_instructions(root: &Path) -> Vec<InstructionFile> {
    let mut files = Vec::new();

    for &(name, source) in INSTRUCTION_FILES {
        let path = root.join(name);
        if !path.exists() {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Ok(content) if !content.trim().is_empty() => {
                files.push(InstructionFile {
                    source: source.to_string(),
                    path,
                    content,
                });
            }
            _ => continue,
        }
    }

    files
}
