use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use http::{HeaderValue, Method};
use midgard_agent::{
    AgentRunEvent, AgentRunStatus, AgentRunner, AgentSession, ApprovalDecision, CompleteTaskTool,
    LlmProvider, LlmResponse, PendingApproval, ScriptedLlmProvider,
};
use midgard_controller::{MiddlewareController, MiddlewarePlugin};
use midgard_plugin_example::ExampleRedisPlugin;
use midgard_storage::{MemoryAgentSessionStore, SharedAgentSessionStore};
use midgard_tools::{ToolDefinition, ToolRegistry};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, sync::Arc};
use tokio_stream::Stream;
use tower_http::cors::CorsLayer;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    tools: Arc<ToolRegistry>,
    runner: Arc<AgentRunner>,
    plugins: Arc<Vec<PluginResponse>>,
    sessions: SharedAgentSessionStore,
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
    };

    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/tools", get(list_tools))
        .route("/api/plugins", get(list_plugins))
        .route("/api/agent/sessions", post(create_session))
        .route("/api/agent/sessions/{id}/messages", post(send_message))
        .route("/api/agent/sessions/{id}/runs", post(run_agent))
        .route("/api/agent/sessions/{id}/runs/stream", post(stream_agent))
        .route("/api/agent/sessions/{id}/approvals", post(record_approval))
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

    Ok(Json(session))
}

async fn run_agent(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<Json<AgentRunResponse>, AppError> {
    let session = load_or_resumed_session(&state, id).await?;
    let result = state.runner.run(session).await?;
    let session = state.sessions.save_session(result.session).await?;

    Ok(Json(AgentRunResponse {
        status: session.status.clone(),
        pending_approval: session.pending_approval.clone(),
        events: result.events,
        session,
    }))
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
        events.into_iter().map(|event| Ok(sse_event(event))),
    )))
}

async fn record_approval(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<ApprovalRequest>,
) -> Result<Json<ApprovalResponse>, AppError> {
    let mut session = load_or_resumed_session(&state, id).await?;
    let approval = session.record_approval_decision(request.decision)?;
    let session = state.sessions.save_session(session).await?;

    Ok(Json(ApprovalResponse { approval, session }))
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

fn sse_event(event: AgentRunEvent) -> Event {
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

#[derive(Clone, Debug, Serialize)]
struct HealthResponse {
    status: String,
}

#[derive(Clone, Debug, Serialize)]
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
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentRunResponse {
    pub status: AgentRunStatus,
    pub pending_approval: Option<PendingApproval>,
    pub events: Vec<AgentRunEvent>,
    pub session: AgentSession,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApprovalResponse {
    pub approval: PendingApproval,
    pub session: AgentSession,
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
