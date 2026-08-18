use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::services::brain::executor::ToolExecutor;

pub struct SkillTool {
    pub workspace_root: PathBuf,
}

impl SkillTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    fn extract_frontmatter(content: &str) -> Option<(String, String)> {
        // Simple frontmatter parser for `---` delimited blocks
        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() || lines[0].trim() != "---" {
            return None;
        }

        let mut name = String::new();
        let mut desc = String::new();

        let mut in_frontmatter = true;
        for line in lines.into_iter().skip(1) {
            if line.trim() == "---" {
                in_frontmatter = false;
                break;
            }
            if let Some(idx) = line.find(':') {
                let key = line[..idx].trim();
                let val = line[idx + 1..].trim().trim_matches('"').trim_matches('\'');
                if key == "name" {
                    name = val.to_string();
                } else if key == "description" {
                    desc = val.to_string();
                }
            }
        }

        if !in_frontmatter && !name.is_empty() {
            Some((name, desc))
        } else {
            None
        }
    }
}

impl ToolExecutor for SkillTool {
    fn name(&self) -> &'static str {
        "skill_tool"
    }

    fn description(&self) -> &'static str {
        "Manage and read dynamic agent skills (custom rules, conventions, workflows) available in the workspace. Use action='list' to see available skills, and action='read' with 'skill_name' to read a specific skill."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "The action to perform: 'list' (list available skills) or 'read' (read a specific skill's instructions).",
                    "enum": ["list", "read"]
                },
                "skill_name": {
                    "type": "string",
                    "description": "The name of the skill to read. Required when action is 'read'."
                }
            },
            "required": ["action"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let action_opt = args.get("action").and_then(|v| v.as_str());
        let action = action_opt.unwrap_or("list");

        let skills_dir = self.workspace_root.join(".agents").join("skills");

        if action == "list" {
            if !skills_dir.exists() || !skills_dir.is_dir() {
                return Ok(json!({
                    "skills": [],
                    "message": "No skills directory found at .agents/skills"
                }));
            }

            let mut skills = Vec::new();
            for entry in WalkDir::new(&skills_dir).max_depth(2).into_iter().filter_map(|e| e.ok()) {
                if entry.file_name() == "SKILL.md" {
                    if let Ok(content) = fs::read_to_string(entry.path()) {
                        if let Some((name, desc)) = Self::extract_frontmatter(&content) {
                            skills.push(json!({
                                "name": name,
                                "description": desc,
                                "path": entry.path().strip_prefix(&self.workspace_root).unwrap_or(entry.path()).to_string_lossy().to_string()
                            }));
                        } else {
                            // Fallback if no frontmatter
                            if let Some(parent) = entry.path().parent() {
                                if let Some(parent_name) = parent.file_name().and_then(|n| n.to_str()) {
                                    skills.push(json!({
                                        "name": parent_name,
                                        "description": "No description available",
                                        "path": entry.path().strip_prefix(&self.workspace_root).unwrap_or(entry.path()).to_string_lossy().to_string()
                                    }));
                                }
                            }
                        }
                    }
                }
            }
            Ok(json!({ "skills": skills }))
        } else if action == "read" {
            let skill_name = args
                .get("skill_name")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "skill_name is required for action 'read'".to_string())?;

            if !skills_dir.exists() {
                return Err("No skills directory found".to_string());
            }

            // Look for the specific skill
            for entry in WalkDir::new(&skills_dir).max_depth(2).into_iter().filter_map(|e| e.ok()) {
                if entry.file_name() == "SKILL.md" {
                    let is_match = if let Ok(content) = fs::read_to_string(entry.path()) {
                        if let Some((name, _)) = Self::extract_frontmatter(&content) {
                            name == skill_name
                        } else {
                            // Fallback folder name
                            entry.path().parent().and_then(|p| p.file_name()).and_then(|n| n.to_str()) == Some(skill_name)
                        }
                    } else {
                        false
                    };

                    if is_match {
                        if let Ok(content) = fs::read_to_string(entry.path()) {
                            return Ok(json!({
                                "skill_name": skill_name,
                                "content": content
                            }));
                        }
                    }
                }
            }

            Err(format!("Skill '{}' not found", skill_name))
        } else {
            Err(format!("Unknown action: {}", action))
        }
    }
}
