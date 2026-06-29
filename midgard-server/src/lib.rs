use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use http::{HeaderValue, Method};
use midgard_agent::{
    AgentRunEvent, AgentRunStatus, AgentRunner, AgentSession, ApprovalDecision, ApprovalRecord,
    CompleteTaskTool, LlmProvider, LlmResponse, PendingApproval, ScriptedLlmProvider,
};
use midgard_controller::{MiddlewareController, MiddlewarePlugin};
use midgard_plugin_example::ExampleRedisPlugin;
use midgard_storage::{MemoryAgentSessionStore, SharedAgentSessionStore};
use midgard_tools::{ToolDefinition, ToolRegistry};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio_stream::Stream;
use tower_http::cors::CorsLayer;
use ts_rs::TS;
use uuid::Uuid;

mod workspace;

pub use workspace::{
    agent_run_event_payload, DashboardTone, MiddlewareDashboardState, MiddlewareMetric,
    MiddlewareTimelineEvent, MiddlewareWorkload, WorkspaceEvent, WorkspaceEventBus,
    WorkspaceEventPayload, WorkspaceEventType, WorkspaceSnapshot, WORKSPACE_PROTOCOL_VERSION,
};

#[derive(Clone)]
pub struct AppState {
    tools: Arc<ToolRegistry>,
    runner: Arc<AgentRunner>,
    plugins: Arc<Vec<PluginResponse>>,
    sessions: SharedAgentSessionStore,
    events: WorkspaceEventBus,
    middleware: Arc<MiddlewareDashboardState>,
}

pub fn app() -> Router {
    app_with_storage(Arc::new(MemoryAgentSessionStore::new()))
}

pub fn app_with_storage(sessions: SharedAgentSessionStore) -> Router {
    app_with_provider(
        sessions,
        Arc::new(ScriptedLlmProvider::single(LlmResponse::text(
            "LLM provider is not configured for this server instance.",
        ))),
    )
}

pub fn app_with_provider(
    sessions: SharedAgentSessionStore,
    provider: Arc<dyn LlmProvider>,
) -> Router {
    let mut registry = ToolRegistry::default();
    registry.register(CompleteTaskTool);

    let plugin = ExampleRedisPlugin;
    let controller = plugin.controller();
    controller.register_tools(&mut registry);
    let tools = Arc::new(registry);
    let runner = Arc::new(AgentRunner::new(provider, tools.clone()));

    let state = AppState {
        tools,
        runner,
        plugins: Arc::new(vec![PluginResponse::from(plugin.metadata())]),
        sessions,
        events: WorkspaceEventBus::new(),
        middleware: Arc::new(MiddlewareDashboardState::mock()),
    };

    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/workspace/events", get(workspace_events))
        .route("/api/tools", get(list_tools))
        .route("/api/plugins", get(list_plugins))
        .route("/api/agent/sessions", post(create_session))
        .route("/api/agent/sessions/{id}/messages", post(send_message))
        .route("/api/agent/sessions/{id}/runs", post(run_agent))
        .route("/api/agent/sessions/{id}/runs/stream", post(stream_agent))
        .route(
            "/api/agent/sessions/{id}/approvals",
            get(list_approval_records).post(record_approval),
        )
        .layer(
            CorsLayer::new()
                .allow_origin([
                    HeaderValue::from_static("http://localhost:3000"),
                    HeaderValue::from_static("http://127.0.0.1:3000"),
                ])
                .allow_methods([Method::GET, Method::POST, Method::OPTIONS])
                .allow_headers(tower_http::cors::Any),
        )
        .with_state(state)
}

async fn healthz() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}

async fn list_tools(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.tools.definitions())
}

async fn list_plugins(State(state): State<AppState>) -> impl IntoResponse {
    Json((*state.plugins).clone())
}

async fn create_session(
    State(state): State<AppState>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<AgentSession>, AppError> {
    let session = state.sessions.create_session(request.goal).await?;
    state
        .events
        .publish(WorkspaceEventPayload::AgentSessionUpdated {
            session: session.clone(),
        });

    Ok(Json(session))
}

async fn send_message(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Json<AgentSession>, AppError> {
    let session = state
        .sessions
        .append_user_message(id, request.message)
        .await?;
    if let Some(message) = session.messages.last().cloned() {
        state
            .events
            .publish(WorkspaceEventPayload::AgentMessageCommitted {
                session_id: id.to_string(),
                message,
            });
    }
    state
        .events
        .publish(WorkspaceEventPayload::AgentSessionUpdated {
            session: session.clone(),
        });

    Ok(Json(session))
}

async fn run_agent(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<RunAccepted>), AppError> {
    let run_id = Uuid::new_v4();
    spawn_agent_run(state, id, run_id);

    Ok((
        StatusCode::ACCEPTED,
        Json(RunAccepted {
            run_id,
            session_id: id,
            status: AgentRunStatus::Running,
        }),
    ))
}

async fn stream_agent(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let session = load_or_resumed_session(&state, id).await?;
    let result = state.runner.run(session).await?;
    state.sessions.save_session(result.session).await?;
    let events = result.events;

    Ok(Sse::new(tokio_stream::iter(
        events.into_iter().map(|event| Ok(agent_sse_event(event))),
    )))
}

async fn record_approval(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<ApprovalRequest>,
) -> Result<Json<ApprovalResponse>, AppError> {
    let mut session = load_or_resumed_session(&state, id).await?;
    let actor = validated_actor(request.actor)?;
    let approval = session.record_approval_decision(request.decision.clone())?;
    let approval_record = state
        .sessions
        .record_approval_decision(id, approval, request.decision, actor, request.reason)
        .await?;
    let session = state.sessions.save_session(session).await?;
    state
        .events
        .publish(WorkspaceEventPayload::ApprovalDecided {
            approval_record: approval_record.clone(),
            session: session.clone(),
        });
    state
        .events
        .publish(WorkspaceEventPayload::AgentSessionUpdated {
            session: session.clone(),
        });
    if request.resume {
        spawn_agent_run(state.clone(), id, Uuid::new_v4());
    }

    Ok(Json(ApprovalResponse {
        approval_record,
        session,
    }))
}

async fn list_approval_records(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ApprovalRecord>>, AppError> {
    Ok(Json(state.sessions.list_approval_records(id).await?))
}

fn validated_actor(actor: String) -> Result<String, AppError> {
    let actor = actor.trim().to_string();
    if actor.is_empty() {
        return Err(AppError(midgard_core::MidgardError::Configuration(
            "approval actor is required".to_string(),
        )));
    }

    Ok(actor)
}

async fn load_or_resumed_session(state: &AppState, id: Uuid) -> Result<AgentSession, AppError> {
    Ok(match state.sessions.load_session(id).await? {
        Some(session) => session,
        None => {
            let mut session = AgentSession::new("resumed session");
            session.id = id;
            session
        }
    })
}

async fn workspace_events(
    Query(query): Query<WorkspaceEventsQuery>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    let snapshot = workspace_snapshot(&state, query.session_id).await?;
    let mut initial_events = vec![state
        .events
        .local_event(WorkspaceEventPayload::Connected { snapshot })];

    match state.events.replay_after(last_event_id) {
        Some(replay) => initial_events.extend(replay),
        None => initial_events.push(state.events.local_event(WorkspaceEventPayload::Error {
            message: "event buffer expired; snapshot was refreshed".to_string(),
        })),
    }

    initial_events.push(
        state
            .events
            .local_event(WorkspaceEventPayload::MiddlewareSnapshot {
                state: (*state.middleware).clone(),
            }),
    );

    let mut receiver = state.events.subscribe();
    let bus = state.events.clone();
    let once = query.once;
    let stream = async_stream::stream! {
        for event in initial_events {
            yield Ok(workspace_sse_event(event));
        }

        if once {
            return;
        }

        let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
        loop {
            tokio::select! {
                received = receiver.recv() => {
                    match received {
                        Ok(event) => yield Ok(workspace_sse_event(event)),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            yield Ok(workspace_sse_event(bus.local_event(WorkspaceEventPayload::Error {
                                message: "event stream lagged; refresh the workspace snapshot".to_string(),
                            })));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = heartbeat.tick() => {
                    yield Ok(workspace_sse_event(bus.local_event(WorkspaceEventPayload::Heartbeat)));
                }
            }
        }
    };

    Ok(Sse::new(stream))
}

async fn workspace_snapshot(
    state: &AppState,
    session_id: Option<Uuid>,
) -> Result<WorkspaceSnapshot, AppError> {
    let session = match session_id {
        Some(id) => state.sessions.load_session(id).await?,
        None => None,
    };
    let approvals = match session_id {
        Some(id) => state.sessions.list_approval_records(id).await?,
        None => Vec::new(),
    };

    Ok(WorkspaceSnapshot {
        session,
        tools: state.tools.definitions(),
        plugins: (*state.plugins).clone(),
        middleware: (*state.middleware).clone(),
        approvals,
    })
}

fn spawn_agent_run(state: AppState, session_id: Uuid, run_id: Uuid) {
    tokio::spawn(async move {
        state
            .events
            .publish(WorkspaceEventPayload::AgentRunStarted {
                run_id: run_id.to_string(),
                session_id: session_id.to_string(),
            });

        let session = match load_or_resumed_session(&state, session_id).await {
            Ok(session) => session,
            Err(err) => {
                state.events.publish(WorkspaceEventPayload::AgentRunFailed {
                    session_id: session_id.to_string(),
                    error: err.0.to_string(),
                });
                return;
            }
        };

        let bus = state.events.clone();
        let result = state
            .runner
            .run_with_observer(session, move |event| {
                bus.publish(agent_run_event_payload(session_id, event));
            })
            .await;

        match result {
            Ok(result) => match state.sessions.save_session(result.session.clone()).await {
                Ok(session) => {
                    state
                        .events
                        .publish(WorkspaceEventPayload::AgentSessionUpdated { session });
                }
                Err(err) => {
                    state.events.publish(WorkspaceEventPayload::AgentRunFailed {
                        session_id: session_id.to_string(),
                        error: err.to_string(),
                    });
                }
            },
            Err(err) => {
                state.events.publish(WorkspaceEventPayload::AgentRunFailed {
                    session_id: session_id.to_string(),
                    error: err.to_string(),
                });
            }
        }
    });
}

fn workspace_sse_event(event: WorkspaceEvent) -> Event {
    let name = event.event_type.as_str();
    match Event::default()
        .id(event.event_id.to_string())
        .event(name)
        .json_data(event)
    {
        Ok(event) => event,
        Err(err) => Event::default()
            .event("error")
            .data(format!("failed to serialize workspace event: {err}")),
    }
}

fn agent_sse_event(event: AgentRunEvent) -> Event {
    let name = match &event {
        AgentRunEvent::ModelDelta { .. } => "model_delta",
        AgentRunEvent::AssistantMessage { .. } => "assistant_message",
        AgentRunEvent::ToolCallRequested { .. } => "tool_call_requested",
        AgentRunEvent::ToolResult { .. } => "tool_result",
        AgentRunEvent::ApprovalRequired { .. } => "approval_required",
        AgentRunEvent::Completed { .. } => "completed",
        AgentRunEvent::Failed { .. } => "failed",
    };

    match Event::default().event(name).json_data(event) {
        Ok(event) => event,
        Err(err) => Event::default()
            .event("failed")
            .data(format!("failed to serialize run event: {err}")),
    }
}

#[derive(Clone, Debug, Serialize, TS)]
struct HealthResponse {
    status: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
pub struct PluginResponse {
    pub id: String,
    pub name: String,
    pub middleware_kind: String,
}

impl From<midgard_controller::PluginMetadata> for PluginResponse {
    fn from(metadata: midgard_controller::PluginMetadata) -> Self {
        Self {
            id: metadata.id,
            name: metadata.name,
            middleware_kind: metadata.middleware_kind,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CreateSessionRequest {
    goal: String,
}

#[derive(Debug, Deserialize)]
struct SendMessageRequest {
    message: String,
}

#[derive(Debug, Deserialize)]
struct ApprovalRequest {
    decision: ApprovalDecision,
    actor: String,
    reason: Option<String>,
    #[serde(default = "default_resume_approved_run")]
    resume: bool,
}

fn default_resume_approved_run() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct AgentRunResponse {
    pub status: AgentRunStatus,
    pub pending_approval: Option<PendingApproval>,
    pub events: Vec<AgentRunEvent>,
    pub session: AgentSession,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct ApprovalResponse {
    pub approval_record: ApprovalRecord,
    pub session: AgentSession,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct RunAccepted {
    #[ts(type = "string")]
    pub run_id: Uuid,
    #[ts(type = "string")]
    pub session_id: Uuid,
    pub status: AgentRunStatus,
}

#[derive(Debug, Deserialize)]
struct WorkspaceEventsQuery {
    session_id: Option<Uuid>,
    #[serde(default)]
    once: bool,
}

#[derive(Clone, Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug)]
struct AppError(midgard_core::MidgardError);

impl From<midgard_core::MidgardError> for AppError {
    fn from(value: midgard_core::MidgardError) -> Self {
        Self(value)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let status = match self.0 {
            midgard_core::MidgardError::Configuration(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };

        (
            status,
            Json(ErrorResponse {
                error: self.0.to_string(),
            }),
        )
            .into_response()
    }
}

#[allow(dead_code)]
fn _tool_definitions_for_docs(registry: &ToolRegistry) -> Vec<ToolDefinition> {
    registry.definitions()
}
