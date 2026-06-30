use std::sync::Arc;

use async_trait::async_trait;
use midgard_controller::{
    ControllerHealth, MiddlewareController, MiddlewarePlugin, PluginMetadata,
};
use midgard_core::CapabilityDescriptor;
use midgard_tools::ToolRegistry;

use crate::{
    client::DockerRuntimeResolver,
    tool::{DockerTool, DockerToolKind},
};

pub const DOCKER_PLUGIN_ID: &str = "midgard-docker";
pub const DOCKER_MIDDLEWARE_KIND: &str = "docker";

#[derive(Clone)]
pub struct DockerPlugin {
    resolver: Arc<dyn DockerRuntimeResolver>,
}

impl DockerPlugin {
    pub fn new(resolver: Arc<dyn DockerRuntimeResolver>) -> Self {
        Self { resolver }
    }
}

impl MiddlewarePlugin for DockerPlugin {
    type Controller = DockerController;

    fn metadata(&self) -> PluginMetadata {
        PluginMetadata::new(
            DOCKER_PLUGIN_ID,
            "Docker Runtime Plugin",
            DOCKER_MIDDLEWARE_KIND,
        )
    }

    fn controller(&self) -> Self::Controller {
        DockerController {
            resolver: self.resolver.clone(),
        }
    }
}

#[derive(Clone)]
pub struct DockerController {
    resolver: Arc<dyn DockerRuntimeResolver>,
}

#[async_trait]
impl MiddlewareController for DockerController {
    fn name(&self) -> &'static str {
        "docker-controller"
    }

    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        DockerToolKind::all()
            .iter()
            .map(|kind| {
                CapabilityDescriptor::new(
                    kind.capability_id(),
                    kind.capability_name(),
                    kind.risk_level(),
                )
            })
            .collect()
    }

    async fn health(&self) -> ControllerHealth {
        ControllerHealth::healthy("docker controller ready")
    }

    fn register_tools(&self, registry: &mut ToolRegistry) {
        for kind in DockerToolKind::all() {
            registry.register(DockerTool::new(*kind, self.resolver.clone()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midgard_core::RiskLevel;
    use serde_json::Value;

    use crate::{
        client::{DockerClient, DockerRuntimeResolver},
        error::DockerToolError,
    };

    #[test]
    fn docker_plugin_metadata_describes_docker_runtime() {
        let plugin = DockerPlugin::new(Arc::new(MockResolver));
        let metadata = plugin.metadata();

        assert_eq!(metadata.id, DOCKER_PLUGIN_ID);
        assert_eq!(metadata.middleware_kind, DOCKER_MIDDLEWARE_KIND);
    }

    #[tokio::test]
    async fn docker_controller_registers_expected_tools_with_risk_metadata() {
        let plugin = DockerPlugin::new(Arc::new(MockResolver));
        let controller = plugin.controller();
        let mut registry = ToolRegistry::default();

        controller.register_tools(&mut registry);
        let tools = registry.definitions();

        assert!(
            tools
                .iter()
                .any(|tool| tool.name == "docker_container_list")
        );
        assert!(tools.iter().any(|tool| tool.name == "docker_system_prune"));
        let restart = tools
            .iter()
            .find(|tool| tool.name == "docker_container_restart")
            .unwrap();
        assert_eq!(restart.risk_level, RiskLevel::High);
        assert!(restart.requires_approval);
        let prune = tools
            .iter()
            .find(|tool| tool.name == "docker_system_prune")
            .unwrap();
        assert_eq!(prune.risk_level, RiskLevel::Critical);
        assert!(prune.requires_approval);
        assert!(tools.iter().all(|tool| {
            !tool
                .parameters_schema
                .get("properties")
                .and_then(Value::as_object)
                .is_some_and(|properties| properties.contains_key("workspace_id"))
        }));
    }

    struct MockResolver;

    #[async_trait]
    impl DockerRuntimeResolver for MockResolver {
        async fn resolve(
            &self,
            _context: &midgard_tools::ToolCallContext,
        ) -> Result<Arc<dyn DockerClient>, DockerToolError> {
            unreachable!("plugin registration tests must not resolve a Docker client")
        }
    }
}
