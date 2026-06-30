use midgard_core::{MidgardError, MidgardResult};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::PermissionKey;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum OrganizationRole {
    Owner,
    Admin,
    Operator,
    Viewer,
}

impl OrganizationRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            OrganizationRole::Owner => "owner",
            OrganizationRole::Admin => "admin",
            OrganizationRole::Operator => "operator",
            OrganizationRole::Viewer => "viewer",
        }
    }

    pub fn from_storage(value: &str) -> MidgardResult<Self> {
        match value {
            "owner" => Ok(Self::Owner),
            "admin" => Ok(Self::Admin),
            "operator" => Ok(Self::Operator),
            "viewer" => Ok(Self::Viewer),
            other => Err(MidgardError::Storage(format!(
                "unknown stored organization role: {other}"
            ))),
        }
    }

    pub fn can_manage_org(&self) -> bool {
        matches!(self, OrganizationRole::Owner | OrganizationRole::Admin)
    }

    pub fn can_operate(&self) -> bool {
        matches!(
            self,
            OrganizationRole::Owner | OrganizationRole::Admin | OrganizationRole::Operator
        )
    }

    pub fn is_owner(&self) -> bool {
        matches!(self, OrganizationRole::Owner)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRuntimeMode {
    Docker,
    Kubernetes,
}

impl WorkspaceRuntimeMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkspaceRuntimeMode::Docker => "docker",
            WorkspaceRuntimeMode::Kubernetes => "kubernetes",
        }
    }

    pub fn from_storage(value: &str) -> MidgardResult<Self> {
        match value {
            "docker" => Ok(Self::Docker),
            "kubernetes" => Ok(Self::Kubernetes),
            other => Err(MidgardError::Storage(format!(
                "unknown stored workspace runtime mode: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRuntimeConfigStatus {
    Unconfigured,
    Configured,
    Invalid,
}

impl WorkspaceRuntimeConfigStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            WorkspaceRuntimeConfigStatus::Unconfigured => "unconfigured",
            WorkspaceRuntimeConfigStatus::Configured => "configured",
            WorkspaceRuntimeConfigStatus::Invalid => "invalid",
        }
    }

    pub fn from_storage(value: &str) -> MidgardResult<Self> {
        match value {
            "unconfigured" => Ok(Self::Unconfigured),
            "configured" => Ok(Self::Configured),
            "invalid" => Ok(Self::Invalid),
            other => Err(MidgardError::Storage(format!(
                "unknown stored workspace runtime config status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct DockerRuntimeConfigView {
    pub configured: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint_host: Option<String>,
    #[serde(default)]
    pub insecure_allowed: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct KubernetesRuntimeConfigView {
    pub configured: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_server_host: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct WorkspaceRuntimeConfigView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<WorkspaceRuntimeMode>,
    pub status: WorkspaceRuntimeConfigStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker: Option<DockerRuntimeConfigView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kubernetes: Option<KubernetesRuntimeConfigView>,
}

impl Default for WorkspaceRuntimeConfigView {
    fn default() -> Self {
        Self {
            mode: None,
            status: WorkspaceRuntimeConfigStatus::Unconfigured,
            updated_at: None,
            docker: None,
            kubernetes: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRuntimeConfigRecord {
    pub view: WorkspaceRuntimeConfigView,
    pub ciphertext: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceRuntimeConfigSecret {
    pub workspace_id: Uuid,
    pub view: WorkspaceRuntimeConfigView,
    pub ciphertext: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct Organization {
    #[ts(type = "string")]
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    #[ts(type = "string")]
    pub created_by_user_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct OrganizationMembership {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub organization_id: Uuid,
    #[ts(type = "string")]
    pub user_id: Uuid,
    pub role: OrganizationRole,
    #[ts(type = "string")]
    pub role_id: Uuid,
    pub active: bool,
    pub joined_at: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct Workspace {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub organization_id: Uuid,
    pub slug: String,
    pub name: String,
    pub runtime_config: WorkspaceRuntimeConfigView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
pub struct OrganizationContext {
    pub organization: Organization,
    pub membership: OrganizationMembership,
    pub workspaces: Vec<Workspace>,
    pub permissions: Vec<PermissionKey>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewOrganization {
    pub slug: String,
    pub name: String,
    pub created_by_user_id: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewOrganizationMembership {
    pub organization_id: Uuid,
    pub user_id: Uuid,
    pub role: OrganizationRole,
    pub role_id: Option<Uuid>,
    pub active: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrganizationMembershipUpdate {
    pub role: Option<OrganizationRole>,
    pub role_id: Option<Uuid>,
    pub active: Option<bool>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewWorkspace {
    pub organization_id: Uuid,
    pub slug: String,
    pub name: String,
    pub runtime_config: Option<WorkspaceRuntimeConfigRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceUpdate {
    pub name: Option<String>,
    pub archived: Option<bool>,
    pub runtime_config: Option<WorkspaceRuntimeConfigRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MiddlewareDesiredState {
    Enabled,
    Disabled,
}

impl MiddlewareDesiredState {
    pub fn as_str(&self) -> &'static str {
        match self {
            MiddlewareDesiredState::Enabled => "enabled",
            MiddlewareDesiredState::Disabled => "disabled",
        }
    }

    pub fn from_storage(value: &str) -> MidgardResult<Self> {
        match value {
            "enabled" => Ok(Self::Enabled),
            "disabled" => Ok(Self::Disabled),
            other => Err(MidgardError::Storage(format!(
                "unknown stored middleware desired state: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MiddlewareInstanceStatus {
    Pending,
    Running,
    Degraded,
    Stopped,
}

impl MiddlewareInstanceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            MiddlewareInstanceStatus::Pending => "pending",
            MiddlewareInstanceStatus::Running => "running",
            MiddlewareInstanceStatus::Degraded => "degraded",
            MiddlewareInstanceStatus::Stopped => "stopped",
        }
    }

    pub fn from_storage(value: &str) -> MidgardResult<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "running" => Ok(Self::Running),
            "degraded" => Ok(Self::Degraded),
            "stopped" => Ok(Self::Stopped),
            other => Err(MidgardError::Storage(format!(
                "unknown stored middleware instance status: {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
pub struct MiddlewareInstance {
    #[ts(type = "string")]
    pub id: Uuid,
    #[ts(type = "string")]
    pub workspace_id: Uuid,
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub desired_state: MiddlewareDesiredState,
    pub status: MiddlewareInstanceStatus,
    #[serde(default)]
    #[ts(type = "unknown")]
    pub config: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NewMiddlewareInstance {
    pub workspace_id: Uuid,
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub desired_state: MiddlewareDesiredState,
    pub status: MiddlewareInstanceStatus,
    pub config: serde_json::Value,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MiddlewareInstanceUpdate {
    pub desired_state: Option<MiddlewareDesiredState>,
    pub status: Option<MiddlewareInstanceStatus>,
    pub config: Option<serde_json::Value>,
    pub archived: Option<bool>,
}
