use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::domains::model_gateway::model_attempt_router::ModelErrorClass;
use crate::domains::model_gateway::routing::{ModelRequirements, ModelRouteRuntime, ProviderAttemptOutcome, ProviderAttemptReceipt, ProviderCandidate};
use crate::domains::model_gateway::health::{ProviderAdmission, ProviderHealthRuntime};
use crate::domains::model_gateway::protocols::upstream::{adapter_for_provider, UpstreamAdapter};


use super::*;
use super::types::*;

impl LlmProvider {
    /// Analyze a single file with LLM
    pub async fn analyze_file(&self, file_path: &str, content: &str) -> Result<Value, String> {
        let truncated = if content.len() > self.settings.ua_max_file_chars as usize {
            &content[..self.settings.ua_max_file_chars as usize]
        } else {
            content
        };

        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "Analyze this source file and return JSON with: fileSummary (string), tags (array), complexity (simple|moderate|complex).\n\nFile: {}\n\n```\n{}\n```",
                file_path, truncated
            )
        })];

        let (response, _) = self
            .chat(
                messages,
                Some("You are a code analysis assistant. Respond only with valid JSON."),
            )
            .await?;
        let cleaned = strip_code_fences(&response);

        serde_json::from_str(&cleaned)
            .map_err(|_| {
                // If not valid JSON, wrap in a basic structure
                json!({
                    "fileSummary": response,
                    "tags": [],
                    "complexity": "moderate"
                })
            })
            .or_else(|_| {
                Ok(json!({
                    "fileSummary": response,
                    "tags": [],
                    "complexity": "moderate"
                }))
            })
    }

    /// Produce a natural-language comprehension of a single file from its
    /// compressed structural skeleton plus graph-derived structure facts.
    /// Returns Markdown in the requested output language.
    pub async fn explain_file(
        &self,
        file_path: &str,
        skeleton: &str,
        structure: &str,
        lang: &str,
    ) -> Result<String, String> {
        let (directive, _) = lang_directive(lang);
        let skeleton_block = if skeleton.trim().is_empty() {
            String::from("(源码不可用，仅依据代码图谱结构信息分析)")
        } else {
            format!("代码骨架：\n```\n{}\n```", skeleton)
        };
        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "分析这个源文件，输出简洁的 Markdown，包含三部分：\n1) 文件职责：这个文件在项目中的角色与主要职责；\n2) 关键符号：挑选最重要的函数/类型说明其作用；\n3) 依赖关系：结合它依赖的文件与被引用的文件，说明它如何融入整体。\n{}\n\n文件：{}\n\n结构信息(来自代码图谱)：\n{}\n\n{}",
                directive, file_path, structure, skeleton_block
            )
        })];
        self.chat(messages, Some("你是资深代码分析助手，擅长快速讲清一个文件在项目中的作用，回答务必简洁、准确、结构化。")).await.map(|(c, _)| c)
    }

    /// Produce a project-level architecture overview from graph statistics.
    /// Returns Markdown in the requested output language.
    pub async fn explain_project(
        &self,
        structure: &str,
        lang: &str,
        project_language: &str,
    ) -> Result<String, String> {
        let (directive, _) = lang_directive(lang);
        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "总结这个项目的整体架构，输出简洁的 Markdown，包含：\n1) 项目概述：整体是做什么的；\n2) 主要模块：结合顶层模块与文件分布说明各自职责；\n3) 技术栈：结合语言分布做观察；\n4) 核心符号：高频符号反映出的关键能力。\n{}\n\n项目统计(来自代码图谱)：\n{}",
                directive, structure
            )
        })];
        self.chat(messages, Some(&format!("你是资深软件架构分析师，擅长从代码图谱统计推断项目结构，回答务必简洁、准确、结构化。当前项目的主力编程语言是：{}。在分析和输出报告时，请务必贴合该语言的生态特性、代码惯例（如包管理、框架范式）提供最专业的建议。", project_language))).await.map(|(c, _)| c)
    }

    /// 解释修改带来的爆炸半径 (Impact Analyzer)
    pub async fn explain_impact(
        &self,
        changed_files: &str,
        impacted_nodes: &str,
        lang: &str,
    ) -> Result<String, String> {
        let (directive, _) = lang_directive(lang);
        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "分析以下代码变更文件的影响面（爆炸半径），输出简洁的 Markdown 报告：\n1) 直接影响：修改了哪些核心文件；\n2) 级联风险：基于依赖关系，哪些上游调用方受到了影响；\n3) 测试建议：重点回归测试哪些模块。\n{}\n\n变更的文件：\n{}\n\n受影响的上游节点(Graph推演)：\n{}",
                directive, changed_files, impacted_nodes
            )
        })];
        self.chat(
            messages,
            Some("你是资深安全与质量工程师，擅长评估代码修改带来的级联风险。"),
        )
        .await
        .map(|(c, _)| c)
    }

    /// 根据节点特征预测架构分层 (Architecture Analyzer)
    pub async fn explain_architecture(
        &self,
        file_nodes: &str,
        lang: &str,
    ) -> Result<String, String> {
        let (directive, _) = lang_directive(lang);
        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "分析以下文件集合，尝试将它们进行架构分层（如 Controller/Router, Service/Domain, Repository/Database, UI/View 等），输出 Markdown 报告：\n{}\n\n文件列表：\n{}",
                directive, file_nodes
            )
        })];
        self.chat(
            messages,
            Some("你是资深架构师，擅长从文件命名和路径推断其在架构设计中的层级位置。"),
        )
        .await
        .map(|(c, _)| c)
    }

    /// 逆向推演需求：找出拟修改的入口文件
    pub async fn predict_requirement_entry(
        &self,
        requirement: &str,
        file_context: &str,
        lang: &str,
        project_language: &str,
    ) -> Result<String, String> {
        let (directive, _) = lang_directive(lang);
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": format!(
                "需求描述:\n{}\n\n代码库关键文件上下文:\n{}\n\n请输出: \n1. 拟修改的文件列表（提取具体的文件路径）。\n2. 修改这些文件可能对系统造成的影响面评估。\n{}",
                requirement, file_context, directive
            )
        })];

        self.chat(messages, Some(&format!("你是一个高级代码架构师引擎。请根据用户的需求和当前项目的代码库上下文，推演出为了实现该需求，大概率需要修改哪些核心入口文件，以及可能产生的影响面。\n当前项目的主力编程语言是：{}。在分析和输出报告时，请务必贴合该语言的生态特性、代码惯例提供最专业的建议。\n【关键编排准则】：在评估时请重点分析“是否有侵入性更小、更务实的替代方案？”。请根据诊断和事实如实报告，杜绝无根据的臆测。", project_language))).await.map(|(c, _)| c)
    }

    /// 解释业务骨架和调用链路 (Domain Analyzer)
    pub async fn explain_domain_flow(
        &self,
        entry_point: &str,
        downstream_nodes: &str,
        lang: &str,
    ) -> Result<String, String> {
        let (directive, _) = lang_directive(lang);
        let messages = vec![json!({
            "role": "user",
            "content": format!(
                "你是一个业务分析专家。以下是围绕某个入口节点提取的业务调用链路（DFS遍历），请将这段链路翻译成通俗易懂的业务流程文档：\n{}\n\n入口点：{}\n\n下游依赖节点：\n{}",
                directive, entry_point, downstream_nodes
            )
        })];
        self.chat(
            messages,
            Some("你是资深业务架构师，擅长将底层的方法调用图翻译为清晰的业务处理流程。"),
        )
        .await
        .map(|(c, _)| c)
    }
}
