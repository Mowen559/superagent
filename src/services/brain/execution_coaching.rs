//! Explicit component-level execution conversations, never full AgentLoop certification.
//! Model tools can only read two synthetic files, write bounded JSON and run one
//! immutable Node acceptance test. No shell or model-generated code is executed.
use super::cognitive_runtime::{CognitiveRuntime, CompletionDecision, ReasoningMode};
use super::task_evaluation::EvaluationModel;
use super::tool_execution_gateway::{
    ToolExecutionGateway, ToolExecutionRequest, ToolExecutionStatus, ToolRegistry,
};
use super::tools::{
    plan_task::PlanTaskTool, revise_task_plan::ReviseTaskPlanTool, task_progress::TaskProgressTool,
};
use crate::domains::agent_cognitive::executor::ToolExecutor;
use crate::domains::model_gateway::routing::ProviderAttemptOutcome;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const MAX_TURNS: usize = 10;
const CHECK: &str = "node --test acceptance.test.cjs";

#[derive(Clone)]
pub struct CoachingCase {
    pub id: &'static str,
    pub initial: Value,
    pub requests: [String; 2],
    pub expected: [Value; 2],
}

pub fn cases() -> Vec<CoachingCase> {
    [("private-routing", true, "LOCAL", 4, 2, 3),
     ("public-capacity", false, "CLOUD", 8, 1, 2)].into_iter().map(|(id, private, target, limit, retries, next)| {
        let initial = json!({"executionTarget":"INVALID", "privateCredentials":private,
            "maxConcurrency":64, "retries":0, "userNote":"用户备注：原样保留", "extension":{"enabled":true}});
        let mut first = initial.clone();
        first["executionTarget"] = json!(target);
        first["maxConcurrency"] = json!(limit);
        first["retries"] = json!(retries);
        let mut second = first.clone();
        second["retries"] = json!(next);
        CoachingCase { id, initial, requests: [
            format!("修复当前 settings.json 配置：executionTarget 设为 {target}，maxConcurrency 设为 {limit}，retries 设为 {retries}。其余字段和值原样保留。先读取工作区说明和文件，再实际修改并运行验收测试，完成后报告实际结果。"),
            format!("追加要求：将 retries 调整为 {next}，保留先前的执行目标、并发上限和所有其他原始字段。请实际修改，并对更新后的文件重新运行验收测试。")
        ], expected:[first, second] }
    }).collect()
}

#[derive(Serialize)]
pub struct CoachingReport {
    pub schema_version: u32,
    pub scope: &'static str,
    pub max_model_turns: usize,
    pub results: Vec<ConversationReport>,
}

#[derive(Serialize)]
pub struct ConversationReport {
    pub case_id: String,
    pub status: String,
    pub verified_stages: usize,
    pub independent_final_pass: bool,
    pub baseline_rejected: bool,
    pub model_turns: usize,
    pub tool_calls: usize,
    pub failed_tools: usize,
    pub completion_corrections: usize,
    pub actual_models: BTreeSet<String>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub fixture_fingerprint: String,
    pub duration_ms: u64,
    pub transcript: Vec<Value>,
}

/// Restricted adapters for this evaluation only; not installed as agent tools.
struct FixtureTool {
    name: &'static str,
    root: PathBuf,
}

impl ToolExecutor for FixtureTool {
    fn name(&self) -> &'static str {
        self.name
    }
    fn description(&self) -> &'static str {
        match self.name {
            "view_file" => {
                "Read README.md or settings.json in this temporary workspace, using their exact relative names."
            }
            "write_to_file" => {
                "Overwrite settings.json with a JSON object; no other file is writable. Preserve unrelated fields."
            }
            _ => {
                "Run the immutable acceptance test: node --test acceptance.test.cjs. Only this exact command is available."
            }
        }
    }
    fn parameters_schema(&self) -> Value {
        match self.name {
            "view_file" => {
                json!({"type":"object","properties":{"AbsolutePath":{"type":"string","enum":["README.md","settings.json"]}},"required":["AbsolutePath"]})
            }
            "write_to_file" => {
                json!({"type":"object","properties":{"TargetFile":{"type":"string","enum":["settings.json"]},"CodeContent":{"type":"string"}},"required":["TargetFile","CodeContent"]})
            }
            _ => {
                json!({"type":"object","properties":{"CommandLine":{"type":"string","enum":[CHECK]}},"required":["CommandLine"]})
            }
        }
    }
    fn execute(&self, args: Value) -> Result<Value, String> {
        match self.name {
            "view_file" => {
                let name = args["AbsolutePath"].as_str().unwrap_or("");
                if !matches!(name, "README.md" | "settings.json") {
                    return Err(
                        "Read only README.md or settings.json with exact relative names".into(),
                    );
                }
                let content =
                    fs::read_to_string(self.root.join(name)).map_err(|_| "fixture read failed")?;
                Ok(json!({"path":name,"content":content}))
            }
            "write_to_file" => {
                if args["TargetFile"] != "settings.json" {
                    return Err("Only settings.json is writable".into());
                }
                let content = args["CodeContent"]
                    .as_str()
                    .ok_or("CodeContent must be JSON text")?;
                if content.len() > 4096 {
                    return Err("JSON exceeds 4096-byte limit".into());
                }
                let value: Value = serde_json::from_str(content)
                    .map_err(|_| "Invalid JSON; inspect and repair the JSON object")?;
                if !value.is_object() {
                    return Err("Expected a JSON object".into());
                }
                fs::write(self.root.join("settings.json"), content)
                    .map_err(|_| "fixture write failed")?;
                Ok(json!({"path":"settings.json","written":true}))
            }
            _ => {
                if args["CommandLine"] != CHECK || args["RunInBackground"] == true {
                    return Err("Only the fixed foreground acceptance command is allowed".into());
                }
                run_check(&self.root)
            }
        }
    }
}

fn install_check(root: &std::path::Path, expected: &Value) -> Result<(), String> {
    // Expected values are built-in synthetic data, never interpolated user code.
    let source = format!(
        "const test=require('node:test');const assert=require('node:assert/strict');const fs=require('node:fs');\ntest('configuration matches all requirements and preserves unrelated data',()=>{{const actual=JSON.parse(fs.readFileSync('settings.json','utf8'));assert.deepStrictEqual(actual,{});}});\n",
        expected
    );
    fs::write(root.join("acceptance.test.cjs"), source)
        .map_err(|_| "cannot install acceptance check".into())
}

fn run_check(root: &std::path::Path) -> Result<Value, String> {
    let out = tempfile::tempfile().map_err(|_| "cannot create test output")?;
    let err = tempfile::tempfile().map_err(|_| "cannot create test error output")?;
    let mut child = Command::new("node")
        .args(["--test", "acceptance.test.cjs"])
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(out.try_clone().map_err(|_| "output clone failed")?)
        .stderr(err.try_clone().map_err(|_| "error clone failed")?)
        .spawn()
        .map_err(|_| "Node executable unavailable")?;
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if start.elapsed() < Duration::from_secs(5) => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("acceptance check timed out or wait failed".into());
            }
        }
    };
    use std::io::{Read, Seek, SeekFrom};
    fn output(mut file: fs::File) -> String {
        let _ = file.seek(SeekFrom::Start(0));
        let mut text = String::new();
        let _ = file.take(6000).read_to_string(&mut text);
        text
    }
    Ok(
        json!({"exit_code":status.code().unwrap_or(-1),"success":status.success(),"stdout":output(out),"stderr":output(err)}),
    )
}

fn install_planning(registry: &ToolRegistry, cognition: &CognitiveRuntime) {
    let mut tools = registry.lock().unwrap();
    for tool in [
        Arc::new(PlanTaskTool) as Arc<dyn ToolExecutor>,
        Arc::new(ReviseTaskPlanTool(cognition.task_progress())),
        Arc::new(TaskProgressTool(cognition.task_progress())),
    ] {
        tools.insert(tool.name().into(), tool);
    }
}

pub async fn evaluate(model: &impl EvaluationModel, fixtures: &[CoachingCase]) -> CoachingReport {
    let mut results = Vec::new();
    for case in fixtures.iter().take(2) {
        let result = conversation(model, case).await;
        let unavailable = matches!(
            result.status.as_str(),
            "provider_error" | "request_timeout" | "fixture_error"
        );
        eprintln!(
            "[ExecutionCoaching] {}: {} ({} stages; {} turns)",
            case.id, result.status, result.verified_stages, result.model_turns
        );
        results.push(result);
        if unavailable {
            break;
        }
    }
    CoachingReport {
        schema_version: 1,
        scope: "component_execution_coaching; synthetic JSON repair with production cognitive/tool gateways; not full AgentLoop or Golden Evals certification",
        max_model_turns: fixtures.len().min(2) * MAX_TURNS,
        results,
    }
}

async fn conversation(model: &impl EvaluationModel, case: &CoachingCase) -> ConversationReport {
    let start = Instant::now();
    let mut report = ConversationReport {
        case_id: case.id.into(),
        status: "turn_budget_exhausted".into(),
        verified_stages: 0,
        independent_final_pass: false,
        baseline_rejected: false,
        model_turns: 0,
        tool_calls: 0,
        failed_tools: 0,
        completion_corrections: 0,
        actual_models: BTreeSet::new(),
        prompt_tokens: 0,
        completion_tokens: 0,
        fixture_fingerprint: format!(
            "xxh3:{:016x}",
            xxhash_rust::xxh3::xxh3_64(
                json!([case.initial, case.requests, case.expected])
                    .to_string()
                    .as_bytes()
            )
        ),
        duration_ms: 0,
        transcript: vec![],
    };
    let Ok(root) = tempfile::tempdir() else {
        report.status = "fixture_error".into();
        return report;
    };
    let setup = fs::write(root.path().join("settings.json"), case.initial.to_string()).and_then(|_| fs::write(root.path().join("README.md"), format!("Temporary configuration project. Edit only settings.json. Preserve unrelated fields. Run `{CHECK}` after every change. Acceptance tests are immutable.\n")));
    if setup.is_err() || install_check(root.path(), &case.expected[0]).is_err() {
        report.status = "fixture_error".into();
        return report;
    }
    match run_check(root.path()) {
        Ok(result) if result["exit_code"].as_i64().is_some_and(|code| code > 0) => {
            report.baseline_rejected = true
        }
        _ => {
            report.status = "fixture_error".into();
            return report;
        }
    }
    let registry: ToolRegistry = Arc::new(Mutex::new(HashMap::new()));
    for name in ["view_file", "write_to_file", "run_command"] {
        registry.lock().unwrap().insert(
            name.into(),
            Arc::new(FixtureTool {
                name,
                root: root.path().to_path_buf(),
            }),
        );
    }
    let gateway = ToolExecutionGateway::new(registry.clone());
    let mut cognition = CognitiveRuntime::with_mode(&case.requests[0], ReasoningMode::Adaptive);
    cognition.limit_model_turns(MAX_TURNS);
    install_planning(&registry, &cognition);
    let mut messages = vec![json!({"role":"user","content":case.requests[0]})];
    for turn in 1..=MAX_TURNS {
        report.model_turns = turn;
        let guidance = match cognition.next_guidance() {
            Ok(v) => v,
            Err(_) => break,
        };
        let schemas: Vec<Value> = {
            let tools = registry.lock().unwrap();
            let mut names: Vec<_> = tools.keys().collect();
            names.sort();
            names.into_iter().map(|name| {let tool=&tools[name];json!({"type":"function","function":{"name":tool.name(),"description":tool.description(),"parameters":tool.parameters_schema()}})}).collect()
        };
        let system = format!(
            "You are SuperAgent in a temporary configuration workspace. Complete the user's task by using the available tools. Read README.md and settings.json, preserve unrelated data, make the change, run the fixed acceptance command, then report actual results. File arguments use exact relative names. Do not ask for information already in the files. A simple task does not require a plan. Tool results include receipt IDs for optional task progress.\n{guidance}"
        );
        let response = tokio::time::timeout(
            Duration::from_secs(60),
            model.turn(messages.clone(), &system, json!(schemas)),
        )
        .await;
        let (message, usage) = match response {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => {
                report.status = "provider_error".into();
                break;
            }
            Err(_) => {
                report.status = "request_timeout".into();
                break;
            }
        };
        report.prompt_tokens += usage.prompt_tokens as u64;
        report.completion_tokens += usage.completion_tokens as u64;
        for attempt in usage.attempt_receipts {
            if attempt.outcome == ProviderAttemptOutcome::Succeeded {
                report
                    .actual_models
                    .insert(format!("{}:{}", attempt.provider, attempt.resolved_model));
            }
        }
        let calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if calls.len() > 8 {
            report.status = "tool_batch_limit".into();
            break;
        }
        messages.push(message.clone());
        if calls.is_empty() {
            let draft = message["content"].as_str().unwrap_or("");
            match cognition.review_completion(draft) {
                CompletionDecision::Accept => {
                    let passed = run_check(root.path())
                        .ok()
                        .is_some_and(|v| v["exit_code"] == 0);
                    if !passed {
                        report.status = "independent_check_failed".into();
                        break;
                    }
                    report.verified_stages += 1;
                    if report.verified_stages == 2 {
                        report.status = "passed".into();
                        report.independent_final_pass = true;
                        break;
                    }
                    if install_check(root.path(), &case.expected[1]).is_err() {
                        report.status = "fixture_error".into();
                        break;
                    }
                    messages.push(json!({"role":"user","content":case.requests[1]}));
                    // A new user turn is a new Run. No prior receipt can satisfy it.
                    gateway.begin_run();
                    cognition = CognitiveRuntime::with_mode(
                        &format!("{}\n{}", case.requests[0], case.requests[1]),
                        ReasoningMode::Adaptive,
                    );
                    cognition.limit_model_turns(MAX_TURNS - turn);
                    install_planning(&registry, &cognition);
                }
                CompletionDecision::Revise { reason } => {
                    report.completion_corrections += 1;
                    messages.push(
                        json!({"role":"user","content":format!("Completion feedback: {reason}")}),
                    );
                }
                CompletionDecision::Blocked { .. } => {
                    report.status = "completion_blocked".into();
                    break;
                }
            }
            continue;
        }
        for (index, call) in calls.iter().enumerate() {
            let name = call["function"]["name"].as_str().unwrap_or("");
            let args = call["function"]["arguments"]
                .as_str()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(Value::Null);
            let id = call["id"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("turn-{turn}-{index}"));
            let receipt = gateway.execute(ToolExecutionRequest {
                call_id: id.clone(),
                name: name.into(),
                arguments: args,
            });
            report.tool_calls += 1;
            if receipt.status != ToolExecutionStatus::Succeeded {
                report.failed_tools += 1;
            }
            cognition.observe_receipt(&receipt);
            messages.push(json!({"role":"tool","tool_call_id":id,"content":json!({"receipt_id":id,"status":format!("{:?}",receipt.status),"verification":receipt.verification,"result":receipt.result}).to_string()}));
        }
    }
    report.duration_ms = start.elapsed().as_millis() as u64;
    // Only synthetic data appears in messages. Do not persist provider errors/settings.
    report.transcript = messages;
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::model_gateway::providers::TokenUsage;
    use async_trait::async_trait;
    use std::collections::VecDeque;

    struct Scripted(Mutex<VecDeque<Value>>);
    #[async_trait]
    impl EvaluationModel for Scripted {
        async fn turn(
            &self,
            _: Vec<Value>,
            _: &str,
            _: Value,
        ) -> Result<(Value, TokenUsage), String> {
            Ok((
                self.0
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("script exhausted"),
                TokenUsage::default(),
            ))
        }
    }
    fn calls(actions: Vec<(&str, Value)>) -> Value {
        json!({"role":"assistant","content":null,"tool_calls":actions.into_iter().enumerate().map(|(i,(name,args))|json!({"id":format!("call-{i}"),"type":"function","function":{"name":name,"arguments":args.to_string()}})).collect::<Vec<_>>()})
    }
    fn final_answer() -> Value {
        json!({"role":"assistant","content":"已修改配置并运行验收检查，当前要求及保留字段均已通过实际验证。"})
    }

    #[tokio::test]
    async fn real_files_checks_and_new_user_turn_require_fresh_execution() {
        let case = cases().remove(0);
        let model = Scripted(Mutex::new(VecDeque::from([
            calls(vec![
                ("view_file", json!({"AbsolutePath":"README.md"})),
                ("view_file", json!({"AbsolutePath":"settings.json"})),
                ("run_command", json!({"CommandLine":CHECK})), // real red before repair
                (
                    "write_to_file",
                    json!({"TargetFile":"settings.json","CodeContent":case.expected[0].to_string()}),
                ),
                ("run_command", json!({"CommandLine":CHECK})),
            ]),
            final_answer(),
            calls(vec![
                ("view_file", json!({"AbsolutePath":"settings.json"})),
                (
                    "write_to_file",
                    json!({"TargetFile":"settings.json","CodeContent":case.expected[1].to_string()}),
                ),
                ("run_command", json!({"CommandLine":CHECK})),
            ]),
            final_answer(),
        ])));
        let report = conversation(&model, &case).await;
        assert_eq!(report.status, "passed");
        assert!(report.baseline_rejected && report.independent_final_pass);
        assert_eq!(report.verified_stages, 2);
        assert_eq!(report.failed_tools, 1);
        assert!(
            report.actual_models.is_empty(),
            "scripted model must not invent real model identity"
        );
    }

    #[tokio::test]
    async fn completion_claims_and_old_verification_do_not_pass_the_next_run() {
        let case = cases().remove(0);
        let model = Scripted(Mutex::new(VecDeque::from([
            calls(vec![
                ("view_file", json!({"AbsolutePath":"settings.json"})),
                (
                    "write_to_file",
                    json!({"TargetFile":"settings.json","CodeContent":case.expected[0].to_string()}),
                ),
                ("run_command", json!({"CommandLine":CHECK})),
            ]),
            final_answer(),
            final_answer(),
            final_answer(),
        ])));
        let report = conversation(&model, &case).await;
        assert_eq!(report.status, "completion_blocked");
        assert_eq!(report.verified_stages, 1);
        assert!(!report.independent_final_pass);
        assert_eq!(report.completion_corrections, 1);
    }

    #[tokio::test]
    async fn failed_check_feedback_can_correct_an_early_completion() {
        let case = cases().remove(0);
        let model = Scripted(Mutex::new(VecDeque::from([
            final_answer(), // causes bounded completion coaching
            calls(vec![
                ("view_file", json!({"AbsolutePath":"settings.json"})),
                (
                    "write_to_file",
                    json!({"TargetFile":"settings.json","CodeContent":case.expected[0].to_string()}),
                ),
                ("run_command", json!({"CommandLine":CHECK})),
            ]),
            final_answer(),
            calls(vec![
                ("view_file", json!({"AbsolutePath":"settings.json"})),
                (
                    "write_to_file",
                    json!({"TargetFile":"settings.json","CodeContent":case.expected[1].to_string()}),
                ),
                ("run_command", json!({"CommandLine":CHECK})),
            ]),
            final_answer(),
        ])));
        let report = conversation(&model, &case).await;
        assert_eq!(report.status, "passed");
        assert_eq!(report.completion_corrections, 1);
    }

    #[test]
    fn fixture_tools_reject_escape_code_and_arbitrary_commands() {
        let root = tempfile::tempdir().unwrap();
        let tool = |name| FixtureTool {
            name,
            root: root.path().to_path_buf(),
        };
        assert!(
            tool("view_file")
                .execute(json!({"AbsolutePath":"../settings.json"}))
                .is_err()
        );
        assert!(
            tool("write_to_file")
                .execute(
                    json!({"TargetFile":"acceptance.test.cjs","CodeContent":"process.exit(0)"})
                )
                .is_err()
        );
        assert!(
            tool("write_to_file")
                .execute(json!({"TargetFile":"settings.json","CodeContent":"process.exit(0)"}))
                .is_err()
        );
        assert!(
            tool("run_command")
                .execute(json!({"CommandLine":"node --test acceptance.test.cjs; echo pass"}))
                .is_err()
        );
    }
}
