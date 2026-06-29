use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use midgard_agent::{AgentToolCall, LlmResponse, ScriptedLlmProvider};
use midgard_server::{app, app_with_provider, app_with_storage};
use midgard_storage::MemoryAgentSessionStore;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn health_endpoint_returns_ok() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn tools_endpoint_lists_registered_tools() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/tools")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert!(json
        .as_array()
        .unwrap()
        .iter()
        .any(|tool| tool["name"] == "redis_describe"));
}

#[tokio::test]
async fn plugins_endpoint_lists_example_plugin() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/plugins")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json[0]["id"], "example-redis");
}

#[tokio::test]
async fn agent_sessions_endpoint_creates_session() {
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"goal":"inspect redis"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert!(json["id"].as_str().is_some());
    assert_eq!(json["messages"][0]["content"], "inspect redis");
}

#[tokio::test]
async fn app_accepts_injected_session_store() {
    let response = app_with_storage(Arc::new(MemoryAgentSessionStore::new()))
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"goal":"inspect redis"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["messages"][0]["content"], "inspect redis");
}

#[tokio::test]
async fn missing_session_message_preserves_resumed_session_behavior() {
    let id = uuid::Uuid::new_v4();
    let response = app()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/sessions/{id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"continue"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["messages"][0]["content"], "resumed session");
    assert_eq!(json["messages"][1]["content"], "continue");
}

#[tokio::test]
async fn run_endpoint_executes_agent_loop_and_persists_trace() {
    let app = app_with_provider(
        Arc::new(MemoryAgentSessionStore::new()),
        Arc::new(ScriptedLlmProvider::single(LlmResponse::with_tool_calls(
            "",
            vec![AgentToolCall::from_raw(
                "call_1",
                "complete_task",
                r#"{"summary":"Redis is healthy"}"#,
            )],
        ))),
    );
    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/agent/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"goal":"inspect redis"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(create_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: Value = serde_json::from_slice(&body).unwrap();
    let id = created["id"].as_str().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/sessions/{id}/runs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "completed");
    assert_eq!(
        json["session"]["messages"][1]["tool_calls"][0]["name"],
        "complete_task"
    );
    assert!(json["events"].as_array().unwrap().iter().any(|event| {
        event["type"] == "tool_result"
            && event["result"]["output"]
                .as_str()
                .unwrap()
                .contains("Redis is healthy")
    }));
}

#[tokio::test]
async fn stream_endpoint_emits_ordered_sse_run_events() {
    let app = app_with_provider(
        Arc::new(MemoryAgentSessionStore::new()),
        Arc::new(ScriptedLlmProvider::single(LlmResponse::text(
            "Redis is ready",
        ))),
    );
    let id = uuid::Uuid::new_v4();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/sessions/{id}/runs/stream"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    assert!(text.contains("event: assistant_message"));
    assert!(text.contains("event: completed"));
    assert!(text.contains("Redis is ready"));
}

#[tokio::test]
async fn approval_endpoint_records_decision_and_next_run_resumes() {
    let app = app_with_provider(
        Arc::new(MemoryAgentSessionStore::new()),
        Arc::new(ScriptedLlmProvider::new([
            LlmResponse::with_tool_calls(
                "",
                vec![AgentToolCall::from_raw(
                    "call_1",
                    "redis_restart",
                    r#"{"namespace":"default","name":"cache"}"#,
                )],
            ),
            LlmResponse::with_tool_calls(
                "",
                vec![AgentToolCall::from_raw(
                    "call_2",
                    "complete_task",
                    r#"{"summary":"Restart requested"}"#,
                )],
            ),
        ])),
    );
    let id = uuid::Uuid::new_v4();
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/sessions/{id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"restart redis"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::OK);

    let first = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/sessions/{id}/runs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(first.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "awaiting_approval");

    let approval = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/sessions/{id}/approvals"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"decision":"approve"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let approval_status = approval.status();
    let approval_body = to_bytes(approval.into_body(), usize::MAX).await.unwrap();
    assert_eq!(
        approval_status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&approval_body)
    );

    let second = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/agent/sessions/{id}/runs"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(second.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "completed");
    assert!(json["session"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .any(|message| message["content"]
            .as_str()
            .unwrap_or_default()
            .contains("Restart requested")));
}
