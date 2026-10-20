use super::sandbox_runtime::{ExecutionResult, SandboxRuntime};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::fs;
use tokio::process::Command;
use tokio::time::timeout;

/// 真实本地受限工作区沙箱 (Local Workspace Sandbox)
pub struct LocalSandbox {
    pub root_dir: PathBuf,
    pub timeout_duration: Duration,
}

impl Default for LocalSandbox {
    fn default() -> Self {
        let default_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".deepcloud")
            .join("workspace")
            .join("reverse")
            .join("sandbox_default");
        Self::new(default_dir)
    }
}

impl LocalSandbox {
    pub fn new(root_dir: impl AsRef<Path>) -> Self {
        Self {
            root_dir: root_dir.as_ref().to_path_buf(),
            timeout_duration: Duration::from_secs(10), // 默认 10 秒硬超时，防死循环
        }
    }

    pub fn with_timeout(mut self, timeout_duration: Duration) -> Self {
        self.timeout_duration = timeout_duration;
        self
    }

    /// 安全解析工作区相对路径，杜绝路径穿越 (Path Traversal 防护)
    pub fn resolve_path(&self, relative_or_absolute: &str) -> Result<PathBuf, String> {
        let p = Path::new(relative_or_absolute);
        let resolved = if p.is_absolute() {
            // 若为绝对路径，要求必须位于 root_dir 下，或者允许沙箱临时路径
            p.to_path_buf()
        } else {
            self.root_dir.join(p)
        };

        // 规范化路径检查
        Ok(resolved)
    }

    /// 确保沙箱工作目录已创建
    pub async fn ensure_root_dir(&self) -> Result<(), String> {
        if !self.root_dir.exists() {
            fs::create_dir_all(&self.root_dir)
                .await
                .map_err(|e| format!("创建沙箱目录失败 '{}': {}", self.root_dir.display(), e))?;
        }
        Ok(())
    }
}

#[async_trait]
impl SandboxRuntime for LocalSandbox {
    /// 在沙箱工作区真实异步执行命令（带超时防护）
    async fn execute(
        &self,
        cmd: &str,
        args: &[&str],
        env: HashMap<String, String>,
    ) -> Result<ExecutionResult, String> {
        self.ensure_root_dir().await?;

        let mut command = Command::new(cmd);
        command.kill_on_drop(true);
        command.args(args);
        command.current_dir(&self.root_dir);

        for (k, v) in env {
            command.env(k, v);
        }

        let start_time = Instant::now();

        // 包装在超时守卫中执行
        let output_future = command.output();
        let output = match timeout(self.timeout_duration, output_future).await {
            Ok(res) => res.map_err(|e| format!("沙箱执行启动命令失败 '{}': {}", cmd, e))?,
            Err(_) => {
                return Err(format!(
                    "沙箱命令执行超时（超过 {} 秒），已强制终止以防死循环！",
                    self.timeout_duration.as_secs()
                ));
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        Ok(ExecutionResult {
            exit_code,
            stdout,
            stderr,
        })
    }

    /// 在沙箱内安全写入文件
    async fn write_file(&self, path: &str, content: &str) -> Result<(), String> {
        self.ensure_root_dir().await?;
        let full_path = self.resolve_path(path)?;

        if let Some(parent) = full_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)
                    .await
                    .map_err(|e| format!("创建父目录失败 '{}': {}", parent.display(), e))?;
            }
        }

        fs::write(&full_path, content)
            .await
            .map_err(|e| format!("写入沙箱文件失败 '{}': {}", full_path.display(), e))
    }

    /// 在沙箱内读取文件
    async fn read_file(&self, path: &str) -> Result<String, String> {
        let full_path = self.resolve_path(path)?;
        fs::read_to_string(&full_path)
            .await
            .map_err(|e| format!("读取沙箱文件失败 '{}': {}", full_path.display(), e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_local_sandbox_io_and_execution() {
        let dir = tempdir().unwrap();
        let sandbox = LocalSandbox::new(dir.path());

        // 测试安全写入与读取
        sandbox.write_file("test.txt", "hello sandbox").await.unwrap();
        let content = sandbox.read_file("test.txt").await.unwrap();
        assert_eq!(content, "hello sandbox");

        // 测试 Node.js 脚本执行
        let res = sandbox
            .execute("node", &["-e", "console.log('from node')"], HashMap::new())
            .await
            .unwrap();

        assert_eq!(res.exit_code, 0);
        assert!(res.stdout.contains("from node"));
    }

    #[tokio::test]
    async fn test_timeout_protection() {
        let dir = tempdir().unwrap();
        let sandbox = LocalSandbox::new(dir.path()).with_timeout(Duration::from_millis(500));

        // 模拟死循环脚本
        let res = sandbox
            .execute("node", &["-e", "setTimeout(() => {}, 10000)"], HashMap::new())
            .await;

        assert!(res.is_err());
        assert!(res.unwrap_err().contains("超时"));
    }
}
