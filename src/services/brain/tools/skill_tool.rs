use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::config::app_paths::AppPaths;
use crate::config::runtime::RuntimeConfig;
use crate::models::memory::SupermemoryRecord;
use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::services::supermemory_store::SupermemoryStore;

pub struct SkillTool {
    pub workspace_root: PathBuf,
}

impl SkillTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    /// 智能定位宿主工程绝对路径（实现沙箱自愈逃逸）
    fn resolve_project_path(&self) -> PathBuf {
        if self.workspace_root.join(".supermemory").exists()
            || self.workspace_root.join(".agents").exists()
        {
            self.workspace_root.clone()
        } else if let Some(parent) = self.workspace_root.parent() {
            let baseline_file = parent.join("baseline.json");
            if baseline_file.exists() {
                if let Ok(content) = fs::read_to_string(&baseline_file) {
                    if let Ok(v) = serde_json::from_str::<Value>(&content) {
                        if let Some(p) = v.get("projectPath").and_then(|p| p.as_str()) {
                            let norm = p.trim_start_matches(r"\\?\");
                            return PathBuf::from(norm);
                        }
                    }
                }
            }
            self.workspace_root.clone()
        } else {
            self.workspace_root.clone()
        }
    }

    fn extract_frontmatter(content: &str) -> Option<(String, String)> {
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
        "Manage, read, and dynamically accumulate agent skills (custom engineering rules, conventions, and reusable workflows). Use action='list' to see available skills, action='read' to inspect instructions, and action='save' or 'update' to crystallize newly verified best practices, workflows, or troubleshooting procedures into durable workspace skills and SuperMemory."
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(
            r#"
[Tool Projection: skill_tool]
- Role: Workspace Dynamic Skills Manager & Solidifier
- Usage: List available skills using action='list', inspect instructions with action='read', or solidify high-value patterns and verified workflows with action='save'.
"#
            .trim()
            .to_string(),
        )
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
                    "description": "A concise 1-2 sentence description of when and why to apply this skill. Optional for 'update'."
                },
                "instructions": {
                    "type": "string",
                    "description": "The comprehensive markdown instruction body containing operational steps, code patterns, constraints, and pitfalls to avoid. Required for 'save' and 'update'."
                },
                "content": {
                    "type": "string",
                    "description": "Alias for 'instructions'. Full Markdown instructions and rules."
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

        let project_root = self.resolve_project_path();
        let mut candidate_dirs = vec![self.workspace_root.join(".agents").join("skills")];
        if project_root != self.workspace_root {
            candidate_dirs.push(project_root.join(".agents").join("skills"));
        }

        match action {
            "list" => {
                let mut skills = Vec::new();
                let mut seen_names = HashSet::new();

                for skills_dir in &candidate_dirs {
                    if !skills_dir.exists() || !skills_dir.is_dir() {
                        continue;
                    }

                    for entry in WalkDir::new(skills_dir)
                        .max_depth(2)
                        .into_iter()
                        .filter_map(|e| e.ok())
                    {
                        if entry.file_name() == "SKILL.md" {
                            if let Ok(content) = fs::read_to_string(entry.path()) {
                                let (name, desc) = Self::extract_frontmatter(&content).unwrap_or_else(|| {
                                    let parent_name = entry
                                        .path()
                                        .parent()
                                        .and_then(|p| p.file_name())
                                        .and_then(|n| n.to_str())
                                        .unwrap_or("unknown")
                                        .to_string();
                                    (parent_name, "No description available".to_string())
                                });

                                if seen_names.insert(name.clone()) {
                                    let rel_path = entry
                                        .path()
                                        .strip_prefix(&self.workspace_root)
                                        .unwrap_or(entry.path())
                                        .to_string_lossy()
                                        .to_string();
                                    skills.push(json!({
                                        "name": name,
                                        "description": desc,
                                        "path": rel_path
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

                for skills_dir in &candidate_dirs {
                    if !skills_dir.exists() {
                        continue;
                    }

                    for entry in WalkDir::new(skills_dir)
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
                                    let rel_path = entry
                                        .path()
                                        .strip_prefix(&self.workspace_root)
                                        .unwrap_or(entry.path())
                                        .to_string_lossy()
                                        .to_string();
                                    return Ok(json!({
                                        "skill_name": skill_name,
                                        "path": rel_path,
                                        "content": content
                                    }));
                                }
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

                let sanitized_name: String = skill_name
                    .chars()
                    .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
                    .collect();

                let description = args
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Dynamic skill persisted from agent verified execution");

                let instructions_raw = args
                    .get("instructions")
                    .or_else(|| args.get("content"))
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "instructions or content markdown body is required for save/update".to_string())?;

                let category = args
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or("workflow");

                let mut write_dirs = vec![project_root.join(".agents").join("skills").join(&sanitized_name)];
                if self.workspace_root != project_root {
                    write_dirs.push(self.workspace_root.join(".agents").join("skills").join(&sanitized_name));
                }

                let mut exists = false;
                for dir in &write_dirs {
                    if dir.join("SKILL.md").exists() {
                        exists = true;
                        break;
                    }
                }

                let full_content = format!(
                    "---\nname: \"{}\"\ndescription: \"{}\"\ncategory: \"{}\"\nupdated_at: \"{}\"\n---\n\n{}\n",
                    sanitized_name,
                    description.replace('"', "\\\""),
                    category,
                    chrono::Utc::now().to_rfc3339(),
                    instructions_raw.trim()
                );

                for dir in &write_dirs {
                    let _ = fs::create_dir_all(dir);
                    let skill_file = dir.join("SKILL.md");
                    let _ = fs::write(&skill_file, &full_content);
                }

                // 同步注册到 SuperMemory 记忆库
                let app_paths = AppPaths::new();
                let config = RuntimeConfig::from_env(&app_paths);
                let project_path_str = project_root.to_string_lossy().to_string();
                let store = SupermemoryStore::new(&config, Some(&project_path_str));

                let record = SupermemoryRecord {
                    id: format!("skill:{}", sanitized_name),
                    name: format!("Skill: {}", sanitized_name),
                    summary: format!(
                        "Skill: {}\nDescription: {}\n\nInstructions:\n{}",
                        sanitized_name, description, instructions_raw
                    ),
                    tags: vec![
                        "skill".to_string(),
                        sanitized_name.clone(),
                        "superagent".to_string(),
                    ],
                    file_path: format!(".agents/skills/{}/SKILL.md", sanitized_name),
                    complexity: "skill_node".to_string(),
                    node_type: "skill".to_string(),
                    source_graph_path: "".to_string(),
                    source_label: format!(".agents/skills/{}/SKILL.md", sanitized_name),
                    updated_at: chrono::Utc::now().to_rfc3339(),
                    embedding: None,
                };
                store.write_memory(record);
                eprintln!(
                    "\x1b[32m[SkillTool] 🎯 Solidified new skill '{}' to .agents/skills and SuperMemory at {}\x1b[0m",
                    sanitized_name, project_path_str
                );

                let rel_path = format!(".agents/skills/{}/SKILL.md", sanitized_name);

                Ok(json!({
                    "status": "success",
                    "action": if exists { "updated" } else { "created" },
                    "skill_name": sanitized_name,
                    "file_path": rel_path.clone(),
                    "path": rel_path,
                    "message": format!("Skill '{}' successfully crystallized into durable workspace skills and registered into SuperMemory.", sanitized_name)
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

                let mut write_dirs = vec![project_root.join(".agents").join("skills").join(&sanitized_name)];
                if self.workspace_root != project_root {
                    write_dirs.push(self.workspace_root.join(".agents").join("skills").join(&sanitized_name));
                }

                let mut found = false;
                for dir in &write_dirs {
                    if dir.exists() {
                        found = true;
                        let _ = fs::remove_dir_all(dir);
                    }
                }

                if !found {
                    return Err(format!("Skill '{}' not found in candidate skill directories", skill_name));
                }

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
