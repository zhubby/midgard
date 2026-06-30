use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, Sse},
        IntoResponse,
    },
    routing::{get, patch, post},
    Json, Router,
};
use http::{
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE},
    HeaderName, HeaderValue, Method,
};
use midgard_agent::{
    AgentRunEvent, AgentRunStatus, AgentRunner, AgentSession, ApprovalDecision, ApprovalRecord,
    CompleteTaskTool, LlmProvider, LlmResponse, PendingApproval, ScriptedLlmProvider,
};
use midgard_controller::{MiddlewareController, MiddlewarePlugin};
use midgard_plugin_example::ExampleRedisPlugin;
use midgard_storage::{
    permission_catalog, MemoryAgentSessionStore, MemoryAuthStore, MemoryOrganizationStore,
    NewOrganization, NewOrganizationMembership, NewRbacRole, NewWorkspace, Organization,
    OrganizationContext, OrganizationMembership, OrganizationMembershipUpdate, OrganizationRole,
    PermissionCatalogItem, PermissionKey, RbacRole, RbacRoleUpdate, RbacScopeKind,
    SharedAgentSessionStore, SharedAuthStore, SharedOrganizationStore, Workspace, WorkspaceUpdate,
};
use midgard_tools::{ToolDefinition, ToolRegistry};
use serde::{Deserialize, Serialize};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio_stream::Stream;
use tower_http::cors::CorsLayer;
use ts_rs::TS;
use uuid::Uuid;

mod auth;
mod workspace;

pub use auth::{
    AuthContext, AuthSettings, CreateAuthUserRequest, LoginRequest, LogoutResponse,
    UpdateAuthUserRequest,
};
pub use workspace::{
    agent_run_event_payload, DashboardTone, MiddlewareDashboardState, MiddlewareMetric,
    MiddlewareTimelineEvent, MiddlewareWorkload, WorkspaceEvent, WorkspaceEventBus,
    WorkspaceEventPayload, WorkspaceEventType, WorkspaceSnapshot, WORKSPACE_PROTOCOL_VERSION,
};

#[derive(Clone)]
pub struct AppState {
    pub(crate) tools: Arc<ToolRegistry>,
    pub(crate) runner: Arc<AgentRunner>,
    pub(crate) plugins: Arc<Vec<PluginResponse>>,
    pub(crate) sessions: SharedAgentSessionStore,
    pub(crate) auth: SharedAuthStore,
    pub(crate) orgs: SharedOrganizationStore,
    pub(crate) auth_settings: AuthSettings,
    pub(crate) events: WorkspaceEventBus,
    pub(crate) middleware: Arc<MiddlewareDashboardState>,
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
    app_with_provider_and_auth(
        sessions,
        Arc::new(MemoryAuthStore::new()),
        provider,
        AuthSettings::default(),
    )
}

pub fn app_with_provider_and_auth(
    sessions: SharedAgentSessionStore,
    auth: SharedAuthStore,
    provider: Arc<dyn LlmProvider>,
    auth_settings: AuthSettings,
) -> Router {
    app_with_provider_auth_and_orgs(
        sessions,
        auth,
        Arc::new(MemoryOrganizationStore::new()),
        provider,
        auth_settings,
    )
}

pub fn app_with_provider_auth_and_orgs(
    sessions: SharedAgentSessionStore,
    auth: SharedAuthStore,
    orgs: SharedOrganizationStore,
    provider: Arc<dyn LlmProvider>,
    auth_settings: AuthSettings,
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
        auth,
        orgs,
        auth_settings,
        events: WorkspaceEventBus::new(),
        middleware: Arc::new(MiddlewareDashboardState::mock()),
    };

    Router::new()
        .route("/healthz", get(healthz))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/me", get(auth::me))
        .route("/api/permissions/catalog", get(permission_catalog_endpoint))
        .route(
            "/api/auth/users",
            get(auth::list_users).post(auth::create_user),
        )
        .route("/api/auth/users/{id}", patch(auth::update_user))
        .route(
            "/api/rbac/system/roles",
            get(list_system_roles).post(create_system_role),
        )
        .route("/api/rbac/system/roles/{id}", patch(update_system_role))
        .route(
            "/api/rbac/system/roles/{id}/permissions",
            axum::routing::put(replace_system_role_permissions),
        )
        .route(
            "/api/orgs",
            get(list_org_contexts).post(create_organization),
        )
        .route("/api/orgs/{org_slug}", get(get_organization_context))
        .route(
            "/api/orgs/{org_slug}/roles",
            get(list_organization_roles).post(create_organization_role),
        )
        .route(
            "/api/orgs/{org_slug}/roles/{id}",
            patch(update_organization_role),
        )
        .route(
            "/api/orgs/{org_slug}/roles/{id}/permissions",
            axum::routing::put(replace_organization_role_permissions),
        )
        .route(
            "/api/orgs/{org_slug}/members",
            get(list_organization_members).post(add_organization_member),
        )
        .route(
            "/api/orgs/{org_slug}/members/{user_id}",
            patch(update_organization_member),
        )
        .route(
            "/api/orgs/{org_slug}/workspaces",
            get(list_organization_workspaces).post(create_workspace),
        )
        .route(
            "/api/orgs/{org_slug}/workspaces/{workspace_slug}",
            patch(update_workspace),
        )
        .route(
            "/api/orgs/{org_slug}/workspaces/{workspace_slug}/events",
            get(workspace_events),
        )
        .route(
            "/api/orgs/{org_slug}/workspaces/{workspace_slug}/tools",
            get(list_workspace_tools),
        )
        .route(
            "/api/orgs/{org_slug}/workspaces/{workspace_slug}/plugins",
            get(list_workspace_plugins),
        )
        .route(
            "/api/orgs/{org_slug}/workspaces/{workspace_slug}/agent/sessions",
            post(create_session),
        )
        .route(
            "/api/orgs/{org_slug}/workspaces/{workspace_slug}/agent/sessions/{id}/messages",
            post(send_message),
        )
        .route(
            "/api/orgs/{org_slug}/workspaces/{workspace_slug}/agent/sessions/{id}/runs",
            post(run_agent),
        )
        .route(
            "/api/orgs/{org_slug}/workspaces/{workspace_slug}/agent/sessions/{id}/runs/stream",
            post(stream_agent),
        )
        .route(
            "/api/orgs/{org_slug}/workspaces/{workspace_slug}/agent/sessions/{id}/approvals",
            get(list_approval_records).post(record_approval),
        )
        .layer(
            CorsLayer::new()
                .allow_origin([
                    HeaderValue::from_static("http://localhost:3000"),
                    HeaderValue::from_static("http://127.0.0.1:3000"),
                ])
                .allow_methods([
                    Method::GET,
                    Method::POST,
                    Method::PATCH,
                    Method::PUT,
                    Method::OPTIONS,
                ])
                .allow_headers([
                    ACCEPT,
                    AUTHORIZATION,
                    CONTENT_TYPE,
                    HeaderName::from_static("last-event-id"),
                ])
                .allow_credentials(true),
        )
        .with_state(state)
}

async fn healthz() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}

async fn permission_catalog_endpoint(
    _user: auth::AuthenticatedUser,
) -> Json<Vec<PermissionCatalogItem>> {
    Json(permission_catalog())
}

async fn list_org_contexts(
    user: auth::AuthenticatedUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<OrganizationContext>>, AppError> {
    user.require_system_permission(&state, PermissionKey::SystemOrgsRead)
        .await?;
    Ok(Json(
        state
            .orgs
            .list_contexts_for_user(user.0.id)
            .await
            .map_err(storage_app_error)?,
    ))
}

async fn create_organization(
    user: auth::AuthenticatedUser,
    State(state): State<AppState>,
    Json(request): Json<CreateOrganizationRequest>,
) -> Result<(StatusCode, Json<OrganizationContext>), AppError> {
    user.require_system_permission(&state, PermissionKey::SystemOrgsCreate)
        .await?;
    let name = required_request_name(&request.name, "organization name")?;
    let slug = request
        .slug
        .unwrap_or_else(|| slug_from_name(&name, "organization"));
    let workspace_name = request
        .workspace_name
        .map(|value| required_request_name(&value, "workspace name"))
        .transpose()?
        .unwrap_or_else(|| "Operations".to_string());
    let workspace_slug = request
        .workspace_slug
        .unwrap_or_else(|| slug_from_name(&workspace_name, "workspace"));
    let organization = state
        .orgs
        .create_organization(NewOrganization {
            slug,
            name,
            created_by_user_id: user.0.id,
        })
        .await
        .map_err(storage_app_error)?;
    let membership = state
        .orgs
        .create_membership(NewOrganizationMembership {
            organization_id: organization.id,
            user_id: user.0.id,
            role: OrganizationRole::Owner,
            role_id: None,
            active: true,
        })
        .await
        .map_err(storage_app_error)?;
    let workspace = state
        .orgs
        .create_workspace(NewWorkspace {
            organization_id: organization.id,
            slug: workspace_slug,
            name: workspace_name,
        })
        .await
        .map_err(storage_app_error)?;

    Ok((
        StatusCode::CREATED,
        Json(OrganizationContext {
            organization,
            membership,
            workspaces: vec![workspace],
            permissions: PermissionKey::organization_permissions(),
        }),
    ))
}

async fn get_organization_context(
    user: auth::AuthenticatedUser,
    Path(org_slug): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<OrganizationContext>, AppError> {
    let context = load_organization_context(&state, &user, &org_slug).await?;
    require_org_permission(&state, &context.membership, PermissionKey::OrgRead).await?;
    Ok(Json(context))
}

async fn list_system_roles(
    user: auth::AuthenticatedUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<RbacRole>>, AppError> {
    user.require_system_permission(&state, PermissionKey::SystemRolesRead)
        .await?;
    Ok(Json(state.auth.list_system_roles().await?))
}

async fn create_system_role(
    user: auth::AuthenticatedUser,
    State(state): State<AppState>,
    Json(request): Json<CreateRbacRoleRequest>,
) -> Result<(StatusCode, Json<RbacRole>), AppError> {
    user.require_system_permission(&state, PermissionKey::SystemRolesManage)
        .await?;
    let role = state
        .auth
        .create_system_role(NewRbacRole {
            scope_kind: RbacScopeKind::System,
            organization_id: None,
            slug: request.slug,
            name: request.name,
            description: request.description,
            builtin_key: None,
            protected: false,
            permissions: request.permissions,
        })
        .await
        .map_err(storage_app_error)?;

    Ok((StatusCode::CREATED, Json(role)))
}

async fn update_system_role(
    user: auth::AuthenticatedUser,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<UpdateRbacRoleRequest>,
) -> Result<Json<RbacRole>, AppError> {
    user.require_system_permission(&state, PermissionKey::SystemRolesManage)
        .await?;
    let role = state
        .auth
        .update_system_role(
            id,
            RbacRoleUpdate {
                name: request.name,
                description: request.description,
                archived: request.archived,
            },
        )
        .await
        .map_err(storage_app_error)?
        .ok_or_else(|| AppError::NotFound("system role not found".to_string()))?;

    Ok(Json(role))
}

async fn replace_system_role_permissions(
    user: auth::AuthenticatedUser,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<ReplaceRolePermissionsRequest>,
) -> Result<Json<RbacRole>, AppError> {
    user.require_system_permission(&state, PermissionKey::SystemRolesManage)
        .await?;
    let role = state
        .auth
        .replace_system_role_permissions(id, request.permissions)
        .await
        .map_err(storage_app_error)?
        .ok_or_else(|| AppError::NotFound("system role not found".to_string()))?;

    Ok(Json(role))
}

async fn list_organization_roles(
    user: auth::AuthenticatedUser,
    Path(org_slug): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<Vec<RbacRole>>, AppError> {
    let context = load_organization_context(&state, &user, &org_slug).await?;
    require_org_permission(&state, &context.membership, PermissionKey::OrgRolesRead).await?;
    Ok(Json(
        state
            .orgs
            .list_organization_roles(context.organization.id)
            .await?,
    ))
}

async fn create_organization_role(
    user: auth::AuthenticatedUser,
    Path(org_slug): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<CreateRbacRoleRequest>,
) -> Result<(StatusCode, Json<RbacRole>), AppError> {
    let context = load_organization_context(&state, &user, &org_slug).await?;
    require_org_permission(&state, &context.membership, PermissionKey::OrgRolesManage).await?;
    let role = state
        .orgs
        .create_organization_role(NewRbacRole {
            scope_kind: RbacScopeKind::Organization,
            organization_id: Some(context.organization.id),
            slug: request.slug,
            name: request.name,
            description: request.description,
            builtin_key: None,
            protected: false,
            permissions: request.permissions,
        })
        .await
        .map_err(storage_app_error)?;

    Ok((StatusCode::CREATED, Json(role)))
}

async fn update_organization_role(
    user: auth::AuthenticatedUser,
    Path((org_slug, id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
    Json(request): Json<UpdateRbacRoleRequest>,
) -> Result<Json<RbacRole>, AppError> {
    let context = load_organization_context(&state, &user, &org_slug).await?;
    require_org_permission(&state, &context.membership, PermissionKey::OrgRolesManage).await?;
    let role = state
        .orgs
        .update_organization_role(
            context.organization.id,
            id,
            RbacRoleUpdate {
                name: request.name,
                description: request.description,
                archived: request.archived,
            },
        )
        .await
        .map_err(storage_app_error)?
        .ok_or_else(|| AppError::NotFound("organization role not found".to_string()))?;

    Ok(Json(role))
}

async fn replace_organization_role_permissions(
    user: auth::AuthenticatedUser,
    Path((org_slug, id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
    Json(request): Json<ReplaceRolePermissionsRequest>,
) -> Result<Json<RbacRole>, AppError> {
    let context = load_organization_context(&state, &user, &org_slug).await?;
    require_org_permission(&state, &context.membership, PermissionKey::OrgRolesManage).await?;
    let role = state
        .orgs
        .replace_organization_role_permissions(context.organization.id, id, request.permissions)
        .await
        .map_err(storage_app_error)?
        .ok_or_else(|| AppError::NotFound("organization role not found".to_string()))?;

    Ok(Json(role))
}

async fn list_organization_members(
    user: auth::AuthenticatedUser,
    Path(org_slug): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<Vec<OrganizationMemberView>>, AppError> {
    let context = load_organization_context(&state, &user, &org_slug).await?;
    require_org_permission(&state, &context.membership, PermissionKey::OrgMembersRead).await?;
    let memberships = state.orgs.list_memberships(context.organization.id).await?;
    let mut members = Vec::with_capacity(memberships.len());
    for membership in memberships {
        if let Some(user) = state.auth.load_user_by_id(membership.user_id).await? {
            members.push(OrganizationMemberView { membership, user });
        }
    }

    Ok(Json(members))
}

async fn add_organization_member(
    user: auth::AuthenticatedUser,
    Path(org_slug): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<AddOrganizationMemberRequest>,
) -> Result<(StatusCode, Json<OrganizationMembership>), AppError> {
    let context = load_organization_context(&state, &user, &org_slug).await?;
    require_org_permission(&state, &context.membership, PermissionKey::OrgMembersManage).await?;
    let email = midgard_storage::normalize_email(&request.email);
    let target = state
        .auth
        .load_user_by_email(&email)
        .await?
        .ok_or_else(|| AppError::NotFound("user not found".to_string()))?
        .user;

    let membership = state
        .orgs
        .create_membership(NewOrganizationMembership {
            organization_id: context.organization.id,
            user_id: target.id,
            role: request.role.unwrap_or(OrganizationRole::Viewer),
            role_id: request.role_id,
            active: true,
        })
        .await
        .map_err(storage_app_error)?;

    Ok((StatusCode::CREATED, Json(membership)))
}

async fn update_organization_member(
    user: auth::AuthenticatedUser,
    Path((org_slug, user_id)): Path<(String, Uuid)>,
    State(state): State<AppState>,
    Json(request): Json<UpdateOrganizationMemberRequest>,
) -> Result<Json<OrganizationMembership>, AppError> {
    let context = load_organization_context(&state, &user, &org_slug).await?;
    require_org_permission(&state, &context.membership, PermissionKey::OrgMembersManage).await?;
    let membership = state
        .orgs
        .update_membership(
            context.organization.id,
            user_id,
            OrganizationMembershipUpdate {
                role: request.role,
                role_id: request.role_id,
                active: request.active,
            },
        )
        .await
        .map_err(storage_app_error)?
        .ok_or_else(|| AppError::NotFound("organization member not found".to_string()))?;

    Ok(Json(membership))
}

async fn list_organization_workspaces(
    user: auth::AuthenticatedUser,
    Path(org_slug): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<Vec<Workspace>>, AppError> {
    let context = load_organization_context(&state, &user, &org_slug).await?;
    require_org_permission(&state, &context.membership, PermissionKey::WorkspacesRead).await?;
    Ok(Json(context.workspaces))
}

async fn create_workspace(
    user: auth::AuthenticatedUser,
    Path(org_slug): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<CreateWorkspaceRequest>,
) -> Result<(StatusCode, Json<Workspace>), AppError> {
    let context = load_organization_context(&state, &user, &org_slug).await?;
    require_org_permission(&state, &context.membership, PermissionKey::WorkspacesManage).await?;
    let name = required_request_name(&request.name, "workspace name")?;
    let slug = request
        .slug
        .unwrap_or_else(|| slug_from_name(&name, "workspace"));

    let workspace = state
        .orgs
        .create_workspace(NewWorkspace {
            organization_id: context.organization.id,
            slug,
            name,
        })
        .await
        .map_err(storage_app_error)?;

    Ok((StatusCode::CREATED, Json(workspace)))
}

async fn update_workspace(
    user: auth::AuthenticatedUser,
    Path((org_slug, workspace_slug)): Path<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<UpdateWorkspaceRequest>,
) -> Result<Json<Workspace>, AppError> {
    let context = load_organization_context(&state, &user, &org_slug).await?;
    require_org_permission(&state, &context.membership, PermissionKey::WorkspacesManage).await?;
    let workspace = state
        .orgs
        .update_workspace(
            context.organization.id,
            &workspace_slug,
            WorkspaceUpdate {
                name: request.name,
                archived: request.archived,
            },
        )
        .await
        .map_err(storage_app_error)?
        .ok_or_else(|| AppError::NotFound("workspace not found".to_string()))?;

    Ok(Json(workspace))
}

async fn list_workspace_tools(
    user: auth::AuthenticatedUser,
    Path((org_slug, workspace_slug)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ToolDefinition>>, AppError> {
    let scope = load_workspace_scope(&state, &user, &org_slug, &workspace_slug).await?;
    require_org_permission(&state, &scope.membership, PermissionKey::WorkspaceRead).await?;
    Ok(Json(state.tools.definitions()))
}

async fn list_workspace_plugins(
    user: auth::AuthenticatedUser,
    Path((org_slug, workspace_slug)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<Vec<PluginResponse>>, AppError> {
    let scope = load_workspace_scope(&state, &user, &org_slug, &workspace_slug).await?;
    require_org_permission(&state, &scope.membership, PermissionKey::WorkspaceRead).await?;
    Ok(Json((*state.plugins).clone()))
}

async fn create_session(
    user: auth::AuthenticatedUser,
    Path((org_slug, workspace_slug)): Path<(String, String)>,
    State(state): State<AppState>,
    Json(request): Json<CreateSessionRequest>,
) -> Result<Json<AgentSession>, AppError> {
    let scope = load_workspace_scope(&state, &user, &org_slug, &workspace_slug).await?;
    require_org_permission(&state, &scope.membership, PermissionKey::WorkspaceOperate).await?;
    let session = state
        .sessions
        .create_session_in_workspace(scope.workspace.id, request.goal)
        .await?;
    state.events.publish_for_workspace(
        scope.workspace.id.to_string(),
        WorkspaceEventPayload::AgentSessionUpdated {
            session: Box::new(session.clone()),
        },
    );

    Ok(Json(session))
}

async fn send_message(
    user: auth::AuthenticatedUser,
    Path((org_slug, workspace_slug, id)): Path<(String, String, Uuid)>,
    State(state): State<AppState>,
    Json(request): Json<SendMessageRequest>,
) -> Result<Json<AgentSession>, AppError> {
    let scope = load_workspace_scope(&state, &user, &org_slug, &workspace_slug).await?;
    require_org_permission(&state, &scope.membership, PermissionKey::WorkspaceOperate).await?;
    let session = state
        .sessions
        .append_user_message_in_workspace(scope.workspace.id, id, request.message)
        .await
        .map_err(storage_app_error)?;
    if let Some(message) = session.messages.last().cloned() {
        state.events.publish_for_workspace(
            scope.workspace.id.to_string(),
            WorkspaceEventPayload::AgentMessageCommitted {
                session_id: id.to_string(),
                message,
            },
        );
    }
    state.events.publish_for_workspace(
        scope.workspace.id.to_string(),
        WorkspaceEventPayload::AgentSessionUpdated {
            session: Box::new(session.clone()),
        },
    );

    Ok(Json(session))
}

async fn run_agent(
    user: auth::AuthenticatedUser,
    Path((org_slug, workspace_slug, id)): Path<(String, String, Uuid)>,
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<RunAccepted>), AppError> {
    let scope = load_workspace_scope(&state, &user, &org_slug, &workspace_slug).await?;
    require_org_permission(&state, &scope.membership, PermissionKey::WorkspaceOperate).await?;
    if state
        .sessions
        .load_session_in_workspace(scope.workspace.id, id)
        .await?
        .is_none()
    {
        return Err(AppError::NotFound("agent session not found".to_string()));
    }
    let run_id = Uuid::new_v4();
    spawn_agent_run(state, scope.workspace.id, id, run_id);

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
    user: auth::AuthenticatedUser,
    Path((org_slug, workspace_slug, id)): Path<(String, String, Uuid)>,
    State(state): State<AppState>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let scope = load_workspace_scope(&state, &user, &org_slug, &workspace_slug).await?;
    require_org_permission(&state, &scope.membership, PermissionKey::WorkspaceOperate).await?;
    let session = load_or_resumed_session(&state, scope.workspace.id, id).await?;
    let result = state.runner.run(session).await?;
    state
        .sessions
        .save_session_in_workspace(scope.workspace.id, result.session)
        .await?;
    let events = result.events;

    Ok(Sse::new(tokio_stream::iter(
        events.into_iter().map(|event| Ok(agent_sse_event(event))),
    )))
}

async fn record_approval(
    user: auth::AuthenticatedUser,
    Path((org_slug, workspace_slug, id)): Path<(String, String, Uuid)>,
    State(state): State<AppState>,
    Json(request): Json<ApprovalRequest>,
) -> Result<Json<ApprovalResponse>, AppError> {
    let scope = load_workspace_scope(&state, &user, &org_slug, &workspace_slug).await?;
    require_org_permission(&state, &scope.membership, PermissionKey::WorkspaceOperate).await?;
    let mut session = load_existing_session(&state, scope.workspace.id, id).await?;
    let actor = user.actor();
    let approval = session.record_approval_decision(request.decision.clone())?;
    let approval_record = state
        .sessions
        .record_approval_decision(id, approval, request.decision, actor, request.reason)
        .await?;
    let session = state
        .sessions
        .save_session_in_workspace(scope.workspace.id, session)
        .await?;
    state.events.publish_for_workspace(
        scope.workspace.id.to_string(),
        WorkspaceEventPayload::ApprovalDecided {
            approval_record: approval_record.clone(),
            session: Box::new(session.clone()),
        },
    );
    state.events.publish_for_workspace(
        scope.workspace.id.to_string(),
        WorkspaceEventPayload::AgentSessionUpdated {
            session: Box::new(session.clone()),
        },
    );
    if request.resume {
        spawn_agent_run(state.clone(), scope.workspace.id, id, Uuid::new_v4());
    }

    Ok(Json(ApprovalResponse {
        approval_record,
        session,
    }))
}

async fn list_approval_records(
    user: auth::AuthenticatedUser,
    Path((org_slug, workspace_slug, id)): Path<(String, String, Uuid)>,
    State(state): State<AppState>,
) -> Result<Json<Vec<ApprovalRecord>>, AppError> {
    let scope = load_workspace_scope(&state, &user, &org_slug, &workspace_slug).await?;
    require_org_permission(&state, &scope.membership, PermissionKey::WorkspaceRead).await?;
    let _session = load_existing_session(&state, scope.workspace.id, id).await?;
    Ok(Json(state.sessions.list_approval_records(id).await?))
}

async fn load_or_resumed_session(
    state: &AppState,
    workspace_id: Uuid,
    id: Uuid,
) -> Result<AgentSession, AppError> {
    Ok(
        match state
            .sessions
            .load_session_in_workspace(workspace_id, id)
            .await?
        {
            Some(session) => session,
            None => {
                if state.sessions.load_session(id).await?.is_some() {
                    return Err(AppError::NotFound("agent session not found".to_string()));
                }
                let mut session = AgentSession::new("resumed session");
                session.id = id;
                session
            }
        },
    )
}

async fn load_existing_session(
    state: &AppState,
    workspace_id: Uuid,
    id: Uuid,
) -> Result<AgentSession, AppError> {
    state
        .sessions
        .load_session_in_workspace(workspace_id, id)
        .await?
        .ok_or_else(|| AppError::NotFound("agent session not found".to_string()))
}

async fn workspace_events(
    user: auth::AuthenticatedUser,
    Path((org_slug, workspace_slug)): Path<(String, String)>,
    Query(query): Query<WorkspaceEventsQuery>,
    headers: HeaderMap,
    State(state): State<AppState>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, AppError> {
    let scope = load_workspace_scope(&state, &user, &org_slug, &workspace_slug).await?;
    require_org_permission(&state, &scope.membership, PermissionKey::WorkspaceRead).await?;
    let workspace_id = scope.workspace.id.to_string();
    let last_event_id = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    let snapshot = workspace_snapshot(&state, &scope, query.session_id).await?;
    let mut initial_events = vec![state.events.local_event_for_workspace(
        &workspace_id,
        WorkspaceEventPayload::Connected {
            snapshot: Box::new(snapshot),
        },
    )];

    match state
        .events
        .replay_after_for_workspace(last_event_id, &workspace_id)
    {
        Some(replay) => initial_events.extend(replay),
        None => initial_events.push(state.events.local_event_for_workspace(
            &workspace_id,
            WorkspaceEventPayload::Error {
                message: "event buffer expired; snapshot was refreshed".to_string(),
            },
        )),
    }

    initial_events.push(state.events.local_event_for_workspace(
        &workspace_id,
        WorkspaceEventPayload::MiddlewareSnapshot {
            state: (*state.middleware).clone(),
        },
    ));

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
                        Ok(event) => {
                            if event.workspace_id.as_deref() == Some(&workspace_id) {
                                yield Ok(workspace_sse_event(event));
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            yield Ok(workspace_sse_event(bus.local_event_for_workspace(
                                &workspace_id,
                                WorkspaceEventPayload::Error {
                                    message: "event stream lagged; refresh the workspace snapshot".to_string(),
                                },
                            )));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
                _ = heartbeat.tick() => {
                    yield Ok(workspace_sse_event(bus.local_event_for_workspace(&workspace_id, WorkspaceEventPayload::Heartbeat)));
                }
            }
        }
    };

    Ok(Sse::new(stream))
}

async fn workspace_snapshot(
    state: &AppState,
    scope: &WorkspaceScope,
    session_id: Option<Uuid>,
) -> Result<WorkspaceSnapshot, AppError> {
    let session = match session_id {
        Some(id) => {
            state
                .sessions
                .load_session_in_workspace(scope.workspace.id, id)
                .await?
        }
        None => None,
    };
    let approvals = match session_id {
        Some(id) => state.sessions.list_approval_records(id).await?,
        None => Vec::new(),
    };

    Ok(WorkspaceSnapshot {
        organization: scope.organization.clone(),
        workspace: scope.workspace.clone(),
        current_membership: scope.membership.clone(),
        current_permissions: scope.permissions.clone(),
        session,
        tools: state.tools.definitions(),
        plugins: (*state.plugins).clone(),
        middleware: (*state.middleware).clone(),
        approvals,
    })
}

#[derive(Clone, Debug)]
struct WorkspaceScope {
    organization: Organization,
    membership: OrganizationMembership,
    permissions: Vec<PermissionKey>,
    workspace: Workspace,
}

async fn load_organization_context(
    state: &AppState,
    user: &auth::AuthenticatedUser,
    org_slug: &str,
) -> Result<OrganizationContext, AppError> {
    let organization = state
        .orgs
        .load_organization_by_slug(org_slug)
        .await
        .map_err(storage_app_error)?
        .ok_or_else(|| AppError::NotFound("organization not found".to_string()))?;
    let membership = state
        .orgs
        .load_membership(organization.id, user.0.id)
        .await
        .map_err(storage_app_error)?
        .ok_or_else(|| AppError::NotFound("organization not found".to_string()))?;
    let workspaces = state
        .orgs
        .list_workspaces(organization.id)
        .await
        .map_err(storage_app_error)?;
    let permissions = organization_permissions(state, &membership).await?;

    Ok(OrganizationContext {
        organization,
        membership,
        workspaces,
        permissions,
    })
}

async fn load_workspace_scope(
    state: &AppState,
    user: &auth::AuthenticatedUser,
    org_slug: &str,
    workspace_slug: &str,
) -> Result<WorkspaceScope, AppError> {
    let context = load_organization_context(state, user, org_slug).await?;
    let workspace = state
        .orgs
        .load_workspace_by_slug(context.organization.id, workspace_slug)
        .await
        .map_err(storage_app_error)?
        .ok_or_else(|| AppError::NotFound("workspace not found".to_string()))?;

    Ok(WorkspaceScope {
        organization: context.organization,
        membership: context.membership,
        permissions: context.permissions,
        workspace,
    })
}

async fn organization_permissions(
    state: &AppState,
    membership: &OrganizationMembership,
) -> Result<Vec<PermissionKey>, AppError> {
    let role = state
        .orgs
        .load_organization_role(membership.organization_id, membership.role_id)
        .await?
        .ok_or_else(|| AppError::Forbidden("organization role is not available".to_string()))?;
    if role.archived_at.is_some() {
        return Ok(Vec::new());
    }

    Ok(role.permissions)
}

async fn require_org_permission(
    state: &AppState,
    membership: &OrganizationMembership,
    permission: PermissionKey,
) -> Result<(), AppError> {
    let role = state
        .orgs
        .load_organization_role(membership.organization_id, membership.role_id)
        .await?
        .ok_or_else(|| AppError::Forbidden("organization role is not available".to_string()))?;
    if role.has_permission(&permission) {
        return Ok(());
    }

    Err(AppError::Forbidden(format!(
        "permission {} is required",
        permission.as_str()
    )))
}

fn storage_app_error(err: midgard_core::MidgardError) -> AppError {
    match err {
        midgard_core::MidgardError::Storage(message)
            if message.contains("already exists")
                || message.contains("last organization owner")
                || message.contains("last system owner")
                || message.contains("owner role must retain all permissions")
                || message.contains("does not belong to workspace") =>
        {
            AppError::Conflict(message)
        }
        midgard_core::MidgardError::Storage(message)
            if message.contains("is required") || message.contains("invalid slug") =>
        {
            AppError::BadRequest(message)
        }
        err => AppError::Midgard(err),
    }
}

fn slug_from_name(name: &str, fallback: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = false;
    for ch in name.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            last_was_separator = false;
        } else if !last_was_separator && !slug.is_empty() {
            slug.push('-');
            last_was_separator = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        fallback.to_string()
    } else {
        slug
    }
}

fn required_request_name(value: &str, label: &str) -> Result<String, AppError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest(format!("{label} is required")));
    }

    Ok(value.to_string())
}

fn spawn_agent_run(state: AppState, workspace_id: Uuid, session_id: Uuid, run_id: Uuid) {
    tokio::spawn(async move {
        let workspace_id_string = workspace_id.to_string();
        state.events.publish_for_workspace(
            workspace_id_string.clone(),
            WorkspaceEventPayload::AgentRunStarted {
                run_id: run_id.to_string(),
                session_id: session_id.to_string(),
            },
        );

        let session = match load_or_resumed_session(&state, workspace_id, session_id).await {
            Ok(session) => session,
            Err(err) => {
                state.events.publish_for_workspace(
                    workspace_id_string,
                    WorkspaceEventPayload::AgentRunFailed {
                        session_id: session_id.to_string(),
                        error: err.to_string(),
                    },
                );
                return;
            }
        };

        let bus = state.events.clone();
        let event_workspace_id = workspace_id.to_string();
        let result = state
            .runner
            .run_with_observer(session, move |event| {
                bus.publish_for_workspace(
                    event_workspace_id.clone(),
                    agent_run_event_payload(session_id, event),
                );
            })
            .await;

        let workspace_id_string = workspace_id.to_string();
        match result {
            Ok(result) => match state
                .sessions
                .save_session_in_workspace(workspace_id, result.session.clone())
                .await
            {
                Ok(session) => {
                    state.events.publish_for_workspace(
                        workspace_id_string,
                        WorkspaceEventPayload::AgentSessionUpdated {
                            session: Box::new(session),
                        },
                    );
                }
                Err(err) => {
                    state.events.publish_for_workspace(
                        workspace_id_string,
                        WorkspaceEventPayload::AgentRunFailed {
                            session_id: session_id.to_string(),
                            error: err.to_string(),
                        },
                    );
                }
            },
            Err(err) => {
                state.events.publish_for_workspace(
                    workspace_id_string,
                    WorkspaceEventPayload::AgentRunFailed {
                        session_id: session_id.to_string(),
                        error: err.to_string(),
                    },
                );
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

#[derive(Clone, Debug, Deserialize, TS)]
pub struct CreateOrganizationRequest {
    pub name: String,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub workspace_name: Option<String>,
    #[serde(default)]
    pub workspace_slug: Option<String>,
}

#[derive(Clone, Debug, Deserialize, TS)]
pub struct CreateWorkspaceRequest {
    pub name: String,
    #[serde(default)]
    pub slug: Option<String>,
}

#[derive(Clone, Debug, Deserialize, TS)]
pub struct UpdateWorkspaceRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub archived: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, TS)]
pub struct AddOrganizationMemberRequest {
    pub email: String,
    #[serde(default)]
    pub role: Option<OrganizationRole>,
    #[serde(default)]
    #[ts(type = "string | null")]
    pub role_id: Option<Uuid>,
}

#[derive(Clone, Debug, Deserialize, TS)]
pub struct UpdateOrganizationMemberRequest {
    #[serde(default)]
    pub role: Option<OrganizationRole>,
    #[serde(default)]
    #[ts(type = "string | null")]
    pub role_id: Option<Uuid>,
    #[serde(default)]
    pub active: Option<bool>,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct OrganizationMemberView {
    pub membership: OrganizationMembership,
    pub user: midgard_storage::AuthUser,
}

#[derive(Clone, Debug, Deserialize, TS)]
pub struct CreateRbacRoleRequest {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub permissions: Vec<PermissionKey>,
}

#[derive(Clone, Debug, Default, Deserialize, TS)]
pub struct UpdateRbacRoleRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub archived: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, TS)]
pub struct ReplaceRolePermissionsRequest {
    pub permissions: Vec<PermissionKey>,
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
pub(crate) enum AppError {
    Midgard(midgard_core::MidgardError),
    Unauthorized(String),
    Forbidden(String),
    BadRequest(String),
    NotFound(String),
    Conflict(String),
    Internal(String),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::Midgard(err) => write!(f, "{err}"),
            AppError::Unauthorized(message)
            | AppError::Forbidden(message)
            | AppError::BadRequest(message)
            | AppError::NotFound(message)
            | AppError::Conflict(message)
            | AppError::Internal(message) => f.write_str(message),
        }
    }
}

impl From<midgard_core::MidgardError> for AppError {
    fn from(value: midgard_core::MidgardError) -> Self {
        Self::Midgard(value)
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match self {
            AppError::Midgard(midgard_core::MidgardError::Configuration(message)) => {
                (StatusCode::BAD_REQUEST, message)
            }
            AppError::Midgard(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
            AppError::Unauthorized(message) => (StatusCode::UNAUTHORIZED, message),
            AppError::Forbidden(message) => (StatusCode::FORBIDDEN, message),
            AppError::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            AppError::NotFound(message) => (StatusCode::NOT_FOUND, message),
            AppError::Conflict(message) => (StatusCode::CONFLICT, message),
            AppError::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };

        (status, Json(ErrorResponse { error: message })).into_response()
    }
}

#[allow(dead_code)]
fn _tool_definitions_for_docs(registry: &ToolRegistry) -> Vec<ToolDefinition> {
    registry.definitions()
}
