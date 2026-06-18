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

/// Max number of recent diary files to inject into system prompt.
const MAX_DIARY_FILES: usize = 3;

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

    files.extend(scan_diary(root));

    files
}

/// Scan `memory/diary/` for recent diary files (sorted by date, newest first).
/// Only the most recent `MAX_DIARY_FILES` are included.
/// Each file has two sections: compact history (auto-generated) and agent notes.
fn scan_diary(root: &Path) -> Vec<InstructionFile> {
    let diary_dir = root.join("memory").join("diary");
    if !diary_dir.exists() {
        return Vec::new();
    }

    let mut entries: Vec<PathBuf> = match std::fs::read_dir(&diary_dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().map_or(false, |ext| ext == "md"))
            .collect(),
        Err(_) => return Vec::new(),
    };

    entries.sort();
    entries.reverse(); // newest first

    entries
        .into_iter()
        .take(MAX_DIARY_FILES)
        .filter_map(|path| {
            let content = std::fs::read_to_string(&path).ok()?;
            if content.trim().is_empty() {
                return None;
            }
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("diary");
            Some(InstructionFile {
                source: format!("Session Diary ({name})"),
                path,
                content,
            })
        })
        .collect()
}
