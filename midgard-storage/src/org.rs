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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceUpdate {
    pub name: Option<String>,
    pub archived: Option<bool>,
}
