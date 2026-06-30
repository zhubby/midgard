mod codec;
mod models;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use midgard_agent::{
    AgentMessage, AgentSession, ApprovalDecision, ApprovalRecord, PendingApproval,
};
use midgard_core::{MidgardError, MidgardResult};
use toasty::{sql, stmt, Db, Executor};
use uuid::Uuid;

use crate::{
    auth::{
        normalize_email, parse_rfc3339_utc, utc_now_rfc3339, AuthSession, AuthUser, AuthUserRecord,
        AuthUserUpdate, NewAuthAuditEvent, NewAuthSession, NewUser,
    },
    org::{
        MiddlewareDesiredState, MiddlewareInstance, MiddlewareInstanceStatus,
        MiddlewareInstanceUpdate, NewMiddlewareInstance, NewOrganization,
        NewOrganizationMembership, NewWorkspace, Organization, OrganizationContext,
        OrganizationMembership, OrganizationMembershipUpdate, OrganizationRole, Workspace,
        WorkspaceRuntimeConfigStatus, WorkspaceRuntimeConfigView, WorkspaceUpdate,
    },
    rbac::{
        builtin_organization_roles, legacy_organization_role_builtin_key,
        legacy_user_role_builtin_key, NewRbacRole, PermissionKey, RbacRole, RbacRoleUpdate,
        RbacScopeKind, ORG_OWNER_BUILTIN, SYSTEM_OWNER_BUILTIN,
    },
    store::{AgentSessionStore, AuthStore, OrganizationStore},
};

use codec::{
    agent_message_from_row, approval_record_from_row, bool_from_value, i64_from_value, json_string,
    optional_json, optional_pending_approval, optional_string_from_value, optional_tool_calls_json,
    risk_level_to_storage, role_to_storage, status_from_storage, status_to_storage, storage_error,
    string_from_value, uuid_from_value,
};

pub use models::{
    storage_models, StoredAgentApprovalRecord, StoredAgentMessage, StoredAgentSession,
    StoredAuthAuditEvent, StoredAuthSession, StoredAuthUser, StoredMiddlewareInstance,
    StoredOrganization, StoredOrganizationMembership, StoredRbacRole, StoredRbacRolePermission,
    StoredWorkspace,
};

#[derive(Clone)]
pub struct PostgresAgentSessionStore {
    db: Db,
}

impl PostgresAgentSessionStore {
    pub async fn connect(database_url: &str) -> MidgardResult<Self> {
        let db = connect_database(database_url).await?;
        Ok(Self::new(db))
    }

    pub fn new(db: Db) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &Db {
        &self.db
    }
}

pub async fn connect_database(database_url: &str) -> MidgardResult<Db> {
    toasty::Db::builder()
        .models(storage_models())
        .connect(database_url)
        .await
        .map_err(|err| MidgardError::Storage(format!("failed to connect database: {err}")))
}

#[async_trait]
impl AgentSessionStore for PostgresAgentSessionStore {
    async fn create_session_in_workspace(
        &self,
        workspace_id: Uuid,
        goal: String,
    ) -> MidgardResult<AgentSession> {
        let mut session = AgentSession::new(goal);
        session.workspace_id = workspace_id_option(workspace_id);
        self.save_session_in_workspace(workspace_id, session).await
    }

    async fn append_user_message_in_workspace(
        &self,
        workspace_id: Uuid,
        id: Uuid,
        message: String,
    ) -> MidgardResult<AgentSession> {
        let mut session = match self.load_session_in_workspace(workspace_id, id).await? {
            Some(session) => session,
            None => {
                if self.load_session(id).await?.is_some() {
                    return Err(MidgardError::Storage(format!(
                        "session {id} does not belong to workspace {workspace_id}"
                    )));
                }
                let mut session = AgentSession::new("resumed session");
                session.id = id;
                session.workspace_id = workspace_id_option(workspace_id);
                session
            }
        };

        session.workspace_id = workspace_id_option(workspace_id);
        session.messages.push(AgentMessage::user(message));
        self.save_session_in_workspace(workspace_id, session).await
    }

    async fn load_session(&self, id: Uuid) -> MidgardResult<Option<AgentSession>> {
        let mut db = self.db.clone();
        load_session_with_executor(&mut db, id, None).await
    }

    async fn load_session_in_workspace(
        &self,
        workspace_id: Uuid,
        id: Uuid,
    ) -> MidgardResult<Option<AgentSession>> {
        let mut db = self.db.clone();
        load_session_with_executor(&mut db, id, Some(workspace_id)).await
    }

    async fn list_sessions_in_workspace(
        &self,
        workspace_id: Uuid,
    ) -> MidgardResult<Vec<AgentSession>> {
        let mut db = self.db.clone();
        let rows =
            sql::query("SELECT id FROM agent_sessions WHERE workspace_id = $1 ORDER BY id ASC")
                .bind(workspace_id)
                .column_types([stmt::Type::Uuid])
                .exec(&mut db)
                .await
                .map_err(storage_error)?;

        let mut sessions = Vec::with_capacity(rows.len());
        for row in rows {
            let record = row.into_record();
            if let Some(session) = load_session_with_executor(
                &mut db,
                uuid_from_value(&record[0])?,
                Some(workspace_id),
            )
            .await?
            {
                sessions.push(session);
            }
        }

        Ok(sessions)
    }

    async fn save_session_in_workspace(
        &self,
        workspace_id: Uuid,
        session: AgentSession,
    ) -> MidgardResult<AgentSession> {
        let mut session = session;
        session.workspace_id = workspace_id_option(workspace_id);
        let mut db = self.db.clone();
        let mut tx = db.transaction().await.map_err(storage_error)?;

        upsert_session(&mut tx, workspace_id, &session).await?;
        if let Some(approval) = &session.pending_approval {
            if approval.approved.is_none() {
                upsert_pending_approval_record(&mut tx, session.id, approval).await?;
            }
        }
        sql::statement("DELETE FROM agent_messages WHERE session_id = $1")
            .bind(session.id)
            .exec(&mut tx)
            .await
            .map_err(storage_error)?;

        for (sequence, message) in session.messages.iter().enumerate() {
            insert_message(&mut tx, session.id, sequence as i64, message).await?;
        }

        tx.commit().await.map_err(storage_error)?;
        Ok(session)
    }

    async fn list_approval_records(&self, session_id: Uuid) -> MidgardResult<Vec<ApprovalRecord>> {
        let mut db = self.db.clone();
        list_approval_records_with_executor(&mut db, session_id).await
    }

    async fn record_approval_decision(
        &self,
        session_id: Uuid,
        approval: PendingApproval,
        decision: ApprovalDecision,
        actor: String,
        reason: Option<String>,
    ) -> MidgardResult<ApprovalRecord> {
        let mut db = self.db.clone();
        let mut tx = db.transaction().await.map_err(storage_error)?;

        upsert_pending_approval_record(&mut tx, session_id, &approval).await?;
        let mut record =
            load_approval_record_with_executor(&mut tx, session_id, approval.id).await?;
        record.record_decision(decision, actor, reason);
        update_approval_record_decision(&mut tx, &record).await?;
        tx.commit().await.map_err(storage_error)?;

        Ok(record)
    }
}

#[async_trait]
impl AuthStore for PostgresAgentSessionStore {
    async fn create_user(&self, user: NewUser) -> MidgardResult<AuthUser> {
        let email_lower = normalize_email(&user.email);
        if email_lower.is_empty() {
            return Err(MidgardError::Storage("user email is required".to_string()));
        }
        if self.load_user_by_email(&email_lower).await?.is_some() {
            return Err(MidgardError::Storage(format!(
                "user already exists: {email_lower}"
            )));
        }
        let system_role_id = match user.system_role_id {
            Some(id) => {
                let role = self
                    .load_system_role(id)
                    .await?
                    .ok_or_else(|| MidgardError::Storage("system role not found".to_string()))?;
                if role.archived_at.is_some() {
                    return Err(MidgardError::Storage(
                        "archived system role cannot be assigned".to_string(),
                    ));
                }
                id
            }
            None => {
                let builtin_key = legacy_user_role_builtin_key(&user.role);
                self.load_system_role_by_builtin_key(builtin_key)
                    .await?
                    .ok_or_else(|| {
                        MidgardError::Storage(format!(
                            "builtin system role not found: {builtin_key}"
                        ))
                    })?
                    .id
            }
        };

        let now = utc_now_rfc3339();
        let auth_user = AuthUser {
            id: Uuid::new_v4(),
            email: email_lower,
            display_name: user.display_name.trim().to_string(),
            role: user.role,
            system_role_id,
            active: user.active,
            created_at: now.clone(),
            updated_at: now,
            last_login_at: None,
        };

        let mut db = self.db.clone();
        sql::statement(
            "INSERT INTO users
                (id, email_lower, display_name, role, system_role_id, password_hash, active, created_at, updated_at, last_login_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(auth_user.id)
        .bind(auth_user.email.clone())
        .bind(auth_user.display_name.clone())
        .bind(auth_user.role.as_str())
        .bind(auth_user.system_role_id)
        .bind(user.password_hash)
        .bind(auth_user.active)
        .bind(auth_user.created_at.clone())
        .bind(auth_user.updated_at.clone())
        .bind(auth_user.last_login_at.clone())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        Ok(auth_user)
    }

    async fn list_users(&self) -> MidgardResult<Vec<AuthUser>> {
        let mut db = self.db.clone();
        let rows = sql::query(
            "SELECT id, email_lower, display_name, role, system_role_id, password_hash, active, created_at, updated_at, last_login_at
             FROM users
             ORDER BY email_lower ASC",
        )
        .column_types(auth_user_column_types())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        rows.into_iter()
            .map(auth_user_record_from_row)
            .map(|record| record.map(|record| record.user))
            .collect::<MidgardResult<Vec<_>>>()
    }

    async fn load_user_by_id(&self, id: Uuid) -> MidgardResult<Option<AuthUser>> {
        let mut db = self.db.clone();
        let rows = sql::query(
            "SELECT id, email_lower, display_name, role, system_role_id, password_hash, active, created_at, updated_at, last_login_at
             FROM users
             WHERE id = $1",
        )
        .bind(id)
        .column_types(auth_user_column_types())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        rows.into_iter()
            .next()
            .map(auth_user_record_from_row)
            .transpose()
            .map(|record| record.map(|record| record.user))
    }

    async fn load_user_by_email(&self, email_lower: &str) -> MidgardResult<Option<AuthUserRecord>> {
        let mut db = self.db.clone();
        let email_lower = normalize_email(email_lower);
        let rows = sql::query(
            "SELECT id, email_lower, display_name, role, system_role_id, password_hash, active, created_at, updated_at, last_login_at
             FROM users
             WHERE email_lower = $1",
        )
        .bind(email_lower)
        .column_types(auth_user_column_types())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        rows.into_iter()
            .next()
            .map(auth_user_record_from_row)
            .transpose()
    }

    async fn update_user(
        &self,
        id: Uuid,
        update: AuthUserUpdate,
    ) -> MidgardResult<Option<AuthUser>> {
        let Some(current) = self.load_user_by_id(id).await? else {
            return Ok(None);
        };
        let next_system_role_id = match update.system_role_id {
            Some(role_id) => {
                let role = self
                    .load_system_role(role_id)
                    .await?
                    .ok_or_else(|| MidgardError::Storage("system role not found".to_string()))?;
                if role.archived_at.is_some() {
                    return Err(MidgardError::Storage(
                        "archived system role cannot be assigned".to_string(),
                    ));
                }
                role_id
            }
            None => current.system_role_id,
        };
        let next_active = update.active.unwrap_or(current.active);
        let removes_owner = current.active
            && is_system_owner_role(self, current.system_role_id).await?
            && (!next_active || !is_system_owner_role(self, next_system_role_id).await?);
        if removes_owner && active_system_owner_count(self).await? <= 1 {
            return Err(MidgardError::Storage(
                "cannot remove or demote the last system owner".to_string(),
            ));
        }

        let mut db = self.db.clone();
        let updated_at = utc_now_rfc3339();
        sql::statement(
            "UPDATE users
             SET display_name = COALESCE($2, display_name),
                 role = COALESCE($3, role),
                 system_role_id = $4,
                 password_hash = COALESCE($5, password_hash),
                 active = COALESCE($6, active),
                 updated_at = $7
             WHERE id = $1",
        )
        .bind(id)
        .bind(update.display_name.map(|value| value.trim().to_string()))
        .bind(update.role.map(|role| role.as_str().to_string()))
        .bind(next_system_role_id)
        .bind(update.password_hash)
        .bind(update.active)
        .bind(updated_at)
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        self.load_user_by_id(id).await
    }

    async fn create_auth_session(&self, session: NewAuthSession) -> MidgardResult<AuthSession> {
        let auth_session = AuthSession {
            id: Uuid::new_v4(),
            user_id: session.user_id,
            token_hash: session.token_hash,
            created_at: session.created_at,
            expires_at: session.expires_at,
            revoked_at: None,
            user_agent: session.user_agent,
            ip_address: session.ip_address,
        };

        let mut db = self.db.clone();
        let mut tx = db.transaction().await.map_err(storage_error)?;
        sql::statement(
            "INSERT INTO auth_sessions
                (id, user_id, token_hash, created_at, expires_at, revoked_at, user_agent, ip_address)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(auth_session.id)
        .bind(auth_session.user_id)
        .bind(auth_session.token_hash.clone())
        .bind(auth_session.created_at.clone())
        .bind(auth_session.expires_at.clone())
        .bind(auth_session.revoked_at.clone())
        .bind(auth_session.user_agent.clone())
        .bind(auth_session.ip_address.clone())
        .exec(&mut tx)
        .await
        .map_err(storage_error)?;

        sql::statement(
            "UPDATE users
             SET last_login_at = $1, updated_at = $1
             WHERE id = $2",
        )
        .bind(auth_session.created_at.clone())
        .bind(auth_session.user_id)
        .exec(&mut tx)
        .await
        .map_err(storage_error)?;
        tx.commit().await.map_err(storage_error)?;

        Ok(auth_session)
    }

    async fn load_user_by_session(
        &self,
        token_hash: &str,
        now: DateTime<Utc>,
    ) -> MidgardResult<Option<AuthUser>> {
        let mut db = self.db.clone();
        let rows = sql::query(
            "SELECT u.id, u.email_lower, u.display_name, u.role, u.system_role_id, u.password_hash, u.active,
                    u.created_at, u.updated_at, u.last_login_at, s.expires_at
             FROM auth_sessions s
             JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = $1 AND s.revoked_at IS NULL",
        )
        .bind(token_hash.to_string())
        .column_types([
            stmt::Type::Uuid,
            stmt::Type::String,
            stmt::Type::String,
            stmt::Type::String,
            stmt::Type::Uuid,
            stmt::Type::String,
            stmt::Type::Bool,
            stmt::Type::String,
            stmt::Type::String,
            stmt::Type::String,
            stmt::Type::String,
        ])
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        let record = row.into_record();
        let expires_at = string_from_value(&record[10])?;
        if parse_rfc3339_utc(expires_at)? <= now {
            return Ok(None);
        }
        let auth_user = auth_user_record_from_record(&record[0..10])?.user;
        if !auth_user.active {
            return Ok(None);
        }

        Ok(Some(auth_user))
    }

    async fn revoke_auth_session(&self, token_hash: &str, revoked_at: String) -> MidgardResult<()> {
        let mut db = self.db.clone();
        sql::statement(
            "UPDATE auth_sessions
             SET revoked_at = $2
             WHERE token_hash = $1 AND revoked_at IS NULL",
        )
        .bind(token_hash.to_string())
        .bind(revoked_at)
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        Ok(())
    }

    async fn record_auth_audit_event(&self, event: NewAuthAuditEvent) -> MidgardResult<()> {
        let mut db = self.db.clone();
        sql::statement(
            "INSERT INTO auth_audit_events
                (id, user_id, event_type, email_lower, occurred_at, ip_address, user_agent, detail_json)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(Uuid::new_v4())
        .bind(event.user_id)
        .bind(event.event_type)
        .bind(event.email_lower)
        .bind(event.occurred_at)
        .bind(event.ip_address)
        .bind(event.user_agent)
        .bind(event.detail_json)
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        Ok(())
    }

    async fn list_system_roles(&self) -> MidgardResult<Vec<RbacRole>> {
        list_roles(self, RbacScopeKind::System, None).await
    }

    async fn load_system_role(&self, id: Uuid) -> MidgardResult<Option<RbacRole>> {
        load_role_by_id(self, RbacScopeKind::System, None, id).await
    }

    async fn load_system_role_by_builtin_key(
        &self,
        builtin_key: &str,
    ) -> MidgardResult<Option<RbacRole>> {
        load_role_by_builtin_key(self, RbacScopeKind::System, None, builtin_key).await
    }

    async fn create_system_role(&self, role: NewRbacRole) -> MidgardResult<RbacRole> {
        if role.scope_kind != RbacScopeKind::System || role.organization_id.is_some() {
            return Err(MidgardError::Storage(
                "system role must use system scope".to_string(),
            ));
        }
        create_role(self, role).await
    }

    async fn update_system_role(
        &self,
        id: Uuid,
        update: RbacRoleUpdate,
    ) -> MidgardResult<Option<RbacRole>> {
        update_role(self, RbacScopeKind::System, None, id, update).await
    }

    async fn replace_system_role_permissions(
        &self,
        id: Uuid,
        permissions: Vec<PermissionKey>,
    ) -> MidgardResult<Option<RbacRole>> {
        replace_role_permissions(self, RbacScopeKind::System, None, id, permissions).await
    }
}

#[async_trait]
impl OrganizationStore for PostgresAgentSessionStore {
    async fn create_organization(
        &self,
        organization: NewOrganization,
    ) -> MidgardResult<Organization> {
        let slug = normalize_slug(&organization.slug)?;
        let name = required_name(&organization.name, "organization name")?;
        if self.load_organization_by_slug(&slug).await?.is_some() {
            return Err(MidgardError::Storage(format!(
                "organization slug already exists: {slug}"
            )));
        }

        let now = utc_now_rfc3339();
        let created = Organization {
            id: Uuid::new_v4(),
            slug,
            name,
            created_by_user_id: organization.created_by_user_id,
            archived_at: None,
            created_at: now.clone(),
            updated_at: now,
        };

        let mut db = self.db.clone();
        sql::statement(
            "INSERT INTO organizations
                (id, slug, name, created_by_user_id, archived_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(created.id)
        .bind(created.slug.clone())
        .bind(created.name.clone())
        .bind(created.created_by_user_id)
        .bind(created.archived_at.clone())
        .bind(created.created_at.clone())
        .bind(created.updated_at.clone())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;
        seed_builtin_organization_roles(self, created.id).await?;

        Ok(created)
    }

    async fn load_organization_by_slug(&self, slug: &str) -> MidgardResult<Option<Organization>> {
        let slug = normalize_slug(slug)?;
        let mut db = self.db.clone();
        let rows = sql::query(
            "SELECT id, slug, name, created_by_user_id, archived_at, created_at, updated_at
             FROM organizations
             WHERE slug = $1 AND archived_at IS NULL",
        )
        .bind(slug)
        .column_types(organization_column_types())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        rows.into_iter()
            .next()
            .map(organization_from_row)
            .transpose()
    }

    async fn list_contexts_for_user(
        &self,
        user_id: Uuid,
    ) -> MidgardResult<Vec<OrganizationContext>> {
        let mut db = self.db.clone();
        let rows = sql::query(
            "SELECT
                o.id, o.slug, o.name, o.created_by_user_id, o.archived_at, o.created_at, o.updated_at,
                m.id, m.organization_id, m.user_id, m.role, m.role_id, m.active, m.joined_at, m.created_at, m.updated_at
             FROM organization_memberships m
             JOIN organizations o ON o.id = m.organization_id
             WHERE m.user_id = $1 AND m.active = TRUE AND o.archived_at IS NULL
             ORDER BY o.slug ASC",
        )
        .bind(user_id)
        .column_types(organization_context_column_types())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        let mut contexts = Vec::with_capacity(rows.len());
        for row in rows {
            let record = row.into_record();
            let organization = organization_from_record(&record[0..7])?;
            let membership = membership_from_record(&record[7..16])?;
            let permissions = load_role_permissions(self, membership.role_id).await?;
            let workspaces = self.list_workspaces(organization.id).await?;
            contexts.push(OrganizationContext {
                organization,
                membership,
                workspaces,
                permissions,
            });
        }

        Ok(contexts)
    }

    async fn load_membership(
        &self,
        organization_id: Uuid,
        user_id: Uuid,
    ) -> MidgardResult<Option<OrganizationMembership>> {
        load_membership_with_store(self, organization_id, user_id, true).await
    }

    async fn list_memberships(
        &self,
        organization_id: Uuid,
    ) -> MidgardResult<Vec<OrganizationMembership>> {
        let mut db = self.db.clone();
        let rows = sql::query(
            "SELECT id, organization_id, user_id, role, role_id, active, joined_at, created_at, updated_at
             FROM organization_memberships
             WHERE organization_id = $1
             ORDER BY user_id ASC",
        )
        .bind(organization_id)
        .column_types(membership_column_types())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        rows.into_iter()
            .map(membership_from_row)
            .collect::<MidgardResult<Vec<_>>>()
    }

    async fn create_membership(
        &self,
        membership: NewOrganizationMembership,
    ) -> MidgardResult<OrganizationMembership> {
        if load_membership_with_store(self, membership.organization_id, membership.user_id, false)
            .await?
            .is_some()
        {
            return Err(MidgardError::Storage(
                "organization membership already exists".to_string(),
            ));
        }
        let role_id = match membership.role_id {
            Some(role_id) => {
                let role = self
                    .load_organization_role(membership.organization_id, role_id)
                    .await?
                    .ok_or_else(|| {
                        MidgardError::Storage("organization role not found".to_string())
                    })?;
                if role.archived_at.is_some() {
                    return Err(MidgardError::Storage(
                        "archived organization role cannot be assigned".to_string(),
                    ));
                }
                role_id
            }
            None => {
                let builtin_key = legacy_organization_role_builtin_key(&membership.role);
                self.load_organization_role_by_builtin_key(membership.organization_id, builtin_key)
                    .await?
                    .ok_or_else(|| {
                        MidgardError::Storage(format!(
                            "builtin organization role not found: {builtin_key}"
                        ))
                    })?
                    .id
            }
        };

        let now = utc_now_rfc3339();
        let created = OrganizationMembership {
            id: Uuid::new_v4(),
            organization_id: membership.organization_id,
            user_id: membership.user_id,
            role: membership.role,
            role_id,
            active: membership.active,
            joined_at: now.clone(),
            created_at: now.clone(),
            updated_at: now,
        };

        let mut db = self.db.clone();
        sql::statement(
            "INSERT INTO organization_memberships
                (id, organization_id, user_id, role, role_id, active, joined_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(created.id)
        .bind(created.organization_id)
        .bind(created.user_id)
        .bind(created.role.as_str())
        .bind(created.role_id)
        .bind(created.active)
        .bind(created.joined_at.clone())
        .bind(created.created_at.clone())
        .bind(created.updated_at.clone())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        Ok(created)
    }

    async fn update_membership(
        &self,
        organization_id: Uuid,
        user_id: Uuid,
        update: OrganizationMembershipUpdate,
    ) -> MidgardResult<Option<OrganizationMembership>> {
        let Some(mut membership) =
            load_membership_with_store(self, organization_id, user_id, false).await?
        else {
            return Ok(None);
        };

        let next_role_id = match update.role_id {
            Some(role_id) => {
                let role = self
                    .load_organization_role(organization_id, role_id)
                    .await?
                    .ok_or_else(|| {
                        MidgardError::Storage("organization role not found".to_string())
                    })?;
                if role.archived_at.is_some() {
                    return Err(MidgardError::Storage(
                        "archived organization role cannot be assigned".to_string(),
                    ));
                }
                role_id
            }
            None => {
                if let Some(role) = &update.role {
                    let builtin_key = legacy_organization_role_builtin_key(role);
                    self.load_organization_role_by_builtin_key(organization_id, builtin_key)
                        .await?
                        .ok_or_else(|| {
                            MidgardError::Storage(format!(
                                "builtin organization role not found: {builtin_key}"
                            ))
                        })?
                        .id
                } else {
                    membership.role_id
                }
            }
        };
        let next_active = update.active.unwrap_or(membership.active);
        let removes_owner = membership.active
            && is_organization_owner_role(self, organization_id, membership.role_id).await?
            && (!next_active
                || !is_organization_owner_role(self, organization_id, next_role_id).await?);
        if removes_owner && active_organization_owner_count(self, organization_id).await? <= 1 {
            return Err(MidgardError::Storage(
                "cannot remove or demote the last organization owner".to_string(),
            ));
        }

        if let Some(role) = update.role {
            membership.role = role;
        }
        membership.role_id = next_role_id;
        if let Some(active) = update.active {
            membership.active = active;
        }
        membership.updated_at = utc_now_rfc3339();

        let mut db = self.db.clone();
        sql::statement(
            "UPDATE organization_memberships
             SET role = $3, role_id = $4, active = $5, updated_at = $6
             WHERE organization_id = $1 AND user_id = $2",
        )
        .bind(organization_id)
        .bind(user_id)
        .bind(membership.role.as_str())
        .bind(membership.role_id)
        .bind(membership.active)
        .bind(membership.updated_at.clone())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        Ok(Some(membership))
    }

    async fn create_workspace(&self, workspace: NewWorkspace) -> MidgardResult<Workspace> {
        let slug = normalize_slug(&workspace.slug)?;
        let name = required_name(&workspace.name, "workspace name")?;
        if self
            .load_workspace_by_slug(workspace.organization_id, &slug)
            .await?
            .is_some()
        {
            return Err(MidgardError::Storage(format!(
                "workspace slug already exists: {slug}"
            )));
        }

        let now = utc_now_rfc3339();
        let (runtime_config, runtime_config_ciphertext) = match workspace.runtime_config {
            Some(record) => (record.view, Some(record.ciphertext)),
            None => (WorkspaceRuntimeConfigView::default(), None),
        };
        let runtime_config_summary_json = json_string(&runtime_config)?;
        let created = Workspace {
            id: Uuid::new_v4(),
            organization_id: workspace.organization_id,
            slug,
            name,
            runtime_config,
            archived_at: None,
            created_at: now.clone(),
            updated_at: now,
        };

        let mut db = self.db.clone();
        sql::statement(
            "INSERT INTO workspaces
                (id, organization_id, slug, name, runtime_mode, runtime_config_ciphertext,
                 runtime_config_summary_json, runtime_config_status, runtime_config_updated_at,
                 archived_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(created.id)
        .bind(created.organization_id)
        .bind(created.slug.clone())
        .bind(created.name.clone())
        .bind(
            created
                .runtime_config
                .mode
                .as_ref()
                .map(|mode| mode.as_str().to_string()),
        )
        .bind(runtime_config_ciphertext)
        .bind(runtime_config_summary_json)
        .bind(created.runtime_config.status.as_str())
        .bind(created.runtime_config.updated_at.clone())
        .bind(created.archived_at.clone())
        .bind(created.created_at.clone())
        .bind(created.updated_at.clone())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        Ok(created)
    }

    async fn load_workspace_by_slug(
        &self,
        organization_id: Uuid,
        slug: &str,
    ) -> MidgardResult<Option<Workspace>> {
        let slug = normalize_slug(slug)?;
        let mut db = self.db.clone();
        let rows = sql::query(
            "SELECT id, organization_id, slug, name, runtime_mode, runtime_config_summary_json,
                    runtime_config_status, runtime_config_updated_at, archived_at, created_at, updated_at
             FROM workspaces
             WHERE organization_id = $1 AND slug = $2 AND archived_at IS NULL",
        )
        .bind(organization_id)
        .bind(slug)
        .column_types(workspace_column_types())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        rows.into_iter().next().map(workspace_from_row).transpose()
    }

    async fn list_workspaces(&self, organization_id: Uuid) -> MidgardResult<Vec<Workspace>> {
        let mut db = self.db.clone();
        let rows = sql::query(
            "SELECT id, organization_id, slug, name, runtime_mode, runtime_config_summary_json,
                    runtime_config_status, runtime_config_updated_at, archived_at, created_at, updated_at
             FROM workspaces
             WHERE organization_id = $1 AND archived_at IS NULL
             ORDER BY slug ASC",
        )
        .bind(organization_id)
        .column_types(workspace_column_types())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        rows.into_iter()
            .map(workspace_from_row)
            .collect::<MidgardResult<Vec<_>>>()
    }

    async fn update_workspace(
        &self,
        organization_id: Uuid,
        slug: &str,
        update: WorkspaceUpdate,
    ) -> MidgardResult<Option<Workspace>> {
        let Some(mut workspace) = self.load_workspace_by_slug(organization_id, slug).await? else {
            return Ok(None);
        };

        if let Some(name) = update.name {
            workspace.name = required_name(&name, "workspace name")?;
        }
        if let Some(archived) = update.archived {
            workspace.archived_at = archived.then(utc_now_rfc3339);
        }
        let runtime_config_ciphertext = update
            .runtime_config
            .as_ref()
            .map(|record| record.ciphertext.clone());
        let runtime_config_summary_json = update
            .runtime_config
            .as_ref()
            .map(|record| json_string(&record.view))
            .transpose()?;
        if let Some(runtime_config) = update.runtime_config {
            workspace.runtime_config = runtime_config.view;
        }
        workspace.updated_at = utc_now_rfc3339();

        let mut db = self.db.clone();
        sql::statement(
            "UPDATE workspaces
             SET name = $3,
                 runtime_mode = $4,
                 runtime_config_ciphertext = COALESCE($5, runtime_config_ciphertext),
                 runtime_config_summary_json = COALESCE($6, runtime_config_summary_json),
                 runtime_config_status = $7,
                 runtime_config_updated_at = $8,
                 archived_at = $9,
                 updated_at = $10
             WHERE organization_id = $1 AND slug = $2",
        )
        .bind(organization_id)
        .bind(workspace.slug.clone())
        .bind(workspace.name.clone())
        .bind(
            workspace
                .runtime_config
                .mode
                .as_ref()
                .map(|mode| mode.as_str().to_string()),
        )
        .bind(runtime_config_ciphertext)
        .bind(runtime_config_summary_json)
        .bind(workspace.runtime_config.status.as_str())
        .bind(workspace.runtime_config.updated_at.clone())
        .bind(workspace.archived_at.clone())
        .bind(workspace.updated_at.clone())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        Ok(Some(workspace))
    }

    async fn list_middleware_instances(
        &self,
        workspace_id: Uuid,
    ) -> MidgardResult<Vec<MiddlewareInstance>> {
        let mut db = self.db.clone();
        let rows = sql::query(
            "SELECT id, workspace_id, kind, name, namespace, desired_state, status,
                    config_json, archived_at, created_at, updated_at
             FROM middleware_instances
             WHERE workspace_id = $1 AND archived_at IS NULL
             ORDER BY namespace ASC, name ASC, id ASC",
        )
        .bind(workspace_id)
        .column_types(middleware_instance_column_types())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        rows.into_iter()
            .map(middleware_instance_from_row)
            .collect::<MidgardResult<Vec<_>>>()
    }

    async fn list_middleware_instances_for_reconciliation(
        &self,
        workspace_id: Uuid,
    ) -> MidgardResult<Vec<MiddlewareInstance>> {
        let mut db = self.db.clone();
        let rows = sql::query(
            "SELECT id, workspace_id, kind, name, namespace, desired_state, status,
                    config_json, archived_at, created_at, updated_at
             FROM middleware_instances
             WHERE workspace_id = $1
             ORDER BY namespace ASC, name ASC, id ASC",
        )
        .bind(workspace_id)
        .column_types(middleware_instance_column_types())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        rows.into_iter()
            .map(middleware_instance_from_row)
            .collect::<MidgardResult<Vec<_>>>()
    }

    async fn create_middleware_instance(
        &self,
        instance: NewMiddlewareInstance,
    ) -> MidgardResult<MiddlewareInstance> {
        let kind = required_name(&instance.kind, "middleware kind")?;
        let name = required_name(&instance.name, "middleware name")?;
        let namespace = required_name(&instance.namespace, "middleware namespace")?;
        if self
            .list_middleware_instances(instance.workspace_id)
            .await?
            .iter()
            .any(|current| current.namespace == namespace && current.name == name)
        {
            return Err(MidgardError::Storage(format!(
                "middleware instance already exists: {namespace}/{name}"
            )));
        }

        let now = utc_now_rfc3339();
        let created = MiddlewareInstance {
            id: Uuid::new_v4(),
            workspace_id: instance.workspace_id,
            kind,
            name,
            namespace,
            desired_state: instance.desired_state,
            status: instance.status,
            config: instance.config,
            archived_at: None,
            created_at: now.clone(),
            updated_at: now,
        };

        let mut db = self.db.clone();
        sql::statement(
            "INSERT INTO middleware_instances
                (id, workspace_id, kind, name, namespace, desired_state, status,
                 config_json, archived_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(created.id)
        .bind(created.workspace_id)
        .bind(created.kind.clone())
        .bind(created.name.clone())
        .bind(created.namespace.clone())
        .bind(created.desired_state.as_str())
        .bind(created.status.as_str())
        .bind(json_string(&created.config)?)
        .bind(created.archived_at.clone())
        .bind(created.created_at.clone())
        .bind(created.updated_at.clone())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        Ok(created)
    }

    async fn update_middleware_instance(
        &self,
        workspace_id: Uuid,
        id: Uuid,
        update: MiddlewareInstanceUpdate,
    ) -> MidgardResult<Option<MiddlewareInstance>> {
        let mut db = self.db.clone();
        let rows = sql::query(
            "SELECT id, workspace_id, kind, name, namespace, desired_state, status,
                    config_json, archived_at, created_at, updated_at
             FROM middleware_instances
             WHERE workspace_id = $1 AND id = $2",
        )
        .bind(workspace_id)
        .bind(id)
        .column_types(middleware_instance_column_types())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;
        let Some(row) = rows.into_iter().next() else {
            return Ok(None);
        };
        let mut instance = middleware_instance_from_row(row)?;

        if let Some(desired_state) = update.desired_state {
            instance.desired_state = desired_state;
        }
        if let Some(status) = update.status {
            instance.status = status;
        }
        if let Some(config) = update.config {
            instance.config = config;
        }
        if let Some(archived) = update.archived {
            instance.archived_at = archived.then(utc_now_rfc3339);
        }
        instance.updated_at = utc_now_rfc3339();

        sql::statement(
            "UPDATE middleware_instances
             SET desired_state = $3, status = $4, config_json = $5, archived_at = $6, updated_at = $7
             WHERE workspace_id = $1 AND id = $2",
        )
        .bind(workspace_id)
        .bind(id)
        .bind(instance.desired_state.as_str())
        .bind(instance.status.as_str())
        .bind(json_string(&instance.config)?)
        .bind(instance.archived_at.clone())
        .bind(instance.updated_at.clone())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

        Ok(Some(instance))
    }

    async fn list_organization_roles(&self, organization_id: Uuid) -> MidgardResult<Vec<RbacRole>> {
        list_roles(self, RbacScopeKind::Organization, Some(organization_id)).await
    }

    async fn load_organization_role(
        &self,
        organization_id: Uuid,
        id: Uuid,
    ) -> MidgardResult<Option<RbacRole>> {
        load_role_by_id(self, RbacScopeKind::Organization, Some(organization_id), id).await
    }

    async fn load_organization_role_by_builtin_key(
        &self,
        organization_id: Uuid,
        builtin_key: &str,
    ) -> MidgardResult<Option<RbacRole>> {
        load_role_by_builtin_key(
            self,
            RbacScopeKind::Organization,
            Some(organization_id),
            builtin_key,
        )
        .await
    }

    async fn create_organization_role(&self, role: NewRbacRole) -> MidgardResult<RbacRole> {
        if role.scope_kind != RbacScopeKind::Organization || role.organization_id.is_none() {
            return Err(MidgardError::Storage(
                "organization role must use organization scope".to_string(),
            ));
        }
        create_role(self, role).await
    }

    async fn update_organization_role(
        &self,
        organization_id: Uuid,
        id: Uuid,
        update: RbacRoleUpdate,
    ) -> MidgardResult<Option<RbacRole>> {
        update_role(
            self,
            RbacScopeKind::Organization,
            Some(organization_id),
            id,
            update,
        )
        .await
    }

    async fn replace_organization_role_permissions(
        &self,
        organization_id: Uuid,
        id: Uuid,
        permissions: Vec<PermissionKey>,
    ) -> MidgardResult<Option<RbacRole>> {
        replace_role_permissions(
            self,
            RbacScopeKind::Organization,
            Some(organization_id),
            id,
            permissions,
        )
        .await
    }
}

fn auth_user_column_types() -> [stmt::Type; 10] {
    [
        stmt::Type::Uuid,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::Uuid,
        stmt::Type::String,
        stmt::Type::Bool,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
    ]
}

fn auth_user_record_from_row(row: stmt::Value) -> MidgardResult<AuthUserRecord> {
    let record = row.into_record();
    auth_user_record_from_record(record.as_slice())
}

fn auth_user_record_from_record(record: &[stmt::Value]) -> MidgardResult<AuthUserRecord> {
    let id = uuid_from_value(&record[0])?;
    let email = string_from_value(&record[1])?.to_string();
    let display_name = string_from_value(&record[2])?.to_string();
    let role = crate::auth::UserRole::from_storage(string_from_value(&record[3])?)?;
    let system_role_id = uuid_from_value(&record[4])?;
    let password_hash = string_from_value(&record[5])?.to_string();
    let active = bool_from_value(&record[6])?;
    let created_at = string_from_value(&record[7])?.to_string();
    let updated_at = string_from_value(&record[8])?.to_string();
    let last_login_at = optional_string_from_value(&record[9])?;

    Ok(AuthUserRecord {
        user: AuthUser {
            id,
            email,
            display_name,
            role,
            system_role_id,
            active,
            created_at,
            updated_at,
            last_login_at,
        },
        password_hash,
    })
}

fn organization_column_types() -> [stmt::Type; 7] {
    [
        stmt::Type::Uuid,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::Uuid,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
    ]
}

fn membership_column_types() -> [stmt::Type; 9] {
    [
        stmt::Type::Uuid,
        stmt::Type::Uuid,
        stmt::Type::Uuid,
        stmt::Type::String,
        stmt::Type::Uuid,
        stmt::Type::Bool,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
    ]
}

fn workspace_column_types() -> [stmt::Type; 11] {
    [
        stmt::Type::Uuid,
        stmt::Type::Uuid,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
    ]
}

fn organization_context_column_types() -> [stmt::Type; 16] {
    [
        stmt::Type::Uuid,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::Uuid,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::Uuid,
        stmt::Type::Uuid,
        stmt::Type::Uuid,
        stmt::Type::String,
        stmt::Type::Uuid,
        stmt::Type::Bool,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
    ]
}

fn organization_from_row(row: stmt::Value) -> MidgardResult<Organization> {
    let record = row.into_record();
    organization_from_record(record.as_slice())
}

fn organization_from_record(record: &[stmt::Value]) -> MidgardResult<Organization> {
    Ok(Organization {
        id: uuid_from_value(&record[0])?,
        slug: string_from_value(&record[1])?.to_string(),
        name: string_from_value(&record[2])?.to_string(),
        created_by_user_id: uuid_from_value(&record[3])?,
        archived_at: optional_string_from_value(&record[4])?,
        created_at: string_from_value(&record[5])?.to_string(),
        updated_at: string_from_value(&record[6])?.to_string(),
    })
}

fn membership_from_row(row: stmt::Value) -> MidgardResult<OrganizationMembership> {
    let record = row.into_record();
    membership_from_record(record.as_slice())
}

fn membership_from_record(record: &[stmt::Value]) -> MidgardResult<OrganizationMembership> {
    Ok(OrganizationMembership {
        id: uuid_from_value(&record[0])?,
        organization_id: uuid_from_value(&record[1])?,
        user_id: uuid_from_value(&record[2])?,
        role: OrganizationRole::from_storage(string_from_value(&record[3])?)?,
        role_id: uuid_from_value(&record[4])?,
        active: bool_from_value(&record[5])?,
        joined_at: string_from_value(&record[6])?.to_string(),
        created_at: string_from_value(&record[7])?.to_string(),
        updated_at: string_from_value(&record[8])?.to_string(),
    })
}

fn workspace_from_row(row: stmt::Value) -> MidgardResult<Workspace> {
    let record = row.into_record();
    workspace_from_record(record.as_slice())
}

fn workspace_from_record(record: &[stmt::Value]) -> MidgardResult<Workspace> {
    let mut runtime_config = match optional_string_from_value(&record[5])? {
        Some(summary_json) => serde_json::from_str::<WorkspaceRuntimeConfigView>(&summary_json)
            .map_err(|err| {
                MidgardError::Storage(format!(
                    "invalid workspace runtime config summary JSON: {err}"
                ))
            })?,
        None => WorkspaceRuntimeConfigView::default(),
    };
    runtime_config.mode = optional_string_from_value(&record[4])?
        .as_deref()
        .map(crate::org::WorkspaceRuntimeMode::from_storage)
        .transpose()?;
    runtime_config.status =
        WorkspaceRuntimeConfigStatus::from_storage(string_from_value(&record[6])?)?;
    runtime_config.updated_at = optional_string_from_value(&record[7])?;

    Ok(Workspace {
        id: uuid_from_value(&record[0])?,
        organization_id: uuid_from_value(&record[1])?,
        slug: string_from_value(&record[2])?.to_string(),
        name: string_from_value(&record[3])?.to_string(),
        runtime_config,
        archived_at: optional_string_from_value(&record[8])?,
        created_at: string_from_value(&record[9])?.to_string(),
        updated_at: string_from_value(&record[10])?.to_string(),
    })
}

fn middleware_instance_column_types() -> [stmt::Type; 11] {
    [
        stmt::Type::Uuid,
        stmt::Type::Uuid,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
    ]
}

fn middleware_instance_from_row(row: stmt::Value) -> MidgardResult<MiddlewareInstance> {
    let record = row.into_record();
    let config_json = string_from_value(&record[7])?;
    let config = serde_json::from_str(config_json).map_err(|err| {
        MidgardError::Storage(format!("invalid middleware instance config JSON: {err}"))
    })?;

    Ok(MiddlewareInstance {
        id: uuid_from_value(&record[0])?,
        workspace_id: uuid_from_value(&record[1])?,
        kind: string_from_value(&record[2])?.to_string(),
        name: string_from_value(&record[3])?.to_string(),
        namespace: string_from_value(&record[4])?.to_string(),
        desired_state: MiddlewareDesiredState::from_storage(string_from_value(&record[5])?)?,
        status: MiddlewareInstanceStatus::from_storage(string_from_value(&record[6])?)?,
        config,
        archived_at: optional_string_from_value(&record[8])?,
        created_at: string_from_value(&record[9])?.to_string(),
        updated_at: string_from_value(&record[10])?.to_string(),
    })
}

fn rbac_role_column_types() -> [stmt::Type; 11] {
    [
        stmt::Type::Uuid,
        stmt::Type::String,
        stmt::Type::Uuid,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::Bool,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
    ]
}

fn rbac_role_from_record(
    record: &[stmt::Value],
    permissions: Vec<PermissionKey>,
) -> MidgardResult<RbacRole> {
    Ok(RbacRole {
        id: uuid_from_value(&record[0])?,
        scope_kind: RbacScopeKind::from_storage(string_from_value(&record[1])?)?,
        organization_id: optional_uuid_from_value(&record[2])?,
        slug: string_from_value(&record[3])?.to_string(),
        name: string_from_value(&record[4])?.to_string(),
        description: optional_string_from_value(&record[5])?,
        builtin_key: optional_string_from_value(&record[6])?,
        protected: bool_from_value(&record[7])?,
        archived_at: optional_string_from_value(&record[8])?,
        created_at: string_from_value(&record[9])?.to_string(),
        updated_at: string_from_value(&record[10])?.to_string(),
        permissions,
    })
}

async fn list_roles(
    store: &PostgresAgentSessionStore,
    scope_kind: RbacScopeKind,
    organization_id: Option<Uuid>,
) -> MidgardResult<Vec<RbacRole>> {
    let mut db = store.db.clone();
    let rows = match organization_id {
        Some(organization_id) => {
            sql::query(
                "SELECT id, scope_kind, organization_id, slug, name, description, builtin_key, protected, archived_at, created_at, updated_at
                 FROM rbac_roles
                 WHERE scope_kind = $1 AND organization_id = $2
                 ORDER BY slug ASC",
            )
            .bind(scope_kind.as_str())
            .bind(organization_id)
            .column_types(rbac_role_column_types())
            .exec(&mut db)
            .await
            .map_err(storage_error)?
        }
        None => {
            sql::query(
                "SELECT id, scope_kind, organization_id, slug, name, description, builtin_key, protected, archived_at, created_at, updated_at
                 FROM rbac_roles
                 WHERE scope_kind = $1 AND organization_id IS NULL
                 ORDER BY slug ASC",
            )
            .bind(scope_kind.as_str())
            .column_types(rbac_role_column_types())
            .exec(&mut db)
            .await
            .map_err(storage_error)?
        }
    };

    let mut roles = Vec::with_capacity(rows.len());
    for row in rows {
        let record = row.into_record();
        let id = uuid_from_value(&record[0])?;
        let permissions = load_role_permissions(store, id).await?;
        roles.push(rbac_role_from_record(&record, permissions)?);
    }

    Ok(roles)
}

async fn load_role_by_id(
    store: &PostgresAgentSessionStore,
    scope_kind: RbacScopeKind,
    organization_id: Option<Uuid>,
    id: Uuid,
) -> MidgardResult<Option<RbacRole>> {
    let mut db = store.db.clone();
    let rows = match organization_id {
        Some(organization_id) => {
            sql::query(
                "SELECT id, scope_kind, organization_id, slug, name, description, builtin_key, protected, archived_at, created_at, updated_at
                 FROM rbac_roles
                 WHERE id = $1 AND scope_kind = $2 AND organization_id = $3",
            )
            .bind(id)
            .bind(scope_kind.as_str())
            .bind(organization_id)
            .column_types(rbac_role_column_types())
            .exec(&mut db)
            .await
            .map_err(storage_error)?
        }
        None => {
            sql::query(
                "SELECT id, scope_kind, organization_id, slug, name, description, builtin_key, protected, archived_at, created_at, updated_at
                 FROM rbac_roles
                 WHERE id = $1 AND scope_kind = $2 AND organization_id IS NULL",
            )
            .bind(id)
            .bind(scope_kind.as_str())
            .column_types(rbac_role_column_types())
            .exec(&mut db)
            .await
            .map_err(storage_error)?
        }
    };

    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    let record = row.into_record();
    let permissions = load_role_permissions(store, id).await?;
    rbac_role_from_record(&record, permissions).map(Some)
}

async fn load_role_by_builtin_key(
    store: &PostgresAgentSessionStore,
    scope_kind: RbacScopeKind,
    organization_id: Option<Uuid>,
    builtin_key: &str,
) -> MidgardResult<Option<RbacRole>> {
    let mut db = store.db.clone();
    let rows = match organization_id {
        Some(organization_id) => {
            sql::query(
                "SELECT id, scope_kind, organization_id, slug, name, description, builtin_key, protected, archived_at, created_at, updated_at
                 FROM rbac_roles
                 WHERE scope_kind = $1 AND organization_id = $2 AND builtin_key = $3",
            )
            .bind(scope_kind.as_str())
            .bind(organization_id)
            .bind(builtin_key.to_string())
            .column_types(rbac_role_column_types())
            .exec(&mut db)
            .await
            .map_err(storage_error)?
        }
        None => {
            sql::query(
                "SELECT id, scope_kind, organization_id, slug, name, description, builtin_key, protected, archived_at, created_at, updated_at
                 FROM rbac_roles
                 WHERE scope_kind = $1 AND organization_id IS NULL AND builtin_key = $2",
            )
            .bind(scope_kind.as_str())
            .bind(builtin_key.to_string())
            .column_types(rbac_role_column_types())
            .exec(&mut db)
            .await
            .map_err(storage_error)?
        }
    };

    let Some(row) = rows.into_iter().next() else {
        return Ok(None);
    };
    let record = row.into_record();
    let id = uuid_from_value(&record[0])?;
    let permissions = load_role_permissions(store, id).await?;
    rbac_role_from_record(&record, permissions).map(Some)
}

async fn create_role(
    store: &PostgresAgentSessionStore,
    role: NewRbacRole,
) -> MidgardResult<RbacRole> {
    PermissionKey::validate_for_scope(&role.scope_kind, &role.permissions)?;
    let slug = normalize_slug(&role.slug)?;
    let name = required_name(&role.name, "role name")?;
    if role_slug_exists(store, &role.scope_kind, role.organization_id, &slug).await? {
        return Err(MidgardError::Storage(format!(
            "RBAC role slug already exists: {slug}"
        )));
    }
    let now = utc_now_rfc3339();
    let created = RbacRole {
        id: Uuid::new_v4(),
        scope_kind: role.scope_kind,
        organization_id: role.organization_id,
        slug,
        name,
        description: role.description,
        builtin_key: role.builtin_key,
        protected: role.protected,
        archived_at: None,
        created_at: now.clone(),
        updated_at: now,
        permissions: sorted_permissions(role.permissions),
    };
    insert_role(store, &created).await?;
    Ok(created)
}

async fn update_role(
    store: &PostgresAgentSessionStore,
    scope_kind: RbacScopeKind,
    organization_id: Option<Uuid>,
    id: Uuid,
    update: RbacRoleUpdate,
) -> MidgardResult<Option<RbacRole>> {
    let Some(mut role) = load_role_by_id(store, scope_kind.clone(), organization_id, id).await?
    else {
        return Ok(None);
    };
    if update.archived == Some(true) && role.protected {
        return Err(MidgardError::Storage(
            "protected RBAC role cannot be archived".to_string(),
        ));
    }
    if let Some(name) = update.name {
        role.name = required_name(&name, "role name")?;
    }
    if let Some(description) = update.description {
        role.description = if description.trim().is_empty() {
            None
        } else {
            Some(description.trim().to_string())
        };
    }
    if let Some(archived) = update.archived {
        role.archived_at = archived.then(utc_now_rfc3339);
    }
    role.updated_at = utc_now_rfc3339();

    let mut db = store.db.clone();
    sql::statement(
        "UPDATE rbac_roles
         SET name = $2, description = $3, archived_at = $4, updated_at = $5
         WHERE id = $1",
    )
    .bind(role.id)
    .bind(role.name.clone())
    .bind(role.description.clone())
    .bind(role.archived_at.clone())
    .bind(role.updated_at.clone())
    .exec(&mut db)
    .await
    .map_err(storage_error)?;

    Ok(Some(role))
}

async fn replace_role_permissions(
    store: &PostgresAgentSessionStore,
    scope_kind: RbacScopeKind,
    organization_id: Option<Uuid>,
    id: Uuid,
    permissions: Vec<PermissionKey>,
) -> MidgardResult<Option<RbacRole>> {
    PermissionKey::validate_for_scope(&scope_kind, &permissions)?;
    let Some(mut role) = load_role_by_id(store, scope_kind, organization_id, id).await? else {
        return Ok(None);
    };
    if role.archived_at.is_some() {
        return Err(MidgardError::Storage(
            "archived RBAC role permissions cannot be updated".to_string(),
        ));
    }
    let permissions = sorted_permissions(permissions);
    if role.builtin_key.as_deref() == Some(SYSTEM_OWNER_BUILTIN) {
        require_all_permissions(&permissions, PermissionKey::system_permissions())?;
    }
    if role.builtin_key.as_deref() == Some(ORG_OWNER_BUILTIN) {
        require_all_permissions(&permissions, PermissionKey::organization_permissions())?;
    }

    let mut db = store.db.clone();
    sql::statement("DELETE FROM rbac_role_permissions WHERE role_id = $1")
        .bind(id)
        .exec(&mut db)
        .await
        .map_err(storage_error)?;
    insert_role_permissions(store, id, &permissions).await?;
    role.permissions = permissions;
    role.updated_at = utc_now_rfc3339();
    sql::statement("UPDATE rbac_roles SET updated_at = $2 WHERE id = $1")
        .bind(role.id)
        .bind(role.updated_at.clone())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;

    Ok(Some(role))
}

async fn role_slug_exists(
    store: &PostgresAgentSessionStore,
    scope_kind: &RbacScopeKind,
    organization_id: Option<Uuid>,
    slug: &str,
) -> MidgardResult<bool> {
    let mut db = store.db.clone();
    let rows = match organization_id {
        Some(organization_id) => sql::query(
            "SELECT id FROM rbac_roles WHERE scope_kind = $1 AND organization_id = $2 AND slug = $3",
        )
        .bind(scope_kind.as_str())
        .bind(organization_id)
        .bind(slug.to_string())
        .column_types([stmt::Type::Uuid])
        .exec(&mut db)
        .await
        .map_err(storage_error)?,
        None => sql::query(
            "SELECT id FROM rbac_roles WHERE scope_kind = $1 AND organization_id IS NULL AND slug = $2",
        )
        .bind(scope_kind.as_str())
        .bind(slug.to_string())
        .column_types([stmt::Type::Uuid])
        .exec(&mut db)
        .await
        .map_err(storage_error)?,
    };

    Ok(!rows.is_empty())
}

async fn insert_role(store: &PostgresAgentSessionStore, role: &RbacRole) -> MidgardResult<()> {
    let mut db = store.db.clone();
    sql::statement(
        "INSERT INTO rbac_roles
            (id, scope_kind, organization_id, slug, name, description, builtin_key, protected, archived_at, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(role.id)
    .bind(role.scope_kind.as_str())
    .bind(role.organization_id)
    .bind(role.slug.clone())
    .bind(role.name.clone())
    .bind(role.description.clone())
    .bind(role.builtin_key.clone())
    .bind(role.protected)
    .bind(role.archived_at.clone())
    .bind(role.created_at.clone())
    .bind(role.updated_at.clone())
    .exec(&mut db)
    .await
    .map_err(storage_error)?;
    insert_role_permissions(store, role.id, &role.permissions).await
}

async fn insert_role_permissions(
    store: &PostgresAgentSessionStore,
    role_id: Uuid,
    permissions: &[PermissionKey],
) -> MidgardResult<()> {
    let mut db = store.db.clone();
    for permission in permissions {
        sql::statement(
            "INSERT INTO rbac_role_permissions (role_id, permission_key)
             VALUES ($1, $2)",
        )
        .bind(role_id)
        .bind(permission.as_str())
        .exec(&mut db)
        .await
        .map_err(storage_error)?;
    }

    Ok(())
}

async fn load_role_permissions(
    store: &PostgresAgentSessionStore,
    role_id: Uuid,
) -> MidgardResult<Vec<PermissionKey>> {
    let mut db = store.db.clone();
    let rows = sql::query(
        "SELECT permission_key
         FROM rbac_role_permissions
         WHERE role_id = $1
         ORDER BY permission_key ASC",
    )
    .bind(role_id)
    .column_types([stmt::Type::String])
    .exec(&mut db)
    .await
    .map_err(storage_error)?;

    let permissions = rows
        .into_iter()
        .map(|row| {
            let record = row.into_record();
            PermissionKey::from_storage(string_from_value(&record[0])?)
        })
        .collect::<MidgardResult<Vec<_>>>()?;

    Ok(sorted_permissions(permissions))
}

async fn seed_builtin_organization_roles(
    store: &PostgresAgentSessionStore,
    organization_id: Uuid,
) -> MidgardResult<()> {
    for definition in builtin_organization_roles() {
        if load_role_by_builtin_key(
            store,
            RbacScopeKind::Organization,
            Some(organization_id),
            definition.builtin_key,
        )
        .await?
        .is_some()
        {
            continue;
        }
        let now = utc_now_rfc3339();
        let role = RbacRole {
            id: Uuid::new_v4(),
            scope_kind: RbacScopeKind::Organization,
            organization_id: Some(organization_id),
            slug: definition.slug.to_string(),
            name: definition.name.to_string(),
            description: Some(definition.description.to_string()),
            builtin_key: Some(definition.builtin_key.to_string()),
            protected: definition.protected,
            archived_at: None,
            created_at: now.clone(),
            updated_at: now,
            permissions: sorted_permissions(definition.permissions),
        };
        insert_role(store, &role).await?;
    }

    Ok(())
}

async fn is_system_owner_role(
    store: &PostgresAgentSessionStore,
    role_id: Uuid,
) -> MidgardResult<bool> {
    Ok(store
        .load_system_role(role_id)
        .await?
        .is_some_and(|role| role.builtin_key.as_deref() == Some(SYSTEM_OWNER_BUILTIN)))
}

async fn active_system_owner_count(store: &PostgresAgentSessionStore) -> MidgardResult<usize> {
    let mut db = store.db.clone();
    let rows = sql::query(
        "SELECT u.id
         FROM users u
         JOIN rbac_roles r ON r.id = u.system_role_id
         WHERE u.active = TRUE
           AND r.builtin_key = $1
           AND r.archived_at IS NULL",
    )
    .bind(SYSTEM_OWNER_BUILTIN.to_string())
    .column_types([stmt::Type::Uuid])
    .exec(&mut db)
    .await
    .map_err(storage_error)?;

    Ok(rows.len())
}

async fn is_organization_owner_role(
    store: &PostgresAgentSessionStore,
    organization_id: Uuid,
    role_id: Uuid,
) -> MidgardResult<bool> {
    Ok(store
        .load_organization_role(organization_id, role_id)
        .await?
        .is_some_and(|role| role.builtin_key.as_deref() == Some(ORG_OWNER_BUILTIN)))
}

fn optional_uuid_from_value(value: &stmt::Value) -> MidgardResult<Option<Uuid>> {
    match value {
        stmt::Value::Null => Ok(None),
        stmt::Value::Uuid(value) => Ok(Some(*value)),
        other => Err(MidgardError::Storage(format!(
            "expected optional uuid, got {other:?}"
        ))),
    }
}

fn sorted_permissions(permissions: Vec<PermissionKey>) -> Vec<PermissionKey> {
    let mut permissions = permissions;
    permissions.sort();
    permissions.dedup();
    permissions
}

fn require_all_permissions(
    permissions: &[PermissionKey],
    required: Vec<PermissionKey>,
) -> MidgardResult<()> {
    if required
        .into_iter()
        .all(|required| permissions.iter().any(|permission| permission == &required))
    {
        return Ok(());
    }

    Err(MidgardError::Storage(
        "owner role must retain all permissions".to_string(),
    ))
}

async fn load_membership_with_store(
    store: &PostgresAgentSessionStore,
    organization_id: Uuid,
    user_id: Uuid,
    active_only: bool,
) -> MidgardResult<Option<OrganizationMembership>> {
    let mut db = store.db.clone();
    let rows = if active_only {
        sql::query(
            "SELECT id, organization_id, user_id, role, role_id, active, joined_at, created_at, updated_at
             FROM organization_memberships
             WHERE organization_id = $1 AND user_id = $2 AND active = TRUE",
        )
        .bind(organization_id)
        .bind(user_id)
        .column_types(membership_column_types())
        .exec(&mut db)
        .await
        .map_err(storage_error)?
    } else {
        sql::query(
            "SELECT id, organization_id, user_id, role, role_id, active, joined_at, created_at, updated_at
             FROM organization_memberships
             WHERE organization_id = $1 AND user_id = $2",
        )
        .bind(organization_id)
        .bind(user_id)
        .column_types(membership_column_types())
        .exec(&mut db)
        .await
        .map_err(storage_error)?
    };

    rows.into_iter().next().map(membership_from_row).transpose()
}

async fn active_organization_owner_count(
    store: &PostgresAgentSessionStore,
    organization_id: Uuid,
) -> MidgardResult<usize> {
    let mut db = store.db.clone();
    let rows = sql::query(
        "SELECT m.id
         FROM organization_memberships m
         JOIN rbac_roles r ON r.id = m.role_id
         WHERE m.organization_id = $1
           AND m.active = TRUE
           AND r.builtin_key = $2
           AND r.archived_at IS NULL",
    )
    .bind(organization_id)
    .bind(ORG_OWNER_BUILTIN.to_string())
    .column_types([stmt::Type::Uuid])
    .exec(&mut db)
    .await
    .map_err(storage_error)?;

    Ok(rows.len())
}

fn normalize_slug(slug: &str) -> MidgardResult<String> {
    let slug = slug.trim().to_ascii_lowercase();
    if slug.is_empty() {
        return Err(MidgardError::Storage("slug is required".to_string()));
    }
    if !slug
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
    {
        return Err(MidgardError::Storage(format!("invalid slug: {slug}")));
    }

    Ok(slug)
}

fn required_name(name: &str, label: &str) -> MidgardResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(MidgardError::Storage(format!("{label} is required")));
    }

    Ok(name.to_string())
}

fn workspace_id_option(workspace_id: Uuid) -> Option<Uuid> {
    (workspace_id != Uuid::nil()).then_some(workspace_id)
}

async fn upsert_session(
    executor: &mut dyn Executor,
    workspace_id: Uuid,
    session: &AgentSession,
) -> MidgardResult<()> {
    sql::statement(
        "INSERT INTO agent_sessions (id, workspace_id, iteration_count, status, pending_approval_json, last_error)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (id) DO UPDATE SET
            workspace_id = EXCLUDED.workspace_id,
            iteration_count = EXCLUDED.iteration_count,
            status = EXCLUDED.status,
            pending_approval_json = EXCLUDED.pending_approval_json,
            last_error = EXCLUDED.last_error",
    )
    .bind(session.id)
    .bind(workspace_id)
    .bind(session.iteration_count as i64)
    .bind(status_to_storage(&session.status))
    .bind(optional_json(&session.pending_approval)?)
    .bind(session.last_error.clone())
    .exec(executor)
    .await
    .map_err(storage_error)?;

    Ok(())
}

async fn insert_message(
    executor: &mut dyn Executor,
    session_id: Uuid,
    sequence: i64,
    message: &AgentMessage,
) -> MidgardResult<()> {
    sql::statement(
        "INSERT INTO agent_messages (session_id, sequence, role, content, tool_calls_json, tool_call_id)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(session_id)
    .bind(sequence)
    .bind(role_to_storage(&message.role))
    .bind(message.content.clone())
    .bind(optional_tool_calls_json(&message.tool_calls)?)
    .bind(message.tool_call_id.clone())
    .exec(executor)
    .await
    .map_err(storage_error)?;

    Ok(())
}

async fn upsert_pending_approval_record(
    executor: &mut dyn Executor,
    session_id: Uuid,
    approval: &PendingApproval,
) -> MidgardResult<()> {
    let record = ApprovalRecord::pending(session_id, approval);
    sql::statement(
        "INSERT INTO agent_approval_records
            (id, session_id, tool_call_json, risk_level, status, requested_at, decided_at, actor, reason)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(record.id)
    .bind(record.session_id)
    .bind(json_string(&record.tool_call)?)
    .bind(risk_level_to_storage(&record.risk_level))
    .bind(record.status.as_str())
    .bind(record.requested_at)
    .bind(record.decided_at)
    .bind(record.actor)
    .bind(record.reason)
    .exec(executor)
    .await
    .map_err(storage_error)?;

    Ok(())
}

async fn update_approval_record_decision(
    executor: &mut dyn Executor,
    record: &ApprovalRecord,
) -> MidgardResult<()> {
    sql::statement(
        "UPDATE agent_approval_records
         SET status = $1, decided_at = $2, actor = $3, reason = $4
         WHERE id = $5 AND session_id = $6",
    )
    .bind(record.status.as_str())
    .bind(record.decided_at.clone())
    .bind(record.actor.clone())
    .bind(record.reason.clone())
    .bind(record.id)
    .bind(record.session_id)
    .exec(executor)
    .await
    .map_err(storage_error)?;

    Ok(())
}

async fn load_session_with_executor(
    executor: &mut dyn Executor,
    id: Uuid,
    workspace_id: Option<Uuid>,
) -> MidgardResult<Option<AgentSession>> {
    let session_rows = match workspace_id {
        Some(workspace_id) => sql::query(
            "SELECT id, workspace_id, iteration_count, status, pending_approval_json, last_error
                 FROM agent_sessions WHERE id = $1 AND workspace_id = $2",
        )
        .bind(id)
        .bind(workspace_id)
        .column_types([
            stmt::Type::Uuid,
            stmt::Type::Uuid,
            stmt::Type::I64,
            stmt::Type::String,
            stmt::Type::String,
            stmt::Type::String,
        ])
        .exec(executor)
        .await
        .map_err(storage_error)?,
        None => sql::query(
            "SELECT id, workspace_id, iteration_count, status, pending_approval_json, last_error
                 FROM agent_sessions WHERE id = $1",
        )
        .bind(id)
        .column_types([
            stmt::Type::Uuid,
            stmt::Type::Uuid,
            stmt::Type::I64,
            stmt::Type::String,
            stmt::Type::String,
            stmt::Type::String,
        ])
        .exec(executor)
        .await
        .map_err(storage_error)?,
    };

    let Some(session_row) = session_rows.into_iter().next() else {
        return Ok(None);
    };
    let session_record = session_row.into_record();
    let id = uuid_from_value(&session_record[0])?;
    let stored_workspace_id = uuid_from_value(&session_record[1])?;
    let iteration_count = i64_from_value(&session_record[2])? as usize;
    let status = status_from_storage(string_from_value(&session_record[3])?)?;
    let pending_approval = optional_pending_approval(&session_record[4])?;
    let last_error = optional_string_from_value(&session_record[5])?;

    let message_rows = sql::query(
        "SELECT role, content, tool_calls_json, tool_call_id
         FROM agent_messages WHERE session_id = $1 ORDER BY sequence ASC",
    )
    .bind(id)
    .column_types([
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
    ])
    .exec(executor)
    .await
    .map_err(storage_error)?;

    let mut messages = Vec::with_capacity(message_rows.len());
    for row in message_rows {
        messages.push(agent_message_from_row(row)?);
    }

    Ok(Some(AgentSession {
        id,
        workspace_id: workspace_id_option(stored_workspace_id),
        messages,
        iteration_count,
        status,
        pending_approval,
        last_error,
    }))
}

async fn list_approval_records_with_executor(
    executor: &mut dyn Executor,
    session_id: Uuid,
) -> MidgardResult<Vec<ApprovalRecord>> {
    let rows = sql::query(
        "SELECT id, session_id, tool_call_json, risk_level, status, requested_at, decided_at, actor, reason
         FROM agent_approval_records
         WHERE session_id = $1
         ORDER BY requested_at ASC, id ASC",
    )
    .bind(session_id)
    .column_types([
        stmt::Type::Uuid,
        stmt::Type::Uuid,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
    ])
    .exec(executor)
    .await
    .map_err(storage_error)?;

    rows.into_iter()
        .map(approval_record_from_row)
        .collect::<MidgardResult<Vec<_>>>()
}

async fn load_approval_record_with_executor(
    executor: &mut dyn Executor,
    session_id: Uuid,
    approval_id: Uuid,
) -> MidgardResult<ApprovalRecord> {
    let rows = sql::query(
        "SELECT id, session_id, tool_call_json, risk_level, status, requested_at, decided_at, actor, reason
         FROM agent_approval_records
         WHERE session_id = $1 AND id = $2",
    )
    .bind(session_id)
    .bind(approval_id)
    .column_types([
        stmt::Type::Uuid,
        stmt::Type::Uuid,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
        stmt::Type::String,
    ])
    .exec(executor)
    .await
    .map_err(storage_error)?;

    rows.into_iter()
        .next()
        .ok_or_else(|| MidgardError::Storage(format!("approval record not found: {approval_id}")))
        .and_then(approval_record_from_row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn postgres_integration_uses_test_database_url_when_available() {
        let Ok(database_url) = std::env::var("MIDGARD_TEST_DATABASE_URL") else {
            return;
        };
        let store = PostgresAgentSessionStore::connect(&database_url)
            .await
            .unwrap();
        store.db().push_schema().await.unwrap();

        let session = store
            .create_session("inspect redis".to_string())
            .await
            .unwrap();
        let loaded = store.load_session(session.id).await.unwrap().unwrap();

        assert_eq!(loaded.messages[0].content, "inspect redis");
    }
}
