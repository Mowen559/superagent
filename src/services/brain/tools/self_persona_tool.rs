//! 智能体自身画像与能力演进更新工具 (Update Self Persona Tool)
//! 允许智能体在任务执行或反思过程中，自主更新自身的人格画像、能力特质、座右铭与生平履历

use crate::config::app_paths::AppPaths;
use crate::config::persona_settings::{self, PersonaUpdatePatch};
use crate::domains::agent_cognitive::executor::ToolExecutor;
use serde_json::{json, Value};

pub struct UpdateSelfPersonaTool {
    app_paths: AppPaths,
}

impl UpdateSelfPersonaTool {
    pub fn new(app_paths: AppPaths) -> Self {
        Self { app_paths }
    }
}

impl ToolExecutor for UpdateSelfPersonaTool {
    fn name(&self) -> &'static str {
        "update_self_persona"
    }

    fn description(&self) -> &'static str {
        "Update the agent's own persona profile, capability descriptions, personality traits, motto, speaking style, or backstory. Call this when you want to reflect on your capabilities, record new learning milestones, or adapt your identity to better assist the user."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "motto": {
                    "type": "string",
                    "description": "Updated life motto or philosophical conviction."
                },
                "add_traits": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "New personality or capability traits to add (e.g. ['分布式事务治理专家', 'Tree-sitter 语法推演大师'])."
                },
                "speaking_style": {
                    "type": "string",
                    "description": "Updated communication tone or speaking style."
                },
                "add_hobbies": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "New hobbies or specialized learning domains."
                },
                "append_backstory": {
                    "type": "string",
                    "description": "New growth milestones or experience background to append to your backstory."
                },
                "custom_instructions": {
                    "type": "string",
                    "description": "Custom self-guidance or behavioral rules."
                },
                "reflection_note": {
                    "type": "string",
                    "description": "A brief reflective note on why you are updating your persona."
                }
            }
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let reflection_note = args
            .get("reflection_note")
            .and_then(|v| v.as_str())
            .unwrap_or("Self-reflection and continuous capability evolution.")
            .to_string();

        let patch: PersonaUpdatePatch = serde_json::from_value(args)
            .map_err(|e| format!("Failed to parse persona update patch: {e}"))?;

        let updated = persona_settings::update_persona_partial(&self.app_paths, &patch)?;

        eprintln!(
            "[SuperAgent Persona] Self-evolution applied. Agent: '{}', Reflection: '{}'",
            updated.name, reflection_note
        );

        Ok(json!({
            "status": "Persona updated successfully",
            "agentName": updated.name,
            "apparentAge": updated.apparent_age,
            "traits": updated.personality_traits,
            "motto": updated.motto,
            "reflection": reflection_note,
            "message": "Your personal identity profile and prompt directives have been updated and persisted."
        }))

    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_self_persona_tool_metadata() {
        let app_paths = AppPaths::default();
        let tool = UpdateSelfPersonaTool::new(app_paths);
        assert_eq!(tool.name(), "update_self_persona");
        let params = tool.parameters_schema();
        assert!(params.get("properties").is_some());
        assert!(params["properties"].get("motto").is_some());
        assert!(params["properties"].get("add_traits").is_some());
    }
}
