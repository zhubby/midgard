mod auth;
mod memory;
mod org;
mod postgres;
mod rbac;
mod store;

pub use auth::{
    AuthSession, AuthUser, AuthUserRecord, AuthUserUpdate, NewAuthAuditEvent, NewAuthSession,
    NewUser, UserRole, generate_session_token, hash_password, normalize_email, parse_rfc3339_utc,
    session_token_hash, utc_now_rfc3339, verify_password,
};
pub use memory::{MemoryAgentSessionStore, MemoryAuthStore, MemoryOrganizationStore};
pub use org::{
    DockerRuntimeConfigView, KubernetesRuntimeConfigView, MiddlewareDesiredState,
    MiddlewareInstance, MiddlewareInstanceStatus, MiddlewareInstanceUpdate, NewMiddlewareInstance,
    NewOrganization, NewOrganizationMembership, NewWorkspace, Organization, OrganizationContext,
    OrganizationMembership, OrganizationMembershipUpdate, OrganizationRole, Workspace,
    WorkspaceRuntimeConfigRecord, WorkspaceRuntimeConfigSecret, WorkspaceRuntimeConfigStatus,
    WorkspaceRuntimeConfigView, WorkspaceRuntimeMode, WorkspaceUpdate,
};
pub use postgres::{
    PostgresAgentSessionStore, StoredAgentApprovalRecord, StoredAgentMessage, StoredAgentSession,
    StoredAuthAuditEvent, StoredAuthSession, StoredAuthUser, StoredMiddlewareInstance,
    StoredOrganization, StoredOrganizationMembership, StoredRbacRole, StoredRbacRolePermission,
    StoredWorkspace, connect_database, storage_models,
};
pub use rbac::{
    BuiltinRoleDefinition, NewRbacRole, ORG_ADMIN_BUILTIN, ORG_OPERATOR_BUILTIN, ORG_OWNER_BUILTIN,
    ORG_VIEWER_BUILTIN, PermissionCatalogItem, PermissionKey, RbacRole, RbacRoleUpdate,
    RbacScopeKind, SYSTEM_ADMIN_BUILTIN, SYSTEM_ADMIN_ROLE_ID, SYSTEM_OWNER_BUILTIN,
    SYSTEM_OWNER_ROLE_ID, SYSTEM_VIEWER_BUILTIN, SYSTEM_VIEWER_ROLE_ID, builtin_organization_roles,
    builtin_system_roles, legacy_organization_role_builtin_key, legacy_user_role_builtin_key,
    permission_catalog,
};
pub use store::{
    AgentSessionStore, AuthStore, OrganizationStore, SharedAgentSessionStore, SharedAuthStore,
    SharedOrganizationStore,
};
