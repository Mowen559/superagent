//! 能力缺口自省与自主进化动态技能引擎 (Just-In-Time Capability Acquisition & Skill Scout)
//! 当智能体面临其工具库未覆盖的全新领域任务时（如语音合成TTS、格式转换、音视频剪辑、私有逆向等），
//! 主动诊断能力缺口，智能推荐系统原生与开源生态方案，并在沙箱验证后动态沉淀为终生技能。

use serde::{Deserialize, Serialize};

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
}
