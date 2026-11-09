//! 技能自动结晶器 (Auto-Skill Crystallizer & Evolution Engine)
//! 将智能体在实战中攻克的疑难杂症、从报错到自愈的排障轨迹以及工程最佳实践，
//! 自动化提炼、格式化并持久化为规范的工程技能包（SKILL.md），
//! 形成“实战踩坑 -> 自愈攻坚 -> 自动结晶为技能 -> 下次遇同类问题秒解”的飞轮闭环。

use std::path::Path;
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
        let _=task_goal;
        let evidence=crate::services::brain::recovery_candidate::extract(events,false).into_iter().next()?;
        if !crate::services::brain::recovery_candidate::is_current(&evidence,workspace_root){return None;}
        let config=crate::config::runtime::RuntimeConfig::from_env(&crate::config::app_paths::AppPaths::new());
        let description="未验证的恢复流程候选；需要不同目标的独立裁判验证";
        let markdown=format!("---\nname: observed-recovery\ndescription: {}\n---\n\n# 恢复流程候选\n\n{}\n\n适用条件及未验证项：\n{}\n",description,evidence.summary(),serde_json::to_string_pretty(&evidence).ok()?);
        let store=crate::services::brain::candidate_governance::Governance::new(&config,workspace_root);
        let candidate=store.propose_skill("observed-recovery",description,&markdown).ok()?;
        Some(CrystallizedSkillInfo{name:candidate.name.clone(),description:description.into(),path:store.payload_path(&candidate).to_string_lossy().into(),trigger_pattern:"unverified_candidate".into()})
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
