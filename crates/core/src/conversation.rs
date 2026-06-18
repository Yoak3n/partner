use ai_partner_shared::{Message, Role};
use serde_json::Value;

/// Truncate string values inside a JSON object to `max_chars` per value.
/// Preserves structure (keys, numbers, bools, nulls) — only string payloads are cut.
fn truncate_json_values(value: &Value, max_chars: usize) -> Value {
    match value {
        Value::String(s) => {
            let truncated: String = s.chars().take(max_chars).collect();
            if truncated.len() < s.len() {
                Value::String(format!("{}...", truncated))
            } else {
                Value::String(truncated)
            }
        }
        Value::Array(arr) => {
            Value::Array(arr.iter().map(|v| truncate_json_values(v, max_chars)).collect())
        }
        Value::Object(map) => {
            let truncated: serde_json::Map<String, Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), truncate_json_values(v, max_chars)))
                .collect();
            Value::Object(truncated)
        }
        other => other.clone(),
    }
}

/// Manages in-memory conversation state: compaction threshold checks,
/// message eviction, and tool content compression.
pub struct ConversationManager {
    /// Message count threshold that triggers compaction.
    pub max_messages: usize,
    /// Number of recent messages to keep when compacting.
    pub min_recent_messages: usize,
    /// Max characters to keep from tool results/arguments when compressing.
    pub tool_result_truncate: usize,
}

impl ConversationManager {
    pub fn new() -> Self {
        Self {
            max_messages: 100,
            min_recent_messages: 20,
            tool_result_truncate: 200,
        }
    }

    /// Check whether the message list exceeds the compaction threshold.
    pub fn should_compact(&self, messages: &[Message]) -> bool {
        messages.len() > self.max_messages
    }

    /// Split messages into (evicted, kept). The last `keep_count` messages are kept.
    pub fn evict_old_messages(
        &self,
        messages: &[Message],
        keep_count: usize,
    ) -> (Vec<Message>, Vec<Message>) {
        if messages.len() <= keep_count {
            return (Vec::new(), messages.to_vec());
        }
        let split_at = messages.len() - keep_count;
        let evicted = messages[..split_at].to_vec();
        let kept = messages[split_at..].to_vec();
        (evicted, kept)
    }

    /// Format evicted messages into a prompt for the compaction sub-agent.
    pub fn build_compaction_prompt(&self, evicted: &[Message]) -> String {
        let mut lines = Vec::new();
        lines.push(
            "Please compress the following conversation history into a concise summary. \
             Preserve all key information: decisions made, facts discussed, tasks assigned, \
             code changes, errors encountered, and any important context. \
             Output ONLY the summary, no preamble."
                .to_string(),
        );
        lines.push("---".to_string());

        for msg in evicted {
            let role_label = match msg.role {
                Role::User => "User",
                Role::Assistant => "Assistant",
                Role::System => "System",
                Role::Tool => "Tool",
            };

            let mut line = format!("[{}]", role_label);

            if !msg.content.is_empty() {
                line.push(' ');
                line.push_str(&msg.content);
            }

            if let Some(ref tool_calls) = msg.tool_calls {
                let names: Vec<&str> = tool_calls.iter().map(|tc| tc.name.as_str()).collect();
                line.push_str(&format!(" (tool_calls: {})", names.join(", ")));
            }

            if let Some(ref tool_id) = msg.tool_call_id {
                line.push_str(&format!(" (tool_call_id: {})", tool_id));
            }

            lines.push(line);
        }

        lines.join("\n")
    }

    /// Compress a message to reduce context usage (truncates tool payloads).
    pub fn compress_message(&self, msg: &Message) -> Message {
        match msg.role {
            Role::Tool => {
                let truncated: String = msg.content.chars().take(self.tool_result_truncate).collect();
                let content = if truncated.len() < msg.content.len() {
                    format!("{}...", truncated)
                } else {
                    truncated
                };
                Message {
                    content,
                    ..msg.clone()
                }
            }
            Role::Assistant if msg.tool_calls.is_some() => {
                let tool_calls = msg.tool_calls.as_ref().map(|calls| {
                    calls
                        .iter()
                        .map(|tc| ai_partner_shared::ToolCall {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            arguments: truncate_json_values(&tc.arguments, self.tool_result_truncate),
                        })
                        .collect()
                });
                Message {
                    content: String::new(),
                    tool_calls,
                    ..msg.clone()
                }
            }
            _ => msg.clone(),
        }
    }
}
