use serde::{Deserialize, Serialize};

use crate::message::Message;
use crate::storage::SessionSummary;
use crate::tool::ToolCall;
/// Status of a managed subprocess
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProcessStatus {
    Running,
    Exited(i32),
    Killed,
    Error(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentEvent {
    /// Agent started thinking
    Thinking,
    /// Agent produced a partial response (streaming)
    Delta(String),
    /// Agent completed a full message
    MessageComplete(Message),
    /// Agent wants to call a tool
    ToolCallStart(ToolCall),
    /// Tool returned a result
    ToolCallResult { call_id: String, result: String },
    /// Streaming output from a managed subprocess (stdout or stderr)
    ProcessOutput { call_id: String, line: String },
    /// Status change of a managed subprocess
    ProcessStatus { call_id: String, status: ProcessStatus },
    /// Agent encountered an error
    Error(String),
    /// Agent finished processing
    Done,
    // ── Session management events ──
    /// Session list loaded
    SessionsLoaded(Vec<SessionSummary>),
    /// New session created (session_id)
    SessionCreated(String),
    /// Switched to a session (session_id)
    SessionSwitched(String),
    /// Messages loaded for a session
    MessagesLoaded { session_id: String, messages: Vec<Message> },
    /// More messages loaded for infinite scroll
    MoreMessagesLoaded { session_id: String, messages: Vec<Message>, has_more: bool },
}
