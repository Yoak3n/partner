use serde::{Deserialize, Serialize};

use crate::message::Message;
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
}
