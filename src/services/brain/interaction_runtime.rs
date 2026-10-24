use std::io::{self, Write};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ipc::{send_to_hub, IpcMessage};

pub const MAX_QUESTION_CHARS: usize = 2_000;
pub const MAX_CHOICE_CHARS: usize = 300;
pub const MAX_CHOICES: usize = 8;
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 5 * 60;
pub const MAX_TIMEOUT_SECONDS: u64 = 30 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct InteractionChoice {
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub recommended: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionRequest {
    pub request_id: String,
    pub question: String,
    pub choices: Vec<String>,
    /// Structured choice metadata (label/description/recommended), aligned with
    /// ZCode AskUserQuestion & DeepSeek AskUserQuestionItem. Empty for plain choices.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choice_details: Vec<InteractionChoice>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InteractionReceipt {
    pub request_id: String,
    pub response: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InteractionMode {
    Ipc,
    Console,
}

#[derive(Debug)]
struct PendingInteraction {
    request_id: String,
    response: Option<String>,
}

#[derive(Debug, Default)]
struct InteractionState {
    pending: Option<PendingInteraction>,
}

/// Owns the single pending user-input request. IPC and Console are transport
/// adapters; correlation, admission, timeout, and late-response rejection stay
/// behind this Interface.
#[derive(Debug)]
pub struct InteractionRuntime {
    mode: Mutex<InteractionMode>,
    state: Mutex<InteractionState>,
    response_ready: Condvar,
}

impl Default for InteractionRuntime {
    fn default() -> Self {
        Self {
            mode: Mutex::new(InteractionMode::Ipc),
            state: Mutex::new(InteractionState::default()),
            response_ready: Condvar::new(),
        }
    }
}

impl InteractionRuntime {
    pub fn use_console(&self) {
        *self
            .mode
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = InteractionMode::Console;
    }

    pub fn ask(
        &self,
        question: &str,
        choices: Vec<String>,
        timeout: Duration,
    ) -> Result<InteractionReceipt, String> {
        self.ask_with_details(question, choices, Vec::new(), timeout)
    }

    pub fn ask_with_details(
        &self,
        question: &str,
        choices: Vec<String>,
        choice_details: Vec<InteractionChoice>,
        timeout: Duration,
    ) -> Result<InteractionReceipt, String> {
        let request = validate_request(question, choices, choice_details, timeout)?;
        self.begin(&request)?;
        let mode = *self
            .mode
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        match mode {
            InteractionMode::Console => {
                let response = match read_console_response(&request) {
                    Ok(response) => response,
                    Err(error) => {
                        self.clear_pending(&request.request_id);
                        return Err(error);
                    }
                };
                let _ = self.deliver_response(&request.request_id, &response);
            }
            InteractionMode::Ipc => send_to_hub(IpcMessage::AskUser {
                request_id: request.request_id.clone(),
                question: request.question.clone(),
                choices: request.choices.clone(),
                choice_details: request.choice_details.clone(),
            }),
        }

        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (mut state, wait) = self
            .response_ready
            .wait_timeout_while(state, timeout, |state| {
                state
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.response.is_none())
            })
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let pending = state
            .pending
            .take()
            .ok_or_else(|| "interaction request lost its pending state".to_string())?;
        if wait.timed_out() && pending.response.is_none() {
            return Err(format!(
                "interaction request timed out: {}",
                pending.request_id
            ));
        }
        let response = pending
            .response
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "No additional instructions provided. Conclude and deliver final report.".to_string());
        Ok(InteractionReceipt {
            request_id: pending.request_id,
            response,
        })
    }

    pub fn deliver_response(&self, request_id: &str, response: &str) -> bool {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(pending) = state.pending.as_mut() else {
            return false;
        };
        if pending.request_id != request_id || pending.response.is_some() {
            return false;
        }
        pending.response = Some(bounded(response.trim(), MAX_QUESTION_CHARS));
        self.response_ready.notify_all();
        true
    }

    fn begin(&self, request: &InteractionRequest) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.pending.is_some() {
            return Err("another interaction request is already pending".to_string());
        }
        state.pending = Some(PendingInteraction {
            request_id: request.request_id.clone(),
            response: None,
        });
        Ok(())
    }

    fn clear_pending(&self, request_id: &str) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state
            .pending
            .as_ref()
            .is_some_and(|pending| pending.request_id == request_id)
        {
            state.pending = None;
        }
    }
}

fn validate_request(
    question: &str,
    choices: Vec<String>,
    choice_details: Vec<InteractionChoice>,
    timeout: Duration,
) -> Result<InteractionRequest, String> {
    let question = question.trim();
    if question.is_empty() || question.chars().count() > MAX_QUESTION_CHARS {
        return Err(format!(
            "question must contain 1..={MAX_QUESTION_CHARS} characters"
        ));
    }
    if choices.len() > MAX_CHOICES {
        return Err(format!("at most {MAX_CHOICES} choices are allowed"));
    }
    let choices = choices
        .into_iter()
        .map(|choice| bounded(choice.trim(), MAX_CHOICE_CHARS))
        .filter(|choice| !choice.is_empty())
        .collect::<Vec<_>>();
    if timeout.is_zero() || timeout > Duration::from_secs(MAX_TIMEOUT_SECONDS) {
        return Err(format!(
            "timeout must be between 1 and {MAX_TIMEOUT_SECONDS} seconds"
        ));
    }
    // 只保留与净化后 choices 对齐的结构化元数据，防止幽灵选项
    let details: Vec<InteractionChoice> = choice_details
        .into_iter()
        .filter(|detail| choices.iter().any(|c| c == detail.label.as_str()))
        .take(MAX_CHOICES)
        .map(|mut detail| {
            detail.label = bounded(detail.label.trim(), MAX_CHOICE_CHARS);
            detail.description = detail
                .description
                .map(|d| bounded(d.trim(), MAX_CHOICE_CHARS * 2));
            detail
        })
        .collect();
    Ok(InteractionRequest {
        request_id: Uuid::new_v4().to_string(),
        question: question.to_string(),
        choices,
        choice_details: details,
    })
}

fn read_console_response(request: &InteractionRequest) -> Result<String, String> {
    println!("\n\x1b[33m[Agent Asks]: {}\x1b[0m", request.question);
    for (index, choice) in request.choices.iter().enumerate() {
        println!("  {}. {}", index + 1, choice);
    }
    print!("\x1b[33m[Your Answer]: \x1b[0m");
    let _ = io::stdout().flush();
    let mut buffer = String::new();
    io::stdin()
        .read_line(&mut buffer)
        .map_err(|error| format!("failed to read console input: {error}"))?;
    Ok(buffer.trim().to_string())
}

fn bounded(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_question_choices_and_timeout() {
        assert!(validate_request("Choose", vec!["A".to_string()], Vec::new(), Duration::from_secs(1)).is_ok());
        assert!(validate_request("", Vec::new(), Vec::new(), Duration::from_secs(1)).is_err());
        assert!(
            validate_request("Choose", vec!["x".to_string(); 9], Vec::new(), Duration::from_secs(1)).is_err()
        );
        assert!(validate_request("Choose", Vec::new(), Vec::new(), Duration::ZERO).is_err());
    }

    #[test]
    fn structured_details_are_aligned_with_plain_choices() {
        let details = vec![
            InteractionChoice {
                label: "方案 A".to_string(),
                description: Some("低风险".to_string()),
                recommended: true,
            },
            InteractionChoice {
                label: "幽灵选项".to_string(),
                description: None,
                recommended: false,
            },
        ];
        let request = validate_request(
            "选哪个方案？",
            vec!["方案 A".to_string(), "方案 B".to_string()],
            details,
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(request.choices.len(), 2);
        // 只保留与 choices 对齐的元数据，幽灵选项被剔除
        assert_eq!(request.choice_details.len(), 1);
        assert!(request.choice_details[0].recommended);
        assert_eq!(request.choice_details[0].description.as_deref(), Some("低风险"));
    }

    #[test]
    fn response_requires_matching_pending_request() {
        let runtime = InteractionRuntime::default();
        let request = validate_request("Choose", Vec::new(), Vec::new(), Duration::from_secs(1)).unwrap();
        runtime.begin(&request).unwrap();
        assert!(!runtime.deliver_response("wrong", "answer"));
        assert!(runtime.deliver_response(&request.request_id, "answer"));
        assert!(!runtime.deliver_response(&request.request_id, "late"));
    }
}
