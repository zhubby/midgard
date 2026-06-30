use std::collections::HashSet;

use midgard_core::{MidgardError, MidgardResult};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RbacScopeKind {
    System,
    Organization,
}

impl RbacScopeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            RbacScopeKind::System => "system",
            RbacScopeKind::Organization => "organization",
        }
    }

    pub fn from_storage(value: &str) -> MidgardResult<Self> {
        match value {
            "system" => Ok(Self::System),
            "organization" => Ok(Self::Organization),
            other => Err(MidgardError::Storage(format!(
                "unknown stored RBAC scope kind: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, TS)]
pub enum PermissionKey {
    #[serde(rename = "system.users.read")]
    SystemUsersRead,
    #[serde(rename = "system.users.manage")]
    SystemUsersManage,
    #[serde(rename = "system.roles.read")]
    SystemRolesRead,
    #[serde(rename = "system.roles.manage")]
    SystemRolesManage,
    #[serde(rename = "system.orgs.create")]
    SystemOrgsCreate,
    #[serde(rename = "system.orgs.read")]
    SystemOrgsRead,
    #[serde(rename = "org.read")]
    OrgRead,
    #[serde(rename = "org.manage")]
    OrgManage,
    #[serde(rename = "org.members.read")]
    OrgMembersRead,
    #[serde(rename = "org.members.manage")]
    OrgMembersManage,
    #[serde(rename = "org.roles.read")]
    OrgRolesRead,
    #[serde(rename = "org.roles.manage")]
    OrgRolesManage,
    #[serde(rename = "workspaces.read")]
    WorkspacesRead,
    #[serde(rename = "workspaces.manage")]
    WorkspacesManage,
    #[serde(rename = "workspace.read")]
    WorkspaceRead,
    #[serde(rename = "workspace.operate")]
    WorkspaceOperate,
}

impl PermissionKey {
    pub fn as_str(&self) -> &'static str {
        match self {
            PermissionKey::SystemUsersRead => "system.users.read",
            PermissionKey::SystemUsersManage => "system.users.manage",
            PermissionKey::SystemRolesRead => "system.roles.read",
            PermissionKey::SystemRolesManage => "system.roles.manage",
            PermissionKey::SystemOrgsCreate => "system.orgs.create",
            PermissionKey::SystemOrgsRead => "system.orgs.read",
            PermissionKey::OrgRead => "org.read",
            PermissionKey::OrgManage => "org.manage",
            PermissionKey::OrgMembersRead => "org.members.read",
            PermissionKey::OrgMembersManage => "org.members.manage",
            PermissionKey::OrgRolesRead => "org.roles.read",
            PermissionKey::OrgRolesManage => "org.roles.manage",
            PermissionKey::WorkspacesRead => "workspaces.read",
            PermissionKey::WorkspacesManage => "workspaces.manage",
            PermissionKey::WorkspaceRead => "workspace.read",
            PermissionKey::WorkspaceOperate => "workspace.operate",
        }
    }

    pub fn from_storage(value: &str) -> MidgardResult<Self> {
        match value {
            "system.users.read" => Ok(Self::SystemUsersRead),
            "system.users.manage" => Ok(Self::SystemUsersManage),
            "system.roles.read" => Ok(Self::SystemRolesRead),
            "system.roles.manage" => Ok(Self::SystemRolesManage),
            "system.orgs.create" => Ok(Self::SystemOrgsCreate),
            "system.orgs.read" => Ok(Self::SystemOrgsRead),
            "org.read" => Ok(Self::OrgRead),
            "org.manage" => Ok(Self::OrgManage),
            "org.members.read" => Ok(Self::OrgMembersRead),
            "org.members.manage" => Ok(Self::OrgMembersManage),
            "org.roles.read" => Ok(Self::OrgRolesRead),
            "org.roles.manage" => Ok(Self::OrgRolesManage),
            "workspaces.read" => Ok(Self::WorkspacesRead),
            "workspaces.manage" => Ok(Self::WorkspacesManage),
            "workspace.read" => Ok(Self::WorkspaceRead),
            "workspace.operate" => Ok(Self::WorkspaceOperate),
            other => Err(MidgardError::Storage(format!(
                "unknown stored permission key: {other}"
            ))),
        }
    }

    pub fn all() -> Vec<Self> {
        vec![
            Self::SystemUsersRead,
            Self::SystemUsersManage,
            Self::SystemRolesRead,
            Self::SystemRolesManage,
            Self::SystemOrgsCreate,
            Self::SystemOrgsRead,
            Self::OrgRead,
            Self::OrgManage,
            Self::OrgMembersRead,
            Self::OrgMembersManage,
            Self::OrgRolesRead,
            Self::OrgRolesManage,
            Self::WorkspacesRead,
            Self::WorkspacesManage,
            Self::WorkspaceRead,
            Self::WorkspaceOperate,
        ]
    }

    pub fn system_permissions() -> Vec<Self> {
        vec![
            Self::SystemUsersRead,
            Self::SystemUsersManage,
            Self::SystemRolesRead,
            Self::SystemRolesManage,
            Self::SystemOrgsCreate,
            Self::SystemOrgsRead,
        ]
    }

    pub fn organization_permissions() -> Vec<Self> {
        vec![
            Self::OrgRead,
            Self::OrgManage,
            Self::OrgMembersRead,
            Self::OrgMembersManage,
            Self::OrgRolesRead,
            Self::OrgRolesManage,
            Self::WorkspacesRead,
            Self::WorkspacesManage,
            Self::WorkspaceRead,
            Self::WorkspaceOperate,
        ]
    }

    pub fn validate_for_scope(
        scope_kind: &RbacScopeKind,
        permissions: &[Self],
    ) -> MidgardResult<()> {
        let allowed = match scope_kind {
            RbacScopeKind::System => Self::system_permissions(),
            RbacScopeKind::Organization => Self::organization_permissions(),
        }
        .into_iter()
        .collect::<HashSet<_>>();

        if let Some(permission) = permissions
            .iter()
            .find(|permission| !allowed.contains(permission))
        {
            return Err(MidgardError::Storage(format!(
                "permission {} is not valid for {} roles",
                permission.as_str(),
                scope_kind.as_str()
            )));
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct PermissionCatalogItem {
    pub key: PermissionKey,
    pub scope_kind: RbacScopeKind,
    pub group: String,
    pub label: String,
    pub description: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct RbacRole {
    #[ts(type = "string")]
    pub id: Uuid,
    pub scope_kind: RbacScopeKind,
    #[ts(type = "string")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<Uuid>,
    pub slug: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin_key: Option<String>,
    pub protected: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub permissions: Vec<PermissionKey>,
}

impl RbacRole {
    pub fn has_permission(&self, permission: &PermissionKey) -> bool {
        self.archived_at.is_none() && self.permissions.iter().any(|current| current == permission)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewRbacRole {
    pub scope_kind: RbacScopeKind,
    pub organization_id: Option<Uuid>,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub builtin_key: Option<String>,
    pub protected: bool,
    pub permissions: Vec<PermissionKey>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RbacRoleUpdate {
    pub name: Option<String>,
    pub description: Option<String>,
    pub archived: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct BuiltinRoleDefinition {
    pub id: Option<Uuid>,
    pub slug: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub builtin_key: &'static str,
    pub protected: bool,
    pub permissions: Vec<PermissionKey>,
}

pub const SYSTEM_OWNER_ROLE_ID: Uuid = Uuid::from_u128(0x00000000000000000000000000000101);
pub const SYSTEM_ADMIN_ROLE_ID: Uuid = Uuid::from_u128(0x00000000000000000000000000000102);
pub const SYSTEM_VIEWER_ROLE_ID: Uuid = Uuid::from_u128(0x00000000000000000000000000000103);

pub const SYSTEM_OWNER_BUILTIN: &str = "system_owner";
pub const SYSTEM_ADMIN_BUILTIN: &str = "system_admin";
pub const SYSTEM_VIEWER_BUILTIN: &str = "system_viewer";
pub const ORG_OWNER_BUILTIN: &str = "owner";
pub const ORG_ADMIN_BUILTIN: &str = "admin";
pub const ORG_OPERATOR_BUILTIN: &str = "operator";
pub const ORG_VIEWER_BUILTIN: &str = "viewer";

pub fn permission_catalog() -> Vec<PermissionCatalogItem> {
    vec![
        catalog(
            PermissionKey::SystemUsersRead,
            RbacScopeKind::System,
            "System",
            "Read users",
            "View system users.",
        ),
        catalog(
            PermissionKey::SystemUsersManage,
            RbacScopeKind::System,
            "System",
            "Manage users",
            "Create and update system users.",
        ),
        catalog(
            PermissionKey::SystemRolesRead,
            RbacScopeKind::System,
            "System",
            "Read system roles",
            "View system RBAC roles.",
        ),
        catalog(
            PermissionKey::SystemRolesManage,
            RbacScopeKind::System,
            "System",
            "Manage system roles",
            "Create, update, archive, and assign system role permissions.",
        ),
        catalog(
            PermissionKey::SystemOrgsCreate,
            RbacScopeKind::System,
            "System",
            "Create organizations",
            "Create new organizations.",
        ),
        catalog(
            PermissionKey::SystemOrgsRead,
            RbacScopeKind::System,
            "System",
            "Read organizations",
            "List organization contexts for the current user.",
        ),
        catalog(
            PermissionKey::OrgRead,
            RbacScopeKind::Organization,
            "Organization",
            "Read organization",
            "View organization details.",
        ),
        catalog(
            PermissionKey::OrgManage,
            RbacScopeKind::Organization,
            "Organization",
            "Manage organization",
            "Update organization settings.",
        ),
        catalog(
            PermissionKey::OrgMembersRead,
            RbacScopeKind::Organization,
            "Organization",
            "Read members",
            "View organization membership.",
        ),
        catalog(
            PermissionKey::OrgMembersManage,
            RbacScopeKind::Organization,
            "Organization",
            "Manage members",
            "Add, remove, and change organization members.",
        ),
        catalog(
            PermissionKey::OrgRolesRead,
            RbacScopeKind::Organization,
            "Organization",
            "Read roles",
            "View organization RBAC roles.",
        ),
        catalog(
            PermissionKey::OrgRolesManage,
            RbacScopeKind::Organization,
            "Organization",
            "Manage roles",
            "Create, update, archive, and assign organization role permissions.",
        ),
        catalog(
            PermissionKey::WorkspacesRead,
            RbacScopeKind::Organization,
            "Workspace",
            "Read workspaces",
            "View workspace lists.",
        ),
        catalog(
            PermissionKey::WorkspacesManage,
            RbacScopeKind::Organization,
            "Workspace",
            "Manage workspaces",
            "Create, rename, and archive workspaces.",
        ),
        catalog(
            PermissionKey::WorkspaceRead,
            RbacScopeKind::Organization,
            "Workspace",
            "Read workspace",
            "Read dashboard state, tools, plugins, and workspace events.",
        ),
        catalog(
            PermissionKey::WorkspaceOperate,
            RbacScopeKind::Organization,
            "Workspace",
            "Operate workspace",
            "Run agent workflows and decide approvals.",
        ),
    ]
}

pub fn builtin_system_roles() -> Vec<BuiltinRoleDefinition> {
    vec![
        BuiltinRoleDefinition {
            id: Some(SYSTEM_OWNER_ROLE_ID),
            slug: "owner",
            name: "System owner",
            description: "Full system administration.",
            builtin_key: SYSTEM_OWNER_BUILTIN,
            protected: true,
            permissions: PermissionKey::system_permissions(),
        },
        BuiltinRoleDefinition {
            id: Some(SYSTEM_ADMIN_ROLE_ID),
            slug: "admin",
            name: "System admin",
            description: "Manage users and create organizations.",
            builtin_key: SYSTEM_ADMIN_BUILTIN,
            protected: true,
            permissions: vec![
                PermissionKey::SystemUsersRead,
                PermissionKey::SystemUsersManage,
                PermissionKey::SystemRolesRead,
                PermissionKey::SystemOrgsCreate,
                PermissionKey::SystemOrgsRead,
            ],
        },
        BuiltinRoleDefinition {
            id: Some(SYSTEM_VIEWER_ROLE_ID),
            slug: "viewer",
            name: "System viewer",
            description: "Read-only system identity.",
            builtin_key: SYSTEM_VIEWER_BUILTIN,
            protected: true,
            permissions: vec![PermissionKey::SystemOrgsRead],
        },
    ]
}

pub fn builtin_organization_roles() -> Vec<BuiltinRoleDefinition> {
    vec![
        BuiltinRoleDefinition {
            id: None,
            slug: "owner",
            name: "Owner",
            description: "Full organization administration.",
            builtin_key: ORG_OWNER_BUILTIN,
            protected: true,
            permissions: PermissionKey::organization_permissions(),
        },
        BuiltinRoleDefinition {
            id: None,
            slug: "admin",
            name: "Admin",
            description: "Manage members and workspaces.",
            builtin_key: ORG_ADMIN_BUILTIN,
            protected: true,
            permissions: vec![
                PermissionKey::OrgRead,
                PermissionKey::OrgManage,
                PermissionKey::OrgMembersRead,
                PermissionKey::OrgMembersManage,
                PermissionKey::OrgRolesRead,
                PermissionKey::WorkspacesRead,
                PermissionKey::WorkspacesManage,
                PermissionKey::WorkspaceRead,
                PermissionKey::WorkspaceOperate,
            ],
        },
        BuiltinRoleDefinition {
            id: None,
            slug: "operator",
            name: "Operator",
            description: "Operate workspaces.",
            builtin_key: ORG_OPERATOR_BUILTIN,
            protected: true,
            permissions: vec![
                PermissionKey::OrgRead,
                PermissionKey::OrgMembersRead,
                PermissionKey::WorkspacesRead,
                PermissionKey::WorkspaceRead,
                PermissionKey::WorkspaceOperate,
            ],
        },
        BuiltinRoleDefinition {
            id: None,
            slug: "viewer",
            name: "Viewer",
            description: "Read workspace state.",
            builtin_key: ORG_VIEWER_BUILTIN,
            protected: true,
            permissions: vec![
                PermissionKey::OrgRead,
                PermissionKey::OrgMembersRead,
                PermissionKey::WorkspacesRead,
                PermissionKey::WorkspaceRead,
            ],
        },
    ]
}

pub fn legacy_user_role_builtin_key(role: &crate::auth::UserRole) -> &'static str {
    match role {
        crate::auth::UserRole::Admin => SYSTEM_OWNER_BUILTIN,
        crate::auth::UserRole::Operator => SYSTEM_ADMIN_BUILTIN,
        crate::auth::UserRole::Viewer => SYSTEM_VIEWER_BUILTIN,
    }
}

pub fn legacy_organization_role_builtin_key(role: &crate::org::OrganizationRole) -> &'static str {
    match role {
        crate::org::OrganizationRole::Owner => ORG_OWNER_BUILTIN,
        crate::org::OrganizationRole::Admin => ORG_ADMIN_BUILTIN,
        crate::org::OrganizationRole::Operator => ORG_OPERATOR_BUILTIN,
        crate::org::OrganizationRole::Viewer => ORG_VIEWER_BUILTIN,
    }
}

fn catalog(
    key: PermissionKey,
    scope_kind: RbacScopeKind,
    group: &str,
    label: &str,
    description: &str,
) -> PermissionCatalogItem {
    PermissionCatalogItem {
        key,
        scope_kind,
        group: group.to_string(),
        label: label.to_string(),
        description: description.to_string(),
    }
}
