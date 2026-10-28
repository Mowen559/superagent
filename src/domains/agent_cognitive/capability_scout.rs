//! 能力缺口自省与自主进化动态技能引擎 (Just-In-Time Capability Acquisition & Skill Scout)
//! 当智能体面临其工具库未覆盖的全新领域任务时（如语音合成TTS、格式转换、音视频剪辑、私有逆向等），
//! 主动诊断能力缺口，智能推荐系统原生与开源生态方案，并在沙箱验证后动态沉淀为终生技能。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 缺失的专有能力领域
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomainCapabilityType {
    /// 语音合成 (Text-to-Speech)
    AudioSynthesis,
    /// 语音识别转录 (Automatic Speech Recognition)
    AudioTranscription,
    /// 视频处理与裁切 (Video Manipulation / FFmpeg)
    VideoProcessing,
    /// 深度逆向反编译 (Decompilation / Disassembly)
    BinaryReverseEngineering,
    /// 私有二进制/Protobuf协议解析
    BinaryProtocolParsing,
    /// 通用未识别缺口
    GenericMissingTool(String),
}

/// 候选技术方案
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateSolution {
    pub name: String,
    pub solution_type: String, // "system_native", "pypi_cli", "powershell_builtin"
    pub description: String,
    pub install_or_run_command: String,
    pub zero_install: bool,
}

/// 能力缺口诊断报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityGapReport {
    pub task_prompt: String,
    pub missing_capability: DomainCapabilityType,
    pub recommended_search_queries: Vec<String>,
    pub candidate_solutions: Vec<CandidateSolution>,
    pub action_directive: String,
}

/// 自动匹配并挂载的业务技能包
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchedSkill {
    pub name: String,
    pub description: String,
    pub path: String,
    pub instructions: String,
    pub score: f32,
}

impl MatchedSkill {
    pub fn render_system_prompt_section(&self) -> String {
        format!(
            "### 🎯 【已挂载业务技能: {}】\n> 简介: {}\n\n{}\n",
            self.name, self.description, self.instructions
        )
    }
}

/// 能力自省与求索引擎
pub struct CapabilityScout;

impl CapabilityScout {
    /// 自省当前任务是否超越了现有基础工具箱，并诊断出具体的能力缺口
    pub fn inspect_gap(prompt: &str, registered_tools: &[String]) -> Option<CapabilityGapReport> {
        let p_lower = prompt.to_lowercase();

        // 1. 语音合成 (TTS / 音频生成) 缺口诊断
        let asks_for_speech = p_lower.contains("语音")
            || p_lower.contains("朗读")
            || p_lower.contains("说话")
            || p_lower.contains("生成音频")
            || p_lower.contains("生成声音")
            || p_lower.contains("speech")
            || p_lower.contains("tts")
            || p_lower.contains("text to speech")
            || p_lower.contains("text-to-speech")
            || p_lower.contains(".mp3")
            || p_lower.contains(".wav");

        let has_tts_tool = registered_tools.iter().any(|t| t.contains("tts") || t.contains("speech"));

        if asks_for_speech && !has_tts_tool {
            return Some(CapabilityGapReport {
                task_prompt: prompt.to_string(),
                missing_capability: DomainCapabilityType::AudioSynthesis,
                recommended_search_queries: vec![
                    "windows powershell system.speech speechsynthesizer text to speech".to_string(),
                    "python edge-tts lightweight cli free".to_string(),
                ],
                candidate_solutions: vec![
                    CandidateSolution {
                        name: "Windows Native System.Speech".to_string(),
                        solution_type: "powershell_builtin".to_string(),
                        description: "Windows 操作系统内置的语音合成引擎，零依赖、零安装，通过 PowerShell 即可直接调用并保存为音频。".to_string(),
                        install_or_run_command: "Add-Type -AssemblyName System.Speech; $synth = New-Object System.Speech.Synthesis.SpeechSynthesizer; $synth.SetOutputToWaveFile('output.wav'); $synth.Speak('文本内容'); $synth.Dispose()".to_string(),
                        zero_install: true,
                    },
                    CandidateSolution {
                        name: "Microsoft edge-tts (Python)".to_string(),
                        solution_type: "pypi_cli".to_string(),
                        description: "基于微软在线神经网络的高品质语音合成库，开源免费，无需 API Key，自带 edge-tts 命令行工具。".to_string(),
                        install_or_run_command: "pip install edge-tts; edge-tts --text '文本内容' --write-media output.mp3".to_string(),
                        zero_install: false,
                    },
                ],
                action_directive: "【动态能力进化指引】：你当前工具库不直接包含语音合成工具，但检测到目标环境支持 Windows 原生 System.Speech 与 Python edge-tts！请优先调用 run_command 在 scratch/ 编写测试脚本验证该方案，跑通后交付音频文件。".to_string(),
            });
        }

        // 2. 视频处理 (Video / FFmpeg) 缺口诊断
        let asks_for_video = (p_lower.contains("视频") || p_lower.contains("剪辑") || p_lower.contains(".mp4"))
            && (p_lower.contains("剪切") || p_lower.contains("合并") || p_lower.contains("转码") || p_lower.contains("提取音频"));
        let has_video_tool = registered_tools.iter().any(|t| t.contains("ffmpeg") || t.contains("video"));

        if asks_for_video && !has_video_tool {
            return Some(CapabilityGapReport {
                task_prompt: prompt.to_string(),
                missing_capability: DomainCapabilityType::VideoProcessing,
                recommended_search_queries: vec!["ffmpeg windows command line".to_string()],
                candidate_solutions: vec![CandidateSolution {
                    name: "FFmpeg CLI".to_string(),
                    solution_type: "system_native".to_string(),
                    description: "跨平台音视频处理瑞士军刀。".to_string(),
                    install_or_run_command: "ffmpeg -i input.mp4 -ss 00:00:00 -t 00:00:10 -c copy output.mp4".to_string(),
                    zero_install: false,
                }],
                action_directive: "【动态能力进化指引】：检测到视频处理任务。请探针系统是否已安装 ffmpeg，若已就绪直接调用，若无则引导轻量方案。".to_string(),
            });
        }

        None
    }

    /// 将在沙箱中成功验证的新能力，动态生成标准化 Skill 脚本模板，供未来永久复用
    pub fn synthesize_skill_script(
        skill_name: &str,
        description: &str,
        core_command_pattern: &str,
    ) -> String {
        format!(
            r#"# Dynamic Autonomous Skill: {skill_name}
# Description: {description}
# Auto-synthesized by DeepCloud Hub Just-In-Time Capability Engine

import sys
import subprocess

def run_skill(input_text: str, output_path: str):
    cmd = f"""{core_command_pattern}"""
    print(f"[Dynamic Skill: {skill_name}] Executing: {{cmd}}")
    res = subprocess.run(cmd, shell=True, capture_output=True, text=True)
    if res.returncode != 0:
        raise RuntimeError(f"Skill execution failed: {{res.stderr}}")
    return res.stdout

if __name__ == "__main__":
    if len(sys.argv) < 3:
        print("Usage: python skill.py <input_text> <output_path>")
        sys.exit(1)
    run_skill(sys.argv[1], sys.argv[2])
"#
        )
    }

    /// 自动扫描并匹配与 Prompt 最相关的领域技能包 (Proactive Skill Scouting & Mounting)
    pub fn scout_and_match_skills(prompt: &str, workspace_root: &Path) -> Vec<MatchedSkill> {
        let mut candidates = Vec::new();
        let p_lower = prompt.to_lowercase();

        // 收集所有候选技能目录
        let mut skill_roots = vec![
            workspace_root.join(".agents").join("skills"),
            workspace_root.join("skills"),
        ];

        // 尝试解析宿主工程真实根目录（沙箱自愈逃逸）
        let origin = crate::infrastructure::fs_path::resolve_origin_project_from_baseline(
            workspace_root,
            &[".agents", "skills"],
        );
        if origin != workspace_root {
            skill_roots.push(origin.join(".agents").join("skills"));
            skill_roots.push(origin.join("skills"));
        }

        // 全局用户配置目录
        if let Some(home) = dirs::home_dir() {
            skill_roots.push(home.join(".gemini").join("config").join("skills"));
            skill_roots.push(home.join(".agents").join("skills"));
        }

        let mut seen_paths = std::collections::HashSet::new();

        for root in skill_roots {
            if !root.is_dir() {
                continue;
            }
            for entry in walkdir::WalkDir::new(&root)
                .max_depth(3)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if entry.file_name() == "SKILL.md" {
                    let path = entry.path().to_path_buf();
                    if !seen_paths.insert(path.clone()) {
                        continue;
                    }
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        let (name, desc, mut instructions) = Self::parse_skill_file(&content, &path);
                        let score = Self::compute_skill_score(&p_lower, &name, &desc);
                        if score > 0.0 {
                            // 针对规则工程技能，挂载工具链与参考知识，严格确立意图驱动
                            if name.contains("rule-engineer") || name.contains("hiker") {
                                let rel_script_dir = "super-agent/skills/deepcloud-rule-engineer/scripts";
                                instructions.push_str(&format!(
                                    r#"

### 🛠️ 【海阔视界规则工程专家技能工具链与能力支持】
本技能为你提供以下经过沙箱验证的自动化工程 CLI 工具（位于 `{rel_script_dir}`）：
1. **静态语法与合规校验**：`node {rel_script_dir}/validate-rule.cjs <rule_json_path>`
   - 检查 JSON 语法、必要字段、首卡剧名契约、find_rule 规范性；
2. **沙箱运行仿真与质量评分**：`node {rel_script_dir}/test-rule.cjs <rule_json_path>`
   - 在真实沙箱中模拟执行，测试一级列表与二级选集，输出 5 星质量评分；
3. **真实口令与链接生成**：`node {rel_script_dir}/export-command.cjs <rule_json_path>`
   - 真实调用 PasteMe API 生成带 16 位哈希的云短口令（`云6oooole/...`）与 Base64 离线口令。

### 🧠 【智能体行动守则（意图驱动与深度反思）】
1. **【严禁把示例当真实需求】**：技能文档与模板仅作为参考规范（Reference Examples），**绝对禁止直接照抄模板或把模板当作用户的任务**！你必须严格聚焦用户当前的真实输入与报错上下文！
2. **【排查报错与修复场景】**：
   - 当用户提供报错日志或请求修复规则时：
     - ① 仔细阅读分析用户提供的报错栈（如语法错误、选择器失效、直链过期、防盗链拦截等）；
     - ② 若用户附带了代码，或在工作区有规则文件，调用 `read_file` 审阅实际代码；
     - ③ 若需要探针目标网站真实结构，调用 `web_fetch` 或 `run_command` 抓取验证；
     - ④ 定位根因并提供修复后的代码，详细解释修改点；若有规则文件，可在测试验证后交付！
3. **【新规则开发场景】**：
   - 遵循双端同构与脱机自治原则，先调研目标站点结构，在 `scratch/` 编写规则；
   - 调用 `validate-rule.cjs` 与 `test-rule.cjs` 进行沙箱验证，确保 5 星通过；
   - 调用 `export-command.cjs` 获取真实的 PasteMe 在线云短口令；
   - 最终将口令、评分收据与完整规则源码交付用户！
4. **【反思自愈闭环】**：遇到工具报错或网络异常时，主动阅读报错信息进行自省纠偏（Critic），修正后重试，绝不盲目放弃或死循环！
"#
                                ));
                            }

                            candidates.push(MatchedSkill {
                                name,
                                description: desc,
                                path: path.to_string_lossy().to_string(),
                                instructions,
                                score,
                            });
                        }
                    }
                }
            }
        }

        candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        candidates.into_iter().take(2).collect()
    }

    pub fn parse_skill_file(content: &str, path: &Path) -> (String, String, String) {
        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() || lines[0].trim() != "---" {
            let parent_name = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
            return (parent_name, "Custom skill".to_string(), content.to_string());
        }

        let mut name = String::new();
        let mut desc = String::new();
        let mut body_start = 1;

        for (idx, line) in lines.iter().enumerate().skip(1) {
            if line.trim() == "---" {
                body_start = idx + 1;
                break;
            }
            if let Some(colon_pos) = line.find(':') {
                let key = line[..colon_pos].trim();
                let val = line[colon_pos + 1..].trim().trim_matches('"').trim_matches('\'');
                if key == "name" {
                    name = val.to_string();
                } else if key == "description" {
                    desc = val.to_string();
                }
            }
        }

        if name.is_empty() {
            name = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();
        }

        let body = lines.into_iter().skip(body_start).collect::<Vec<_>>().join("\n");
        // 示例剥离与净化 (Progressive Disclosure - Strip Large Embedded Example Traps)
        let sanitized_body = Self::strip_embedded_example_traps(&body);
        let mut final_instructions = sanitized_body;

        // 探测技能目录下的模板资源 (Templates)，供中枢与智能体直观感知磁盘路径，严禁全量内联
        if let Some(parent) = path.parent() {
            let tmpl_dir = parent.join("templates");
            if tmpl_dir.is_dir() {
                if let Ok(entries) = std::fs::read_dir(&tmpl_dir) {
                    let mut tmpls = Vec::new();
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.is_file() {
                            tmpls.push(p.file_name().unwrap_or_default().to_string_lossy().to_string());
                        }
                    }
                    if !tmpls.is_empty() {
                        final_instructions.push_str("\n\n### 📦 技能内置开箱即用磁盘模板资源 (Available Templates on Disk)\n");
                        final_instructions.push_str("【中枢提示】：当前技能目录下包含以下成熟模板。注意：**严禁将模板示例当成真实用户的任务产出！若需要参考写法，请通过 `read_file` 工具按需读取磁盘文件**，严禁凭空复制：\n");
                        for t in tmpls {
                            final_instructions.push_str(&format!("- `{}` (路径: `{}`)\n", t, tmpl_dir.join(&t).display()));
                        }
                    }
                }
            }
        }

        (name, desc, final_instructions)
    }

    /// 剥离技能文档中过长、属于示范性的代码块，防止模型误当成任务捷径进行照抄
    pub fn strip_embedded_example_traps(text: &str) -> String {
        let mut result = String::with_capacity(text.len());
        let mut in_code_block = false;
        let mut code_block_lang = String::new();
        let mut current_block = Vec::new();

        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("```") {
                if !in_code_block {
                    in_code_block = true;
                    code_block_lang = trimmed[3..].trim().to_lowercase();
                    current_block.clear();
                    current_block.push(line);
                } else {
                    in_code_block = false;
                    current_block.push(line);
                    let is_large_example = current_block.len() > 10 && (
                        code_block_lang.contains("json") ||
                        code_block_lang.contains("js") ||
                        code_block_lang.contains("javascript") ||
                        code_block_lang.contains("text")
                    );
                    let contains_rule_markers = current_block.iter().any(|l| {
                        l.contains("\"find_rule\"") ||
                        l.contains("\"detail_find_rule\"") ||
                        l.contains("云6oooole") ||
                        l.contains("TencentVideo")
                    });

                    if is_large_example && contains_rule_markers {
                        result.push_str("```markdown\n[⚠️ 示范性规则完整源码与交付模板已自动剥离。如需参考完整蓝本，请使用 read_file 工具查阅技能下的 templates/ 目录，严禁直接复制或充当任务输出！]\n```\n");
                    } else {
                        for blk_line in &current_block {
                            result.push_str(blk_line);
                            result.push('\n');
                        }
                    }
                    current_block.clear();
                }
            } else if in_code_block {
                current_block.push(line);
            } else {
                result.push_str(line);
                result.push('\n');
            }
        }

        result
    }

    pub fn compute_skill_score(p_lower: &str, name: &str, desc: &str) -> f32 {
        let name_lower = name.to_lowercase();
        let desc_lower = desc.to_lowercase();
        let mut score: f32 = 0.0;

        // 1. 技能名命中
        if p_lower.contains(&name_lower) {
            score += 10.0;
        }

        // 拆解技能名短词
        for part in name_lower.split(&['-', '_', ' '][..]) {
            if part.len() >= 3 && p_lower.contains(part) {
                score += 3.0;
            }
        }

        // 2. 经典技能专有触发词
        let triggers: &[(&str, &[&str])] = &[
            (
                "deepcloud-rule-engineer",
                &[
                    "海阔", "规则", "视界", "hiker", "hikerview", "爬虫", "小程序", "转化",
                    "转为", "写规则", "生成规则", "做成规则", "做规则", "视频网站", "腾讯视频",
                    "爱奇艺", "优酷", "bilibili", "b站", "小红书", "短剧", "免嗅", "口令",
                    "解析", "嗅探", "v.qq.com", "qq.com", "spider", "scraper", "rule", "deepcloud-engine",
                ],
            ),
            ("deepcloud-conventions", &["deepcloud", "爬虫", "scraper", "hiker", "海阔", "规范"]),
            ("deepcloud-service-spec", &["微服务", "mybatis", "spring", "iam", "controller", "service-spec", "后端开发", "接口规范"]),
            ("deepcloud-openspec", &["openspec", "sdd", "契约", "规范先行", "台账", "闭环"]),
            ("deepcloud-skin-spec", &["皮肤", "主题", "skin", "theme", "buttonvariant", "立绘", "壁纸"]),
            ("aistack-frontend-conventions", &["前端", "react", "next", "datatable", "tableactions", "modalform", "drawerform", "页面开发"]),
            ("poison-tongue-pm", &["毒舌", "prd", "产品经理", "需求追问", "需求打磨"]),
        ];

        for (target_name, keywords) in triggers {
            if name_lower.contains(target_name) {
                for kw in *keywords {
                    if p_lower.contains(kw) {
                        score += 5.0;
                    }
                }
            }
        }

        // 3. 中文短语与专有名词扫描
        let domain_phrases = [
            "海阔视界", "海阔规则", "视界小程序", "规则生成", "腾讯视频", "双端同构",
            "免嗅", "短口令", "微服务", "mybatis", "前端开发", "主题皮肤", "契约台账"
        ];
        for phrase in domain_phrases {
            if desc_lower.contains(phrase) && p_lower.contains(phrase) {
                score += 6.0;
            }
        }

        // 4. 描述语义轻量命中
        for token in desc_lower.split(&[' ', ',', '，', '。', '、', '(', ')'][..]) {
            if token.len() >= 4 && p_lower.contains(token) {
                score += 1.5;
            }
        }

        score
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inspect_gap_for_speech_synthesis() {
        let tools = vec!["run_command".into(), "read_file".into(), "write_file".into()];
        let report = CapabilityScout::inspect_gap("请帮我把这段文本生成一段语音文件 welcome.mp3", &tools);
        
        assert!(report.is_some());
        let r = report.unwrap();
        assert_eq!(r.missing_capability, DomainCapabilityType::AudioSynthesis);
        assert!(r.candidate_solutions.iter().any(|s| s.name.contains("System.Speech")));
        assert!(r.candidate_solutions.iter().any(|s| s.name.contains("edge-tts")));
        assert!(r.action_directive.contains("动态能力进化指引"));
    }

    #[test]
    fn test_no_gap_when_tool_already_exists() {
        let tools = vec!["run_command".into(), "read_file".into(), "tts_tool".into()];
        let report = CapabilityScout::inspect_gap("请帮我生成语音", &tools);
        assert!(report.is_none());
    }

    #[test]
    fn test_synthesize_skill_script() {
        let script = CapabilityScout::synthesize_skill_script(
            "tts_native",
            "Windows SAPI Speech Synthesis",
            "powershell -Command \"...\"",
        );
        assert!(script.contains("Dynamic Autonomous Skill: tts_native"));
        assert!(script.contains("subprocess.run"));
    }

    #[test]
    fn test_compute_skill_score_and_parse_skill() {
        let content = r#"---
name: deepcloud-service-spec
description: DeepCloud 微服务 DDD 规范与防踩坑指南
---
# 规范正文
严格返回 Result<T>，禁止用 JPA。
"#;
        let (name, desc, body) = CapabilityScout::parse_skill_file(content, Path::new("dummy/SKILL.md"));
        assert_eq!(name, "deepcloud-service-spec");
        assert!(desc.contains("DDD"));
        assert!(body.contains("严格返回 Result<T>"));

        let score = CapabilityScout::compute_skill_score("请按照微服务规范编写 iam-service 接口", &name, &desc);
        assert!(score > 0.0);
    }
}
