use std::sync::Mutex;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

pub trait ProgressReporter: Send + Sync {
    fn report(&self, step_name: &str, status: StepStatus);
}

pub struct ConsoleReporter {
    last_step: Mutex<Option<(String, Instant)>>,
}

impl ConsoleReporter {
    pub fn new() -> Self {
        Self {
            last_step: Mutex::new(None),
        }
    }
}

impl ProgressReporter for ConsoleReporter {
    fn report(&self, step_name: &str, status: StepStatus) {
        let mut last = self.last_step.lock().unwrap();

        match status {
            StepStatus::InProgress => {
                // Print pending state
                println!("\x1b[90m[ ] {}\x1b[0m", step_name);
                *last = Some((step_name.to_string(), Instant::now()));
            }
            StepStatus::Completed => {
                if let Some((name, start_time)) = last.take() {
                    if name == step_name {
                        // Move cursor up 1 line (\x1b[1A) and clear entire line (\x1b[2K)
                        print!("\x1b[1A\x1b[2K\r");
                        let elapsed = start_time.elapsed().as_millis();
                        println!(
                            "\x1b[32m[x]\x1b[0m \x1b[90m{} ({}ms)\x1b[0m",
                            step_name, elapsed
                        );
                    } else {
                        println!("\x1b[32m[x]\x1b[0m \x1b[90m{}\x1b[0m", step_name);
                    }
                } else {
                    println!("\x1b[32m[x]\x1b[0m \x1b[90m{}\x1b[0m", step_name);
                }
            }
            StepStatus::Failed => {
                if let Some((name, start_time)) = last.take() {
                    if name == step_name {
                        print!("\x1b[1A\x1b[2K\r");
                        let elapsed = start_time.elapsed().as_millis();
                        println!(
                            "\x1b[31m[!] {}\x1b[0m \x1b[90m({}ms)\x1b[0m",
                            step_name, elapsed
                        );
                    } else {
                        println!("\x1b[31m[!]\x1b[0m \x1b[90m{}\x1b[0m", step_name);
                    }
                } else {
                    println!("\x1b[31m[!]\x1b[0m \x1b[90m{}\x1b[0m", step_name);
                }
            }
            StepStatus::Pending => {
                println!("\x1b[90m[ ] {}\x1b[0m", step_name);
            }
        }
    }
}
