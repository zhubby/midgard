use async_trait::async_trait;
use midgard_core::RiskLevel;
use midgard_tools::{Tool, ToolDefinition, ToolRegistry, ToolResult};
use serde_json::{Value, json};

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "echo",
            "Echo input",
            json!({"type": "object"}),
            RiskLevel::Low,
        )
    }

    async fn call(&self, arguments: Value) -> ToolResult {
        ToolResult::success(arguments.to_string())
    }
}

#[tokio::test]
async fn registry_executes_registered_tools() {
    let mut registry = ToolRegistry::default();
    registry.register(EchoTool);

    let result = registry
        .call("echo", json!({"message": "hello"}))
        .await
        .unwrap();

    assert!(result.success);
    assert!(result.should_continue);
    assert!(result.output.contains("hello"));
}

#[tokio::test]
async fn registry_exposes_tool_definitions_with_risk_metadata() {
    let mut registry = ToolRegistry::default();
    registry.register(EchoTool);

    let tools = registry.definitions();

    assert_eq!(tools[0].name, "echo");
    assert_eq!(tools[0].risk_level, RiskLevel::Low);
    assert!(!tools[0].requires_approval);
}

#[tokio::test]
async fn missing_tools_return_an_error() {
    let registry = ToolRegistry::default();

    let error = registry.call("missing", json!({})).await.unwrap_err();

    assert!(error.to_string().contains("missing"));
}
