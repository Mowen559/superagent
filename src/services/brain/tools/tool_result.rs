use crate::services::brain::executor::ToolExecutor;
use crate::services::brain::tool_result_store::ToolResultStore;
use serde_json::{json, Value};

pub struct ReadToolResultTool {
    store: ToolResultStore,
}

impl ReadToolResultTool {
    pub fn new(store: ToolResultStore) -> Self {
        Self { store }
    }

    pub fn default_location() -> Self {
        Self::new(ToolResultStore::default_location())
    }
}

impl ToolExecutor for ReadToolResultTool {
    fn name(&self) -> &'static str {
        "read_tool_result"
    }

    fn description(&self) -> &'static str {
        "Read a bounded slice of a large tool result using its tool-result:// reference."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "reference": { "type": "string", "description": "tool-result:// reference" },
                "offset": { "type": "integer", "minimum": 0, "default": 0 },
                "max_chars": { "type": "integer", "minimum": 100, "maximum": 16000, "default": 8000 }
            },
            "required": ["reference"]
        })
    }

    fn execute(&self, args: Value) -> Result<Value, String> {
        let reference = args
            .get("reference")
            .and_then(Value::as_str)
            .ok_or_else(|| "reference is required".to_string())?;
        let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let max_chars = args
            .get("max_chars")
            .and_then(Value::as_u64)
            .unwrap_or(8_000)
            .clamp(100, 16_000) as usize;
        let slice = self.store.read_slice(reference, offset, max_chars)?;
        Ok(json!({
            "reference": slice.reference,
            "content": slice.content,
            "offset": slice.offset,
            "next_offset": slice.next_offset,
            "total_chars": slice.total_chars,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn reads_stored_result_by_reference() {
        let dir = tempdir().unwrap();
        let store = ToolResultStore::new(dir.path().to_path_buf());
        let stored = store.persist("complete tool output").unwrap();
        let tool = ReadToolResultTool::new(store);

        let result = tool
            .execute(json!({ "reference": stored.reference, "max_chars": 100 }))
            .unwrap();
        assert_eq!(result["content"], "complete tool output");
    }
}
