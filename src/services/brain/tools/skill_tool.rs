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
        "Manage, read, and dynamically accumulate agent skills (custom engineering rules, conventions, and reusable workflows). Use action='list' to see available skills, action='read' to inspect instructions, and action='save' or 'update' to crystallize newly verified best practices, workflows, or troubleshooting procedures into durable workspace skills."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "The action to perform: 'list' (list available skills), 'read' (read a skill), 'save' (persist a newly discovered workflow/skill), 'update' (refine an existing skill), or 'delete' (remove an obsolete skill).",
                    "enum": ["list", "read", "save", "update", "delete"]
                },
                "skill_name": {
                    "type": "string",
                    "description": "The unique name of the skill (e.g. 'mybatis-plus-paging', 'vault-credentials-pattern', 'liteflow-pipeline-setup'). Required for read, save, update, delete."
                },
                "description": {
                    "type": "string",
                    "description": "A concise 1-2 sentence description of when and why to apply this skill. Required for 'save' and optional for 'update'."
                },
                "instructions": {
                    "type": "string",
                    "description": "The comprehensive markdown instruction body containing operational steps, code patterns, constraints, and pitfalls to avoid. Required for 'save' and 'update'."
                },
                "category": {
                    "type": "string",
                    "description": "Optional category tag: 'workflow', 'architecture', 'troubleshooting', 'testing', or 'conventions'."
                }
            },
            "required": ["action"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let action_opt = args.get("action").and_then(|v| v.as_str());
        let action = action_opt.unwrap_or("list");

        let skills_dir = self.workspace_root.join(".agents").join("skills");

        match action {
            "list" => {
                if !skills_dir.exists() || !skills_dir.is_dir() {
                    return Ok(json!({
                        "skills": [],
                        "message": "No skills directory found at .agents/skills"
                    }));
                }

                let mut skills = Vec::new();
                for entry in WalkDir::new(&skills_dir)
                    .max_depth(2)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    if entry.file_name() == "SKILL.md" {
                        if let Ok(content) = fs::read_to_string(entry.path()) {
                            if let Some((name, desc)) = Self::extract_frontmatter(&content) {
                                skills.push(json!({
                                    "name": name,
                                    "description": desc,
                                    "path": entry.path().strip_prefix(&self.workspace_root).unwrap_or(entry.path()).to_string_lossy().to_string()
                                }));
                            } else if let Some(parent) = entry.path().parent() {
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
                Ok(json!({ "skills": skills, "total": skills.len() }))
            }
            "read" => {
                let skill_name = args
                    .get("skill_name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "skill_name is required for action 'read'".to_string())?;

                if !skills_dir.exists() {
                    return Err("No skills directory found at .agents/skills".to_string());
                }

                for entry in WalkDir::new(&skills_dir)
                    .max_depth(2)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    if entry.file_name() == "SKILL.md" {
                        if let Ok(content) = fs::read_to_string(entry.path()) {
                            let is_match = if let Some((name, _)) = Self::extract_frontmatter(&content) {
                                name.eq_ignore_ascii_case(skill_name)
                            } else {
                                entry
                                    .path()
                                    .parent()
                                    .and_then(|p| p.file_name())
                                    .and_then(|n| n.to_str())
                                    .map(|n| n.eq_ignore_ascii_case(skill_name))
                                    .unwrap_or(false)
                            };

                            if is_match {
                                return Ok(json!({
                                    "skill_name": skill_name,
                                    "path": entry.path().strip_prefix(&self.workspace_root).unwrap_or(entry.path()).to_string_lossy().to_string(),
                                    "content": content
                                }));
                            }
                        }
                    }
                }

                Err(format!("Skill '{}' not found in .agents/skills", skill_name))
            }
            "save" | "update" => {
                let skill_name = args
                    .get("skill_name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "skill_name is required for save/update".to_string())?
                    .trim();

                if skill_name.is_empty() {
                    return Err("skill_name cannot be empty".to_string());
                }

                // Sanitize name for filesystem safety
                let sanitized_name: String = skill_name
                    .chars()
                    .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
                    .collect();

                let description = args
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Dynamic skill persisted from agent verified execution");

                let instructions = args
                    .get("instructions")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "instructions markdown body is required for save/update".to_string())?;

                let category = args
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("workflow");

                let skill_folder = skills_dir.join(&sanitized_name);
                fs::create_dir_all(&skill_folder)
                    .map_err(|e| format!("Failed to create skill directory: {}", e))?;

                let skill_file = skill_folder.join("SKILL.md");
                let exists = skill_file.exists();

                let full_content = format!(
                    "---\nname: \"{}\"\ndescription: \"{}\"\ncategory: \"{}\"\nupdated_at: \"{}\"\n---\n\n{}",
                    sanitized_name,
                    description.replace('"', "\\\""),
                    category,
                    chrono::Utc::now().to_rfc3339(),
                    instructions.trim()
                );

                fs::write(&skill_file, full_content)
                    .map_err(|e| format!("Failed to write SKILL.md: {}", e))?;

                let rel_path = skill_file.strip_prefix(&self.workspace_root).unwrap_or(&skill_file).to_string_lossy().to_string();

                Ok(json!({
                    "status": "success",
                    "action": if exists { "updated" } else { "created" },
                    "skill_name": sanitized_name,
                    "file_path": rel_path,
                    "message": format!("Skill '{}' successfully crystallized into durable workspace skills. Future runs can read and leverage it.", sanitized_name)
                }))
            }
            "delete" => {
                let skill_name = args
                    .get("skill_name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "skill_name is required for delete".to_string())?;

                let sanitized_name: String = skill_name
                    .chars()
                    .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
                    .collect();

                let skill_folder = skills_dir.join(&sanitized_name);
                if !skill_folder.exists() {
                    return Err(format!("Skill '{}' not found at {:?}", skill_name, skill_folder));
                }

                fs::remove_dir_all(&skill_folder)
                    .map_err(|e| format!("Failed to remove skill folder: {}", e))?;

                Ok(json!({
                    "status": "success",
                    "action": "deleted",
                    "skill_name": sanitized_name,
                    "message": format!("Skill '{}' deleted successfully.", sanitized_name)
                }))
            }
            _ => Err(format!("Unknown action: {}", action)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_skill_lifecycle_save_list_read_update_delete() {
        let temp_dir = tempdir().unwrap();
        let tool = SkillTool::new(temp_dir.path().to_path_buf());

        // 1. Initially empty
        let list_res = tool.execute(json!({ "action": "list" })).unwrap();
        assert_eq!(list_res["skills"].as_array().unwrap().len(), 0);

        // 2. Save new skill
        let save_res = tool.execute(json!({
            "action": "save",
            "skill_name": "mybatis-paging",
            "description": "Standard MyBatis-Plus pagination best practice in DeepCloud",
            "instructions": "# MyBatis Paging\nAlways use Page<T> and LambdaQueryWrapper.",
            "category": "architecture"
        })).unwrap();
        assert_eq!(save_res["status"], "success");
        assert_eq!(save_res["action"], "created");

        // 3. Read skill
        let read_res = tool.execute(json!({
            "action": "read",
            "skill_name": "mybatis-paging"
        })).unwrap();
        assert_eq!(read_res["skill_name"], "mybatis-paging");
        let content = read_res["content"].as_str().unwrap();
        assert!(content.contains("name: \"mybatis-paging\""));
        assert!(content.contains("Always use Page<T> and LambdaQueryWrapper"));

        // 4. List skills
        let list_res2 = tool.execute(json!({ "action": "list" })).unwrap();
        assert_eq!(list_res2["skills"].as_array().unwrap().len(), 1);

        // 5. Update skill
        let update_res = tool.execute(json!({
            "action": "update",
            "skill_name": "mybatis-paging",
            "instructions": "# MyBatis Paging V2\nUpdated with SQL injection prevention."
        })).unwrap();
        assert_eq!(update_res["action"], "updated");

        // 6. Delete skill
        let del_res = tool.execute(json!({
            "action": "delete",
            "skill_name": "mybatis-paging"
        })).unwrap();
        assert_eq!(del_res["action"], "deleted");

        // 7. Verify deletion
        let list_res3 = tool.execute(json!({ "action": "list" })).unwrap();
        assert_eq!(list_res3["skills"].as_array().unwrap().len(), 0);
    }
}
