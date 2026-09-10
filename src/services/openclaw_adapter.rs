pub mod openclaw_adapter {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    #[derive(Serialize, Deserialize, Debug)]
    pub struct OpenclawEvent {
        pub event_type: String,
        pub agent_id: String,
        pub payload: Value,
        pub timestamp: u64,
    }

    pub fn emit_event(event: &OpenclawEvent) {
        // True Openclaw Event Stream Adapter
        if let Ok(json_str) = serde_json::to_string(event) {
            eprintln!(
                "data: {}

",
                json_str
            );
        }
    }
}
