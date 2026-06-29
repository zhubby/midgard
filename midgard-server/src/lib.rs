use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use http::Method;
use midgard_agent::{AgentSession, CompleteTaskTool};
use midgard_controller::{MiddlewareController, MiddlewarePlugin};
use midgard_plugin_example::ExampleRedisPlugin;
use midgard_storage::{MemoryAgentSessionStore, SharedAgentSessionStore};
use midgard_tools::{ToolDefinition, ToolRegistry};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    tools: Arc<ToolRegistry>,
    plugins: Arc<Vec<PluginResponse>>,
    sessions: SharedAgentSessionStore,
}

pub fn app() -> Router {
    app_with_storage(Arc::new(MemoryAgentSessionStore::new()))
}

pub fn app_with_storage(sessions: SharedAgentSessionStore) -> Router {
    let mut registry = ToolRegistry::default();
    registry.register(CompleteTaskTool);

    let plugin = ExampleRedisPlugin;
    let controller = plugin.controller();
    controller.register_tools(&mut registry);

    let state = AppState {
        tools: Arc::new(registry),
        plugins: Arc::new(vec![PluginResponse::from(plugin.metadata())]),
        sessions,
    };

    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/tools", get(list_tools))
        .route("/api/plugins", get(list_plugins))
        .route("/api/agent/sessions", post(create_session))
        .route("/api/agent/sessions/{id}/messages", post(send_message))
        .layer(
            CorsLayer::new()
                .allow_origin([
                    "http://localhost:3000".parse().unwrap(),
                    "http://127.0.0.1:3000".parse().unwrap(),
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
