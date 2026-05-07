use axum::{
    extract::{Path, State},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use midgard_agent::{AgentMessage, AgentSession, CompleteTaskTool};
use midgard_controller::{MiddlewareController, MiddlewarePlugin};
use midgard_plugin_example::ExampleRedisPlugin;
use midgard_tools::{ToolDefinition, ToolRegistry};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    tools: Arc<ToolRegistry>,
    plugins: Arc<Vec<PluginResponse>>,
    sessions: Arc<Mutex<BTreeMap<Uuid, AgentSession>>>,
}

pub fn app() -> Router {
    let mut registry = ToolRegistry::default();
    registry.register(CompleteTaskTool);

    let plugin = ExampleRedisPlugin;
    let controller = plugin.controller();
    controller.register_tools(&mut registry);

    let state = AppState {
        tools: Arc::new(registry),
        plugins: Arc::new(vec![PluginResponse::from(plugin.metadata())]),
        sessions: Arc::new(Mutex::new(BTreeMap::new())),
    };

    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/tools", get(list_tools))
        .route("/api/plugins", get(list_plugins))
        .route("/api/agent/sessions", post(create_session))
        .route("/api/agent/sessions/{id}/messages", post(send_message))
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
) -> impl IntoResponse {
    let session = AgentSession::new(request.goal);
    state
        .sessions
        .lock()
        .expect("session store poisoned")
        .insert(session.id, session.clone());

    Json(session)
}

async fn send_message(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<SendMessageRequest>,
) -> impl IntoResponse {
    let mut sessions = state.sessions.lock().expect("session store poisoned");
    let session = sessions
        .entry(id)
        .or_insert_with(|| AgentSession::new("resumed session"));

    session.messages.push(AgentMessage::user(request.message));

    Json(session.clone())
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

#[allow(dead_code)]
fn _tool_definitions_for_docs(registry: &ToolRegistry) -> Vec<ToolDefinition> {
    registry.definitions()
}

