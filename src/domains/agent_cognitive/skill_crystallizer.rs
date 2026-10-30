//! 技能自动结晶器 (Auto-Skill Crystallizer & Evolution Engine)
//! 将智能体在实战中攻克的疑难杂症、从报错到自愈的排障轨迹以及工程最佳实践，
//! 自动化提炼、格式化并持久化为规范的工程技能包（SKILL.md），
//! 形成“实战踩坑 -> 自愈攻坚 -> 自动结晶为技能 -> 下次遇同类问题秒解”的飞轮闭环。

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::services::brain::events::SessionEvent;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrystallizedSkillInfo {
    pub name: String,
    pub description: String,
    pub path: String,
    pub trigger_pattern: String,
}

pub struct AutoSkillCrystallizer;

impl AutoSkillCrystallizer {
    /// 检查执行轨迹是否具备自动结晶价值，并在具备时自动沉淀为持久化 SKILL.md
    pub fn crystallize_from_trajectory(
        task_goal: &str,
        events: &[SessionEvent],
        workspace_root: &Path,
    ) -> Option<CrystallizedSkillInfo> {
        if events.is_empty() || task_goal.trim().is_empty() {
            return None;
        }

        // 1. 轨迹自省：收集改动的文件、报错历史与最终自愈凭据
        let mut modified_files = Vec::new();
        let mut error_snippets = Vec::new();
        let mut verified_commands = Vec::new();
        let mut had_self_healing = false;

        for (idx, event) in events.iter().enumerate() {
            match event {
                SessionEvent::ToolCall { name, arguments, .. } => {
                    if matches!(
                        name.as_str(),
                        "replace_in_file" | "multi_replace" | "write_file" | "write_to_file"
                    ) {
                        let path_arg = arguments
                            .get("path")
                            .or_else(|| arguments.get("Path"))
                            .and_then(|v| v.as_str());
                        if let Some(p) = path_arg {
                            if !modified_files.contains(&p.to_string()) {
                                modified_files.push(p.to_string());
                            }
                        }
                    } else if name == "run_command" {
                        let cmd_str = arguments
                            .get("command")
                            .or_else(|| arguments.get("CommandLine"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if !cmd_str.trim().is_empty() && !verified_commands.contains(&cmd_str.to_string()) {
                            verified_commands.push(cmd_str.to_string());
                        }
                    }
                }
                SessionEvent::ToolResult { result, .. } => {
                    let out_str = if let Some(s) = result.as_str() {
                        s.to_string()
                    } else {
                        result.to_string()
                    };

                    let is_error = out_str.contains("Compilation failure")
                        || out_str.contains("BUILD FAILURE")
                        || out_str.contains("[ERROR]")
                        || out_str.contains("error[E")
                        || out_str.contains("error: ")
                        || out_str.contains("failed with exit code")
                        || out_str.contains("403 Forbidden")
                        || out_str.contains("panic:");

                    if is_error {
                        let line = out_str
                            .lines()
                            .find(|l| {
                                l.contains("[ERROR]")
                                    || l.contains("error:")
                                    || l.contains("Compilation failure")
                                    || l.contains("error[E")
                            })
                            .unwrap_or("Encountered unexpected execution error")
                            .trim()
                            .chars()
                            .take(150)
                            .collect::<String>();
                        if !error_snippets.contains(&line) {
                            error_snippets.push(line);
                        }
                    }

                    // 检查后续是否有成功测试或构建，构成“自愈闭环”
                    if !error_snippets.is_empty() {
                        let is_recovery_success = out_str.contains("BUILD SUCCESS")
                            || out_str.contains("test result: ok")
                            || out_str.contains("Finished `")
                            || out_str.contains("tests passed");
                        if is_recovery_success {
                            had_self_healing = true;
                        }
                    }
                }
                _ => {}
            }
        }

        // 结晶准入条件：
        // ① 发生了实质性代码修改 且 成功自愈（排障攻坚模式）；
        // ② 或者任务明确产出了多文件协作并完成了物理测试验证。
        let should_crystallize = (!modified_files.is_empty() && had_self_healing)
            || (!modified_files.is_empty() && !verified_commands.is_empty() && events.len() >= 6);

        if !should_crystallize {
            return None;
        }

        // 2. 生成技能唯一 Slug 名称
        let skill_name = Self::generate_skill_slug(task_goal);
        let skill_desc = format!(
            "从实战攻坚中自动沉淀的领域技能：解决“{}”，包含经物理检验的代码修改路径与测试断言",
            task_goal.chars().take(60).collect::<String>()
        );

        // 3. 构建标准规范的 SKILL.md 文档
        let content = Self::render_skill_markdown(
            &skill_name,
            &skill_desc,
            task_goal,
            &error_snippets,
            &modified_files,
            &verified_commands,
        );

        // 4. 安全持久化写入工作区专属技能目录 (.agents/skills/{skill_name}/SKILL.md)
        let skill_dir = workspace_root.join(".agents").join("skills").join(&skill_name);
        if let Err(e) = fs::create_dir_all(&skill_dir) {
            eprintln!("[AutoSkillCrystallizer] Failed to create skill dir: {e}");
            return None;
        }

        let skill_file = skill_dir.join("SKILL.md");
        if let Err(e) = fs::write(&skill_file, content) {
            eprintln!("[AutoSkillCrystallizer] Failed to write SKILL.md: {e}");
            return None;
        }

        let rel_path = skill_file
            .strip_prefix(workspace_root)
            .unwrap_or(&skill_file)
            .to_string_lossy()
            .to_string();

        Some(CrystallizedSkillInfo {
            name: skill_name,
            description: skill_desc,
            path: rel_path,
            trigger_pattern: task_goal.to_string(),
        })
    }

    /// 根据用户任务目标生成安全、可读的 kebab-case 技能标识符
    pub fn generate_skill_slug(goal: &str) -> String {
        let mut clean = String::new();
        for ch in goal.chars() {
            if ch.is_ascii_alphanumeric() {
                clean.push(ch.to_ascii_lowercase());
            } else if ch == ' ' || ch == '-' || ch == '_' || ch == '/' || ch == '.' {
                if !clean.ends_with('-') && !clean.is_empty() {
                    clean.push('-');
                }
            } else if ch.is_alphabetic() {
                // 中文等非 ASCII 字符保留，方便国内开发者检索
                clean.push(ch);
            }
        }

        let mut slug = clean.trim_matches('-').to_string();
        if slug.chars().count() > 36 {
            slug = slug.chars().take(36).collect();
            slug = slug.trim_matches('-').to_string();
        }

        if slug.is_empty() {
            slug = format!("auto-skill-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
        }

        if !slug.starts_with("auto-") && !slug.starts_with("skill-") {
            slug = format!("auto-{}", slug);
        }

        slug
    }

    /// 渲染完全符合 DeepCloud 与 Antigravity 标准的 SKILL.md 文档
    pub fn render_skill_markdown(
        name: &str,
        desc: &str,
        goal: &str,
        error_snippets: &[String],
        modified_files: &[String],
        verified_commands: &[String],
    ) -> String {
        let now_str = Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();
        let mut md = String::new();

        // 1. YAML Frontmatter
        md.push_str("---\n");
        md.push_str(&format!("name: \"{}\"\n", name));
        md.push_str(&format!("description: \"{}\"\n", desc.replace('"', "\\\"")));
        md.push_str("---\n\n");

        // 2. 标题与结晶声明
        md.push_str(&format!("# {}\n\n", name));
        md.push_str(&format!(
            "> 💎 **实战结晶资产**：此技能由智能体在攻坚任务中自动提炼沉淀生成。\n> 结晶时间：`{}`\n\n",
            now_str
        ));

        // 3. 适用场景与问题背景
        md.push_str("## 一、 适用场景与实战目标\n");
        md.push_str(&format!("* **核心目标**：{}\n", goal));
        if !error_snippets.is_empty() {
            md.push_str("* **曾遭遇的典型报错/陷阱阻断**：\n");
            for err in error_snippets {
                md.push_str(&format!("  - `⚠️ {}`\n", err));
            }
        }
        md.push_str("\n");

        // 4. 黄金解法与受控变异路径
        md.push_str("## 二、 经实战检验的黄金解法 (Golden Patterns)\n");
        if !modified_files.is_empty() {
            md.push_str("* **涉及的关键模块与文件路径**：\n");
            for f in modified_files {
                md.push_str(&format!("  - `{}`\n", f));
            }
        }
        md.push_str("* **外科手术式修改准则**：\n");
        md.push_str("  1. 优先使用局部增量替换（`replace_in_file` / `multi_replace`），保留存量注释与未改动代码；\n");
        md.push_str("  2. 修改前审视模块依赖与架构契约，禁止污染通用公共模块（如 `common/`）；\n");
        md.push_str("  3. 严格遵循 Red-to-Green 状态机驱动，修复完必须跑回归测试。\n\n");

        // 5. 验收断言与回归测试清单
        md.push_str("## 三、 验收断言与回归测试清单 (Verification Checklist)\n");
        if !verified_commands.is_empty() {
            md.push_str("* **必须通过的物理机评命令**：\n```bash\n");
            for cmd in verified_commands {
                md.push_str(&format!("{}\n", cmd));
            }
            md.push_str("```\n");
        } else {
            md.push_str("* 必须执行对应的单元测试套件，退出码必须为 0。\n");
        }
        md.push_str("* **质量闸门准则**：严禁口头虚假交付，未见物理机评通过日志一律视为任务未完成。\n");

        md
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_skill_slug() {
        let slug1 = AutoSkillCrystallizer::generate_skill_slug("修复 iam-service 接口 401 权限问题");
        assert!(slug1.starts_with("auto-"));
        assert!(slug1.contains("iam-service"));

        let slug2 = AutoSkillCrystallizer::generate_skill_slug("Fix Mybatis Plus Paging Query Error");
        assert_eq!(slug2, "auto-fix-mybatis-plus-paging-query-error");
    }

    #[test]
    fn test_render_skill_markdown_format() {
        let name = "auto-mybatis-plus-tenant-fix";
        let desc = "修复多租户隔离下数据查询拦截问题";
        let goal = "在租户上下文中跳过特定白名单查询";
        let errors = vec!["TenantContextException: missing tenant_id in thread local".to_string()];
        let files = vec!["iam-service/src/main/java/TenantAspect.java".to_string()];
        let cmds = vec!["mvn test -pl iam-service".to_string()];

        let md = AutoSkillCrystallizer::render_skill_markdown(
            name, desc, goal, &errors, &files, &cmds,
        );

        assert!(md.starts_with("---\n"));
        assert!(md.contains("name: \"auto-mybatis-plus-tenant-fix\""));
        assert!(md.contains("TenantContextException"));
        assert!(md.contains("iam-service/src/main/java/TenantAspect.java"));
        assert!(md.contains("mvn test -pl iam-service"));
    }
}
