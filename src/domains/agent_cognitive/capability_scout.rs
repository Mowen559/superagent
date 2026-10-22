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
                        let (name, desc, instructions) = Self::parse_skill_file(&content, &path);
                        let score = Self::compute_skill_score(&p_lower, &name, &desc);
                        if score > 0.0 {
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
        (name, desc, body)
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
            ("deepcloud-service-spec", &["微服务", "mybatis", "spring", "iam", "controller", "service-spec", "后端开发", "接口规范"]),
            ("deepcloud-openspec", &["openspec", "sdd", "契约", "规范先行", "台账", "闭环"]),
            ("deepcloud-skin-spec", &["皮肤", "主题", "skin", "theme", "buttonvariant", "立绘", "壁纸"]),
            ("deepcloud-conventions", &["deepcloud", "爬虫", "scraper", "hiker", "海阔", "规范"]),
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

        // 3. 描述语义轻量命中
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
