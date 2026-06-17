use async_trait::async_trait;

use crate::runtime::selector::{HeuristicSelector, SkillSelector};
use crate::subagent::{SubAgent, SubAgentContext, SubAgentResult};

/// Adapts the existing `HeuristicSelector` into the `SubAgent` interface.
pub struct SkillSelectorAgent {
    heuristic: HeuristicSelector,
}

impl SkillSelectorAgent {
    pub fn new() -> Self {
        Self {
            heuristic: HeuristicSelector,
        }
    }
}

impl Default for SkillSelectorAgent {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SubAgent for SkillSelectorAgent {
    fn name(&self) -> &str {
        "skill_selector"
    }

    fn description(&self) -> &str {
        "Select relevant skills based on user message keywords and read_when scenarios"
    }

    async fn execute(&self, input: &str, ctx: SubAgentContext<'_>) -> SubAgentResult {
        let skills = self.heuristic.select(input, ctx.available_skills);
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        SubAgentResult {
            output: serde_json::to_string(&names).unwrap_or_default(),
            metadata: Some(serde_json::json!({ "selected_count": skills.len() })),
        }
    }
}
