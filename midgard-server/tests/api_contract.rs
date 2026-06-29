use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
};
use midgard_server::{app, app_with_storage};
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
