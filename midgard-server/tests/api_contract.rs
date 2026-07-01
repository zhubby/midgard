use axum::{
    Router,
    body::{Body, to_bytes},
    http::{
        Request, StatusCode,
        header::{COOKIE, SET_COOKIE},
    },
};
use midgard_agent::{
    AgentToolCall, LlmProvider, LlmRequest, LlmResponse, LlmStream, LlmStreamEvent,
    ScriptedLlmProvider,
};
use midgard_server::{
    AuthSettings, WorkspaceCredentialSettings, app, app_with_provider_auth_orgs_and_credentials,
};
use midgard_storage::{
    AuthStore, MemoryAgentSessionStore, MemoryAuthStore, MemoryOrganizationStore, NewOrganization,
    NewOrganizationMembership, NewUser, NewWorkspace, OrganizationRole, OrganizationStore,
    UserRole, hash_password,
};
use serde_json::Value;
use std::sync::{Arc, Mutex};
use tokio::time::{Duration, sleep};
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
        runtime_config: None,
    })
    .await
    .unwrap();

    let app = app_with_provider_auth_orgs_and_credentials(
        Arc::new(MemoryAgentSessionStore::new()),
        auth.clone(),
        orgs,
        provider,
        AuthSettings::default(),
        WorkspaceCredentialSettings::new(Some("test workspace credential key".to_string())),
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

fn workspace_slug_uri(workspace_slug: &str, suffix: &str) -> String {
    format!("/api/orgs/{TEST_ORG}/workspaces/{workspace_slug}{suffix}")
}

async fn configure_default_workspace_for_docker(app: &Router, cookie: &str) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(workspace_uri(""))
                .header(COOKIE, cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"runtime_config":{"mode":"docker","docker_api_url":"https://docker.example.com:2376"}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
}

async fn create_kubernetes_workspace(app: &Router, cookie: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/orgs/{TEST_ORG}/workspaces"))
                .header(COOKIE, cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"name":"Kubernetes Runtime","runtime_config":{"mode":"kubernetes","kubeconfig":"apiVersion: v1\nkind: Config\ncurrent-context: test\ncontexts:\n- name: test\n  context:\n    cluster: test\nclusters:\n- name: test\n  cluster:\n    server: https://kubernetes.example.com"}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let workspace: Value = serde_json::from_slice(&body).unwrap();

    workspace["slug"].as_str().unwrap().to_string()
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

#[derive(Clone, Default)]
struct CapturingLlmProvider {
    tool_names: Arc<Mutex<Vec<Vec<String>>>>,
}

impl CapturingLlmProvider {
    fn captured_tool_names(&self) -> Vec<Vec<String>> {
        self.tool_names.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl LlmProvider for CapturingLlmProvider {
    async fn complete(&self, request: LlmRequest) -> midgard_core::MidgardResult<LlmResponse> {
        self.tool_names
            .lock()
            .unwrap()
            .push(request.tools.into_iter().map(|tool| tool.name).collect());
        Ok(LlmResponse::text("ok"))
    }

    fn stream(&self, request: LlmRequest) -> LlmStream {
        self.tool_names
            .lock()
            .unwrap()
            .push(request.tools.into_iter().map(|tool| tool.name).collect());
        Box::pin(tokio_stream::iter([Ok(LlmStreamEvent::MessageDone(
            LlmResponse::text("ok"),
        ))]))
    }
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
    let app = app_with_provider_auth_orgs_and_credentials(
        Arc::new(MemoryAgentSessionStore::new()),
        auth,
        Arc::new(MemoryOrganizationStore::new()),
        Arc::new(ScriptedLlmProvider::single(LlmResponse::text("ok"))),
        AuthSettings::default(),
        WorkspaceCredentialSettings::default(),
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
async fn register_creates_initial_admin_session() {
    let auth = Arc::new(MemoryAuthStore::new());
    let app = app_with_provider_auth_orgs_and_credentials(
        Arc::new(MemoryAgentSessionStore::new()),
        auth,
        Arc::new(MemoryOrganizationStore::new()),
        Arc::new(ScriptedLlmProvider::single(LlmResponse::text("ok"))),
        AuthSettings::default(),
        WorkspaceCredentialSettings::default(),
    );

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/register")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"email":"owner@example.com","password":"valid-password","display_name":"Owner"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let cookie = response
        .headers()
        .get(SET_COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["user"]["email"], "owner@example.com");
    assert_eq!(json["user"]["role"], "admin");
    assert_eq!(json["system_role"]["builtin_key"], "system_owner");
    assert!(json["user"].get("password_hash").is_none());

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

    assert_eq!(me.status(), StatusCode::OK);
}

#[tokio::test]
async fn register_creates_later_viewer_without_admin_permissions() {
    let (app, _, _) = app_with_role(UserRole::Admin).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/register")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"email":"viewer@example.com","password":"valid-password","display_name":"Viewer"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["user"]["email"], "viewer@example.com");
    assert_eq!(json["user"]["role"], "viewer");
    assert_eq!(json["system_role"]["builtin_key"], "system_viewer");
    assert!(
        !json["system_permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|permission| permission.as_str() == Some("system.users.manage"))
    );
}

#[tokio::test]
async fn register_rejects_duplicate_email_without_cookie() {
    let (app, _, _) = app_with_role(UserRole::Admin).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/register")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"email":"{TEST_EMAIL}","password":"valid-password","display_name":"Duplicate"}}"#
                )))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);
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
    let app = app_with_provider_auth_orgs_and_credentials(
        Arc::new(MemoryAgentSessionStore::new()),
        auth,
        Arc::new(MemoryOrganizationStore::new()),
        Arc::new(ScriptedLlmProvider::single(LlmResponse::text("ok"))),
        AuthSettings::default(),
        WorkspaceCredentialSettings::new(Some("test workspace credential key".to_string())),
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
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/orgs")
                .header(COOKIE, cookie.clone())
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
    assert_eq!(context["workspaces"].as_array().unwrap().len(), 0);

    let workspace = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/orgs/platform-ops/workspaces")
                .header(COOKIE, cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"name":"Operations","runtime_config":{"mode":"docker","docker_api_url":"https://docker.example.com:2376"}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(workspace.status(), StatusCode::CREATED);
    let body = to_bytes(workspace.into_body(), usize::MAX).await.unwrap();
    let workspace: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(workspace["slug"], "operations");
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
                .body(Body::from(
                    r#"{"name":"Secondary","runtime_config":{"mode":"kubernetes","kubeconfig":"apiVersion: v1\nkind: Config\ncurrent-context: test\ncontexts:\n- name: test\n  context:\n    cluster: test\nclusters:\n- name: test\n  cluster:\n    server: https://kubernetes.example.com"}}"#,
                ))
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
async fn workspace_creation_requires_runtime_config() {
    let (app, cookie, _) = app_with_role(UserRole::Admin).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/orgs/{TEST_ORG}/workspaces"))
                .header(COOKIE, cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"name":"Missing Runtime"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn workspace_runtime_response_is_redacted() {
    let (app, cookie, _) = app_with_role(UserRole::Admin).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/orgs/{TEST_ORG}/workspaces"))
                .header(COOKIE, cookie.clone())
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"name":"Docker Runtime","runtime_config":{"mode":"docker","docker_api_url":"https://secret-docker.example.com:2376"}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let workspace: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(workspace["runtime_config"]["mode"], "docker");
    assert_eq!(
        workspace["runtime_config"]["docker"]["endpoint_host"],
        "secret-docker.example.com"
    );
    assert!(workspace.get("runtime_config_ciphertext").is_none());
    assert!(!String::from_utf8_lossy(&body).contains("https://secret-docker.example.com:2376"));

    let fetched = app
        .oneshot(
            Request::builder()
                .uri(format!("/api/orgs/{TEST_ORG}/workspaces/docker-runtime"))
                .header(COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);
    let fetched_body = to_bytes(fetched.into_body(), usize::MAX).await.unwrap();
    assert!(
        !String::from_utf8_lossy(&fetched_body).contains("https://secret-docker.example.com:2376")
    );
}

#[tokio::test]
async fn middleware_instances_require_configured_workspace_and_manage_permission() {
    let (viewer_app, viewer_cookie, _) = app_with_role(UserRole::Viewer).await;
    let viewer_response = viewer_app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri("/middleware"))
                .header(COOKIE, viewer_cookie)
                .header("content-type", "application/json")
                .body(Body::from(r#"{"kind":"redis","name":"cache"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(viewer_response.status(), StatusCode::FORBIDDEN);

    let (app, cookie, _) = app_with_role(UserRole::Admin).await;
    let unconfigured_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri("/middleware"))
                .header(COOKIE, cookie.clone())
                .header("content-type", "application/json")
                .body(Body::from(r#"{"kind":"redis","name":"cache"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unconfigured_response.status(), StatusCode::BAD_REQUEST);

    let configure = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(workspace_uri(""))
                .header(COOKIE, cookie.clone())
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"runtime_config":{"mode":"docker","docker_api_url":"https://docker.example.com:2376"}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(configure.status(), StatusCode::OK);

    let created = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri("/middleware"))
                .header(COOKIE, cookie.clone())
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"kind":"redis","name":"cache","namespace":"data","config":{"memory":"512Mi"}}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let body = to_bytes(created.into_body(), usize::MAX).await.unwrap();
    let instance: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(instance["status"], "pending");
    let id = instance["id"].as_str().unwrap();

    let updated = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri(workspace_uri(&format!("/middleware/{id}")))
                .header(COOKIE, cookie.clone())
                .header("content-type", "application/json")
                .body(Body::from(r#"{"status":"running"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);

    let list = app
        .oneshot(
            Request::builder()
                .uri(workspace_uri("/middleware"))
                .header(COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(list.into_body(), usize::MAX).await.unwrap();
    let instances: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(instances.as_array().unwrap().len(), 1);
    assert_eq!(instances[0]["status"], "running");
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
async fn tools_endpoint_filters_docker_tools_by_runtime() {
    let (app, cookie, _) = app_with_role(UserRole::Admin).await;
    let response = app
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
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    let tools = json.as_array().unwrap();
    assert!(
        !tools
            .iter()
            .any(|tool| tool["name"].as_str().unwrap().starts_with("docker_"))
    );
    assert!(!tools.iter().any(|tool| {
        tool["name"]
            .as_str()
            .unwrap()
            .starts_with("operator_capability_")
    }));
    let create_tool = tools
        .iter()
        .find(|tool| tool["name"] == "middleware_create")
        .unwrap();
    assert_eq!(create_tool["risk_level"], "high");
    assert_eq!(create_tool["requires_approval"], true);
    let delete_tool = tools
        .iter()
        .find(|tool| tool["name"] == "middleware_delete")
        .unwrap();
    assert_eq!(delete_tool["risk_level"], "critical");
    assert_eq!(delete_tool["requires_approval"], true);

    configure_default_workspace_for_docker(&app, &cookie).await;
    let response = app
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
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    let tools = json.as_array().unwrap();
    let docker_info = tools
        .iter()
        .find(|tool| tool["name"] == "docker_info")
        .unwrap();
    assert_eq!(docker_info["risk_level"], "low");
    assert_eq!(docker_info["requires_approval"], false);
    assert!(
        !docker_info["parameters_schema"]["properties"]
            .as_object()
            .unwrap()
            .contains_key("workspace_id")
    );
    let docker_restart = tools
        .iter()
        .find(|tool| tool["name"] == "docker_container_restart")
        .unwrap();
    assert_eq!(docker_restart["risk_level"], "high");
    assert_eq!(docker_restart["requires_approval"], true);
    let docker_prune = tools
        .iter()
        .find(|tool| tool["name"] == "docker_system_prune")
        .unwrap();
    assert_eq!(docker_prune["risk_level"], "critical");
    assert_eq!(docker_prune["requires_approval"], true);
    assert!(!tools.iter().any(|tool| {
        tool["name"]
            .as_str()
            .unwrap()
            .starts_with("operator_capability_")
    }));

    let kubernetes_slug = create_kubernetes_workspace(&app, &cookie).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri(workspace_slug_uri(&kubernetes_slug, "/tools"))
                .header(COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();
    assert!(
        !json
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"].as_str().unwrap().starts_with("docker_"))
    );
    let tools = json.as_array().unwrap();
    let capability_list = tools
        .iter()
        .find(|tool| tool["name"] == "operator_capability_list")
        .unwrap();
    assert_eq!(capability_list["risk_level"], "low");
    assert_eq!(capability_list["requires_approval"], false);
    let capability_execute = tools
        .iter()
        .find(|tool| tool["name"] == "operator_capability_execute")
        .unwrap();
    assert_eq!(capability_execute["risk_level"], "critical");
    assert_eq!(capability_execute["requires_approval"], true);
    assert!(
        !capability_execute["parameters_schema"]["properties"]
            .as_object()
            .unwrap()
            .contains_key("workspace_id")
    );
}

#[tokio::test]
async fn plugins_endpoint_filters_docker_plugin_by_runtime() {
    let (app, cookie, _) = app_with_role(UserRole::Admin).await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(workspace_uri("/plugins"))
                .header(COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json.as_array().unwrap().len(), 0);

    configure_default_workspace_for_docker(&app, &cookie).await;
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

    assert_eq!(json[0]["id"], "midgard-docker");
    assert_eq!(json[0]["middleware_kind"], "docker");
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
        runtime_config: None,
    })
    .await
    .unwrap();
    let app = app_with_provider_auth_orgs_and_credentials(
        Arc::new(MemoryAgentSessionStore::new()),
        auth,
        orgs,
        Arc::new(ScriptedLlmProvider::single(LlmResponse::text("ok"))),
        AuthSettings::default(),
        WorkspaceCredentialSettings::default(),
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
async fn agent_run_passes_docker_workspace_tools_to_llm() {
    let provider = Arc::new(CapturingLlmProvider::default());
    let (app, cookie, _) = app_with_role_and_provider(UserRole::Admin, provider.clone()).await;
    configure_default_workspace_for_docker(&app, &cookie).await;

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri("/agent/sessions"))
                .header(COOKIE, cookie.clone())
                .header("content-type", "application/json")
                .body(Body::from(r#"{"goal":"inspect docker"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::OK);
    let body = to_bytes(create_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: Value = serde_json::from_slice(&body).unwrap();
    let id = created["id"].as_str().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri(&format!("/agent/sessions/{id}/runs")))
                .header(COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    sleep(Duration::from_millis(20)).await;
    let captured = provider.captured_tool_names();
    assert!(!captured.is_empty());
    assert!(captured[0].iter().any(|name| name == "complete_task"));
    assert!(captured[0].iter().any(|name| name == "docker_info"));
    assert!(
        captured[0]
            .iter()
            .any(|name| name == "docker_container_restart")
    );
}

#[tokio::test]
async fn agent_run_passes_kubernetes_protocol_tools_to_llm() {
    let provider = Arc::new(CapturingLlmProvider::default());
    let (app, cookie, _) = app_with_role_and_provider(UserRole::Admin, provider.clone()).await;
    let kubernetes_slug = create_kubernetes_workspace(&app, &cookie).await;

    let create_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_slug_uri(&kubernetes_slug, "/agent/sessions"))
                .header(COOKIE, cookie.clone())
                .header("content-type", "application/json")
                .body(Body::from(r#"{"goal":"inspect kubernetes operators"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_response.status(), StatusCode::OK);
    let body = to_bytes(create_response.into_body(), usize::MAX)
        .await
        .unwrap();
    let created: Value = serde_json::from_slice(&body).unwrap();
    let id = created["id"].as_str().unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_slug_uri(
                    &kubernetes_slug,
                    &format!("/agent/sessions/{id}/runs"),
                ))
                .header(COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    sleep(Duration::from_millis(20)).await;
    let captured = provider.captured_tool_names();
    assert!(!captured.is_empty());
    assert!(captured[0].iter().any(|name| name == "complete_task"));
    assert!(
        captured[0]
            .iter()
            .any(|name| name == "operator_capability_list")
    );
    assert!(
        captured[0]
            .iter()
            .any(|name| name == "operator_capability_execute")
    );
    assert!(!captured[0].iter().any(|name| name == "docker_info"));
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
    let (app, cookie, _) = app_with_role(UserRole::Admin).await;

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(workspace_uri("/events?once=true"))
                .header(COOKIE, cookie.clone())
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
    assert!(!text.contains("docker_info"));
    assert!(!text.contains("operator_capability_execute"));
    assert!(!text.contains("midgard-docker"));

    configure_default_workspace_for_docker(&app, &cookie).await;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(workspace_uri("/events?once=true"))
                .header(COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    assert!(text.contains("docker_info"));
    assert!(text.contains("midgard-docker"));
    assert!(!text.contains("operator_capability_execute"));

    let kubernetes_slug = create_kubernetes_workspace(&app, &cookie).await;
    let response = app
        .oneshot(
            Request::builder()
                .uri(workspace_slug_uri(&kubernetes_slug, "/events?once=true"))
                .header(COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let text = String::from_utf8(body.to_vec()).unwrap();

    assert!(text.contains("operator_capability_list"));
    assert!(text.contains("operator_capability_execute"));
    assert!(!text.contains("docker_info"));
}

#[tokio::test]
async fn approval_endpoint_records_docker_tool_decision_without_execution() {
    let (app, cookie, _) = app_with_role_and_provider(
        UserRole::Admin,
        Arc::new(ScriptedLlmProvider::single(LlmResponse::with_tool_calls(
            "",
            vec![AgentToolCall::from_raw(
                "call_1",
                "docker_container_restart",
                r#"{"container":"cache"}"#,
            )],
        ))),
    )
    .await;
    configure_default_workspace_for_docker(&app, &cookie).await;
    let id = uuid::Uuid::new_v4();
    let create = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri(&format!("/agent/sessions/{id}/messages")))
                .header(COOKIE, cookie.clone())
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"restart docker container"}"#))
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
    assert_eq!(history[0]["tool_call"]["name"], "docker_container_restart");

    let approval = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(workspace_uri(&format!("/agent/sessions/{id}/approvals")))
                .header(COOKIE, cookie.clone())
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"decision":"approve","reason":"maintenance window","resume":false}"#,
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
    assert!(!text.contains("event: agent_run_completed"));

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
