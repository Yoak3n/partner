use ai_partner_shared::{Message, ModelKind, ModelProvider, ToolCall};

use crate::adapter::AgentResponse;
use crate::hook::{HookContext, HookResult};

use super::Runtime;

impl Runtime {
    pub(crate) async fn run_hooks_before_llm(
        &mut self,
        ctx: &HookContext<'_>,
        messages: &mut Vec<Message>,
    ) -> HookResult {
        for hook in &self.hooks {
            match hook.before_llm_call(ctx, messages).await {
                HookResult::Continue => {}
                other => return other,
            }
        }
        HookResult::Continue
    }

    pub(crate) async fn run_hooks_after_llm(
        &mut self,
        ctx: &HookContext<'_>,
        response: &mut AgentResponse,
    ) -> HookResult {
        for hook in &self.hooks {
            match hook.after_llm_call(ctx, response).await {
                HookResult::Continue => {}
                other => return other,
            }
        }
        HookResult::Continue
    }

    pub(crate) async fn run_hooks_before_tool(
        &mut self,
        ctx: &HookContext<'_>,
        call: &ToolCall,
    ) -> HookResult {
        for hook in &self.hooks {
            match hook.before_tool_call(ctx, call).await {
                HookResult::Continue => {}
                other => return other,
            }
        }
        HookResult::Continue
    }

    pub(crate) async fn run_hooks_after_tool(
        &mut self,
        ctx: &HookContext<'_>,
        call: &ToolCall,
        result: &mut String,
    ) -> HookResult {
        for hook in &self.hooks {
            match hook.after_tool_call(ctx, call, result).await {
                HookResult::Continue => {}
                other => return other,
            }
        }
        HookResult::Continue
    }

    pub(crate) async fn run_hooks_on_error(&mut self, error: &str) {
        let dummy_provider = ModelProvider::new(ModelKind::Chat, "", "", "", "");
        let ctx = HookContext {
            provider: &dummy_provider,
            round: 0,
            session_id: &self.session_id,
        };
        for hook in &self.hooks {
            hook.on_error(&ctx, error).await;
        }
    }
}
