use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use crate::domains::agent_cognitive::capability_scout::CapabilityScout;

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
        crate::infrastructure::fs_path::resolve_origin_project_from_baseline(
            &self.workspace_root,
            &["skills", ".agents"],
        )
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
- Read references/scripts outside the task workspace with action='read', skill_name, resource_path='references/file.md' (relative to that skill). Keep generated artifacts in the task workspace.
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
                "resource_path": {
                    "type": "string",
                    "description": "For read only: optional resource path relative to the selected skill root, e.g. references/api.md or scripts/validate-rule.cjs. Cannot escape that skill directory."
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
        match action {
            "list" => {
                let skills = CapabilityScout::discover_skills(&self.workspace_root)
                    .into_iter().map(|skill| json!({
                        "name": skill.name,
                        "description": skill.description,
                        "path": skill.path,
                        "resource_root": std::path::Path::new(&skill.path).parent(),
                    })).collect::<Vec<_>>();
                Ok(json!({ "total": skills.len(), "skills": skills }))
            }
            "read" => {
                let skill_name = args.get("skill_name").and_then(|v| v.as_str())
                    .ok_or_else(|| "skill_name is required for action 'read'".to_string())?;
                let skill = CapabilityScout::discover_skills(&self.workspace_root).into_iter()
                    .find(|s| s.name.eq_ignore_ascii_case(skill_name))
                    .ok_or_else(|| format!("Skill '{}' not found in configured skill roots", skill_name))?;
                let root = std::path::Path::new(&skill.path).parent().ok_or("Skill has no resource root")?;
                let path = match args.get("resource_path").and_then(|v| v.as_str()) {
                    Some(resource) => crate::infrastructure::fs_path::resolve_safe_path(resource, root)?,
                    None => PathBuf::from(&skill.path),
                };
                let size = fs::metadata(&path).map_err(|e| e.to_string())?.len();
                if size > 1_048_576 { return Err("Skill resource exceeds 1 MiB; read a smaller resource".into()); }
                let content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
                let resource_guide = skill.instructions.find("## 已定位的技能资源")
                    .map(|start| &skill.instructions[start..]).unwrap_or("");
                Ok(json!({
                    "skill_name": skill.name,
                    "path": path,
                    "skill_path": skill.path,
                    "resource_root": root,
                    "content": content,
                    "resource_guidance": resource_guide,
                }))
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
    fn list_read_and_mount_select_the_same_workspace_skill() {
        let tmp = tempdir().unwrap();
        for (root, body) in [(".agents/skills", "selected-body"), ("skills", "shadowed-body")] {
            let dir = tmp.path().join(root).join("only-this-fixture");
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("SKILL.md"), format!(
                "---\nname: skill-catalog-fixture\ndescription: >\n  first line\n  second line\n---\n{body}"
            )).unwrap();
        }
        let tool = SkillTool::new(tmp.path().to_path_buf());
        let listed = tool.execute(json!({"action":"list"})).unwrap();
        let listed = listed["skills"].as_array().unwrap().iter()
            .filter(|s| s["name"] == "skill-catalog-fixture").collect::<Vec<_>>();
        assert_eq!(listed.len(), 1);
        let read = tool.execute(json!({"action":"read","skill_name":"SKILL-CATALOG-FIXTURE"})).unwrap();
        let mounted = CapabilityScout::scout_and_match_skills("skill-catalog-fixture", tmp.path());
        assert_eq!(read["path"], mounted[0].path);
        assert_eq!(read["path"], listed[0]["path"]);
        assert_eq!(listed[0]["description"], "first line second line");
        assert!(read["content"].as_str().unwrap().contains("selected-body"));
        let selected = tmp.path().join(".agents/skills/only-this-fixture");
        fs::create_dir_all(selected.join("references")).unwrap();
        fs::write(selected.join("references/api.md"), "verified-resource").unwrap();
        assert_eq!(tool.execute(json!({"action":"read","skill_name":"skill-catalog-fixture",
            "resource_path":"references/api.md"})).unwrap()["content"], "verified-resource");
        assert!(tool.execute(json!({"action":"read","skill_name":"skill-catalog-fixture",
            "resource_path":"../../../../outside.md"})).is_err());

        // A workspace with only skills/ must also be readable (the original regression).
        let only = tmp.path().join("standalone");
        let dir = only.join("skills/unique-local-fixture");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), "---\nname: unique-local-fixture\n---\nlocal-only").unwrap();
        assert!(SkillTool::new(only).execute(json!({"action":"read","skill_name":"unique-local-fixture"})).unwrap()["content"]
            .as_str().unwrap().contains("local-only"));
    }

    #[test]
    fn test_skill_lifecycle_save_list_read_update_delete() {
        let temp_dir = tempdir().unwrap();
        let tool = SkillTool::new(temp_dir.path().to_path_buf());

        // 1. Initially empty
        let list_res = tool.execute(json!({ "action": "list" })).unwrap();
        assert!(!list_res["skills"].as_array().unwrap().iter().any(|s| s["name"] == "mybatis-paging"));

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
        assert_eq!(list_res2["skills"].as_array().unwrap().iter().filter(|s| s["name"] == "mybatis-paging").count(), 1);

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
        assert!(!list_res3["skills"].as_array().unwrap().iter().any(|s| s["name"] == "mybatis-paging"));
    }
}
