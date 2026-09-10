use crate::services::brain::executor::AgentLoop;
use crate::services::brain::ipc::{IpcClient, IpcMessage};
use serde_json::{json, Value};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::process::Command;

fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> std::io::Result<()> {
    let src = src.as_ref();
    let dst = dst.as_ref();
    if !dst.exists() {
        std::fs::create_dir_all(dst)?;
    }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(entry.path(), dst.join(entry.file_name()))?;
        } else {
            std::fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

pub async fn run_superagent_with_evolution(prompt: &str, project: Option<String>) {
    let mut current_prompt = prompt.to_string();
    let mut crash_report_to_send: Option<String> = None;

    // 确保启动前有一个初始的备份
    if !Path::new(".src_backup").exists() {
        if let Err(e) = copy_dir_all("src", ".src_backup") {
            eprintln!("[Brain A] Failed to create initial source backup: {}", e);
        }
    }

    loop {
        println!(
            "
[Brain A] Saving Pre-Task Snapshot (LKG Backup)..."
        );
        let _ = std::fs::remove_dir_all(".src_backup");
        if let Err(e) = copy_dir_all("src", ".src_backup") {
            eprintln!("[Brain A] Failed to backup src: {}", e);
        }
        let _ = std::fs::copy("Cargo.toml", ".Cargo.toml.bak");

        println!(
            "
[Brain A] (Re)starting Superagent (Brain B)..."
        );

        let mut child = match Command::new("cargo")
            .arg("run")
            .arg("--bin")
            .arg("superagent")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()) // Capture stderr for panic logs if needed
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[Brain A] Failed to start superagent: {}", e);
                return;
            }
        };

        // Forward stderr in background so we still see it, but we can capture it
        let mut stderr = child.stderr.take().unwrap();
        let stderr_log = Arc::new(Mutex::new(String::new()));
        let stderr_log_clone = stderr_log.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = [0; 1024];
            while let Ok(n) = stderr.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                let s = String::from_utf8_lossy(&buf[..n]);
                eprint!("{}", s);
                let mut guard = stderr_log_clone.lock().unwrap();
                guard.push_str(&s);
                // Keep only last 2000 chars
                if guard.len() > 2000 {
                    let len = guard.len();
                    *guard = guard[len - 2000..].to_string();
                }
            }
        });

        let mut ipc_client = IpcClient::new(&mut child);

        // 如果我们是从一次崩溃或编译失败中恢复，把报告喂给它
        if let Some(report) = crash_report_to_send.take() {
            println!("[Brain A] Sending CrashReport to Brain B...");
            if let Err(e) = ipc_client
                .send_message(IpcMessage::CrashReport(report))
                .await
            {
                eprintln!("[Brain A] Failed to send CrashReport: {}", e);
            }
        }

        println!("[Brain A] Sending prompt to Brain B...");
        if let Err(e) = ipc_client
            .send_message(IpcMessage::Prompt(current_prompt.clone()))
            .await
        {
            eprintln!("[Brain A] Failed to send prompt: {}", e);
            let _ = child.kill().await;
            break;
        }

        let mut should_reload = false;
        let mut task_completed = false;
        let mut heartbeat_timeout = false;

        loop {
            // 设置 15 秒心跳超时 (Agent 每 3 秒发一次)
            match tokio::time::timeout(
                tokio::time::Duration::from_secs(15),
                ipc_client.read_message(),
            )
            .await
            {
                Ok(Ok(msg)) => match msg {
                    IpcMessage::Result(res) => {
                        println!("\n[Brain A] Brain B Result: \n{}", res);
                        task_completed = true;
                        break;
                    }
                    IpcMessage::Error(err) => {
                        eprintln!("\n[Brain A] Brain B Error: \n{}", err);
                        task_completed = true;
                        break;
                    }
                    IpcMessage::ReloadRequest => {
                        println!("\n[Brain A] Received ReloadRequest from Brain B! Initiating evolution sequence...");
                        should_reload = true;
                        break;
                    }
                    IpcMessage::Heartbeat => {
                        // 心跳正常，忽略
                    }
                    IpcMessage::AskUser { question, .. } => {
                        println!("\n\x1b[35m[Brain B Asks]: {}\x1b[0m", question);
                        use std::io::{self, Write};
                        print!("> ");
                        let _ = io::stdout().flush();
                        let mut answer = String::new();
                        let _ = io::stdin().read_line(&mut answer);

                        if let Err(e) = ipc_client
                            .send_message(IpcMessage::UserResponse {
                                request_id: "".to_string(),
                                response: answer.trim().to_string(),
                            })
                            .await
                        {
                            eprintln!("[Brain A] Failed to send user response: {}", e);
                            break;
                        }
                    }
                    _ => {}
                },
                Ok(Err(e)) => {
                    if e.contains("EOF") {
                        println!("\n[Brain A] Brain B process exited unexpectedly.");
                    } else {
                        eprintln!("\n[Brain A] IPC read error: {}", e);
                    }
                    break; // 异常退出
                }
                Err(_) => {
                    // 超时
                    eprintln!("\n[Brain A] HEARTBEAT TIMEOUT! Brain B is frozen or deadlocked.");
                    heartbeat_timeout = true;
                    break;
                }
            }
        }

        // 杀死当前的 Brain B（不管它死没死，确保清理）
        let _ = child.kill().await;
        let _ = child.wait().await;

        if should_reload {
            println!("[Brain A] Compiling new evolution of Brain B...");
            let build_output = Command::new("cargo")
                .arg("build")
                .arg("--bin")
                .arg("superagent")
                .output()
                .await
                .expect("Failed to execute cargo build");

            if !build_output.status.success() {
                let stderr = String::from_utf8_lossy(&build_output.stderr);
                eprintln!("[Brain A] Evolution failed: Compilation error. Rolling back!");
                let _ = std::fs::remove_dir_all("src");
                let _ = copy_dir_all(".src_backup", "src");
                let _ = std::fs::copy(".Cargo.toml.bak", "Cargo.toml");

                crash_report_to_send = Some(format!("Compilation Error:\n{}", stderr));
                continue; // Restart the old version with the crash report
            }
            println!("[Brain A] Evolution successful! Restarting Brain B...");
            // 新版本成功！它将继承接下来的循环并正常运行
        } else if !task_completed {
            // 非正常退出（Panic 或 Deadlock）
            eprintln!("[Brain A] FATAL: Brain B did not complete the task gracefully.");
            eprintln!("[Brain A] Rolling back source code to Last Known Good state...");
            let _ = std::fs::remove_dir_all("src");
            let _ = copy_dir_all(".src_backup", "src");
            let _ = std::fs::copy(".Cargo.toml.bak", "Cargo.toml");

            let log = stderr_log.lock().unwrap().clone();
            let reason = if heartbeat_timeout {
                "Async Runtime Deadlock or Infinite Loop (Heartbeat Timeout)".to_string()
            } else {
                format!("Process crashed or panicked. Stderr Log:\n{}", log)
            };

            crash_report_to_send = Some(reason);
            println!("[Brain A] Restarting Brain B to learn from its death...");
            continue;
        } else {
            // 正常完成
            break;
        }
    }
}

pub struct Supervisor {
    pub global_plan: Vec<String>,
    pub completed_tasks: Vec<String>,
    pub base_prompt: String,
}

impl Supervisor {
    pub fn new(base_prompt: &str) -> Self {
        Self {
            global_plan: Vec::new(),
            completed_tasks: Vec::new(),
            base_prompt: base_prompt.to_string(),
        }
    }

    pub async fn decompose_task(&mut self, task: &str) -> Result<Vec<String>, String> {
        let mut planner = AgentLoop::new(&format!(
            "You are a Supervisor Agent. Your job is to break down the following complex task into a sequence of clear, actionable sub-tasks. \
            Output ONLY a JSON array of strings, where each string is a sub-task. Do not output any markdown formatting or other text.\n\nTask: {}",
            task
        ));

        let response = planner.run("Generate the plan.").await?;

        let plan: Vec<String> = serde_json::from_str(&response).unwrap_or_else(|_| {
            response
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| {
                    l.trim_start_matches("- ")
                        .trim_start_matches("* ")
                        .to_string()
                })
                .collect()
        });

        self.global_plan = plan.clone();
        Ok(plan)
    }

    pub async fn execute_task(&mut self, task: &str) -> Result<String, String> {
        let plan = self.decompose_task(task).await?;
        let mut final_report = String::new();
        final_report.push_str(&format!("Supervisor Plan:\n{:#?}\n\n", plan));

        for (i, sub_task) in plan.iter().enumerate() {
            let role = if sub_task.to_lowercase().contains("research")
                || sub_task.to_lowercase().contains("search")
                || sub_task.to_lowercase().contains("find")
            {
                "Researcher"
            } else if sub_task.to_lowercase().contains("code")
                || sub_task.to_lowercase().contains("write")
                || sub_task.to_lowercase().contains("implement")
            {
                "Coder"
            } else {
                "Executor"
            };

            let mut sub_agent = AgentLoop::new(&format!(
                "You are a {} Agent. Your current task is: {}. \
                You must complete this task using the tools available to you. \
                When you are done, provide a concise summary of your findings or actions.",
                role, sub_task
            ));

            sub_agent.plugin_registry.load_dynamic_plugins();

            let result = sub_agent
                .run(&format!("Execute task: {}", sub_task))
                .await?;

            self.completed_tasks.push(sub_task.clone());
            final_report.push_str(&format!(
                "--- Task {} ({}) Result ---\n{}\n\n",
                i + 1,
                role,
                result
            ));
        }

        Ok(final_report)
    }
}
