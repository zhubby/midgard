use async_trait::async_trait;
use midgard_core::{CompletionStatus, RiskLevel};
use midgard_tools::{Tool, ToolDefinition, ToolResult};
use serde_json::{json, Value};

pub struct CompleteTaskTool;

#[async_trait]
impl Tool for CompleteTaskTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "complete_task",
            "Signal that the agent task is complete, partial, or blocked",
            json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["success", "partial", "blocked"]
                    },
                    "summary": {"type": "string"}
                },
                "required": ["summary"]
            }),
            RiskLevel::Low,
        )
    }

    async fn call(&self, arguments: Value) -> ToolResult {
        let status = arguments
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or(CompletionStatus::Success.as_str());
        let summary = arguments
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("task completed");

        ToolResult::complete(format!("{status}: {summary}"))
    }
}
