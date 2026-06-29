use async_trait::async_trait;
use midgard_agent::{AgentSession, ApprovalDecision, ApprovalRecord, PendingApproval};
use midgard_core::MidgardResult;
use std::sync::Arc;
use uuid::Uuid;

use crate::auth::{
    AuthSession, AuthUser, AuthUserRecord, AuthUserUpdate, NewAuthAuditEvent, NewAuthSession,
    NewUser,
};
use crate::org::{
    NewOrganization, NewOrganizationMembership, NewWorkspace, Organization, OrganizationContext,
    OrganizationMembership, OrganizationMembershipUpdate, Workspace, WorkspaceUpdate,
};

#[async_trait]
pub trait AgentSessionStore: Send + Sync {
    async fn create_session(&self, goal: String) -> MidgardResult<AgentSession> {
        self.create_session_in_workspace(Uuid::nil(), goal).await
    }

    async fn create_session_in_workspace(
        &self,
        workspace_id: Uuid,
        goal: String,
    ) -> MidgardResult<AgentSession>;

    async fn append_user_message(&self, id: Uuid, message: String) -> MidgardResult<AgentSession> {
        self.append_user_message_in_workspace(Uuid::nil(), id, message)
            .await
    }

    async fn append_user_message_in_workspace(
        &self,
        workspace_id: Uuid,
        id: Uuid,
        message: String,
    ) -> MidgardResult<AgentSession>;

    async fn load_session(&self, id: Uuid) -> MidgardResult<Option<AgentSession>>;

    async fn load_session_in_workspace(
        &self,
        workspace_id: Uuid,
        id: Uuid,
    ) -> MidgardResult<Option<AgentSession>>;

    async fn save_session(&self, session: AgentSession) -> MidgardResult<AgentSession> {
        self.save_session_in_workspace(Uuid::nil(), session).await
    }

    async fn save_session_in_workspace(
        &self,
        workspace_id: Uuid,
        session: AgentSession,
    ) -> MidgardResult<AgentSession>;

    async fn list_approval_records(&self, session_id: Uuid) -> MidgardResult<Vec<ApprovalRecord>>;
    async fn record_approval_decision(
        &self,
        session_id: Uuid,
        approval: PendingApproval,
        decision: ApprovalDecision,
        actor: String,
        reason: Option<String>,
    ) -> MidgardResult<ApprovalRecord>;
}

pub type SharedAgentSessionStore = Arc<dyn AgentSessionStore>;

#[async_trait]
pub trait AuthStore: Send + Sync {
    async fn create_user(&self, user: NewUser) -> MidgardResult<AuthUser>;
    async fn list_users(&self) -> MidgardResult<Vec<AuthUser>>;
    async fn load_user_by_id(&self, id: Uuid) -> MidgardResult<Option<AuthUser>>;
    async fn load_user_by_email(&self, email_lower: &str) -> MidgardResult<Option<AuthUserRecord>>;
    async fn update_user(
        &self,
        id: Uuid,
        update: AuthUserUpdate,
    ) -> MidgardResult<Option<AuthUser>>;
    async fn create_auth_session(&self, session: NewAuthSession) -> MidgardResult<AuthSession>;
    async fn load_user_by_session(
        &self,
        token_hash: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> MidgardResult<Option<AuthUser>>;
    async fn revoke_auth_session(&self, token_hash: &str, revoked_at: String) -> MidgardResult<()>;
    async fn record_auth_audit_event(&self, event: NewAuthAuditEvent) -> MidgardResult<()>;
}

pub type SharedAuthStore = Arc<dyn AuthStore>;

#[async_trait]
pub trait OrganizationStore: Send + Sync {
    async fn create_organization(
        &self,
        organization: NewOrganization,
    ) -> MidgardResult<Organization>;
    async fn load_organization_by_slug(&self, slug: &str) -> MidgardResult<Option<Organization>>;
    async fn list_contexts_for_user(
        &self,
        user_id: Uuid,
    ) -> MidgardResult<Vec<OrganizationContext>>;
    async fn load_membership(
        &self,
        organization_id: Uuid,
        user_id: Uuid,
    ) -> MidgardResult<Option<OrganizationMembership>>;
    async fn create_membership(
        &self,
        membership: NewOrganizationMembership,
    ) -> MidgardResult<OrganizationMembership>;
    async fn update_membership(
        &self,
        organization_id: Uuid,
        user_id: Uuid,
        update: OrganizationMembershipUpdate,
    ) -> MidgardResult<Option<OrganizationMembership>>;
    async fn create_workspace(&self, workspace: NewWorkspace) -> MidgardResult<Workspace>;
    async fn load_workspace_by_slug(
        &self,
        organization_id: Uuid,
        slug: &str,
    ) -> MidgardResult<Option<Workspace>>;
    async fn list_workspaces(&self, organization_id: Uuid) -> MidgardResult<Vec<Workspace>>;
    async fn update_workspace(
        &self,
        organization_id: Uuid,
        slug: &str,
        update: WorkspaceUpdate,
    ) -> MidgardResult<Option<Workspace>>;
}

pub type SharedOrganizationStore = Arc<dyn OrganizationStore>;
