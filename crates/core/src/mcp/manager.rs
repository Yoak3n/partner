use std::collections::HashMap;

use ai_partner_shared::{ToolCall, ToolDefinition};

use super::{McpClient, McpError, McpTool};

/// 管理多个 MCP 服务器连接，合并工具定义，路由工具调用
pub struct McpManager {
    /// server_name → (client, tools)
    servers: HashMap<String, (McpClient, Vec<McpTool>)>,
}

impl McpManager {
    pub fn new() -> Self {
        Self {
            servers: HashMap::new(),
        }
    }

    /// 连接 MCP 服务器：spawn 进程 → initialize → list_tools
    pub async fn connect(
        &mut self,
        name: &str,
        command: &str,
        args: &[String],
    ) -> Result<(), McpError> {
        let mut client = McpClient::connect(command, args).await?;
        let info = client.initialize().await?;
        let tools = client.list_tools().await?;

        let server_name = if info.name.is_empty() {
            name.to_string()
        } else {
            info.name
        };

        log::info!(
            "MCP server '{}' connected: {} tools",
            server_name,
            tools.len()
        );

        self.servers.insert(server_name, (client, tools));
        Ok(())
    }

    /// 合并所有服务器的工具定义，转换为 ToolDefinition
    pub fn all_tool_definitions(&self) -> Vec<ToolDefinition> {
        self.servers
            .values()
            .flat_map(|(_, tools)| tools)
            .map(|t| ToolDefinition {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.input_schema.clone(),
            })
            .collect()
    }

    /// 检查工具名是否来自 MCP（而非内置）
    pub fn is_mcp_tool(&self, tool_name: &str) -> bool {
        self.servers
            .values()
            .any(|(_, tools)| tools.iter().any(|t| t.name == tool_name))
    }

    /// 调用 MCP 工具（路由到正确的服务器）
    pub async fn call_tool(
        &mut self,
        call: &ToolCall,
    ) -> Result<String, McpError> {
        for (_name, (client, tools)) in &mut self.servers {
            if tools.iter().any(|t| t.name == call.name) {
                return client.call_tool(&call.name, call.arguments.clone()).await;
            }
        }
        Err(McpError::Protocol(format!(
            "tool '{}' not found in any MCP server",
            call.name
        )))
    }

    /// 断开所有服务器
    pub async fn disconnect_all(&mut self) {
        for (_, (client, _)) in &mut self.servers {
            client.shutdown().await;
        }
        self.servers.clear();
    }

    /// 已连接的服务器名称列表
    pub fn server_names(&self) -> Vec<&str> {
        self.servers.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}
