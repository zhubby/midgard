use axum::{
    body::{to_bytes, Body},
    http::{
        header::{COOKIE, SET_COOKIE},
        Request, StatusCode,
    },
    Router,
};
use midgard_agent::{AgentToolCall, LlmProvider, LlmResponse, ScriptedLlmProvider};
use midgard_server::{app, app_with_provider_auth_and_orgs, AuthSettings};
use midgard_storage::{
    hash_password, AuthStore, MemoryAgentSessionStore, MemoryAuthStore, MemoryOrganizationStore,
    NewOrganization, NewOrganizationMembership, NewUser, NewWorkspace, OrganizationRole,
    OrganizationStore, UserRole,
};
use serde_json::Value;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tower::ServiceExt;

const TEST_EMAIL: &str = "operator@example.com";
const TEST_PASSWORD: &str = "valid-password";
const TEST_ORG: &str = "test-ops";
const TEST_WORKSPACE: &str = "operations";

async fn app_with_role(role: UserRole) -> (Router, String, Arc<MemoryAuthStore>) {
    app_with_role_and_provider(
        role,
        Arc::new(ScriptedLlmProvider::single(LlmResponse::text(
            "LLM provider is not configured for this server instance.",
        ))),
    )
    .await
}

async fn app_with_role_and_provider(
    role: UserRole,
    provider: Arc<dyn LlmProvider>,
) -> (Router, String, Arc<MemoryAuthStore>) {
    let auth = Arc::new(MemoryAuthStore::new());
    let user = auth
        .create_user(NewUser {
            email: TEST_EMAIL.to_string(),
            display_name: "Test Operator".to_string(),
            role,
            system_role_id: None,
            password_hash: hash_password(TEST_PASSWORD).unwrap(),
            active: true,
        })
        .await
        .unwrap();
    let orgs = Arc::new(MemoryOrganizationStore::new());
    let organization = orgs
        .create_organization(NewOrganization {
            slug: TEST_ORG.to_string(),
            name: "Test Ops".to_string(),
            created_by_user_id: user.id,
        })
        .await
        .unwrap();
    orgs.create_membership(NewOrganizationMembership {
        organization_id: organization.id,
        user_id: user.id,
        role: organization_role_for_user_role(&user.role),
        role_id: None,
        active: true,
    })
    .await
    .unwrap();
    orgs.create_workspace(NewWorkspace {
        organization_id: organization.id,
        slug: TEST_WORKSPACE.to_string(),
        name: "Operations".to_string(),
    })
    .await
    .unwrap();

    let app = app_with_provider_auth_and_orgs(
        Arc::new(MemoryAgentSessionStore::new()),
        auth.clone(),
        orgs,
        provider,
        AuthSettings::default(),
    );
    let cookie = login_cookie(&app, TEST_EMAIL, TEST_PASSWORD).await;

    (app, cookie, auth)
}

fn organization_role_for_user_role(role: &UserRole) -> OrganizationRole {
    match role {
        UserRole::Admin => OrganizationRole::Owner,
        UserRole::Operator => OrganizationRole::Operator,
        UserRole::Viewer => OrganizationRole::Viewer,
    }
}

fn workspace_uri(suffix: &str) -> String {
    format!("/api/orgs/{TEST_ORG}/workspaces/{TEST_WORKSPACE}{suffix}")
}

async fn login_cookie(app: &Router, email: &str, password: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"email":"{email}","password":"{password}"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response
        .headers()
        .get(SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

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
async fn protected_api_requires_authentication() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/api/orgs")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn login_sets_session_cookie_and_me_returns_user() {
    let (app, cookie, _) = app_with_role(UserRole::Operator).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/auth/me")
                .header(COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["user"]["email"], TEST_EMAIL);
    assert_eq!(json["user"]["role"], "operator");
    assert_eq!(json["system_role"]["builtin_key"], "system_admin");
    assert!(json["system_permissions"].as_array().unwrap().len() > 1);
    assert!(json["user"].get("password_hash").is_none());
}

#[tokio::test]
async fn login_rejects_bad_password_without_cookie() {
    let auth = Arc::new(MemoryAuthStore::new());
    auth.create_user(NewUser {
        email: TEST_EMAIL.to_string(),
        display_name: "Test Operator".to_string(),
        role: UserRole::Operator,
        system_role_id: None,
        password_hash: hash_password(TEST_PASSWORD).unwrap(),
        active: true,
    })
    .await
    .unwrap();
    let app = app_with_provider_auth_and_orgs(
        Arc::new(MemoryAgentSessionStore::new()),
        auth,
        Arc::new(MemoryOrganizationStore::new()),
        Arc::new(ScriptedLlmProvider::single(LlmResponse::text("ok"))),
        AuthSettings::default(),
    );

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"email":"operator@example.com","password":"wrong-password"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert!(response.headers().get(SET_COOKIE).is_none());
}

#[tokio::test]
async fn logout_revokes_current_session() {
    let (app, cookie, _) = app_with_role(UserRole::Operator).await;

    let logout = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header(COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(logout.status(), StatusCode::OK);

    let me = app
        .oneshot(
            Request::builder()
                .uri("/api/auth/me")
                .header(COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(me.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn viewer_can_read_but_cannot_mutate_agent_sessions() {
    let (app, cookie, _) = app_with_role(UserRole::Viewer).await;

    let read = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(workspace_uri("/tools"))
                .header(COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::OK);

    let write = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri("/agent/sessions"))
                .header(COOKIE, cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"goal":"inspect redis"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(write.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn user_without_organizations_can_create_first_organization() {
    let auth = Arc::new(MemoryAuthStore::new());
    auth.create_user(NewUser {
        email: TEST_EMAIL.to_string(),
        display_name: "Test Operator".to_string(),
        role: UserRole::Operator,
        system_role_id: None,
        password_hash: hash_password(TEST_PASSWORD).unwrap(),
        active: true,
    })
    .await
    .unwrap();
    let app = app_with_provider_auth_and_orgs(
        Arc::new(MemoryAgentSessionStore::new()),
        auth,
        Arc::new(MemoryOrganizationStore::new()),
        Arc::new(ScriptedLlmProvider::single(LlmResponse::text("ok"))),
        AuthSettings::default(),
    );
    let cookie = login_cookie(&app, TEST_EMAIL, TEST_PASSWORD).await;

    let list = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/orgs")
                .header(COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let body = to_bytes(list.into_body(), usize::MAX).await.unwrap();
    let contexts: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(contexts.as_array().unwrap().len(), 0);

    let create = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/orgs")
                .header(COOKIE, cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"Platform Ops"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let body = to_bytes(create.into_body(), usize::MAX).await.unwrap();
    let context: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(context["organization"]["slug"], "platform-ops");
    assert_eq!(context["membership"]["role"], "owner");
    assert_eq!(context["workspaces"][0]["slug"], "operations");
}

#[tokio::test]
async fn non_member_cannot_access_workspace() {
    let (app, _cookie, auth) = app_with_role(UserRole::Operator).await;
    auth.create_user(NewUser {
        email: "outsider@example.com".to_string(),
        display_name: "Outsider".to_string(),
        role: UserRole::Operator,
        system_role_id: None,
        password_hash: hash_password(TEST_PASSWORD).unwrap(),
        active: true,
    })
    .await
    .unwrap();
    let outsider_cookie = login_cookie(&app, "outsider@example.com", TEST_PASSWORD).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri(workspace_uri("/events?once=true"))
                .header(COOKIE, outsider_cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn agent_sessions_are_isolated_by_workspace() {
    let (app, cookie, _) = app_with_role(UserRole::Admin).await;
    let workspace = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/orgs/{TEST_ORG}/workspaces"))
                .header(COOKIE, cookie.clone())
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"Secondary"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(workspace.status(), StatusCode::CREATED);

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri("/agent/sessions"))
                .header(COOKIE, cookie.clone())
                .header("content-type", "application/json")
                .body(Body::from(r#"{"goal":"inspect redis"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(create.into_body(), usize::MAX).await.unwrap();
    let session: Value = serde_json::from_slice(&body).unwrap();
    let id = session["id"].as_str().unwrap();

    let cross_workspace = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/orgs/{TEST_ORG}/workspaces/secondary/agent/sessions/{id}/runs"
                ))
                .header(COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(cross_workspace.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_can_create_and_list_users() {
    let (app, cookie, _) = app_with_role(UserRole::Admin).await;

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/users")
                .header(COOKIE, cookie.clone())
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"email":"viewer@example.com","password":"valid-password","display_name":"Viewer","role":"viewer"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let body = to_bytes(create.into_body(), usize::MAX).await.unwrap();
    let created: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(created["email"], "viewer@example.com");
    assert!(created.get("password_hash").is_none());

    let list = app
        .oneshot(
            Request::builder()
                .uri("/api/auth/users")
                .header(COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let body = to_bytes(list.into_body(), usize::MAX).await.unwrap();
    let users: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(users.as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn system_admin_can_read_roles_but_cannot_manage_roles() {
    let (app, cookie, _) = app_with_role(UserRole::Operator).await;

    let read = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/rbac/system/roles")
                .header(COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read.status(), StatusCode::OK);

    let write = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/rbac/system/roles")
                .header(COOKIE, cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"slug":"auditor","name":"Auditor","permissions":["system.orgs.read"]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(write.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn system_owner_can_create_system_role_and_replace_permissions() {
    let (app, cookie, _) = app_with_role(UserRole::Admin).await;

    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/rbac/system/roles")
                .header(COOKIE, cookie.clone())
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"slug":"auditor","name":"Auditor","permissions":["system.orgs.read"]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let body = to_bytes(create.into_body(), usize::MAX).await.unwrap();
    let created: Value = serde_json::from_slice(&body).unwrap();
    let id = created["id"].as_str().unwrap();

    let update = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/rbac/system/roles/{id}/permissions"))
                .header(COOKIE, cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"permissions":["system.users.read","system.orgs.read"]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(update.status(), StatusCode::OK);
    let body = to_bytes(update.into_body(), usize::MAX).await.unwrap();
    let updated: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(updated["permissions"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn system_owner_role_must_retain_all_permissions() {
    let (app, cookie, _) = app_with_role(UserRole::Admin).await;

    let roles = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/rbac/system/roles")
                .header(COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(roles.into_body(), usize::MAX).await.unwrap();
    let roles: Value = serde_json::from_slice(&body).unwrap();
    let owner_id = roles
        .as_array()
        .unwrap()
        .iter()
        .find(|role| role["slug"] == "owner")
        .and_then(|role| role["id"].as_str())
        .unwrap();

    let update = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/rbac/system/roles/{owner_id}/permissions"))
                .header(COOKIE, cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"permissions":["system.orgs.read"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(update.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn organization_owner_role_must_retain_all_permissions() {
    let (app, cookie, _) = app_with_role(UserRole::Admin).await;

    let roles = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/orgs/{TEST_ORG}/roles"))
                .header(COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(roles.into_body(), usize::MAX).await.unwrap();
    let roles: Value = serde_json::from_slice(&body).unwrap();
    let owner_id = roles
        .as_array()
        .unwrap()
        .iter()
        .find(|role| role["slug"] == "owner")
        .and_then(|role| role["id"].as_str())
        .unwrap();

    let update = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/orgs/{TEST_ORG}/roles/{owner_id}/permissions"))
                .header(COOKIE, cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"permissions":["workspace.read"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(update.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn tools_endpoint_lists_registered_tools() {
    let (app, cookie, _) = app_with_role(UserRole::Viewer).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri(workspace_uri("/tools"))
                .header(COOKIE, cookie)
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
    let (app, cookie, _) = app_with_role(UserRole::Viewer).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri(workspace_uri("/plugins"))
                .header(COOKIE, cookie)
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
    let (app, cookie, _) = app_with_role(UserRole::Operator).await;
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri("/agent/sessions"))
                .header(COOKIE, cookie)
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
    let auth = Arc::new(MemoryAuthStore::new());
    let user = auth
        .create_user(NewUser {
            email: TEST_EMAIL.to_string(),
            display_name: "Test Operator".to_string(),
            role: UserRole::Operator,
            system_role_id: None,
            password_hash: hash_password(TEST_PASSWORD).unwrap(),
            active: true,
        })
        .await
        .unwrap();
    let orgs = Arc::new(MemoryOrganizationStore::new());
    let organization = orgs
        .create_organization(NewOrganization {
            slug: TEST_ORG.to_string(),
            name: "Test Ops".to_string(),
            created_by_user_id: user.id,
        })
        .await
        .unwrap();
    orgs.create_membership(NewOrganizationMembership {
        organization_id: organization.id,
        user_id: user.id,
        role: OrganizationRole::Operator,
        role_id: None,
        active: true,
    })
    .await
    .unwrap();
    orgs.create_workspace(NewWorkspace {
        organization_id: organization.id,
        slug: TEST_WORKSPACE.to_string(),
        name: "Operations".to_string(),
    })
    .await
    .unwrap();
    let app = app_with_provider_auth_and_orgs(
        Arc::new(MemoryAgentSessionStore::new()),
        auth,
        orgs,
        Arc::new(ScriptedLlmProvider::single(LlmResponse::text("ok"))),
        AuthSettings::default(),
    );
    let cookie = login_cookie(&app, TEST_EMAIL, TEST_PASSWORD).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri("/agent/sessions"))
                .header(COOKIE, cookie)
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
    let (app, cookie, _) = app_with_role(UserRole::Operator).await;
    let id = uuid::Uuid::new_v4();
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri(&format!("/agent/sessions/{id}/messages")))
                .header(COOKIE, cookie)
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
    let (app, cookie, _) = app_with_role_and_provider(
        UserRole::Operator,
        Arc::new(ScriptedLlmProvider::single(LlmResponse::with_tool_calls(
            "",
            vec![AgentToolCall::from_raw(
                "call_1",
                "complete_task",
                r#"{"summary":"Redis is healthy"}"#,
            )],
        ))),
    )
    .await;
    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri("/agent/sessions"))
                .header(COOKIE, cookie.clone())
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
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri(&format!("/agent/sessions/{id}/runs")))
                .header(COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "running");
    assert_eq!(json["session_id"], id);
    assert!(json["run_id"].as_str().is_some());

    sleep(Duration::from_millis(20)).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri(workspace_uri(&format!("/events?session_id={id}&once=true")))
                .header(COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    assert!(text.contains("event: connected"));
    assert!(text.contains("event: agent_run_started"));
    assert!(text.contains("event: tool_result_received"));
    assert!(text.contains("Redis is healthy"));
}

#[tokio::test]
async fn stream_endpoint_emits_ordered_sse_run_events() {
    let (app, cookie, _) = app_with_role_and_provider(
        UserRole::Operator,
        Arc::new(ScriptedLlmProvider::single(LlmResponse::text(
            "Redis is ready",
        ))),
    )
    .await;
    let id = uuid::Uuid::new_v4();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri(&format!("/agent/sessions/{id}/runs/stream")))
                .header(COOKIE, cookie)
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
async fn workspace_events_endpoint_emits_connected_snapshot() {
    let (app, cookie, _) = app_with_role(UserRole::Viewer).await;

    let response = app
        .oneshot(
            Request::builder()
                .uri(workspace_uri("/events?once=true"))
                .header(COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    assert!(text.contains("id:"));
    assert!(text.contains("event: connected"));
    assert!(text.contains("\"protocol_version\":1"));
    assert!(text.contains("\"current_permissions\""));
    assert!(text.contains("event: middleware_snapshot"));
}

#[tokio::test]
async fn approval_endpoint_records_decision_and_next_run_resumes() {
    let (app, cookie, _) = app_with_role_and_provider(
        UserRole::Operator,
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
    )
    .await;
    let id = uuid::Uuid::new_v4();
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri(&format!("/agent/sessions/{id}/messages")))
                .header(COOKIE, cookie.clone())
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
                .uri(workspace_uri(&format!("/agent/sessions/{id}/runs")))
                .header(COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    let body = to_bytes(first.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "running");

    sleep(Duration::from_millis(20)).await;

    let history = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(workspace_uri(&format!("/agent/sessions/{id}/approvals")))
                .header(COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(history.into_body(), usize::MAX).await.unwrap();
    let history: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(history.as_array().unwrap().len(), 1);
    assert_eq!(history[0]["status"], "pending");
    assert_eq!(history[0]["tool_call"]["name"], "redis_restart");

    let approval = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri(&format!("/agent/sessions/{id}/approvals")))
                .header(COOKIE, cookie.clone())
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"decision":"approve","reason":"maintenance window"}"#,
                ))
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
    let approval_json: Value = serde_json::from_slice(&approval_body).unwrap();
    assert_eq!(approval_json["approval_record"]["status"], "approved");
    assert_eq!(
        approval_json["approval_record"]["actor"],
        "operator@example.com"
    );
    assert_eq!(
        approval_json["approval_record"]["reason"],
        "maintenance window"
    );

    sleep(Duration::from_millis(20)).await;

    let events = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(workspace_uri(&format!("/events?session_id={id}&once=true")))
                .header(COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(events.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();
    assert!(text.contains("event: approval_decided"));
    assert!(text.contains("event: agent_run_completed"));
    assert!(text.contains("Restart requested"));

    let history = app
        .oneshot(
            Request::builder()
                .uri(workspace_uri(&format!("/agent/sessions/{id}/approvals")))
                .header(COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(history.into_body(), usize::MAX).await.unwrap();
    let history: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(history.as_array().unwrap().len(), 1);
    assert_eq!(history[0]["status"], "approved");
    assert_eq!(history[0]["actor"], "operator@example.com");
}
