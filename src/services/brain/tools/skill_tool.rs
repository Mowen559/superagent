use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use crate::domains::agent_cognitive::capability_scout::CapabilityScout;

use crate::config::app_paths::AppPaths;
use crate::config::runtime::RuntimeConfig;
use crate::domains::agent_cognitive::executor::ToolExecutor;

pub struct SkillTool {
    pub workspace_root: PathBuf,
}

impl SkillTool {
    pub fn new(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }


}

impl ToolExecutor for SkillTool {
    fn name(&self) -> &'static str {
        "skill_tool"
    }

    fn description(&self) -> &'static str {
        "Manage, read, and dynamically accumulate agent skills (custom engineering rules, conventions, and reusable workflows). Use action='list' to see available skills, action='read' to inspect instructions, and action='save' or 'update' to crystallize newly verified best practices, workflows, or troubleshooting procedures into quarantined candidates. save/update never activate skills; read_candidate is explicit draft inspection."
    }

    fn prompt_projection(&self) -> Option<String> {
        Some(
            r#"
[Tool Projection: skill_tool]
- Role: Workspace Dynamic Skills Manager & Solidifier
- Usage: List available skills using action='list', inspect instructions with action='read', or save a quarantined draft with action='save'. Drafts require independent held-out validation before activation.
- Inspect an unvalidated draft only with action='read_candidate', candidate_id from save/update; normal list/read exclude it. delete revokes a managed candidate and preserves files.
- Read references/scripts outside the task workspace with action='read', skill_name, resource_path='references/file.md' (relative to that skill). Keep generated artifacts in the task workspace.
"#
            .trim()
            .to_string(),
        )
    }

    fn read_only_resource_request(&self, path: &std::path::Path) -> Option<Value> {
        if !path.is_absolute() { return None; }
        let canonical = fs::canonicalize(path).ok()?;
        if !canonical.is_file() { return None; }
        for skill in CapabilityScout::discover_skills(&self.workspace_root) {
            let Some(root) = std::path::Path::new(&skill.path).parent().and_then(|p|fs::canonicalize(p).ok()) else { continue; };
            if let Ok(relative) = canonical.strip_prefix(&root) {
                return Some(json!({"action":"read","skill_name":skill.name,
                    "resource_path":relative.to_str()?.replace('\\',"/")}));
            }
        }
        None
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "The action to perform: 'list' (list available skills), 'read' (read a skill), 'save' (persist a newly discovered workflow/skill), 'update' (refine an existing skill), 'delete' (revoke a managed candidate), or 'read_candidate' (inspect a quarantined draft).",
                    "enum": ["list", "read", "save", "update", "delete", "read_candidate"]
                },
                "candidate_id":{"type":"string","description":"Managed candidate ID for explicit read_candidate or revoke via delete."},
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
                "start_line": {"type":"integer","minimum":1,"description":"Optional first resource line, inclusive (read only)."},
                "end_line": {"type":"integer","minimum":1,"description":"Optional last resource line, inclusive (read only)."},
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
                let skill = match CapabilityScout::discover_skills(&self.workspace_root).into_iter()
                    .find(|s|s.name.eq_ignore_ascii_case(skill_name)) {
                    Some(skill)=>skill,
                    None=>return Ok(json!({"status":"unavailable","available":false,"inspection_verified":true,"skill_name":skill_name,"reason":"not_in_effective_catalog","message":"查询完成；该技能当前不在有效目录中，未返回正文，也未激活任何候选。"})),
                };
                let root = std::path::Path::new(&skill.path).parent().ok_or("Skill has no resource root")?;
                let path = match args.get("resource_path").and_then(|v| v.as_str()) {
                    Some(resource) => crate::infrastructure::fs_path::resolve_safe_path(resource, root)?,
                    None => PathBuf::from(&skill.path),
                };
                let size = fs::metadata(&path).map_err(|e| e.to_string())?.len();
                if size > 1_048_576 { return Err("Skill resource exceeds 1 MiB; read a smaller resource".into()); }
                let mut content = fs::read_to_string(&path).map_err(|e| e.to_string())?;
                if args.get("start_line").is_some() || args.get("end_line").is_some() {
                    let line = |key: &str, default: usize| -> Result<usize,String> {
                        match args.get(key) {
                            None => Ok(default),
                            Some(value) => value.as_u64().and_then(|v| usize::try_from(v).ok()).filter(|v| *v>0)
                                .ok_or_else(|| format!("{key} must be a positive integer")),
                        }
                    };
                    let start = line("start_line",1)?;
                    let end = line("end_line",usize::MAX)?;
                    if end < start { return Err("end_line must not precede start_line".into()); }
                    content = content.split_inclusive('\n').skip(start-1).take(end-start+1).collect();
                }
                let resource_guide = skill.instructions.find("## 已定位的技能资源")
                    .map(|start| &skill.instructions[start..]).unwrap_or("");
                Ok(json!({
                    "skill_name": skill.name,
                    "path": path,
                    "skill_path": skill.path,
                    "resource_root": root,
                    "content": content,
                    "resource_guidance": resource_guide,
                    "scope_guidance": "Apply only the parts needed for the user's current outcome and required acceptance. Reading this resource does not authorize or require its entire lifecycle. For a documentation edit, revise and verify the requested document; do not regenerate, republish, or investigate unrelated runtimes unless required to resolve a material factual gap.",
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

                let config=RuntimeConfig::from_env(&AppPaths::new());
                let store=crate::services::brain::candidate_governance::Governance::new(&config,&self.workspace_root);
                let full_content=format!("---\nname: {}\ndescription: {}\ncategory: {}\n---\n\n{}\n",serde_json::to_string(&sanitized_name).map_err(|_|"skill_encoding_failed")?,serde_json::to_string(description).map_err(|_|"skill_encoding_failed")?,serde_json::to_string(category).map_err(|_|"skill_encoding_failed")?,instructions_raw);
                let candidate=store.propose_skill(&sanitized_name,description,&full_content)?;
                Ok(json!({"status":"candidate","action":"candidate_created","candidate_id":candidate.id,"skill_name":candidate.name,"revision":candidate.revision,"path":store.payload_path(&candidate),"message":"隔离候选已保存；尚未通过不同目标留出检查，未激活技能。"}))
            }
            "read_candidate"=>{
                let id=args.get("candidate_id").and_then(Value::as_str).ok_or("candidate_id required")?;
                let config=RuntimeConfig::from_env(&AppPaths::new());
                let store=crate::services::brain::candidate_governance::Governance::new(&config,&self.workspace_root);
                Ok(json!({"status":"candidate_data_only","content":store.read_candidate(id)?,"candidate":store.get(id)?}))
            }
            "delete"=>{
                let id=args.get("candidate_id").and_then(Value::as_str).ok_or("candidate_id required; unmanaged user skills are not deleted by this tool")?;
                let config=RuntimeConfig::from_env(&AppPaths::new());let store=crate::services::brain::candidate_governance::Governance::new(&config,&self.workspace_root);
                let candidate=store.get(id)?;let revoked=store.revoke(id,candidate.revision)?;
                Ok(json!({"status":"revoked","candidate":revoked}))
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
    fn test_skill_draft_versions_are_quarantined_and_revocation_preserves_evidence() {
        let temp=tempdir().unwrap();let tool=SkillTool::new(temp.path().into());
        let saved=tool.execute(json!({"action":"save","skill_name":"mybatis-paging","description":"Paging draft","instructions":"# MyBatis Paging\nAlways use Page<T> and LambdaQueryWrapper."})).unwrap();
        assert_eq!(saved["status"],"candidate");let id=saved["candidate_id"].as_str().unwrap();
        assert!(!temp.path().join(".agents").exists());assert!(std::path::Path::new(saved["path"].as_str().unwrap()).is_file());
        let unavailable=tool.execute(json!({"action":"read","skill_name":saved["skill_name"]})).unwrap();assert_eq!(unavailable["available"],false);assert!(unavailable.get("content").is_none());assert_eq!(unavailable["status"],"unavailable");
        let read=tool.execute(json!({"action":"read_candidate","candidate_id":id})).unwrap();assert!(read["content"].as_str().unwrap().contains("LambdaQueryWrapper"));
        let list=tool.execute(json!({"action":"list"})).unwrap();assert!(!list["skills"].as_array().unwrap().iter().any(|s|s["name"]==saved["skill_name"]));
        let updated=tool.execute(json!({"action":"update","skill_name":"mybatis-paging","instructions":"# Updated candidate"})).unwrap();assert_eq!(updated["status"],"candidate");assert_ne!(saved["candidate_id"],updated["candidate_id"]);
        assert_eq!(read["content"],tool.execute(json!({"action":"read_candidate","candidate_id":id})).unwrap()["content"]);
        let revoked=tool.execute(json!({"action":"delete","candidate_id":id})).unwrap();assert_eq!(revoked["status"],"revoked");assert_eq!(revoked["candidate"]["state"],"revoked");
        assert!(tool.execute(json!({"action":"read_candidate","candidate_id":id})).is_ok());assert!(std::path::Path::new(saved["path"].as_str().unwrap()).is_file());
    }

}
