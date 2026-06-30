use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use chrono::{SecondsFormat, Utc};
use midgard_agent::{
    AgentMessage, AgentRunEvent, AgentRunStatus, AgentSession, AgentToolCall, ApprovalRecord,
    PendingApproval,
};
use midgard_storage::{
    MiddlewareInstance, Organization, OrganizationMembership, PermissionKey, Workspace,
    WorkspaceRuntimeConfigView,
};
use midgard_tools::{ToolDefinition, ToolResult};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use ts_rs::TS;
use uuid::Uuid;

use crate::PluginResponse;

pub const WORKSPACE_PROTOCOL_VERSION: u8 = 1;
const EVENT_BUFFER_SIZE: usize = 256;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
pub struct WorkspaceSnapshot {
    pub organization: Organization,
    pub workspace: Workspace,
    pub runtime_config: WorkspaceRuntimeConfigView,
    pub current_membership: OrganizationMembership,
    pub current_permissions: Vec<PermissionKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<AgentSession>,
    pub sessions: Vec<crate::AgentSessionSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "string | null")]
    pub active_session_id: Option<Uuid>,
    pub tools: Vec<ToolDefinition>,
    pub plugins: Vec<PluginResponse>,
    pub middleware_instances: Vec<MiddlewareInstance>,
    pub middleware: MiddlewareDashboardState,
    pub approvals: Vec<ApprovalRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DashboardTone {
    Ready,
    Warn,
    Danger,
    Neutral,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct MiddlewareMetric {
    pub id: String,
    pub label: String,
    pub value: String,
    pub detail: String,
    pub tone: DashboardTone,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct MiddlewareWorkload {
    pub id: String,
    pub namespace: String,
    pub name: String,
    pub kind: String,
    pub health: String,
    pub saturation: u8,
    pub risk: String,
    pub tone: DashboardTone,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct MiddlewareTimelineEvent {
    pub id: String,
    pub namespace: String,
    pub target: String,
    pub reason: String,
    pub message: String,
    pub observed_at: String,
    pub tone: DashboardTone,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct MiddlewareDashboardState {
    pub metrics: Vec<MiddlewareMetric>,
    pub workloads: Vec<MiddlewareWorkload>,
    pub events: Vec<MiddlewareTimelineEvent>,
}

impl MiddlewareDashboardState {
    pub fn mock() -> Self {
        Self {
            metrics: vec![
                MiddlewareMetric {
                    id: "healthy_workloads".to_string(),
                    label: "Healthy workloads".to_string(),
                    value: "12/14".to_string(),
                    detail: "2 need attention".to_string(),
                    tone: DashboardTone::Ready,
                },
                MiddlewareMetric {
                    id: "approval_queue".to_string(),
                    label: "Approval queue".to_string(),
                    value: "2".to_string(),
                    detail: "high-risk actions".to_string(),
                    tone: DashboardTone::Warn,
                },
                MiddlewareMetric {
                    id: "registered_tools".to_string(),
                    label: "Registered tools".to_string(),
                    value: "18".to_string(),
                    detail: "7 gated".to_string(),
                    tone: DashboardTone::Neutral,
                },
                MiddlewareMetric {
                    id: "controller_latency".to_string(),
                    label: "Controller latency".to_string(),
                    value: "118ms".to_string(),
                    detail: "p95 mock sample".to_string(),
                    tone: DashboardTone::Ready,
                },
            ],
            workloads: vec![
                MiddlewareWorkload {
                    id: "default/redis-cache".to_string(),
                    namespace: "default".to_string(),
                    name: "redis-cache".to_string(),
                    kind: "Redis".to_string(),
                    health: "Healthy".to_string(),
                    saturation: 41,
                    risk: "Low".to_string(),
                    tone: DashboardTone::Ready,
                },
                MiddlewareWorkload {
                    id: "streaming/kafka-brokers".to_string(),
                    namespace: "streaming".to_string(),
                    name: "kafka-brokers".to_string(),
                    kind: "Kafka".to_string(),
                    health: "Degraded".to_string(),
                    saturation: 73,
                    risk: "High".to_string(),
                    tone: DashboardTone::Warn,
                },
                MiddlewareWorkload {
                    id: "data/postgres-primary".to_string(),
                    namespace: "data".to_string(),
                    name: "postgres-primary".to_string(),
                    kind: "PostgreSQL".to_string(),
                    health: "Watch".to_string(),
                    saturation: 62,
                    risk: "Medium".to_string(),
                    tone: DashboardTone::Neutral,
                },
            ],
            events: vec![MiddlewareTimelineEvent {
                id: "mock-event-1".to_string(),
                namespace: "default".to_string(),
                target: "redis-cache".to_string(),
                reason: "Ready".to_string(),
                message: "Redis workload is serving traffic.".to_string(),
                observed_at: utc_now_rfc3339(),
                tone: DashboardTone::Ready,
            }],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceEventType {
    Connected,
    Heartbeat,
    Error,
    AgentSessionUpdated,
    AgentRunStarted,
    AgentMessageDelta,
    AgentMessageCommitted,
    ToolCallRequested,
    ToolResultReceived,
    AgentRunCompleted,
    AgentRunFailed,
    ApprovalRequired,
    ApprovalDecided,
    MiddlewareSnapshot,
    MiddlewareInstanceUpserted,
    MiddlewareInstanceRemoved,
    MiddlewareWorkloadUpserted,
    MiddlewareWorkloadRemoved,
    MiddlewareMetricChanged,
    MiddlewareEventObserved,
    ToolCatalogUpdated,
    PluginCatalogUpdated,
}

impl WorkspaceEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkspaceEventType::Connected => "connected",
            WorkspaceEventType::Heartbeat => "heartbeat",
            WorkspaceEventType::Error => "error",
            WorkspaceEventType::AgentSessionUpdated => "agent_session_updated",
            WorkspaceEventType::AgentRunStarted => "agent_run_started",
            WorkspaceEventType::AgentMessageDelta => "agent_message_delta",
            WorkspaceEventType::AgentMessageCommitted => "agent_message_committed",
            WorkspaceEventType::ToolCallRequested => "tool_call_requested",
            WorkspaceEventType::ToolResultReceived => "tool_result_received",
            WorkspaceEventType::AgentRunCompleted => "agent_run_completed",
            WorkspaceEventType::AgentRunFailed => "agent_run_failed",
            WorkspaceEventType::ApprovalRequired => "approval_required",
            WorkspaceEventType::ApprovalDecided => "approval_decided",
            WorkspaceEventType::MiddlewareSnapshot => "middleware_snapshot",
            WorkspaceEventType::MiddlewareInstanceUpserted => "middleware_instance_upserted",
            WorkspaceEventType::MiddlewareInstanceRemoved => "middleware_instance_removed",
            WorkspaceEventType::MiddlewareWorkloadUpserted => "middleware_workload_upserted",
            WorkspaceEventType::MiddlewareWorkloadRemoved => "middleware_workload_removed",
            WorkspaceEventType::MiddlewareMetricChanged => "middleware_metric_changed",
            WorkspaceEventType::MiddlewareEventObserved => "middleware_event_observed",
            WorkspaceEventType::ToolCatalogUpdated => "tool_catalog_updated",
            WorkspaceEventType::PluginCatalogUpdated => "plugin_catalog_updated",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceEventPayload {
    Connected {
        snapshot: Box<WorkspaceSnapshot>,
    },
    Heartbeat,
    Error {
        message: String,
    },
    AgentSessionUpdated {
        session: Box<AgentSession>,
    },
    AgentRunStarted {
        run_id: String,
        session_id: String,
    },
    AgentMessageDelta {
        session_id: String,
        content: String,
    },
    AgentMessageCommitted {
        session_id: String,
        message: AgentMessage,
    },
    ToolCallRequested {
        session_id: String,
        tool_call: AgentToolCall,
    },
    ToolResultReceived {
        session_id: String,
        tool_call_id: String,
        name: String,
        result: ToolResult,
    },
    AgentRunCompleted {
        session_id: String,
        status: AgentRunStatus,
        output: String,
    },
    AgentRunFailed {
        session_id: String,
        error: String,
    },
    ApprovalRequired {
        session_id: String,
        approval: PendingApproval,
    },
    ApprovalDecided {
        approval_record: ApprovalRecord,
        session: Box<AgentSession>,
    },
    MiddlewareSnapshot {
        state: MiddlewareDashboardState,
    },
    MiddlewareInstanceUpserted {
        instance: MiddlewareInstance,
    },
    MiddlewareInstanceRemoved {
        id: String,
    },
    MiddlewareWorkloadUpserted {
        workload: MiddlewareWorkload,
    },
    MiddlewareWorkloadRemoved {
        namespace: String,
        name: String,
    },
    MiddlewareMetricChanged {
        metric: MiddlewareMetric,
    },
    MiddlewareEventObserved {
        event: MiddlewareTimelineEvent,
    },
    ToolCatalogUpdated {
        tools: Vec<ToolDefinition>,
    },
    PluginCatalogUpdated {
        plugins: Vec<PluginResponse>,
    },
}

impl WorkspaceEventPayload {
    pub fn event_type(&self) -> WorkspaceEventType {
        match self {
            WorkspaceEventPayload::Connected { .. } => WorkspaceEventType::Connected,
            WorkspaceEventPayload::Heartbeat => WorkspaceEventType::Heartbeat,
            WorkspaceEventPayload::Error { .. } => WorkspaceEventType::Error,
            WorkspaceEventPayload::AgentSessionUpdated { .. } => {
                WorkspaceEventType::AgentSessionUpdated
            }
            WorkspaceEventPayload::AgentRunStarted { .. } => WorkspaceEventType::AgentRunStarted,
            WorkspaceEventPayload::AgentMessageDelta { .. } => {
                WorkspaceEventType::AgentMessageDelta
            }
            WorkspaceEventPayload::AgentMessageCommitted { .. } => {
                WorkspaceEventType::AgentMessageCommitted
            }
            WorkspaceEventPayload::ToolCallRequested { .. } => {
                WorkspaceEventType::ToolCallRequested
            }
            WorkspaceEventPayload::ToolResultReceived { .. } => {
                WorkspaceEventType::ToolResultReceived
            }
            WorkspaceEventPayload::AgentRunCompleted { .. } => {
                WorkspaceEventType::AgentRunCompleted
            }
            WorkspaceEventPayload::AgentRunFailed { .. } => WorkspaceEventType::AgentRunFailed,
            WorkspaceEventPayload::ApprovalRequired { .. } => WorkspaceEventType::ApprovalRequired,
            WorkspaceEventPayload::ApprovalDecided { .. } => WorkspaceEventType::ApprovalDecided,
            WorkspaceEventPayload::MiddlewareSnapshot { .. } => {
                WorkspaceEventType::MiddlewareSnapshot
            }
            WorkspaceEventPayload::MiddlewareInstanceUpserted { .. } => {
                WorkspaceEventType::MiddlewareInstanceUpserted
            }
            WorkspaceEventPayload::MiddlewareInstanceRemoved { .. } => {
                WorkspaceEventType::MiddlewareInstanceRemoved
            }
            WorkspaceEventPayload::MiddlewareWorkloadUpserted { .. } => {
                WorkspaceEventType::MiddlewareWorkloadUpserted
            }
            WorkspaceEventPayload::MiddlewareWorkloadRemoved { .. } => {
                WorkspaceEventType::MiddlewareWorkloadRemoved
            }
            WorkspaceEventPayload::MiddlewareMetricChanged { .. } => {
                WorkspaceEventType::MiddlewareMetricChanged
            }
            WorkspaceEventPayload::MiddlewareEventObserved { .. } => {
                WorkspaceEventType::MiddlewareEventObserved
            }
            WorkspaceEventPayload::ToolCatalogUpdated { .. } => {
                WorkspaceEventType::ToolCatalogUpdated
            }
            WorkspaceEventPayload::PluginCatalogUpdated { .. } => {
                WorkspaceEventType::PluginCatalogUpdated
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
pub struct WorkspaceEvent {
    #[ts(type = "number")]
    pub event_id: u64,
    pub protocol_version: u8,
    pub occurred_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(rename = "type")]
    pub event_type: WorkspaceEventType,
    pub payload: WorkspaceEventPayload,
}

#[derive(Clone)]
pub struct WorkspaceEventBus {
    inner: Arc<Mutex<EventBusState>>,
    sender: broadcast::Sender<WorkspaceEvent>,
}

#[derive(Debug)]
struct EventBusState {
    next_event_id: u64,
    recent: VecDeque<WorkspaceEvent>,
}

impl Default for WorkspaceEventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceEventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(EVENT_BUFFER_SIZE);

        Self {
            inner: Arc::new(Mutex::new(EventBusState {
                next_event_id: 1,
                recent: VecDeque::with_capacity(EVENT_BUFFER_SIZE),
            })),
            sender,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WorkspaceEvent> {
        self.sender.subscribe()
    }

    pub fn publish(&self, payload: WorkspaceEventPayload) -> WorkspaceEvent {
        let event = self.next_event(None, payload);
        self.remember(event.clone());
        let _ = self.sender.send(event.clone());
        event
    }

    pub fn local_event(&self, payload: WorkspaceEventPayload) -> WorkspaceEvent {
        self.next_event(None, payload)
    }

    pub fn publish_for_workspace(
        &self,
        workspace_id: impl Into<String>,
        payload: WorkspaceEventPayload,
    ) -> WorkspaceEvent {
        let event = self.next_event(Some(workspace_id.into()), payload);
        self.remember(event.clone());
        let _ = self.sender.send(event.clone());
        event
    }

    pub fn local_event_for_workspace(
        &self,
        workspace_id: impl Into<String>,
        payload: WorkspaceEventPayload,
    ) -> WorkspaceEvent {
        self.next_event(Some(workspace_id.into()), payload)
    }

    pub fn replay_after(&self, event_id: u64) -> Option<Vec<WorkspaceEvent>> {
        let inner = self.inner.lock().ok()?;
        if event_id == 0 {
            return Some(inner.recent.iter().cloned().collect());
        }

        let oldest = inner.recent.front().map(|event| event.event_id);
        if matches!(oldest, Some(oldest) if event_id < oldest.saturating_sub(1)) {
            return None;
        }

        Some(
            inner
                .recent
                .iter()
                .filter(|event| event.event_id > event_id)
                .cloned()
                .collect(),
        )
    }

    pub fn replay_after_for_workspace(
        &self,
        event_id: u64,
        workspace_id: &str,
    ) -> Option<Vec<WorkspaceEvent>> {
        self.replay_after(event_id).map(|events| {
            events
                .into_iter()
                .filter(|event| event.workspace_id.as_deref() == Some(workspace_id))
                .collect()
        })
    }

    fn next_event(
        &self,
        workspace_id: Option<String>,
        payload: WorkspaceEventPayload,
    ) -> WorkspaceEvent {
        let mut inner = self.inner.lock().expect("workspace event bus poisoned");
        let event_id = inner.next_event_id;
        inner.next_event_id += 1;

        WorkspaceEvent {
            event_id,
            protocol_version: WORKSPACE_PROTOCOL_VERSION,
            occurred_at: utc_now_rfc3339(),
            workspace_id,
            event_type: payload.event_type(),
            payload,
        }
    }

    fn remember(&self, event: WorkspaceEvent) {
        let mut inner = self.inner.lock().expect("workspace event bus poisoned");
        if inner.recent.len() == EVENT_BUFFER_SIZE {
            inner.recent.pop_front();
        }
        inner.recent.push_back(event);
    }
}

pub fn agent_run_event_payload(session_id: Uuid, event: AgentRunEvent) -> WorkspaceEventPayload {
    let session_id = session_id.to_string();

    match event {
        AgentRunEvent::ModelDelta { content } => WorkspaceEventPayload::AgentMessageDelta {
            session_id,
            content,
        },
        AgentRunEvent::AssistantMessage { message } => {
            WorkspaceEventPayload::AgentMessageCommitted {
                session_id,
                message,
            }
        }
        AgentRunEvent::ToolCallRequested { tool_call } => {
            WorkspaceEventPayload::ToolCallRequested {
                session_id,
                tool_call,
            }
        }
        AgentRunEvent::ToolResult {
            tool_call_id,
            name,
            result,
        } => WorkspaceEventPayload::ToolResultReceived {
            session_id,
            tool_call_id,
            name,
            result,
        },
        AgentRunEvent::ApprovalRequired { approval } => WorkspaceEventPayload::ApprovalRequired {
            session_id,
            approval,
        },
        AgentRunEvent::Completed { status, output } => WorkspaceEventPayload::AgentRunCompleted {
            session_id,
            status,
            output,
        },
        AgentRunEvent::Failed { error } => {
            WorkspaceEventPayload::AgentRunFailed { session_id, error }
        }
    }
}

fn utc_now_rfc3339() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}
