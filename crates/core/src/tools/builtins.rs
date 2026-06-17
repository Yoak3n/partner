use std::path::Path;
use std::sync::Arc;

use ai_partner_shared::{AgentEvent, MemoryEntry, Storage, ToolDefinition};
use serde_json::json;
use tokio::sync::mpsc;

use crate::provider::EmbeddingAdapter;
use super::registry::ToolRegistry;

/// Register all builtin tools into the registry.
///
/// `event_tx` is needed by `run_command` for subprocess output streaming.
pub fn register_builtins(
    registry: &mut ToolRegistry,
    event_tx: mpsc::UnboundedSender<AgentEvent>,
) {
    registry.register(read_file_def(), |args| {
        let path = args["path"]
            .as_str()
            .ok_or("missing 'path' argument")?;
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read '{path}': {e}"))?;
        Ok(content)
    });

    registry.register(write_file_def(), |args| {
        let path = args["path"]
            .as_str()
            .ok_or("missing 'path' argument")?;
        let content = args["content"]
            .as_str()
            .ok_or("missing 'content' argument")?;
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create dirs: {e}"))?;
        }
        std::fs::write(path, content)
            .map_err(|e| format!("failed to write '{path}': {e}"))?;
        Ok(format!("wrote {} bytes to {path}", content.len()))
    });

    registry.register(search_files_def(), |args| {
        let pattern = args["pattern"]
            .as_str()
            .ok_or("missing 'pattern' argument")?;
        let dir = args["dir"]
            .as_str()
            .unwrap_or(".");

        let mut results = Vec::new();
        visit_dir(dir, pattern, &mut results, 0)?;
        Ok(results.join("\n"))
    });

    // run_command: async tool using ProcessManager
    let tx = event_tx;
    registry.register_async(run_command_def(), move |args, pm| {
        let tx = tx.clone();
        Box::pin(async move {
            let command = args["command"]
                .as_str()
                .ok_or("missing 'command' argument")?
                .to_string();

            let process_id = pm.spawn(&command, &tx).await?;

            // Wait for process to finish by polling status
            let exit_status = loop {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                if let Some(status) = pm.status(&process_id).await {
                    match &status {
                        ai_partner_shared::ProcessStatus::Running => continue,
                        _ => break Some(status),
                    }
                } else {
                    break None;
                }
            };

            let output = pm.output(&process_id).await.unwrap_or_default();
            pm.remove(&process_id).await;

            if output.is_empty() {
                if let Some(ai_partner_shared::ProcessStatus::Exited(code)) = exit_status {
                    return Ok(format!("(exit code: {code})"));
                }
                return Ok("(no output)".to_string());
            }

            Ok(output.join("\n"))
        })
    });
}

/// Register the unified memory_manage tool backed by Storage.
pub fn register_memory_manage(
    registry: &mut ToolRegistry,
    storage: Arc<Storage>,
    embedding: Option<Arc<dyn EmbeddingAdapter>>,
) {
    registry.register_async(memory_manage_def(), move |args, _pm| {
        let db = storage.clone();
        let embedding = embedding.clone();
        Box::pin(async move {
            let action = args["action"]
                .as_str()
                .ok_or("missing 'action' argument (save/load/list/search/activate/delete/query)")?;

            match action {
                "save" => {
                    let title = args["title"]
                        .as_str()
                        .ok_or("missing 'title' for save")?;
                    let content = args["content"]
                        .as_str()
                        .ok_or("missing 'content' for save")?;
                    let id = args["id"].as_str();
                    let tags = args["tags"].as_str();
                    let conversation_id = args["conversation_id"].as_str();
                    let new_id = db.save_memory(id, title, content, tags, conversation_id)
                        .map_err(|e| format!("failed to save memory: {e}"))?;
                    Ok(format!("saved memory: {new_id}"))
                }
                "activate" => {
                    let id = args["id"]
                        .as_str()
                        .ok_or("missing 'id' for activate")?;
                    match db.activate_memory(id)
                        .map_err(|e| format!("failed to activate memory: {e}"))?
                    {
                        Some(entry) => Ok(format_memory(&entry)),
                        None => Err(format!("memory '{id}' not found")),
                    }
                }
                "load" => {
                    let id = args["id"]
                        .as_str()
                        .ok_or("missing 'id' for load")?;
                    let entry = db.get_memory(id)
                        .map_err(|e| format!("failed to get memory: {e}"))?
                        .ok_or_else(|| format!("memory '{id}' not found"))?;
                    let session_id = entry.session_id
                        .as_deref()
                        .ok_or_else(|| format!("memory '{id}' has no associated session"))?;
                    let messages = db.load_messages(session_id)
                        .map_err(|e| format!("failed to load session: {e}"))?;
                    if messages.is_empty() {
                        return Ok(format!("session {session_id} has no messages"));
                    }
                    let lines: Vec<String> = messages.iter().map(|m| {
                        format!("[{}] {:?}: {}", &m.id.to_string()[..8], m.role, m.content)
                    }).collect();
                    Ok(format!("session: {}\n---\n{}", session_id, lines.join("\n")))
                }
                "list" => {
                    let limit = args["limit"].as_i64().unwrap_or(20);
                    let page = args["page"].as_i64().unwrap_or(1).max(1);
                    let offset = (page - 1) * limit;
                    let entries = db.list_memories(offset, limit)
                        .map_err(|e| format!("failed to list memories: {e}"))?;
                    if entries.is_empty() {
                        return Ok("(no memories)".to_string());
                    }
                    let lines: Vec<String> = entries.iter().map(|e| {
                        let tags = e.tags.as_deref().unwrap_or("");
                        let session = e.session_id.as_deref().unwrap_or("-");
                        format!("[{}] {} (w:{:.2}, act:{}, tags:{}, session:{})",
                            &e.id[..8], e.title, e.weight, e.activation_count,
                            if tags.is_empty() { "none" } else { tags },
                            &session[..session.len().min(8)])
                    }).collect();
                    Ok(format!("page {page} (showing {})\n{}", entries.len(), lines.join("\n")))
                }
                "search" => {
                    let query = args["query"]
                        .as_str()
                        .ok_or("missing 'query' for search")?;
                    let limit = args["limit"].as_i64().unwrap_or(20);
                    let page = args["page"].as_i64().unwrap_or(1).max(1);
                    let offset = (page - 1) * limit;
                    let entries = db.search_memories(query, offset, limit)
                        .map_err(|e| format!("failed to search memories: {e}"))?;
                    if entries.is_empty() {
                        return Ok(format!("no memories matching '{query}'"));
                    }
                    let lines: Vec<String> = entries.iter().map(format_memory).collect();
                    Ok(format!("page {page} (showing {})\n{}", entries.len(), lines.join("\n---\n")))
                }
                "delete" => {
                    let id = args["id"]
                        .as_str()
                        .ok_or("missing 'id' for delete")?;
                    let deleted = db.delete_memory(id)
                        .map_err(|e| format!("failed to delete memory: {e}"))?;
                    if deleted {
                        Ok(format!("deleted memory: {id}"))
                    } else {
                        Err(format!("memory '{id}' not found"))
                    }
                }
                "query" => {
                    let adapter = embedding.as_ref()
                        .ok_or("no embedding provider configured — cannot use query action")?;
                    let query_text = args["query"]
                        .as_str()
                        .ok_or("missing 'query' for query")?;
                    let limit = args["limit"].as_i64().unwrap_or(10) as usize;

                    let embed_vec = adapter.embed(query_text).await
                        .map_err(|e| format!("embedding failed: {e}"))?;
                    let results = db.search_documents(&embed_vec, limit)
                        .map_err(|e| format!("document search failed: {e}"))?;

                    if results.is_empty() {
                        return Ok(format!("no documents matched query: '{query_text}'"));
                    }

                    let lines: Vec<String> = results.iter().map(|r| {
                        format!(
                            "[doc:{}] session:{} score:{:.4}\n{}",
                            &r.document_id[..8.min(r.document_id.len())],
                            &r.session_id[..8.min(r.session_id.len())],
                            r.score,
                            r.content
                        )
                    }).collect();
                    Ok(format!("RAG results ({}):\n{}", results.len(), lines.join("\n---\n")))
                }
                _ => Err(format!("unknown action '{action}', use: save, activate, load, list, search, delete, query")),
            }
        })
    });
}

fn format_memory(e: &MemoryEntry) -> String {
    let tags = e.tags.as_deref().unwrap_or("");
    let session = e.session_id.as_deref().unwrap_or("(none)");
    format!(
        "[{}] {}\ntags: {}\nweight: {:.2} | activations: {} | last: {}\nsession: {}\ncreated: {}\n\n{}",
        e.id, e.title,
        if tags.is_empty() { "none" } else { tags },
        e.weight, e.activation_count, e.last_activated_at,
        session,
        e.created_at, e.content
    )
}

fn visit_dir(
    dir: &str,
    pattern: &str,
    results: &mut Vec<String>,
    depth: usize,
) -> Result<(), String> {
    if depth > 10 {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|e| format!("failed to read dir: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("entry error: {e}"))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            if name == ".git" || name == "node_modules" || name == "target" {
                continue;
            }
            visit_dir(&path.to_string_lossy(), pattern, results, depth + 1)?;
        } else if name.contains(pattern) {
            results.push(path.to_string_lossy().to_string());
        }
    }
    Ok(())
}

fn read_file_def() -> ToolDefinition {
    ToolDefinition {
        name: "read_file".into(),
        description: "Read the contents of a file".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read"
                }
            },
            "required": ["path"]
        }),
    }
}

fn write_file_def() -> ToolDefinition {
    ToolDefinition {
        name: "write_file".into(),
        description: "Write content to a file, creating parent directories if needed".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write to the file"
                }
            },
            "required": ["path", "content"]
        }),
    }
}

fn search_files_def() -> ToolDefinition {
    ToolDefinition {
        name: "search_files".into(),
        description: "Search for files by name pattern in a directory (recursive, skips .git/node_modules/target)".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Substring to match in file names"
                },
                "dir": {
                    "type": "string",
                    "description": "Directory to search in (default: current directory)"
                }
            },
            "required": ["pattern"]
        }),
    }
}

fn run_command_def() -> ToolDefinition {
    ToolDefinition {
        name: "run_command".into(),
        description: "Execute a shell command and return its output".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                }
            },
            "required": ["command"]
        }),
    }
}

fn memory_manage_def() -> ToolDefinition {
    ToolDefinition {
        name: "memory_manage".into(),
        description: "Manage persistent memories with weighted forgetting curve. Actions: save (create/update), activate (read content + boost weight), load (load conversation by memory id), list (all by weight), search (text match), delete, query (RAG vector similarity search). Unused memories decay over time.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["save", "load", "list", "search", "activate", "delete", "query"],
                    "description": "Operation to perform"
                },
                "id": {
                    "type": "string",
                    "description": "Memory ID (required for load/activate/delete, optional for save to update)"
                },
                "title": {
                    "type": "string",
                    "description": "Memory title (required for save)"
                },
                "content": {
                    "type": "string",
                    "description": "Memory content (required for save)"
                },
                "tags": {
                    "type": "string",
                    "description": "Comma-separated tags (optional, for save)"
                },
                "conversation_id": {
                    "type": "string",
                    "description": "ID of the conversation that created this memory (optional, for save)"
                },
                "query": {
                    "type": "string",
                    "description": "Search query (required for search/query)"
                },
                "page": {
                    "type": "integer",
                    "description": "Page number, 1-indexed (default: 1)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Items per page (default: 20 for list/search, 10 for query)"
                }
            },
            "required": ["action"]
        }),
    }
}
