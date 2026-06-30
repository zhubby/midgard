use std::{collections::BTreeMap, sync::Arc};

use async_trait::async_trait;
use midgard_controller::{MiddlewareController, MiddlewarePlugin};
use midgard_core::RiskLevel;
use midgard_docker::{
    DOCKER_MIDDLEWARE_KIND, DOCKER_PLUGIN_ID, DockerClient, DockerOperation, DockerPlugin,
    DockerRuntimeResolver, DockerToolError,
};
use midgard_tools::{ToolCallContext, ToolRegistry};
use serde_json::{Value, json};

#[test]
fn docker_plugin_describes_docker_runtime() {
    let plugin = DockerPlugin::new(Arc::new(MockResolver));
    let metadata = plugin.metadata();

    assert_eq!(metadata.id, DOCKER_PLUGIN_ID);
    assert_eq!(metadata.middleware_kind, DOCKER_MIDDLEWARE_KIND);
}

#[tokio::test]
async fn docker_controller_registers_docker_tools() {
    let plugin = DockerPlugin::new(Arc::new(MockResolver));
    let controller = plugin.controller();
    let mut registry = ToolRegistry::default();

    controller.register_tools(&mut registry);
    let tools = registry.definitions();

    assert!(tools.iter().any(|tool| tool.name == "docker_info"));
    assert!(
        tools
            .iter()
            .any(|tool| tool.name == "docker_container_create")
    );
    assert!(tools.iter().any(|tool| tool.name == "docker_system_prune"));
    let create = tools
        .iter()
        .find(|tool| tool.name == "docker_container_create")
        .unwrap();
    assert_eq!(create.risk_level, RiskLevel::High);
    assert!(create.requires_approval);
}

#[tokio::test]
async fn docker_tool_definitions_have_stable_names_risks_and_approval_flags() {
    let plugin = DockerPlugin::new(Arc::new(MockResolver));
    let controller = plugin.controller();
    let mut registry = ToolRegistry::default();

    controller.register_tools(&mut registry);
    let tools = registry
        .definitions()
        .into_iter()
        .map(|tool| (tool.name.clone(), tool))
        .collect::<BTreeMap<_, _>>();
    let expected = [
        ("docker_info", RiskLevel::Low, false),
        ("docker_version", RiskLevel::Low, false),
        ("docker_container_list", RiskLevel::Low, false),
        ("docker_container_inspect", RiskLevel::Low, false),
        ("docker_container_logs", RiskLevel::Medium, false),
        ("docker_container_create", RiskLevel::High, true),
        ("docker_container_start", RiskLevel::High, true),
        ("docker_container_stop", RiskLevel::High, true),
        ("docker_container_restart", RiskLevel::High, true),
        ("docker_container_remove", RiskLevel::Critical, true),
        ("docker_container_prune", RiskLevel::Critical, true),
        ("docker_image_list", RiskLevel::Low, false),
        ("docker_image_inspect", RiskLevel::Low, false),
        ("docker_image_pull", RiskLevel::High, true),
        ("docker_image_remove", RiskLevel::High, true),
        ("docker_image_prune", RiskLevel::Critical, true),
        ("docker_network_list", RiskLevel::Low, false),
        ("docker_network_inspect", RiskLevel::Low, false),
        ("docker_network_create", RiskLevel::High, true),
        ("docker_network_connect", RiskLevel::High, true),
        ("docker_network_disconnect", RiskLevel::High, true),
        ("docker_network_remove", RiskLevel::Critical, true),
        ("docker_network_prune", RiskLevel::Critical, true),
        ("docker_volume_list", RiskLevel::Low, false),
        ("docker_volume_inspect", RiskLevel::Low, false),
        ("docker_volume_create", RiskLevel::High, true),
        ("docker_volume_remove", RiskLevel::Critical, true),
        ("docker_volume_prune", RiskLevel::Critical, true),
        ("docker_system_prune", RiskLevel::Critical, true),
    ];

    assert_eq!(tools.len(), expected.len());
    for (name, risk_level, requires_approval) in expected {
        let tool = tools.get(name).unwrap_or_else(|| panic!("missing {name}"));
        assert_eq!(tool.risk_level, risk_level, "{name}");
        assert_eq!(tool.requires_approval, requires_approval, "{name}");
        assert!(
            !tool
                .parameters_schema
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|properties| properties.contains_key("workspace_id")),
            "{name} must use ToolCallContext, not workspace_id arguments",
        );
    }
}

#[tokio::test]
async fn docker_info_tool_uses_current_workspace_context() {
    let plugin = DockerPlugin::new(Arc::new(MockResolver));
    let controller = plugin.controller();
    let mut registry = ToolRegistry::default();
    controller.register_tools(&mut registry);

    let result = registry
        .call_with_context(
            "docker_info",
            json!({}),
            ToolCallContext {
                workspace_id: Some("workspace-1".to_string()),
            },
        )
        .await
        .unwrap();

    assert!(result.success);
    assert!(result.output.contains("\"operation\": \"info\""));
}

struct MockResolver;

#[async_trait]
impl DockerRuntimeResolver for MockResolver {
    async fn resolve(
        &self,
        context: &ToolCallContext,
    ) -> Result<Arc<dyn DockerClient>, DockerToolError> {
        assert_eq!(context.workspace_id.as_deref(), Some("workspace-1"));
        Ok(Arc::new(MockClient))
    }
}

struct MockClient;

#[async_trait]
impl DockerClient for MockClient {
    async fn execute(&self, operation: DockerOperation) -> Result<Value, DockerToolError> {
        let operation = match operation {
            DockerOperation::Info => "info",
            _ => "other",
        };
        Ok(json!({ "operation": operation }))
    }
}
