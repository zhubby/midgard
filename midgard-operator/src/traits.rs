use std::time::Duration;

use kube::Resource as KubeResource;
use midgard_core::CapabilityDescriptor;
use midgard_protocol::operator::MiddlewareResource;

pub trait OperatorDefinition: Send + Sync {
    fn operator_id(&self) -> String;
    fn workspace_id(&self) -> &str;
    fn middleware_kind(&self) -> &str;
    fn display_name(&self) -> &str;
    fn supported_operations(&self) -> Vec<String>;
    fn capabilities(&self) -> Vec<CapabilityDescriptor>;
    fn heartbeat_interval(&self) -> Duration;
}

pub trait OperatorResourceAdapter {
    type Resource: Clone + KubeResource<DynamicType = ()>;
    type Error: std::error::Error + Send + Sync + 'static;

    fn middleware_kind(&self) -> &str;

    fn resource_from_middleware(
        &self,
        resource: MiddlewareResource,
    ) -> Result<Self::Resource, Self::Error>;

    fn middleware_from_resource(
        &self,
        resource: &Self::Resource,
        fallback: Option<&MiddlewareResource>,
    ) -> Result<MiddlewareResource, Self::Error>;
}
