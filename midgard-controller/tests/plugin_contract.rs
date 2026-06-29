use async_trait::async_trait;
use midgard_controller::{
    ControllerHealth, MiddlewareController, MiddlewarePlugin, PluginMetadata,
};
use midgard_core::{CapabilityDescriptor, RiskLevel};
use midgard_tools::ToolRegistry;

struct TestController;

#[async_trait]
impl MiddlewareController for TestController {
    fn name(&self) -> &'static str {
        "test-controller"
    }

    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![CapabilityDescriptor::new(
            "test.inspect",
            "Inspect test",
            RiskLevel::Low,
        )]
    }

    async fn health(&self) -> ControllerHealth {
        ControllerHealth::healthy("ready")
    }

    fn register_tools(&self, _registry: &mut ToolRegistry) {}
}

struct TestPlugin;

impl MiddlewarePlugin for TestPlugin {
    type Controller = TestController;

    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new("test", "Test Plugin", "example")
    }

    fn controller(&self) -> Self::Controller {
        TestController
    }
}

#[tokio::test]
async fn controller_reports_health_and_capabilities() {
    let controller = TestController;

    let health = controller.health().await;
    let capabilities = controller.capabilities();

    assert!(health.healthy);
    assert_eq!(capabilities[0].id, "test.inspect");
}

#[test]
fn plugin_metadata_is_stable_for_discovery() {
    let plugin = TestPlugin;
    let metadata = plugin.metadata();

    assert_eq!(metadata.id, "test");
    assert_eq!(metadata.middleware_kind, "example");
}
