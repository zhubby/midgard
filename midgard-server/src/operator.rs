use std::{
    collections::BTreeMap,
    pin::Pin,
    sync::{Arc, Mutex},
};

use futures_util::{Stream, StreamExt};
use midgard_core::{CapabilityDescriptor, RiskLevel};
use midgard_protocol::{
    CommandType, MiddlewareResource, MiddlewareStatus, OPERATOR_PROTOCOL_VERSION,
    operator::{
        OperatorCapability, OperatorRegistration, OperatorToServer, ServerAck, ServerCommand,
        ServerToOperator,
        operator_control_server::{OperatorControl, OperatorControlServer},
        operator_to_server, server_to_operator,
    },
};
use midgard_storage::{
    MiddlewareDesiredState, MiddlewareInstanceStatus, MiddlewareInstanceUpdate,
    NewMiddlewareInstance, SharedOrganizationStore,
};
use midgard_tools::{Tool, ToolCallContext, ToolDefinition, ToolRegistry, ToolResult};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;

use crate::{AppError, AppState, publish_middleware_instance_change, storage_app_error};

type OperatorResponseStream =
    Pin<Box<dyn Stream<Item = Result<ServerToOperator, Status>> + Send + 'static>>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct OperatorConnectionSnapshot {
    pub operator_id: String,
    pub workspace_id: String,
    pub middleware_kind: String,
    pub display_name: String,
    pub connected: bool,
    pub supported_operations: Vec<String>,
    pub capabilities: Vec<CapabilityDescriptor>,
    pub last_heartbeat_unix_ms: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperatorDispatchOutcome {
    Delivered,
    NotConnected,
    Backpressure,
}

#[derive(Clone, Default)]
pub struct OperatorRegistry {
    inner: Arc<Mutex<OperatorRegistryState>>,
}

#[derive(Default)]
struct OperatorRegistryState {
    connections: BTreeMap<OperatorKey, OperatorConnectionState>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct OperatorKey {
    workspace_id: String,
    middleware_kind: String,
}

struct OperatorConnectionState {
    connection_id: Uuid,
    operator_id: String,
    display_name: String,
    connected: bool,
    supported_operations: Vec<String>,
    capabilities: Vec<CapabilityDescriptor>,
    last_heartbeat_unix_ms: Option<i64>,
    sender: Option<mpsc::Sender<ServerToOperator>>,
}

impl OperatorRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn validate_registration(&self, registration: &OperatorRegistration) -> Result<(), Status> {
        if registration.protocol_version != OPERATOR_PROTOCOL_VERSION {
            return Err(Status::failed_precondition(format!(
                "unsupported operator protocol version: {}",
                registration.protocol_version
            )));
        }
        if registration.operator_id.trim().is_empty() {
            return Err(Status::invalid_argument("operator_id is required"));
        }
        if registration.workspace_id.trim().is_empty() {
            return Err(Status::invalid_argument("workspace_id is required"));
        }
        if Uuid::parse_str(&registration.workspace_id).is_err() {
            return Err(Status::invalid_argument("workspace_id must be a UUID"));
        }
        if registration.middleware_kind.trim().is_empty() {
            return Err(Status::invalid_argument("middleware_kind is required"));
        }

        Ok(())
    }

    fn register(
        &self,
        registration: OperatorRegistration,
        sender: mpsc::Sender<ServerToOperator>,
    ) -> Result<OperatorConnectionLease, Status> {
        self.validate_registration(&registration)?;

        let key = OperatorKey {
            workspace_id: registration.workspace_id.clone(),
            middleware_kind: registration.middleware_kind.clone(),
        };
        let connection_id = Uuid::new_v4();
        let mut state = self
            .inner
            .lock()
            .map_err(|_| Status::internal("operator registry lock poisoned"))?;

        if let Some(current) = state.connections.get(&key)
            && current.connected
            && current.operator_id != registration.operator_id
        {
            return Err(Status::already_exists(format!(
                "operator already connected for workspace {} and middleware kind {}",
                key.workspace_id, key.middleware_kind
            )));
        }

        state.connections.insert(
            key.clone(),
            OperatorConnectionState {
                connection_id,
                operator_id: registration.operator_id.clone(),
                display_name: registration.display_name.clone(),
                connected: true,
                supported_operations: registration.supported_operations.clone(),
                capabilities: Vec::new(),
                last_heartbeat_unix_ms: None,
                sender: Some(sender),
            },
        );

        Ok(OperatorConnectionLease {
            registry: self.clone(),
            key,
            connection_id,
        })
    }

    pub fn dispatch_command(
        &self,
        workspace_id: &str,
        middleware_kind: &str,
        command: ServerCommand,
    ) -> OperatorDispatchOutcome {
        let request_id = if command.operation_id.is_empty() {
            Uuid::new_v4().to_string()
        } else {
            command.operation_id.clone()
        };
        let message = ServerToOperator {
            request_id,
            payload: Some(server_to_operator::Payload::Command(command)),
        };

        let sender = self.inner.lock().ok().and_then(|state| {
            state
                .connections
                .get(&OperatorKey {
                    workspace_id: workspace_id.to_string(),
                    middleware_kind: middleware_kind.to_string(),
                })
                .filter(|connection| connection.connected)
                .and_then(|connection| connection.sender.clone())
        });

        let Some(sender) = sender else {
            return OperatorDispatchOutcome::NotConnected;
        };

        match sender.try_send(message) {
            Ok(()) => OperatorDispatchOutcome::Delivered,
            Err(mpsc::error::TrySendError::Full(_)) => OperatorDispatchOutcome::Backpressure,
            Err(mpsc::error::TrySendError::Closed(_)) => OperatorDispatchOutcome::NotConnected,
        }
    }

    pub fn snapshots(&self) -> Vec<OperatorConnectionSnapshot> {
        self.inner
            .lock()
            .map(|state| {
                state
                    .connections
                    .iter()
                    .map(|(key, connection)| OperatorConnectionSnapshot {
                        operator_id: connection.operator_id.clone(),
                        workspace_id: key.workspace_id.clone(),
                        middleware_kind: key.middleware_kind.clone(),
                        display_name: connection.display_name.clone(),
                        connected: connection.connected,
                        supported_operations: connection.supported_operations.clone(),
                        capabilities: connection.capabilities.clone(),
                        last_heartbeat_unix_ms: connection.last_heartbeat_unix_ms,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn update_heartbeat(&self, operator_id: &str, observed_at_unix_ms: i64) {
        if let Ok(mut state) = self.inner.lock() {
            for connection in state.connections.values_mut() {
                if connection.operator_id == operator_id && connection.connected {
                    connection.last_heartbeat_unix_ms = Some(observed_at_unix_ms);
                }
            }
        }
    }

    fn update_capabilities(&self, operator_id: &str, capabilities: Vec<CapabilityDescriptor>) {
        if let Ok(mut state) = self.inner.lock() {
            for connection in state.connections.values_mut() {
                if connection.operator_id == operator_id && connection.connected {
                    connection.capabilities = capabilities.clone();
                }
            }
        }
    }

    fn disconnect(&self, key: &OperatorKey, connection_id: Uuid) {
        if let Ok(mut state) = self.inner.lock()
            && let Some(connection) = state.connections.get_mut(key)
            && connection.connection_id == connection_id
        {
            connection.connected = false;
            connection.sender = None;
        }
    }
}

struct OperatorConnectionLease {
    registry: OperatorRegistry,
    key: OperatorKey,
    connection_id: Uuid,
}

impl Drop for OperatorConnectionLease {
    fn drop(&mut self) {
        self.registry.disconnect(&self.key, self.connection_id);
    }
}

#[derive(Clone)]
pub struct OperatorControlService {
    state: AppState,
}

impl OperatorControlService {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }

    pub fn into_server(self) -> OperatorControlServer<Self> {
        OperatorControlServer::new(self)
    }

    async fn send_initial_reconcile(
        &self,
        registration: &OperatorRegistration,
        sender: &mpsc::Sender<ServerToOperator>,
    ) -> Result<(), Status> {
        let workspace_id = Uuid::parse_str(&registration.workspace_id)
            .map_err(|_| Status::invalid_argument("workspace_id must be a UUID"))?;
        let instances = self
            .state
            .orgs
            .list_middleware_instances_for_reconciliation(workspace_id)
            .await
            .map_err(|err| Status::internal(err.to_string()))?
            .into_iter()
            .filter(|instance| instance.kind == registration.middleware_kind)
            .map(|instance| MiddlewareResource::from(&instance))
            .collect::<Vec<_>>();

        sender
            .send(ServerToOperator {
                request_id: Uuid::new_v4().to_string(),
                payload: Some(server_to_operator::Payload::Command(ServerCommand {
                    operation_id: Uuid::new_v4().to_string(),
                    command_type: CommandType::Reconcile as i32,
                    instance: None,
                    instances,
                })),
            })
            .await
            .map_err(|_| Status::unavailable("operator command stream closed"))
    }

    async fn handle_operator_message(&self, message: OperatorToServer) -> Result<(), Status> {
        let Some(payload) = message.payload else {
            return Ok(());
        };

        match payload {
            operator_to_server::Payload::Heartbeat(heartbeat) => {
                self.state
                    .operator_registry
                    .update_heartbeat(&heartbeat.operator_id, heartbeat.observed_at_unix_ms);
            }
            operator_to_server::Payload::CapabilityReport(report) => {
                let capabilities = report
                    .capabilities
                    .into_iter()
                    .map(capability_descriptor_from_protocol)
                    .collect();
                self.state
                    .operator_registry
                    .update_capabilities(&report.operator_id, capabilities);
            }
            operator_to_server::Payload::OperationStatus(status) => {
                self.apply_operator_status(status.instance_id, status.status)
                    .await?;
            }
            operator_to_server::Payload::OperationResult(result) => {
                if result.success {
                    if let Some(instance) = result.instance {
                        self.apply_operator_resource(instance).await?;
                    }
                } else if let Some(instance) = result.instance {
                    self.apply_operator_status(instance.id, MiddlewareStatus::Degraded as i32)
                        .await?;
                }
            }
            operator_to_server::Payload::InventorySnapshot(snapshot) => {
                for instance in snapshot.instances {
                    self.apply_operator_resource(instance).await?;
                }
            }
            operator_to_server::Payload::Registration(_) => {}
        }

        Ok(())
    }

    async fn apply_operator_resource(&self, resource: MiddlewareResource) -> Result<(), Status> {
        let instance = midgard_storage::MiddlewareInstance::try_from(resource)
            .map_err(Status::invalid_argument)?;
        let update = MiddlewareInstanceUpdate {
            desired_state: Some(instance.desired_state),
            status: Some(instance.status),
            config: Some(instance.config),
            archived: instance.archived_at.as_ref().map(|_| true),
        };
        let updated = self
            .state
            .orgs
            .update_middleware_instance(instance.workspace_id, instance.id, update)
            .await
            .map_err(|err| Status::internal(err.to_string()))?;
        if let Some(updated) = updated {
            publish_middleware_instance_change(
                &self.state,
                updated.workspace_id,
                &updated,
                updated.archived_at.is_some(),
            );
        }

        Ok(())
    }

    async fn apply_operator_status(&self, instance_id: String, status: i32) -> Result<(), Status> {
        if instance_id.is_empty() {
            return Ok(());
        }
        let instance_id = Uuid::parse_str(&instance_id)
            .map_err(|_| Status::invalid_argument("instance_id must be a UUID"))?;
        let status =
            MiddlewareStatus::try_from(status).unwrap_or(MiddlewareStatus::UnknownMiddlewareStatus);
        let status = MiddlewareInstanceStatus::from(status);
        let Some(workspace_id) = self.find_instance_workspace(instance_id).await? else {
            return Ok(());
        };
        let updated = self
            .state
            .orgs
            .update_middleware_instance(
                workspace_id,
                instance_id,
                MiddlewareInstanceUpdate {
                    desired_state: None,
                    status: Some(status),
                    config: None,
                    archived: None,
                },
            )
            .await
            .map_err(|err| Status::internal(err.to_string()))?;
        if let Some(updated) = updated {
            publish_middleware_instance_change(&self.state, workspace_id, &updated, false);
        }

        Ok(())
    }

    async fn find_instance_workspace(&self, instance_id: Uuid) -> Result<Option<Uuid>, Status> {
        // Operator status messages are keyed by instance ID. The current store API is
        // workspace-scoped, so resolve the workspace from active/reconciliation rows.
        let snapshots = self.state.operator_registry.snapshots();
        for snapshot in snapshots {
            let Ok(workspace_id) = Uuid::parse_str(&snapshot.workspace_id) else {
                continue;
            };
            let instances = self
                .state
                .orgs
                .list_middleware_instances_for_reconciliation(workspace_id)
                .await
                .map_err(|err| Status::internal(err.to_string()))?;
            if instances.iter().any(|instance| instance.id == instance_id) {
                return Ok(Some(workspace_id));
            }
        }

        Ok(None)
    }
}

#[tonic::async_trait]
impl OperatorControl for OperatorControlService {
    type OpenChannelStream = OperatorResponseStream;

    async fn open_channel(
        &self,
        request: Request<Streaming<OperatorToServer>>,
    ) -> Result<Response<Self::OpenChannelStream>, Status> {
        let mut inbound = request.into_inner();
        let Some(first_message) = inbound.message().await? else {
            return Err(Status::invalid_argument(
                "operator registration message is required",
            ));
        };
        let Some(operator_to_server::Payload::Registration(registration)) = first_message.payload
        else {
            return Err(Status::invalid_argument(
                "first operator message must be registration",
            ));
        };

        let (sender, receiver) = mpsc::channel(64);
        let lease = self
            .state
            .operator_registry
            .register(registration.clone(), sender.clone())?;
        sender
            .send(ServerToOperator {
                request_id: first_message.request_id,
                payload: Some(server_to_operator::Payload::Ack(ServerAck {
                    accepted: true,
                    message: "operator registered".to_string(),
                })),
            })
            .await
            .map_err(|_| Status::unavailable("operator command stream closed"))?;
        self.send_initial_reconcile(&registration, &sender).await?;

        let service = self.clone();
        tokio::spawn(async move {
            let _lease = lease;
            loop {
                match inbound.message().await {
                    Ok(Some(message)) => {
                        if let Err(err) = service.handle_operator_message(message).await {
                            tracing::warn!(error = %err, "operator message rejected");
                        }
                    }
                    Ok(None) => break,
                    Err(err) => {
                        tracing::warn!(error = %err, "operator stream failed");
                        break;
                    }
                }
            }
        });

        Ok(Response::new(Box::pin(
            ReceiverStream::new(receiver).map(Ok),
        )))
    }
}

pub fn command_for_instance(
    command_type: CommandType,
    instance: &midgard_storage::MiddlewareInstance,
) -> ServerCommand {
    ServerCommand {
        operation_id: Uuid::new_v4().to_string(),
        command_type: command_type as i32,
        instance: Some(MiddlewareResource::from(instance)),
        instances: Vec::new(),
    }
}

pub(crate) fn operator_app_error(outcome: OperatorDispatchOutcome) -> Result<(), AppError> {
    match outcome {
        OperatorDispatchOutcome::Delivered | OperatorDispatchOutcome::NotConnected => Ok(()),
        OperatorDispatchOutcome::Backpressure => Err(storage_app_error(
            midgard_core::MidgardError::Controller("operator command queue is full".to_string()),
        )),
    }
}

pub(crate) fn register_operator_tools(
    registry: &mut ToolRegistry,
    orgs: SharedOrganizationStore,
    operators: OperatorRegistry,
) {
    registry.register(MiddlewareListTool { orgs: orgs.clone() });
    registry.register(MiddlewareRefreshTool {
        operators: operators.clone(),
    });
    registry.register(MiddlewareCreateTool {
        orgs: orgs.clone(),
        operators: operators.clone(),
    });
    registry.register(MiddlewareUpdateTool {
        orgs: orgs.clone(),
        operators: operators.clone(),
    });
    registry.register(MiddlewareDeleteTool { orgs, operators });
}

pub(crate) fn register_protocol_capability_tools(
    registry: &mut ToolRegistry,
    orgs: SharedOrganizationStore,
    operators: OperatorRegistry,
) {
    registry.register(OperatorCapabilityListTool {
        operators: operators.clone(),
    });
    registry.register(OperatorCapabilityExecuteTool { orgs, operators });
}

struct OperatorCapabilityListTool {
    operators: OperatorRegistry,
}

#[tonic::async_trait]
impl Tool for OperatorCapabilityListTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "operator_capability_list",
            "List connected Kubernetes middleware operators and their reported protocol capabilities for the current workspace.",
            json!({
                "type": "object",
                "properties": {
                    "middleware_kind": {"type": "string"}
                },
                "additionalProperties": false
            }),
            RiskLevel::Low,
        )
    }

    async fn call(&self, arguments: Value) -> ToolResult {
        self.call_with_context(arguments, ToolCallContext::default())
            .await
    }

    async fn call_with_context(&self, arguments: Value, context: ToolCallContext) -> ToolResult {
        let workspace_id = match scoped_workspace_string(&arguments, &context) {
            Ok(workspace_id) => workspace_id,
            Err(err) => return ToolResult::error(err),
        };
        let middleware_kind = optional_string(&arguments, "middleware_kind");
        let operators = self
            .operators
            .snapshots()
            .into_iter()
            .filter(|snapshot| snapshot.connected && snapshot.workspace_id == workspace_id)
            .filter(|snapshot| {
                middleware_kind
                    .as_ref()
                    .map(|kind| &snapshot.middleware_kind == kind)
                    .unwrap_or(true)
            })
            .collect::<Vec<_>>();

        ToolResult::success(json!({ "operators": operators }).to_string())
    }
}

struct OperatorCapabilityExecuteTool {
    orgs: SharedOrganizationStore,
    operators: OperatorRegistry,
}

#[tonic::async_trait]
impl Tool for OperatorCapabilityExecuteTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "operator_capability_execute",
            "Execute one reported Kubernetes operator protocol capability for the current workspace.",
            json!({
                "type": "object",
                "properties": {
                    "middleware_kind": {"type": "string"},
                    "capability_id": {"type": "string"},
                    "operation": {
                        "type": "string",
                        "enum": ["create", "update", "delete", "query", "refresh", "reconcile"]
                    },
                    "instance_id": {"type": "string", "format": "uuid"},
                    "name": {"type": "string"},
                    "namespace": {"type": "string", "default": "default"},
                    "desired_state": {
                        "type": "string",
                        "enum": ["enabled", "disabled"],
                        "default": "enabled"
                    },
                    "status": {
                        "type": "string",
                        "enum": ["pending", "running", "degraded", "stopped"]
                    },
                    "config": {"type": "object"}
                },
                "required": ["middleware_kind", "capability_id", "operation"],
                "additionalProperties": false
            }),
            RiskLevel::Critical,
        )
    }

    async fn call(&self, arguments: Value) -> ToolResult {
        self.call_with_context(arguments, ToolCallContext::default())
            .await
    }

    async fn call_with_context(&self, arguments: Value, context: ToolCallContext) -> ToolResult {
        let workspace_id = match scoped_workspace_string(&arguments, &context) {
            Ok(workspace_id) => workspace_id,
            Err(err) => return ToolResult::error(err),
        };
        let workspace_uuid = match Uuid::parse_str(&workspace_id) {
            Ok(workspace_id) => workspace_id,
            Err(err) => return ToolResult::error(format!("workspace_id must be a UUID: {err}")),
        };
        let middleware_kind = match required_string(&arguments, "middleware_kind") {
            Ok(kind) => kind,
            Err(err) => return ToolResult::error(err),
        };
        let capability_id = match required_string(&arguments, "capability_id") {
            Ok(capability_id) => capability_id,
            Err(err) => return ToolResult::error(err),
        };
        let operation = match required_operation(&arguments) {
            Ok(operation) => operation,
            Err(err) => return ToolResult::error(err),
        };
        let command_type = match command_type_for_operation(&operation) {
            Some(command_type) => command_type,
            None => return ToolResult::error(format!("unsupported operation: {operation}")),
        };

        if let Err(err) = self.validate_operator_capability(
            &workspace_id,
            &middleware_kind,
            &capability_id,
            &operation,
        ) {
            return ToolResult::error(err);
        }

        match command_type {
            CommandType::Create => {
                let name = match required_string(&arguments, "name") {
                    Ok(name) => name,
                    Err(err) => return ToolResult::error(err),
                };
                let namespace = optional_string(&arguments, "namespace")
                    .unwrap_or_else(|| "default".to_string());
                let desired_state =
                    desired_state_argument(&arguments).unwrap_or(MiddlewareDesiredState::Enabled);
                let config = arguments
                    .get("config")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let instance = match self
                    .orgs
                    .create_middleware_instance(NewMiddlewareInstance {
                        workspace_id: workspace_uuid,
                        kind: middleware_kind.clone(),
                        name,
                        namespace,
                        desired_state,
                        status: MiddlewareInstanceStatus::Pending,
                        config,
                    })
                    .await
                {
                    Ok(instance) => instance,
                    Err(err) => return ToolResult::error(err.to_string()),
                };
                let outcome = self.operators.dispatch_command(
                    &workspace_id,
                    &middleware_kind,
                    command_for_instance(CommandType::Create, &instance),
                );
                protocol_dispatch_tool_result(
                    "operator capability create dispatched",
                    outcome,
                    Some(&instance),
                )
            }
            CommandType::Update => {
                let instance_id = match required_uuid(&arguments, "instance_id") {
                    Ok(instance_id) => instance_id,
                    Err(err) => return ToolResult::error(err),
                };
                match load_middleware_instance(self.orgs.as_ref(), workspace_uuid, instance_id)
                    .await
                {
                    Ok(Some(instance)) if instance.kind == middleware_kind => {}
                    Ok(Some(_)) => {
                        return ToolResult::error(
                            "middleware instance kind does not match middleware_kind",
                        );
                    }
                    Ok(None) => return ToolResult::error("middleware instance not found"),
                    Err(err) => return ToolResult::error(err.to_string()),
                }
                let instance = match self
                    .orgs
                    .update_middleware_instance(
                        workspace_uuid,
                        instance_id,
                        MiddlewareInstanceUpdate {
                            desired_state: desired_state_argument(&arguments),
                            status: status_argument(&arguments),
                            config: arguments.get("config").cloned(),
                            archived: None,
                        },
                    )
                    .await
                {
                    Ok(Some(instance)) => instance,
                    Ok(None) => return ToolResult::error("middleware instance not found"),
                    Err(err) => return ToolResult::error(err.to_string()),
                };
                let outcome = self.operators.dispatch_command(
                    &workspace_id,
                    &middleware_kind,
                    command_for_instance(CommandType::Update, &instance),
                );
                protocol_dispatch_tool_result(
                    "operator capability update dispatched",
                    outcome,
                    Some(&instance),
                )
            }
            CommandType::Delete => {
                let instance_id = match required_uuid(&arguments, "instance_id") {
                    Ok(instance_id) => instance_id,
                    Err(err) => return ToolResult::error(err),
                };
                match load_middleware_instance(self.orgs.as_ref(), workspace_uuid, instance_id)
                    .await
                {
                    Ok(Some(instance)) if instance.kind == middleware_kind => {}
                    Ok(Some(_)) => {
                        return ToolResult::error(
                            "middleware instance kind does not match middleware_kind",
                        );
                    }
                    Ok(None) => return ToolResult::error("middleware instance not found"),
                    Err(err) => return ToolResult::error(err.to_string()),
                }
                let instance = match self
                    .orgs
                    .update_middleware_instance(
                        workspace_uuid,
                        instance_id,
                        MiddlewareInstanceUpdate {
                            desired_state: None,
                            status: None,
                            config: None,
                            archived: Some(true),
                        },
                    )
                    .await
                {
                    Ok(Some(instance)) => instance,
                    Ok(None) => return ToolResult::error("middleware instance not found"),
                    Err(err) => return ToolResult::error(err.to_string()),
                };
                let outcome = self.operators.dispatch_command(
                    &workspace_id,
                    &middleware_kind,
                    command_for_instance(CommandType::Delete, &instance),
                );
                protocol_dispatch_tool_result(
                    "operator capability delete dispatched",
                    outcome,
                    Some(&instance),
                )
            }
            CommandType::Query => {
                let instance_id = match required_uuid(&arguments, "instance_id") {
                    Ok(instance_id) => instance_id,
                    Err(err) => return ToolResult::error(err),
                };
                let instance =
                    match load_middleware_instance(self.orgs.as_ref(), workspace_uuid, instance_id)
                        .await
                    {
                        Ok(Some(instance)) => instance,
                        Ok(None) => return ToolResult::error("middleware instance not found"),
                        Err(err) => return ToolResult::error(err.to_string()),
                    };
                if instance.kind != middleware_kind {
                    return ToolResult::error(
                        "middleware instance kind does not match middleware_kind",
                    );
                }
                let outcome = self.operators.dispatch_command(
                    &workspace_id,
                    &middleware_kind,
                    command_for_instance(CommandType::Query, &instance),
                );
                protocol_dispatch_tool_result(
                    "operator capability query dispatched",
                    outcome,
                    Some(&instance),
                )
            }
            CommandType::Refresh => {
                let outcome = self.operators.dispatch_command(
                    &workspace_id,
                    &middleware_kind,
                    ServerCommand {
                        operation_id: Uuid::new_v4().to_string(),
                        command_type: CommandType::Refresh as i32,
                        instance: None,
                        instances: Vec::new(),
                    },
                );
                protocol_dispatch_tool_result(
                    "operator capability refresh dispatched",
                    outcome,
                    None,
                )
            }
            CommandType::Reconcile => {
                let instances = match self
                    .orgs
                    .list_middleware_instances_for_reconciliation(workspace_uuid)
                    .await
                {
                    Ok(instances) => instances,
                    Err(err) => return ToolResult::error(err.to_string()),
                };
                let resources = instances
                    .iter()
                    .filter(|instance| instance.kind == middleware_kind)
                    .map(MiddlewareResource::from)
                    .collect::<Vec<_>>();
                let outcome = self.operators.dispatch_command(
                    &workspace_id,
                    &middleware_kind,
                    ServerCommand {
                        operation_id: Uuid::new_v4().to_string(),
                        command_type: CommandType::Reconcile as i32,
                        instance: None,
                        instances: resources,
                    },
                );
                protocol_dispatch_tool_result(
                    "operator capability reconcile dispatched",
                    outcome,
                    None,
                )
            }
            CommandType::UnknownCommandType => {
                ToolResult::error(format!("unsupported operation: {operation}"))
            }
        }
    }
}

impl OperatorCapabilityExecuteTool {
    fn validate_operator_capability(
        &self,
        workspace_id: &str,
        middleware_kind: &str,
        capability_id: &str,
        operation: &str,
    ) -> Result<(), String> {
        let Some(snapshot) = self.operators.snapshots().into_iter().find(|snapshot| {
            snapshot.connected
                && snapshot.workspace_id == workspace_id
                && snapshot.middleware_kind == middleware_kind
        }) else {
            return Err(
                "operator is not connected for this workspace and middleware kind".to_string(),
            );
        };
        if !snapshot
            .capabilities
            .iter()
            .any(|capability| capability.id == capability_id)
        {
            return Err(format!(
                "operator capability is not available for this workspace and kind: {capability_id}"
            ));
        }
        if !snapshot
            .supported_operations
            .iter()
            .any(|supported| supported.eq_ignore_ascii_case(operation))
        {
            return Err(format!(
                "operator does not support operation {operation} for middleware kind {middleware_kind}"
            ));
        }

        Ok(())
    }
}

fn capability_descriptor_from_protocol(capability: OperatorCapability) -> CapabilityDescriptor {
    CapabilityDescriptor::new(
        capability.id,
        capability.name,
        risk_level_from_protocol_label(&capability.risk_level),
    )
}

fn risk_level_from_protocol_label(value: &str) -> RiskLevel {
    match value.trim().to_ascii_lowercase().as_str() {
        "low" => RiskLevel::Low,
        "medium" => RiskLevel::Medium,
        "high" => RiskLevel::High,
        "critical" => RiskLevel::Critical,
        _ => RiskLevel::Critical,
    }
}

fn required_operation(arguments: &Value) -> Result<String, String> {
    let operation = required_string(arguments, "operation")?.to_ascii_lowercase();
    if command_type_for_operation(&operation).is_some() {
        Ok(operation)
    } else {
        Err(format!("unsupported operation: {operation}"))
    }
}

fn command_type_for_operation(operation: &str) -> Option<CommandType> {
    match operation {
        "create" => Some(CommandType::Create),
        "update" => Some(CommandType::Update),
        "delete" => Some(CommandType::Delete),
        "query" => Some(CommandType::Query),
        "refresh" => Some(CommandType::Refresh),
        "reconcile" => Some(CommandType::Reconcile),
        _ => None,
    }
}

async fn load_middleware_instance(
    orgs: &dyn midgard_storage::OrganizationStore,
    workspace_id: Uuid,
    instance_id: Uuid,
) -> midgard_core::MidgardResult<Option<midgard_storage::MiddlewareInstance>> {
    Ok(orgs
        .list_middleware_instances_for_reconciliation(workspace_id)
        .await?
        .into_iter()
        .find(|instance| instance.id == instance_id))
}

fn protocol_dispatch_tool_result(
    message: &str,
    outcome: OperatorDispatchOutcome,
    instance: Option<&midgard_storage::MiddlewareInstance>,
) -> ToolResult {
    match outcome {
        OperatorDispatchOutcome::Delivered => match instance {
            Some(instance) => ToolResult::success(
                json!({
                    "message": message,
                    "operator_dispatch": "delivered",
                    "instance": instance,
                })
                .to_string(),
            ),
            None => ToolResult::success(
                json!({
                    "message": message,
                    "operator_dispatch": "delivered",
                })
                .to_string(),
            ),
        },
        OperatorDispatchOutcome::NotConnected => {
            ToolResult::error("operator is not connected for this workspace and kind")
        }
        OperatorDispatchOutcome::Backpressure => {
            ToolResult::error("operator command queue is full")
        }
    }
}

struct MiddlewareListTool {
    orgs: SharedOrganizationStore,
}

#[tonic::async_trait]
impl Tool for MiddlewareListTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "middleware_list",
            "List middleware instances persisted for a workspace, optionally filtered by middleware kind.",
            json!({
                "type": "object",
                "properties": {
                    "workspace_id": {"type": "string", "format": "uuid"},
                    "kind": {"type": "string"}
                },
                "required": ["workspace_id"]
            }),
            RiskLevel::Low,
        )
    }

    async fn call(&self, arguments: Value) -> ToolResult {
        self.call_with_context(arguments, ToolCallContext::default())
            .await
    }

    async fn call_with_context(&self, arguments: Value, context: ToolCallContext) -> ToolResult {
        let workspace_id = match scoped_workspace_uuid(&arguments, &context) {
            Ok(workspace_id) => workspace_id,
            Err(err) => return ToolResult::error(err),
        };
        let kind = optional_string(&arguments, "kind");

        match self.orgs.list_middleware_instances(workspace_id).await {
            Ok(instances) => {
                let instances = instances
                    .into_iter()
                    .filter(|instance| {
                        kind.as_ref()
                            .map(|kind| &instance.kind == kind)
                            .unwrap_or(true)
                    })
                    .collect::<Vec<_>>();
                ToolResult::success(json!(instances).to_string())
            }
            Err(err) => ToolResult::error(err.to_string()),
        }
    }
}

struct MiddlewareRefreshTool {
    operators: OperatorRegistry,
}

#[tonic::async_trait]
impl Tool for MiddlewareRefreshTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "middleware_refresh",
            "Ask a connected middleware operator to refresh its inventory for a workspace and middleware kind.",
            json!({
                "type": "object",
                "properties": {
                    "workspace_id": {"type": "string", "format": "uuid"},
                    "kind": {"type": "string"}
                },
                "required": ["workspace_id", "kind"]
            }),
            RiskLevel::Medium,
        )
    }

    async fn call(&self, arguments: Value) -> ToolResult {
        self.call_with_context(arguments, ToolCallContext::default())
            .await
    }

    async fn call_with_context(&self, arguments: Value, context: ToolCallContext) -> ToolResult {
        let workspace_id = match scoped_workspace_string(&arguments, &context) {
            Ok(workspace_id) => workspace_id,
            Err(err) => return ToolResult::error(err),
        };
        let kind = match required_string(&arguments, "kind") {
            Ok(kind) => kind,
            Err(err) => return ToolResult::error(err),
        };

        match self.operators.dispatch_command(
            &workspace_id,
            &kind,
            ServerCommand {
                operation_id: Uuid::new_v4().to_string(),
                command_type: CommandType::Refresh as i32,
                instance: None,
                instances: Vec::new(),
            },
        ) {
            OperatorDispatchOutcome::Delivered => ToolResult::success("refresh requested"),
            OperatorDispatchOutcome::NotConnected => {
                ToolResult::error("operator is not connected for this workspace and kind")
            }
            OperatorDispatchOutcome::Backpressure => {
                ToolResult::error("operator command queue is full")
            }
        }
    }
}

struct MiddlewareCreateTool {
    orgs: SharedOrganizationStore,
    operators: OperatorRegistry,
}

#[tonic::async_trait]
impl Tool for MiddlewareCreateTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "middleware_create",
            "Create desired middleware state and ask the connected operator to reconcile it.",
            json!({
                "type": "object",
                "properties": {
                    "workspace_id": {"type": "string", "format": "uuid"},
                    "kind": {"type": "string"},
                    "name": {"type": "string"},
                    "namespace": {"type": "string", "default": "default"},
                    "desired_state": {"type": "string", "enum": ["enabled", "disabled"], "default": "enabled"},
                    "config": {"type": "object"}
                },
                "required": ["workspace_id", "kind", "name"]
            }),
            RiskLevel::High,
        )
    }

    async fn call(&self, arguments: Value) -> ToolResult {
        self.call_with_context(arguments, ToolCallContext::default())
            .await
    }

    async fn call_with_context(&self, arguments: Value, context: ToolCallContext) -> ToolResult {
        let workspace_id = match scoped_workspace_uuid(&arguments, &context) {
            Ok(workspace_id) => workspace_id,
            Err(err) => return ToolResult::error(err),
        };
        let kind = match required_string(&arguments, "kind") {
            Ok(kind) => kind,
            Err(err) => return ToolResult::error(err),
        };
        let name = match required_string(&arguments, "name") {
            Ok(name) => name,
            Err(err) => return ToolResult::error(err),
        };
        let namespace =
            optional_string(&arguments, "namespace").unwrap_or_else(|| "default".to_string());
        let desired_state =
            desired_state_argument(&arguments).unwrap_or(MiddlewareDesiredState::Enabled);
        let config = arguments
            .get("config")
            .cloned()
            .unwrap_or_else(|| json!({}));

        let instance = match self
            .orgs
            .create_middleware_instance(NewMiddlewareInstance {
                workspace_id,
                kind: kind.clone(),
                name,
                namespace,
                desired_state,
                status: MiddlewareInstanceStatus::Pending,
                config,
            })
            .await
        {
            Ok(instance) => instance,
            Err(err) => return ToolResult::error(err.to_string()),
        };
        let outcome = self.operators.dispatch_command(
            &workspace_id.to_string(),
            &kind,
            command_for_instance(CommandType::Create, &instance),
        );
        persisted_tool_result("middleware desired state created", outcome, &instance)
    }
}

struct MiddlewareUpdateTool {
    orgs: SharedOrganizationStore,
    operators: OperatorRegistry,
}

#[tonic::async_trait]
impl Tool for MiddlewareUpdateTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "middleware_update",
            "Update desired middleware state and ask the connected operator to reconcile it.",
            json!({
                "type": "object",
                "properties": {
                    "workspace_id": {"type": "string", "format": "uuid"},
                    "instance_id": {"type": "string", "format": "uuid"},
                    "desired_state": {"type": "string", "enum": ["enabled", "disabled"]},
                    "status": {"type": "string", "enum": ["pending", "running", "degraded", "stopped"]},
                    "config": {"type": "object"}
                },
                "required": ["workspace_id", "instance_id"]
            }),
            RiskLevel::High,
        )
    }

    async fn call(&self, arguments: Value) -> ToolResult {
        self.call_with_context(arguments, ToolCallContext::default())
            .await
    }

    async fn call_with_context(&self, arguments: Value, context: ToolCallContext) -> ToolResult {
        let workspace_id = match scoped_workspace_uuid(&arguments, &context) {
            Ok(workspace_id) => workspace_id,
            Err(err) => return ToolResult::error(err),
        };
        let instance_id = match required_uuid(&arguments, "instance_id") {
            Ok(instance_id) => instance_id,
            Err(err) => return ToolResult::error(err),
        };
        let update = MiddlewareInstanceUpdate {
            desired_state: desired_state_argument(&arguments),
            status: status_argument(&arguments),
            config: arguments.get("config").cloned(),
            archived: None,
        };

        let instance = match self
            .orgs
            .update_middleware_instance(workspace_id, instance_id, update)
            .await
        {
            Ok(Some(instance)) => instance,
            Ok(None) => return ToolResult::error("middleware instance not found"),
            Err(err) => return ToolResult::error(err.to_string()),
        };
        let outcome = self.operators.dispatch_command(
            &workspace_id.to_string(),
            &instance.kind,
            command_for_instance(CommandType::Update, &instance),
        );
        persisted_tool_result("middleware desired state updated", outcome, &instance)
    }
}

struct MiddlewareDeleteTool {
    orgs: SharedOrganizationStore,
    operators: OperatorRegistry,
}

#[tonic::async_trait]
impl Tool for MiddlewareDeleteTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            "middleware_delete",
            "Archive desired middleware state and ask the connected operator to delete the middleware resource.",
            json!({
                "type": "object",
                "properties": {
                    "workspace_id": {"type": "string", "format": "uuid"},
                    "instance_id": {"type": "string", "format": "uuid"}
                },
                "required": ["workspace_id", "instance_id"]
            }),
            RiskLevel::Critical,
        )
    }

    async fn call(&self, arguments: Value) -> ToolResult {
        self.call_with_context(arguments, ToolCallContext::default())
            .await
    }

    async fn call_with_context(&self, arguments: Value, context: ToolCallContext) -> ToolResult {
        let workspace_id = match scoped_workspace_uuid(&arguments, &context) {
            Ok(workspace_id) => workspace_id,
            Err(err) => return ToolResult::error(err),
        };
        let instance_id = match required_uuid(&arguments, "instance_id") {
            Ok(instance_id) => instance_id,
            Err(err) => return ToolResult::error(err),
        };
        let instance = match self
            .orgs
            .update_middleware_instance(
                workspace_id,
                instance_id,
                MiddlewareInstanceUpdate {
                    desired_state: None,
                    status: None,
                    config: None,
                    archived: Some(true),
                },
            )
            .await
        {
            Ok(Some(instance)) => instance,
            Ok(None) => return ToolResult::error("middleware instance not found"),
            Err(err) => return ToolResult::error(err.to_string()),
        };
        let outcome = self.operators.dispatch_command(
            &workspace_id.to_string(),
            &instance.kind,
            command_for_instance(CommandType::Delete, &instance),
        );
        persisted_tool_result("middleware desired state archived", outcome, &instance)
    }
}

fn persisted_tool_result(
    message: &str,
    outcome: OperatorDispatchOutcome,
    instance: &midgard_storage::MiddlewareInstance,
) -> ToolResult {
    match outcome {
        OperatorDispatchOutcome::Delivered => ToolResult::success(
            json!({
                "message": message,
                "operator_dispatch": "delivered",
                "instance": instance,
            })
            .to_string(),
        ),
        OperatorDispatchOutcome::NotConnected => ToolResult::success(
            json!({
                "message": message,
                "operator_dispatch": "not_connected",
                "operator_note": "desired state is persisted and will reconcile when the operator reconnects",
                "instance": instance,
            })
            .to_string(),
        ),
        OperatorDispatchOutcome::Backpressure => ToolResult::error("operator command queue is full"),
    }
}

fn required_string(arguments: &Value, key: &str) -> Result<String, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{key} is required"))
}

fn optional_string(arguments: &Value, key: &str) -> Option<String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn required_uuid(arguments: &Value, key: &str) -> Result<Uuid, String> {
    let value = required_string(arguments, key)?;
    Uuid::parse_str(&value).map_err(|err| format!("{key} must be a UUID: {err}"))
}

fn scoped_workspace_string(arguments: &Value, context: &ToolCallContext) -> Result<String, String> {
    let argument_workspace_id = optional_string(arguments, "workspace_id");
    match (&context.workspace_id, argument_workspace_id) {
        (Some(context_workspace_id), Some(argument_workspace_id))
            if context_workspace_id != &argument_workspace_id =>
        {
            Err("workspace_id does not match the current agent workspace".to_string())
        }
        (Some(context_workspace_id), _) => Ok(context_workspace_id.clone()),
        (None, Some(argument_workspace_id)) => Ok(argument_workspace_id),
        (None, None) => Err("workspace_id is required".to_string()),
    }
}

fn scoped_workspace_uuid(arguments: &Value, context: &ToolCallContext) -> Result<Uuid, String> {
    let workspace_id = scoped_workspace_string(arguments, context)?;
    Uuid::parse_str(&workspace_id).map_err(|err| format!("workspace_id must be a UUID: {err}"))
}

fn desired_state_argument(arguments: &Value) -> Option<MiddlewareDesiredState> {
    optional_string(arguments, "desired_state").and_then(|value| match value.as_str() {
        "enabled" => Some(MiddlewareDesiredState::Enabled),
        "disabled" => Some(MiddlewareDesiredState::Disabled),
        _ => None,
    })
}

fn status_argument(arguments: &Value) -> Option<MiddlewareInstanceStatus> {
    optional_string(arguments, "status").and_then(|value| match value.as_str() {
        "pending" => Some(MiddlewareInstanceStatus::Pending),
        "running" => Some(MiddlewareInstanceStatus::Running),
        "degraded" => Some(MiddlewareInstanceStatus::Degraded),
        "stopped" => Some(MiddlewareInstanceStatus::Stopped),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use midgard_agent::{LlmResponse, ScriptedLlmProvider};
    use midgard_storage::{
        MemoryAgentSessionStore, MemoryAuthStore, MemoryOrganizationStore, NewMiddlewareInstance,
        OrganizationStore,
    };

    use crate::{
        AuthSettings, WorkspaceCredentialSettings,
        app_state_with_provider_auth_orgs_credentials_and_operator_registry,
    };

    fn registration(operator_id: &str) -> OperatorRegistration {
        registration_for(operator_id, Uuid::nil(), "redis", ["create", "delete"])
    }

    fn registration_for<const N: usize>(
        operator_id: &str,
        workspace_id: Uuid,
        middleware_kind: &str,
        supported_operations: [&str; N],
    ) -> OperatorRegistration {
        OperatorRegistration {
            protocol_version: OPERATOR_PROTOCOL_VERSION,
            operator_id: operator_id.to_string(),
            workspace_id: workspace_id.to_string(),
            middleware_kind: middleware_kind.to_string(),
            display_name: format!("{middleware_kind} Operator"),
            supported_operations: supported_operations
                .into_iter()
                .map(str::to_string)
                .collect(),
        }
    }

    fn capability(id: &str, name: &str, risk_level: &str) -> OperatorCapability {
        OperatorCapability {
            id: id.to_string(),
            name: name.to_string(),
            risk_level: risk_level.to_string(),
        }
    }

    #[test]
    fn registry_rejects_incomplete_registration() {
        let registry = OperatorRegistry::new();
        let mut registration = registration("operator-1");
        registration.workspace_id.clear();

        let result = registry.validate_registration(&registration);

        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn registry_rejects_registration_with_non_uuid_workspace() {
        let registry = OperatorRegistry::new();
        let mut registration = registration("operator-1");
        registration.workspace_id = "not-a-uuid".to_string();

        let result = registry.validate_registration(&registration);

        assert_eq!(result.unwrap_err().code(), tonic::Code::InvalidArgument);
    }

    #[test]
    fn registry_tracks_valid_registration_and_disconnect() {
        let registry = OperatorRegistry::new();
        let (sender, _receiver) = mpsc::channel(1);
        let lease = registry
            .register(registration("operator-1"), sender)
            .unwrap();

        let snapshot = registry.snapshots().pop().unwrap();
        assert!(snapshot.connected);
        assert_eq!(snapshot.operator_id, "operator-1");

        drop(lease);

        let snapshot = registry.snapshots().pop().unwrap();
        assert!(!snapshot.connected);
    }

    #[test]
    fn registry_preserves_capability_metadata_and_defaults_unknown_risk_to_critical() {
        let registry = OperatorRegistry::new();
        let (sender, _receiver) = mpsc::channel(1);
        let _lease = registry
            .register(registration("operator-1"), sender)
            .unwrap();

        registry.update_capabilities(
            "operator-1",
            vec![
                capability_descriptor_from_protocol(capability(
                    "redis.query",
                    "Query Redis",
                    "low",
                )),
                capability_descriptor_from_protocol(capability(
                    "redis.destroy",
                    "Destroy Redis",
                    "unexpected",
                )),
            ],
        );

        let snapshot = registry.snapshots().pop().unwrap();
        assert_eq!(snapshot.capabilities[0].id, "redis.query");
        assert_eq!(snapshot.capabilities[0].name, "Query Redis");
        assert_eq!(snapshot.capabilities[0].risk_level, RiskLevel::Low);
        assert!(!snapshot.capabilities[0].requires_approval);
        assert_eq!(snapshot.capabilities[1].risk_level, RiskLevel::Critical);
        assert!(snapshot.capabilities[1].requires_approval);
    }

    #[test]
    fn registry_rejects_duplicate_kind_from_different_operator() {
        let registry = OperatorRegistry::new();
        let (sender, _receiver) = mpsc::channel(1);
        let _lease = registry
            .register(registration("operator-1"), sender)
            .unwrap();
        let (sender, _receiver) = mpsc::channel(1);

        match registry.register(registration("operator-2"), sender) {
            Ok(_) => panic!("duplicate operator registration should fail"),
            Err(err) => assert_eq!(err.code(), tonic::Code::AlreadyExists),
        }
    }

    #[tokio::test]
    async fn initial_reconcile_includes_active_and_archived_instances() {
        let workspace_id = Uuid::new_v4();
        let orgs = Arc::new(MemoryOrganizationStore::new());
        let active = orgs
            .create_middleware_instance(NewMiddlewareInstance {
                workspace_id,
                kind: "redis".to_string(),
                name: "cache".to_string(),
                namespace: "data".to_string(),
                desired_state: MiddlewareDesiredState::Enabled,
                status: MiddlewareInstanceStatus::Pending,
                config: json!({}),
            })
            .await
            .unwrap();
        let archived = orgs
            .create_middleware_instance(NewMiddlewareInstance {
                workspace_id,
                kind: "redis".to_string(),
                name: "old-cache".to_string(),
                namespace: "data".to_string(),
                desired_state: MiddlewareDesiredState::Enabled,
                status: MiddlewareInstanceStatus::Stopped,
                config: json!({}),
            })
            .await
            .unwrap();
        orgs.update_middleware_instance(
            workspace_id,
            archived.id,
            MiddlewareInstanceUpdate {
                archived: Some(true),
                ..MiddlewareInstanceUpdate::default()
            },
        )
        .await
        .unwrap();
        let state = app_state_with_provider_auth_orgs_credentials_and_operator_registry(
            Arc::new(MemoryAgentSessionStore::new()),
            Arc::new(MemoryAuthStore::new()),
            orgs,
            Arc::new(ScriptedLlmProvider::single(LlmResponse::text("unused"))),
            AuthSettings::default(),
            WorkspaceCredentialSettings::default(),
            OperatorRegistry::default(),
        );
        let service = OperatorControlService::new(state);
        let (sender, mut receiver) = mpsc::channel(4);

        service
            .send_initial_reconcile(
                &OperatorRegistration {
                    protocol_version: OPERATOR_PROTOCOL_VERSION,
                    operator_id: "operator-1".to_string(),
                    workspace_id: workspace_id.to_string(),
                    middleware_kind: "redis".to_string(),
                    display_name: "Redis Operator".to_string(),
                    supported_operations: Vec::new(),
                },
                &sender,
            )
            .await
            .unwrap();

        let message = receiver.recv().await.unwrap();
        let Some(server_to_operator::Payload::Command(command)) = message.payload else {
            panic!("expected reconcile command");
        };
        assert_eq!(
            CommandType::try_from(command.command_type).unwrap(),
            CommandType::Reconcile
        );
        let ids = command
            .instances
            .iter()
            .map(|instance| instance.id.as_str())
            .collect::<Vec<_>>();
        let active_id = active.id.to_string();
        let archived_id = archived.id.to_string();
        assert!(ids.iter().any(|id| *id == active_id));
        assert!(ids.iter().any(|id| *id == archived_id));
    }

    #[tokio::test]
    async fn operator_status_updates_persisted_instance_status() {
        let workspace_id = Uuid::new_v4();
        let orgs = Arc::new(MemoryOrganizationStore::new());
        let instance = orgs
            .create_middleware_instance(NewMiddlewareInstance {
                workspace_id,
                kind: "redis".to_string(),
                name: "cache".to_string(),
                namespace: "data".to_string(),
                desired_state: MiddlewareDesiredState::Enabled,
                status: MiddlewareInstanceStatus::Pending,
                config: json!({}),
            })
            .await
            .unwrap();
        let registry = OperatorRegistry::new();
        let (sender, _receiver) = mpsc::channel(4);
        let _lease = registry
            .register(
                OperatorRegistration {
                    protocol_version: OPERATOR_PROTOCOL_VERSION,
                    operator_id: "operator-1".to_string(),
                    workspace_id: workspace_id.to_string(),
                    middleware_kind: "redis".to_string(),
                    display_name: "Redis Operator".to_string(),
                    supported_operations: Vec::new(),
                },
                sender,
            )
            .unwrap();
        let state = app_state_with_provider_auth_orgs_credentials_and_operator_registry(
            Arc::new(MemoryAgentSessionStore::new()),
            Arc::new(MemoryAuthStore::new()),
            orgs.clone(),
            Arc::new(ScriptedLlmProvider::single(LlmResponse::text("unused"))),
            AuthSettings::default(),
            WorkspaceCredentialSettings::default(),
            registry,
        );
        let service = OperatorControlService::new(state);

        service
            .apply_operator_status(instance.id.to_string(), MiddlewareStatus::Running as i32)
            .await
            .unwrap();

        let instances = orgs.list_middleware_instances(workspace_id).await.unwrap();
        assert_eq!(instances[0].status, MiddlewareInstanceStatus::Running);
    }

    #[tokio::test]
    async fn operator_tools_reject_cross_workspace_arguments() {
        let tool = MiddlewareCreateTool {
            orgs: Arc::new(MemoryOrganizationStore::new()),
            operators: OperatorRegistry::default(),
        };
        let current_workspace_id = Uuid::new_v4();
        let other_workspace_id = Uuid::new_v4();

        let result = tool
            .call_with_context(
                json!({
                    "workspace_id": other_workspace_id,
                    "kind": "redis",
                    "name": "cache"
                }),
                ToolCallContext {
                    workspace_id: Some(current_workspace_id.to_string()),
                },
            )
            .await;

        assert!(result.is_error);
        assert!(result.output.contains("current agent workspace"));
    }

    #[tokio::test]
    async fn operator_capability_list_filters_connected_capabilities_by_workspace_and_kind() {
        let workspace_id = Uuid::new_v4();
        let other_workspace_id = Uuid::new_v4();
        let registry = OperatorRegistry::new();
        let (sender, _receiver) = mpsc::channel(4);
        let _lease = registry
            .register(
                registration_for("operator-1", workspace_id, "valkey", ["query"]),
                sender,
            )
            .unwrap();
        registry.update_capabilities(
            "operator-1",
            vec![capability_descriptor_from_protocol(capability(
                "valkey.query",
                "Query Valkey clusters",
                "low",
            ))],
        );
        let (sender, _receiver) = mpsc::channel(4);
        let _other_lease = registry
            .register(
                registration_for("operator-2", other_workspace_id, "redis", ["query"]),
                sender,
            )
            .unwrap();
        let tool = OperatorCapabilityListTool {
            operators: registry,
        };

        let result = tool
            .call_with_context(
                json!({"middleware_kind": "valkey"}),
                ToolCallContext {
                    workspace_id: Some(workspace_id.to_string()),
                },
            )
            .await;

        assert!(result.success);
        let output: Value = serde_json::from_str(&result.output).unwrap();
        let operators = output["operators"].as_array().unwrap();
        assert_eq!(operators.len(), 1);
        assert_eq!(operators[0]["middleware_kind"], "valkey");
        assert_eq!(operators[0]["capabilities"][0]["id"], "valkey.query");
        assert_eq!(operators[0]["capabilities"][0]["risk_level"], "low");
    }

    #[tokio::test]
    async fn operator_capability_execute_rejects_disconnected_operator_before_persisting() {
        let workspace_id = Uuid::new_v4();
        let orgs = Arc::new(MemoryOrganizationStore::new());
        let tool = OperatorCapabilityExecuteTool {
            orgs: orgs.clone(),
            operators: OperatorRegistry::default(),
        };

        let result = tool
            .call_with_context(
                json!({
                    "middleware_kind": "valkey",
                    "capability_id": "valkey.create",
                    "operation": "create",
                    "name": "cache",
                    "namespace": "data",
                    "config": {"shards": 3}
                }),
                ToolCallContext {
                    workspace_id: Some(workspace_id.to_string()),
                },
            )
            .await;

        assert!(result.is_error);
        assert!(result.output.contains("operator is not connected"));
        assert!(
            orgs.list_middleware_instances(workspace_id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn operator_capability_execute_rejects_unknown_capability_and_unsupported_operation() {
        let workspace_id = Uuid::new_v4();
        let registry = OperatorRegistry::new();
        let (sender, _receiver) = mpsc::channel(4);
        let _lease = registry
            .register(
                registration_for("operator-1", workspace_id, "valkey", ["create"]),
                sender,
            )
            .unwrap();
        registry.update_capabilities(
            "operator-1",
            vec![capability_descriptor_from_protocol(capability(
                "valkey.create",
                "Create Valkey clusters",
                "high",
            ))],
        );
        let tool = OperatorCapabilityExecuteTool {
            orgs: Arc::new(MemoryOrganizationStore::new()),
            operators: registry,
        };
        let context = ToolCallContext {
            workspace_id: Some(workspace_id.to_string()),
        };

        let unknown_capability = tool
            .call_with_context(
                json!({
                    "middleware_kind": "valkey",
                    "capability_id": "valkey.delete",
                    "operation": "create",
                    "name": "cache"
                }),
                context.clone(),
            )
            .await;
        let unsupported_operation = tool
            .call_with_context(
                json!({
                    "middleware_kind": "valkey",
                    "capability_id": "valkey.create",
                    "operation": "delete",
                    "instance_id": Uuid::new_v4().to_string()
                }),
                context,
            )
            .await;

        assert!(unknown_capability.is_error);
        assert!(unknown_capability.output.contains("not available"));
        assert!(unsupported_operation.is_error);
        assert!(
            unsupported_operation
                .output
                .contains("does not support operation")
        );
    }

    #[tokio::test]
    async fn operator_capability_execute_rejects_instance_kind_mismatch_before_mutating() {
        let workspace_id = Uuid::new_v4();
        let orgs = Arc::new(MemoryOrganizationStore::new());
        let instance = orgs
            .create_middleware_instance(NewMiddlewareInstance {
                workspace_id,
                kind: "redis".to_string(),
                name: "cache".to_string(),
                namespace: "data".to_string(),
                desired_state: MiddlewareDesiredState::Enabled,
                status: MiddlewareInstanceStatus::Pending,
                config: json!({"memory": "512Mi"}),
            })
            .await
            .unwrap();
        let registry = OperatorRegistry::new();
        let (sender, _receiver) = mpsc::channel(4);
        let _lease = registry
            .register(
                registration_for("operator-1", workspace_id, "valkey", ["update"]),
                sender,
            )
            .unwrap();
        registry.update_capabilities(
            "operator-1",
            vec![capability_descriptor_from_protocol(capability(
                "valkey.update",
                "Update Valkey clusters",
                "high",
            ))],
        );
        let tool = OperatorCapabilityExecuteTool {
            orgs: orgs.clone(),
            operators: registry,
        };

        let result = tool
            .call_with_context(
                json!({
                    "middleware_kind": "valkey",
                    "capability_id": "valkey.update",
                    "operation": "update",
                    "instance_id": instance.id.to_string(),
                    "config": {"shards": 5}
                }),
                ToolCallContext {
                    workspace_id: Some(workspace_id.to_string()),
                },
            )
            .await;

        assert!(result.is_error);
        assert!(result.output.contains("kind does not match"));
        let unchanged = orgs
            .list_middleware_instances(workspace_id)
            .await
            .unwrap()
            .into_iter()
            .find(|current| current.id == instance.id)
            .unwrap();
        assert_eq!(unchanged.config, json!({"memory": "512Mi"}));
    }

    #[tokio::test]
    async fn operator_capability_execute_dispatches_query_refresh_and_reconcile() {
        let workspace_id = Uuid::new_v4();
        let orgs = Arc::new(MemoryOrganizationStore::new());
        let valkey = orgs
            .create_middleware_instance(NewMiddlewareInstance {
                workspace_id,
                kind: "valkey".to_string(),
                name: "cache".to_string(),
                namespace: "data".to_string(),
                desired_state: MiddlewareDesiredState::Enabled,
                status: MiddlewareInstanceStatus::Pending,
                config: json!({"shards": 3}),
            })
            .await
            .unwrap();
        orgs.create_middleware_instance(NewMiddlewareInstance {
            workspace_id,
            kind: "redis".to_string(),
            name: "legacy".to_string(),
            namespace: "data".to_string(),
            desired_state: MiddlewareDesiredState::Enabled,
            status: MiddlewareInstanceStatus::Pending,
            config: json!({}),
        })
        .await
        .unwrap();
        let registry = OperatorRegistry::new();
        let (sender, mut receiver) = mpsc::channel(8);
        let _lease = registry
            .register(
                registration_for(
                    "operator-1",
                    workspace_id,
                    "valkey",
                    ["query", "refresh", "reconcile"],
                ),
                sender,
            )
            .unwrap();
        registry.update_capabilities(
            "operator-1",
            vec![
                capability_descriptor_from_protocol(capability(
                    "valkey.query",
                    "Query Valkey clusters",
                    "low",
                )),
                capability_descriptor_from_protocol(capability(
                    "valkey.refresh",
                    "Refresh Valkey inventory",
                    "medium",
                )),
                capability_descriptor_from_protocol(capability(
                    "valkey.reconcile",
                    "Reconcile Valkey clusters",
                    "medium",
                )),
            ],
        );
        let tool = OperatorCapabilityExecuteTool {
            orgs,
            operators: registry,
        };
        let context = ToolCallContext {
            workspace_id: Some(workspace_id.to_string()),
        };

        let query = tool
            .call_with_context(
                json!({
                    "middleware_kind": "valkey",
                    "capability_id": "valkey.query",
                    "operation": "query",
                    "instance_id": valkey.id.to_string()
                }),
                context.clone(),
            )
            .await;
        assert!(query.success);
        let query_command = dispatched_command(&mut receiver).await;
        assert_eq!(
            CommandType::try_from(query_command.command_type).unwrap(),
            CommandType::Query
        );
        assert_eq!(query_command.instance.unwrap().id, valkey.id.to_string());

        let refresh = tool
            .call_with_context(
                json!({
                    "middleware_kind": "valkey",
                    "capability_id": "valkey.refresh",
                    "operation": "refresh"
                }),
                context.clone(),
            )
            .await;
        assert!(refresh.success);
        let refresh_command = dispatched_command(&mut receiver).await;
        assert_eq!(
            CommandType::try_from(refresh_command.command_type).unwrap(),
            CommandType::Refresh
        );
        assert!(refresh_command.instance.is_none());

        let reconcile = tool
            .call_with_context(
                json!({
                    "middleware_kind": "valkey",
                    "capability_id": "valkey.reconcile",
                    "operation": "reconcile"
                }),
                context,
            )
            .await;
        assert!(reconcile.success);
        let reconcile_command = dispatched_command(&mut receiver).await;
        assert_eq!(
            CommandType::try_from(reconcile_command.command_type).unwrap(),
            CommandType::Reconcile
        );
        assert_eq!(reconcile_command.instances.len(), 1);
        assert_eq!(reconcile_command.instances[0].kind, "valkey");
    }

    #[tokio::test]
    async fn operator_capability_execute_persists_state_and_dispatches_commands() {
        let workspace_id = Uuid::new_v4();
        let orgs = Arc::new(MemoryOrganizationStore::new());
        let registry = OperatorRegistry::new();
        let (sender, mut receiver) = mpsc::channel(8);
        let _lease = registry
            .register(
                registration_for(
                    "operator-1",
                    workspace_id,
                    "valkey",
                    ["create", "update", "delete"],
                ),
                sender,
            )
            .unwrap();
        registry.update_capabilities(
            "operator-1",
            vec![
                capability_descriptor_from_protocol(capability(
                    "valkey.create",
                    "Create Valkey clusters",
                    "high",
                )),
                capability_descriptor_from_protocol(capability(
                    "valkey.update",
                    "Update Valkey clusters",
                    "high",
                )),
                capability_descriptor_from_protocol(capability(
                    "valkey.delete",
                    "Delete Valkey clusters",
                    "critical",
                )),
            ],
        );
        let tool = OperatorCapabilityExecuteTool {
            orgs: orgs.clone(),
            operators: registry,
        };
        let context = ToolCallContext {
            workspace_id: Some(workspace_id.to_string()),
        };

        let create = tool
            .call_with_context(
                json!({
                    "middleware_kind": "valkey",
                    "capability_id": "valkey.create",
                    "operation": "create",
                    "name": "cache",
                    "namespace": "data",
                    "config": {"shards": 3}
                }),
                context.clone(),
            )
            .await;
        assert!(create.success);
        let create_output: Value = serde_json::from_str(&create.output).unwrap();
        let instance_id =
            Uuid::parse_str(create_output["instance"]["id"].as_str().unwrap()).unwrap();
        let create_command = dispatched_command(&mut receiver).await;
        assert_eq!(
            CommandType::try_from(create_command.command_type).unwrap(),
            CommandType::Create
        );
        assert_eq!(create_command.instance.unwrap().kind, "valkey");

        let update = tool
            .call_with_context(
                json!({
                    "middleware_kind": "valkey",
                    "capability_id": "valkey.update",
                    "operation": "update",
                    "instance_id": instance_id.to_string(),
                    "config": {"shards": 5}
                }),
                context.clone(),
            )
            .await;
        assert!(update.success);
        let update_command = dispatched_command(&mut receiver).await;
        assert_eq!(
            CommandType::try_from(update_command.command_type).unwrap(),
            CommandType::Update
        );
        assert_eq!(update_command.instance.unwrap().id, instance_id.to_string());

        let delete = tool
            .call_with_context(
                json!({
                    "middleware_kind": "valkey",
                    "capability_id": "valkey.delete",
                    "operation": "delete",
                    "instance_id": instance_id.to_string()
                }),
                context,
            )
            .await;
        assert!(delete.success);
        let delete_command = dispatched_command(&mut receiver).await;
        assert_eq!(
            CommandType::try_from(delete_command.command_type).unwrap(),
            CommandType::Delete
        );
        let archived = orgs
            .list_middleware_instances_for_reconciliation(workspace_id)
            .await
            .unwrap()
            .into_iter()
            .find(|instance| instance.id == instance_id)
            .unwrap();
        assert!(archived.archived_at.is_some());
    }

    async fn dispatched_command(receiver: &mut mpsc::Receiver<ServerToOperator>) -> ServerCommand {
        let message = receiver.recv().await.unwrap();
        let Some(server_to_operator::Payload::Command(command)) = message.payload else {
            panic!("expected command");
        };
        command
    }
}
