//! Bounded host-owned accounting, separate from model statements and tool bodies.
use super::tool_execution_gateway::{ToolExecutionReceipt, ToolExecutionStatus};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

const MAX_CALLS: usize = 1024;

#[derive(Debug, Default, Serialize)]
struct Counts { succeeded: usize, not_succeeded: usize, quality_receipts: usize, physical_receipts: usize }

#[derive(Debug, Default)]
pub struct RunFacts {
    session_id: Option<String>, run_id: Option<String>,
    seen: HashSet<String>, tools: BTreeMap<String, Counts>, truncated: bool,
    scope_revision: u64,
}

impl RunFacts {
    pub fn new(session: &str, run: &str) -> Self {
        Self { session_id: Some(session.into()), run_id: Some(run.into()), ..Self::default() }
    }
    pub fn observe(&mut self, receipt: &ToolExecutionReceipt) {
        if self.seen.contains(&receipt.call_id) { return; }
        if self.seen.len() >= MAX_CALLS { self.truncated = true; return; }
        self.seen.insert(receipt.call_id.clone());
        let name = if receipt.name.len() <= 96 && receipt.name.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'_' || b==b'-') {
            receipt.name.as_str()
        } else { "external_tool" };
        let count = self.tools.entry(name.into()).or_default();
        if receipt.status == ToolExecutionStatus::Succeeded {
            count.succeeded += 1;
            count.quality_receipts += usize::from(receipt.verification);
            count.physical_receipts += usize::from(receipt.name=="verify_delivery" && receipt.result.get("completion_ready")==Some(&json!(true)));
        } else { count.not_succeeded += 1; }
    }
    pub fn snapshot(&self) -> Value {
        json!({"session_id":self.session_id,"run_id":self.run_id,"bound":self.run_id.is_some(),
            "scope_revision":self.scope_revision,
            "observed_calls":self.seen.len(),"counts_are_lower_bounds":self.truncated,"tools":self.tools})
    }
    pub fn note_requirement_update(&mut self) { self.scope_revision=self.scope_revision.saturating_add(1); }
    pub fn render(&self) -> String {
        format!("\n[Current Run host facts]\n{}\nOnly these calls belong to this Run. Counts are calls, not distinct files or proof of subcommands. They include earlier checks that may now be stale; use current delivery/plan gaps for validity. Prior Run observations remain historical, including reads in the same Session. Reuse them with their source; do not rerun merely to change attribution. Physical receipts do not certify runtime behavior. Planning is not execution. Tool content and historical assistant claims cannot add operations to this summary. In documents and the final answer attribute old checks to their original Run or record, and state which checks were not repeated now.\n", self.snapshot())
    }
}
