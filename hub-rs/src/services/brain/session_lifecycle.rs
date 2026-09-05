use crate::services::brain::events::{SessionEvent, SessionStatus};

pub struct SessionLifecycle {
    status: SessionStatus,
}

impl SessionLifecycle {
    pub fn from_events(events: &[SessionEvent]) -> Self {
        let status = events
            .iter()
            .rev()
            .find_map(|event| match event {
                SessionEvent::LifecycleChanged { to, .. } => Some(*to),
                _ => None,
            })
            .unwrap_or(SessionStatus::Created);
        Self { status }
    }

    pub fn status(&self) -> SessionStatus {
        self.status
    }

    pub fn transition(
        &mut self,
        to: SessionStatus,
        reason: Option<String>,
    ) -> Result<Option<SessionEvent>, String> {
        if self.status == to {
            return Ok(None);
        }
        if !allowed(self.status, to) {
            return Err(format!(
                "invalid session lifecycle transition: {:?} -> {:?}",
                self.status, to
            ));
        }
        let from = self.status;
        self.status = to;
        Ok(Some(SessionEvent::LifecycleChanged {
            from,
            to,
            reason,
            at: chrono::Utc::now().to_rfc3339(),
        }))
    }
}

fn allowed(from: SessionStatus, to: SessionStatus) -> bool {
    use SessionStatus::*;
    matches!(
        (from, to),
        (Created, Running | Cancelled)
            | (Running, NeedsInput | Completed | Failed | Cancelled)
            | (NeedsInput, Running | Failed | Cancelled)
            | (Completed | Failed | Cancelled, Running)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enforces_lifecycle_and_supports_a_new_turn() {
        let mut lifecycle = SessionLifecycle::from_events(&[]);
        lifecycle.transition(SessionStatus::Running, None).unwrap();
        lifecycle
            .transition(SessionStatus::Completed, None)
            .unwrap();
        lifecycle.transition(SessionStatus::Running, None).unwrap();
        assert_eq!(lifecycle.status(), SessionStatus::Running);
        assert!(lifecycle
            .transition(SessionStatus::NeedsInput, None)
            .is_ok());
        assert!(lifecycle
            .transition(SessionStatus::Completed, None)
            .is_err());
    }
}
