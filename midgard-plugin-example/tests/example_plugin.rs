use midgard_controller::{MiddlewareController, MiddlewarePlugin};
use midgard_plugin_example::ExampleRedisPlugin;
use midgard_tools::ToolRegistry;
use serde_json::json;

#[test]
fn example_plugin_describes_redis_middleware() {
    let plugin = ExampleRedisPlugin;
    let metadata = plugin.metadata();

    assert_eq!(metadata.id, "example-redis");
    assert_eq!(metadata.middleware_kind, "redis");
}

#[tokio::test]
async fn example_controller_registers_redis_tools() {
    let plugin = ExampleRedisPlugin;
    let controller = plugin.controller();
    let mut registry = ToolRegistry::default();

    controller.register_tools(&mut registry);
    let tools = registry.definitions();

    assert!(tools.iter().any(|tool| tool.name == "redis_describe"));
    assert!(tools.iter().any(|tool| tool.name == "redis_restart"));
    assert!(tools.iter().any(|tool| tool.requires_approval));
}

#[tokio::test]
async fn redis_describe_tool_returns_namespace_context() {
    let plugin = ExampleRedisPlugin;
    let controller = plugin.controller();
    let mut registry = ToolRegistry::default();
    controller.register_tools(&mut registry);

    let result = registry
        .call(
            "redis_describe",
            json!({"namespace": "default", "name": "cache"}),
        )
        .await
        .unwrap();

    assert!(result.output.contains("default/cache"));
}
