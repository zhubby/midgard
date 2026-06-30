use std::{env, fs, path::PathBuf};

use midgard_agent::{
    AgentMessage, AgentRole, AgentRunEvent, AgentRunStatus, AgentSession, AgentToolCall,
    ApprovalDecision, ApprovalRecord, ApprovalStatus, PendingApproval,
};
use midgard_core::{CompletionStatus, RiskLevel};
use midgard_server::{
    AddOrganizationMemberRequest, AgentRunResponse, AgentSessionSummary, ApprovalResponse,
    AuthContext, CreateAuthUserRequest, CreateMiddlewareInstanceRequest, CreateOrganizationRequest,
    CreateRbacRoleRequest, CreateWorkspaceRequest, DashboardTone, LoginRequest, LogoutResponse,
    MiddlewareDashboardState, MiddlewareMetric, MiddlewareTimelineEvent, MiddlewareWorkload,
    OrganizationMemberView, PluginResponse, RegisterRequest, ReplaceRolePermissionsRequest,
    RunAccepted, UpdateAuthUserRequest, UpdateMiddlewareInstanceRequest,
    UpdateOrganizationMemberRequest, UpdateRbacRoleRequest, UpdateWorkspaceRequest, WorkspaceEvent,
    WorkspaceEventPayload, WorkspaceEventType, WorkspaceRuntimeConfigInput, WorkspaceSnapshot,
};
use midgard_storage::{
    AuthUser, DockerRuntimeConfigView, KubernetesRuntimeConfigView, MiddlewareDesiredState,
    MiddlewareInstance, MiddlewareInstanceStatus, Organization, OrganizationContext,
    OrganizationMembership, OrganizationRole, PermissionCatalogItem, PermissionKey, RbacRole,
    RbacScopeKind, UserRole, Workspace, WorkspaceRuntimeConfigStatus, WorkspaceRuntimeConfigView,
    WorkspaceRuntimeMode,
};
use midgard_tools::{ToolDefinition, ToolResult};
use ts_rs::{Config, TS};

#[test]
fn generated_protocol_typescript_is_current() {
    let path = protocol_path();
    let generated = protocol_typescript();

    if env::var("MIDGARD_UPDATE_PROTOCOL_TS").as_deref() == Ok("1") {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, generated).unwrap();
        return;
    }

    let current = fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        current, generated,
        "generated protocol types are stale; run `MIDGARD_UPDATE_PROTOCOL_TS=1 cargo test -p midgard-server --test type_exports`"
    );
}

fn protocol_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../midgard-web/lib/generated/protocol.ts")
}

fn protocol_typescript() -> String {
    let cfg = Config::default();
    let declarations = [
        RiskLevel::decl(&cfg),
        CompletionStatus::decl(&cfg),
        ToolDefinition::decl(&cfg),
        ToolResult::decl(&cfg),
        AgentRole::decl(&cfg),
        AgentToolCall::decl(&cfg),
        AgentMessage::decl(&cfg),
        AgentRunStatus::decl(&cfg),
        PendingApproval::decl(&cfg),
        ApprovalDecision::decl(&cfg),
        ApprovalStatus::decl(&cfg),
        ApprovalRecord::decl(&cfg),
        AgentSession::decl(&cfg),
        AgentRunEvent::decl(&cfg),
        UserRole::decl(&cfg),
        RbacScopeKind::decl(&cfg),
        PermissionKey::decl(&cfg),
        PermissionCatalogItem::decl(&cfg),
        RbacRole::decl(&cfg),
        AuthUser::decl(&cfg),
        AuthContext::decl(&cfg),
        LoginRequest::decl(&cfg),
        RegisterRequest::decl(&cfg),
        CreateAuthUserRequest::decl(&cfg),
        UpdateAuthUserRequest::decl(&cfg),
        LogoutResponse::decl(&cfg),
        OrganizationRole::decl(&cfg),
        Organization::decl(&cfg),
        OrganizationMembership::decl(&cfg),
        WorkspaceRuntimeMode::decl(&cfg),
        WorkspaceRuntimeConfigStatus::decl(&cfg),
        DockerRuntimeConfigView::decl(&cfg),
        KubernetesRuntimeConfigView::decl(&cfg),
        WorkspaceRuntimeConfigView::decl(&cfg),
        Workspace::decl(&cfg),
        OrganizationContext::decl(&cfg),
        WorkspaceRuntimeConfigInput::decl(&cfg),
        CreateOrganizationRequest::decl(&cfg),
        CreateWorkspaceRequest::decl(&cfg),
        UpdateWorkspaceRequest::decl(&cfg),
        MiddlewareDesiredState::decl(&cfg),
        MiddlewareInstanceStatus::decl(&cfg),
        MiddlewareInstance::decl(&cfg),
        CreateMiddlewareInstanceRequest::decl(&cfg),
        UpdateMiddlewareInstanceRequest::decl(&cfg),
        AddOrganizationMemberRequest::decl(&cfg),
        UpdateOrganizationMemberRequest::decl(&cfg),
        OrganizationMemberView::decl(&cfg),
        CreateRbacRoleRequest::decl(&cfg),
        UpdateRbacRoleRequest::decl(&cfg),
        ReplaceRolePermissionsRequest::decl(&cfg),
        PluginResponse::decl(&cfg),
        RunAccepted::decl(&cfg),
        AgentSessionSummary::decl(&cfg),
        AgentRunResponse::decl(&cfg),
        ApprovalResponse::decl(&cfg),
        DashboardTone::decl(&cfg),
        MiddlewareMetric::decl(&cfg),
        MiddlewareWorkload::decl(&cfg),
        MiddlewareTimelineEvent::decl(&cfg),
        MiddlewareDashboardState::decl(&cfg),
        WorkspaceSnapshot::decl(&cfg),
        WorkspaceEventType::decl(&cfg),
        WorkspaceEventPayload::decl(&cfg),
        WorkspaceEvent::decl(&cfg),
    ];

    let declarations = declarations
        .into_iter()
        .map(|declaration| format!("export {declaration}"))
        .collect::<Vec<_>>()
        .join("\n\n");

    format!(
        "/* eslint-disable */\n/* @generated by ts-rs. Do not edit manually. */\n\n{}\n",
        declarations
    )
}
