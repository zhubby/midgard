use async_trait::async_trait;
use midgard_controller::{
    ControllerHealth, MiddlewareController, MiddlewarePlugin, PluginMetadata,
};
use midgard_core::{CapabilityDescriptor, RiskLevel};
use midgard_tools::{Tool, ToolDefinition, ToolRegistry, ToolResult};
use serde_json::{Value, json};

pub struct ExampleRedisPlugin;

impl MiddlewarePlugin for ExampleRedisPlugin {
    type Controller = ExampleRedisController;

    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("example-redis", "Example Redis Plugin", "redis")
    }

    fn controller(&self) -> Self::Controller {
        ExampleRedisController
    }
}

pub struct ExampleRedisController;

#[async_trait]
impl MiddlewareController for ExampleRedisController {
    fn name(&self) -> &'static str {
        "example-redis-controller"
    }

    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![
            CapabilityDescriptor::new("redis.describe", "Describe Redis workload", RiskLevel::Low),
            CapabilityDescriptor::new("redis.restart", "Restart Redis workload", RiskLevel::High),
        ]
    }

    async fn health(&self) -> ControllerHealth {
        ControllerHealth::healthy("example redis controller ready")
    }

    fn register_tools(&self, registry: &mut ToolRegistry) {
        registry.register(RedisDescribeTool);
        registry.register(RedisRestartTool);
    }
}

struct RedisDescribeTool;

#[async_trait]
impl Tool for RedisDescribeTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "redis_describe",
            "Describe a Redis middleware workload",
            json!({
                "type": "object",
                "properties": {
                    "namespace": {"type": "string"},
                    "name": {"type": "string"}
                },
                "required": ["namespace", "name"]
            }),
            RiskLevel::Low,
        )
    }

    async fn call(&self, arguments: Value) -> ToolResult {
        let namespace = argument_string(&arguments, "namespace");
        let name = argument_string(&arguments, "name");

        ToolResult::success(format!("Redis workload {namespace}/{name} is ready"))
    }
}

struct RedisRestartTool;

#[async_trait]
impl Tool for RedisRestartTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "redis_restart",
            "Restart a Redis middleware workload",
            json!({
                "type": "object",
                "properties": {
                    "namespace": {"type": "string"},
                    "name": {"type": "string"}
                },
                "required": ["namespace", "name"]
            }),
            RiskLevel::High,
        )
    }

    async fn call(&self, arguments: Value) -> ToolResult {
        let namespace = argument_string(&arguments, "namespace");
        let name = argument_string(&arguments, "name");

        ToolResult::success(format!(
            "Restart requested for Redis workload {namespace}/{name}"
        ))
    }
}

fn argument_string(arguments: &Value, key: &str) -> String {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}
