use crate::services::brain::events::{RunOutcome, SessionEvent};
use std::collections::HashSet;

pub fn start_run(events: &mut Vec<SessionEvent>, run_id: String) -> Result<(), String> {
    if events.iter().any(|event| {
        matches!(
            event,
            SessionEvent::RunStarted { run_id: existing, .. } if existing == &run_id
        )
    }) {
        return Err(format!("run already exists: {run_id}"));
    }
    events.push(SessionEvent::RunStarted {
        run_id,
        at: chrono::Utc::now().to_rfc3339(),
    });
    Ok(())
}

pub fn finish_run(
    events: &mut Vec<SessionEvent>,
    run_id: &str,
    outcome: RunOutcome,
    detail: Option<String>,
) -> Result<(), String> {
    let started = events.iter().any(|event| {
        matches!(
            event,
            SessionEvent::RunStarted { run_id: existing, .. } if existing == run_id
        )
    });
    if !started {
        return Err(format!("cannot finish unknown run: {run_id}"));
    }
    let finished = events.iter().any(|event| {
        matches!(
            event,
            SessionEvent::RunFinished { run_id: existing, .. } if existing == run_id
        )
    });
    if finished {
        return Err(format!("terminal receipt already exists for run: {run_id}"));
    }
    events.push(SessionEvent::RunFinished {
        run_id: run_id.to_string(),
        outcome,
        detail,
        at: chrono::Utc::now().to_rfc3339(),
    });
    Ok(())
}

pub fn repair_incomplete_runs(events: &mut Vec<SessionEvent>) -> usize {
    let mut started = HashSet::new();
    let mut finished = HashSet::new();
    for event in events.iter() {
        match event {
            SessionEvent::RunStarted { run_id, .. } => {
                started.insert(run_id.clone());
            }
            SessionEvent::RunFinished { run_id, .. } => {
                finished.insert(run_id.clone());
            }
            _ => {}
        }
    }
    let incomplete = started.difference(&finished).cloned().collect::<Vec<_>>();
    for run_id in &incomplete {
        events.push(SessionEvent::RunFinished {
            run_id: run_id.clone(),
            outcome: RunOutcome::Interrupted,
            detail: Some("process restarted before terminal delivery".to_string()),
            at: chrono::Utc::now().to_rfc3339(),
        });
    }
    incomplete.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_receipt_is_recorded_exactly_once() {
        let mut events = Vec::new();
        start_run(&mut events, "run-1".into()).unwrap();
        finish_run(&mut events, "run-1", RunOutcome::Completed, None).unwrap();
        assert!(finish_run(&mut events, "run-1", RunOutcome::Failed, None).is_err());
    }

    #[test]
    fn repairs_run_interrupted_by_restart() {
        let mut events = Vec::new();
        start_run(&mut events, "run-1".into()).unwrap();
        assert_eq!(repair_incomplete_runs(&mut events), 1);
        assert_eq!(repair_incomplete_runs(&mut events), 0);
        assert!(events.iter().any(|event| matches!(
            event,
            SessionEvent::RunFinished {
                outcome: RunOutcome::Interrupted,
                ..
            }
        )));
    }
}
