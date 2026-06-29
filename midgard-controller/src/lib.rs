use async_trait::async_trait;
use midgard_core::CapabilityDescriptor;
use midgard_tools::ToolRegistry;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ControllerHealth {
    pub healthy: bool,
    pub message: String,
}

impl ControllerHealth {
    pub fn healthy(message: impl Into<String>) -> Self {
        Self {
            healthy: true,
            message: message.into(),
        }
    }

    pub fn unhealthy(message: impl Into<String>) -> Self {
        Self {
            healthy: false,
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait MiddlewareController: Send + Sync {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> Vec<CapabilityDescriptor>;
    async fn health(&self) -> ControllerHealth;
    fn register_tools(&self, registry: &mut ToolRegistry);
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PluginMetadata {
    pub id: String,
    pub name: String,
    pub middleware_kind: String,
}

impl PluginMetadata {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        middleware_kind: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            middleware_kind: middleware_kind.into(),
        }
    }
}

pub trait MiddlewarePlugin {
    type Controller: MiddlewareController;

    fn metadata(&self) -> PluginMetadata;
    fn controller(&self) -> Self::Controller;
}
