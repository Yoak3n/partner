pub mod manager;

use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, Command};

// ── JSON-RPC 协议类型 ──

#[derive(Debug, Serialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Option<u64>,
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON-RPC error {}: {}", self.code, self.message)
    }
}

// ── MCP 工具定义 ──

#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

// ── MCP 服务器信息 ──

#[derive(Debug, Deserialize)]
pub struct McpServerInfo {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
}

// ── tools/list 响应 ──

#[derive(Debug, Deserialize)]
struct ToolsListResult {
    tools: Vec<McpToolDef>,
}

#[derive(Debug, Deserialize)]
struct McpToolDef {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default, rename = "inputSchema")]
    input_schema: Value,
}

// ── tools/call 响应 ──

#[derive(Debug, Deserialize)]
struct ToolCallResult {
    content: Vec<ToolCallContent>,
}

#[derive(Debug, Deserialize)]
struct ToolCallContent {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(default)]
    text: String,
}

// ── 错误类型 ──

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("RPC error: {0}")]
    Rpc(String),

    #[error("Protocol error: {0}")]
    Protocol(String),
}

// ── McpClient ──

pub struct McpClient {
    child: Child,
    stdin: BufWriter<tokio::process::ChildStdin>,
    reader: tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    next_id: AtomicU64,
    server_name: String,
}

impl McpClient {
    /// 连接 MCP 服务器（stdio 传输）
    pub async fn connect(
        command: &str,
        args: &[String],
    ) -> Result<Self, McpError> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;

        let stdin = BufWriter::new(child.stdin.take().expect("stdin not captured"));
        let stdout = child.stdout.take().expect("stdout not captured");
        let reader = BufReader::new(stdout).lines();

        Ok(Self {
            child,
            stdin,
            reader,
            next_id: AtomicU64::new(1),
            server_name: String::new(),
        })
    }

    /// 发送 JSON-RPC 请求并等待响应
    async fn request(
        &mut self,
        method: &str,
        params: Option<Value>,
    ) -> Result<Value, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);

        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params,
        };

        let mut line = serde_json::to_string(&request)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;

        // 读取响应（跳过非 JSON-RPC 的日志行）
        loop {
            let raw = self
                .reader
                .next_line()
                .await?
                .ok_or_else(|| McpError::Protocol("connection closed".into()))?;

            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }

            // 尝试解析为 JSON-RPC 响应
            if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(raw) {
                if resp.id == Some(id) {
                    if let Some(err) = resp.error {
                        return Err(McpError::Rpc(err.to_string()));
                    }
                    return Ok(resp.result.unwrap_or(Value::Null));
                }
            }
            // 非目标响应（如日志），继续读
        }
    }

    /// 初始化 MCP 连接
    pub async fn initialize(&mut self) -> Result<McpServerInfo, McpError> {
        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "ai-partner",
                "version": "0.1.0"
            }
        });

        let result = self.request("initialize", Some(params)).await?;
        let info: McpServerInfo = serde_json::from_value(result)?;

        // 发送 initialized 通知
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        });
        let mut line = serde_json::to_string(&notification)?;
        line.push('\n');
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.flush().await?;

        self.server_name = info.name.clone();
        Ok(info)
    }

    /// 列出服务器提供的工具
    pub async fn list_tools(&mut self) -> Result<Vec<McpTool>, McpError> {
        let result = self.request("tools/list", None).await?;
        let list: ToolsListResult = serde_json::from_value(result)?;

        Ok(list
            .tools
            .into_iter()
            .map(|t| McpTool {
                name: t.name,
                description: t.description,
                input_schema: t.input_schema,
            })
            .collect())
    }

    /// 调用服务器上的工具
    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: Value,
    ) -> Result<String, McpError> {
        let params = serde_json::json!({
            "name": name,
            "arguments": arguments
        });

        let result = self.request("tools/call", Some(params)).await?;
        let call_result: ToolCallResult = serde_json::from_value(result)?;

        // 合并所有 content 的 text
        let text: String = call_result
            .content
            .into_iter()
            .filter(|c| c.content_type == "text")
            .map(|c| c.text)
            .collect::<Vec<_>>()
            .join("\n");

        Ok(text)
    }

    /// 服务器名称
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// 关闭连接
    pub async fn shutdown(&mut self) {
        let _ = self.child.kill().await;
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}
