use ai_partner_shared::{AgentEvent, Message, ToolCall};

use crate::hook::{HookContext, HookResult};
use crate::state::AgentState;
use crate::subagent::SubAgentContext;

use super::Runtime;

pub(crate) enum ToolExecutionResult {
    Continue,
    Abort,
}

impl Runtime {
    /// Execute a batch of tool calls: before hooks → parallel builtin + sequential MCP → after hooks → store results.
    pub(crate) async fn execute_tool_calls(
        &mut self,
        ctx: &HookContext<'_>,
        calls: &[ToolCall],
        round: usize,
        max_rounds: usize,
    ) -> ToolExecutionResult {
        // before_tool_call hooks
        let mut allowed_calls = Vec::new();
        for call in calls {
            match self.run_hooks_before_tool(ctx, call).await {
                HookResult::Skip => continue,
                HookResult::Abort(reason) => {
                    self.state = AgentState::Error { message: reason.clone() };
                    self.run_hooks_on_error(&reason).await;
                    let _ = self.event_tx.send(AgentEvent::Error(reason));
                    return ToolExecutionResult::Abort;
                }
                HookResult::Continue => allowed_calls.push(call.clone()),
            }
        }

        if allowed_calls.is_empty() {
            if round == max_rounds - 1 {
                let err = format!("Max tool rounds ({max_rounds}) exceeded");
                self.state = AgentState::Error { message: err.clone() };
                self.run_hooks_on_error(&err).await;
                let _ = self.event_tx.send(AgentEvent::Error(err));
                return ToolExecutionResult::Abort;
            }
            return ToolExecutionResult::Continue;
        }

        self.state = AgentState::UsingTool {
            tool_name: format!("{} tools", allowed_calls.len()),
        };

        // Split into MCP and builtin calls
        let mut mcp_calls = Vec::new();
        let mut builtin_calls = Vec::new();
        for call in &allowed_calls {
            if call.name == "use_skill" || !self.mcp_manager.is_mcp_tool(&call.name) {
                builtin_calls.push(call.clone());
            } else {
                mcp_calls.push(call.clone());
            }
        }

        // Builtin tools: parallel execution
        let tool_registry = &self.tool_registry;
        let available_skills = &self.available_skills;
        let process_manager = &self.process_manager;
        let subagent_registry = &self.subagent_registry;
        let session_id = &self.session_id;
        let message_history = &self.conversation.messages;
        let app_config = &self.app_config;
        let builtin_futures: Vec<_> = builtin_calls
            .iter()
            .map(|call| {
                let call = call.clone();
                async move {
                    let result = if call.name == "use_skill" {
                        let skill_name = call.arguments["name"]
                            .as_str()
                            .unwrap_or("");
                        match available_skills.iter().find(|s| s.name == skill_name) {
                            Some(skill) => skill.instructions.clone(),
                            None => format!("Skill '{skill_name}' not found"),
                        }
                    } else if call.name == "call_subagent" {
                        let agent_name = call.arguments["name"]
                            .as_str()
                            .unwrap_or("");
                        let input = call.arguments["input"]
                            .as_str()
                            .unwrap_or("");
                        let ctx = SubAgentContext {
                            session_id,
                            message_history,
                            available_skills,
                            app_config,
                        };
                        match subagent_registry.execute(agent_name, input, ctx).await {
                            Ok(result) => result.output,
                            Err(e) => format!("Sub-agent error: {e}"),
                        }
                    } else {
                        let mut args = call.arguments.clone();
                        // Auto-inject session_id for memory_manage
                        if call.name == "memory_manage" {
                            if args.get("session_id").is_none() {
                                args["session_id"] = serde_json::json!(session_id);
                            }
                        }
                        match tool_registry.call(&call.name, args, process_manager).await {
                            Ok(r) => r,
                            Err(e) => format!("Tool error: {e}"),
                        }
                    };
                    (call, result)
                }
            })
            .collect();

        let mut results = futures::future::join_all(builtin_futures).await;

        // MCP tools: sequential execution
        for call in &mcp_calls {
            let result = match self.mcp_manager.call_tool(call).await {
                Ok(r) => r,
                Err(e) => format!("MCP tool error: {e}"),
            };
            results.push((call.clone(), result));
        }

        // after_tool_call hooks + store results
        for (call, mut result) in results {
            match self.run_hooks_after_tool(ctx, &call, &mut result).await {
                HookResult::Abort(reason) => {
                    self.state = AgentState::Error { message: reason.clone() };
                    self.run_hooks_on_error(&reason).await;
                    let _ = self.event_tx.send(AgentEvent::Error(reason));
                    return ToolExecutionResult::Abort;
                }
                _ => {}
            }

            let _ = self.event_tx.send(AgentEvent::ToolCallResult {
                call_id: call.id.clone(),
                result: result.clone(),
            });

            let tool_msg = Message::tool_result(&call.id, &result);
            self.conversation.push(tool_msg.clone());
            let order = self.conversation.messages.len() as i64 - 1;
            let _ = self.storage.save_message(&self.session_id, &self.conversation_id, &tool_msg, order);
        }

        if round == max_rounds - 1 {
            let err = format!("Max tool rounds ({max_rounds}) exceeded");
            self.state = AgentState::Error { message: err.clone() };
            self.run_hooks_on_error(&err).await;
            let _ = self.event_tx.send(AgentEvent::Error(err));
            return ToolExecutionResult::Abort;
        }

        ToolExecutionResult::Continue
    }
}
