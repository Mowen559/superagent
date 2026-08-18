use crate::services::brain::ipc::{IpcClient, IpcMessage};
use std::process::Stdio;
use tokio::process::Command;

pub async fn run_superagent_with_evolution(prompt: &str) {
    let mut current_prompt = prompt.to_string();

    loop {
        println!("\n[Brain A] (Re)starting Superagent (Brain B)...");
        
        // 启动 superagent 进程
        let mut child = match Command::new("cargo")
            .arg("run")
            .arg("--bin")
            .arg("superagent")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                eprintln!("[Brain A] Failed to start superagent: {}", e);
                return;
            }
        };

        let mut ipc_client = IpcClient::new(&mut child);

        // 发送 Prompt
        println!("[Brain A] Sending prompt to Brain B...");
        if let Err(e) = ipc_client.send_message(IpcMessage::Prompt(current_prompt.clone())).await {
            eprintln!("[Brain A] Failed to send prompt: {}", e);
            let _ = child.kill().await;
            break;
        }

        let mut should_reload = false;

        // 监听消息
        loop {
            match ipc_client.read_message().await {
                Ok(msg) => match msg {
                    IpcMessage::Result(res) => {
                        println!("\n[Brain A] Brain B Result: \n{}", res);
                        // 任务完成，退出内层循环并结束整个推演
                        break;
                    }
                    IpcMessage::Error(err) => {
                        eprintln!("\n[Brain A] Brain B Error: \n{}", err);
                        // 发生错误，结束推演
                        break;
                    }
                    IpcMessage::ReloadRequest => {
                        println!("\n[Brain A] Received ReloadRequest from Brain B! Initiating evolution sequence...");
                        should_reload = true;
                        break;
                    }
                    IpcMessage::AskUser { question } => {
                        println!("\n\x1b[35m[Brain B Asks]: {}\x1b[0m", question);
                        use std::io::{self, Write};
                        print!("> ");
                        let _ = io::stdout().flush();
                        let mut answer = String::new();
                        let _ = io::stdin().read_line(&mut answer);
                        
                        if let Err(e) = ipc_client.send_message(IpcMessage::UserResponse(answer.trim().to_string())).await {
                            eprintln!("[Brain A] Failed to send user response: {}", e);
                            break;
                        }
                    }
                    _ => {}
                },
                Err(e) => {
                    if e.contains("EOF") {
                        println!("\n[Brain A] Brain B process exited.");
                    } else {
                        eprintln!("\n[Brain A] IPC read error: {}", e);
                    }
                    break;
                }
            }
        }

        // 杀死当前的 Brain B（不管它死没死，确保清理）
        let _ = child.kill().await;
        let _ = child.wait().await;

        if should_reload {
            println!("[Brain A] Compiling new evolution of Brain B...");
            let build_status = Command::new("cargo")
                .arg("build")
                .arg("--bin")
                .arg("superagent")
                .status()
                .await
                .expect("Failed to execute cargo build");

            if !build_status.success() {
                eprintln!("[Brain A] Evolution failed: Compilation error. Stopping here.");
                break;
            }
            println!("[Brain A] Evolution successful! Restarting Brain B...");
            // 如果成功编译，自动进入下一轮 loop，重启 Brain B
            // 注意：我们让新一代 Brain B 继续执行之前的 prompt（或者你也可以要求它上报当前执行状态并接力）
        } else {
            // 没有触发重载，表示正常结束
            break;
        }
    }
}
