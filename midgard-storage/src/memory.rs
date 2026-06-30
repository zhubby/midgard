use async_trait::async_trait;
use midgard_agent::{
    AgentMessage, AgentSession, ApprovalDecision, ApprovalRecord, PendingApproval,
};
use midgard_core::{MidgardError, MidgardResult};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
};
use uuid::Uuid;

use crate::{
    auth::{
        normalize_email, parse_rfc3339_utc, utc_now_rfc3339, AuthSession, AuthUser, AuthUserRecord,
        AuthUserUpdate, NewAuthAuditEvent, NewAuthSession, NewUser,
    },
    org::{
        NewOrganization, NewOrganizationMembership, NewWorkspace, Organization,
        OrganizationContext, OrganizationMembership, OrganizationMembershipUpdate, Workspace,
        WorkspaceUpdate,
    },
    rbac::{
        builtin_organization_roles, builtin_system_roles, legacy_organization_role_builtin_key,
        legacy_user_role_builtin_key, NewRbacRole, PermissionKey, RbacRole, RbacRoleUpdate,
        RbacScopeKind, ORG_OWNER_BUILTIN, SYSTEM_OWNER_BUILTIN,
    },
    store::{AgentSessionStore, AuthStore, OrganizationStore},
};

#[derive(Default)]
pub struct MemoryAgentSessionStore {
    sessions: Mutex<BTreeMap<Uuid, AgentSession>>,
    session_workspaces: Mutex<BTreeMap<Uuid, Uuid>>,
    approval_records: Mutex<BTreeMap<Uuid, Vec<ApprovalRecord>>>,
}

impl MemoryAgentSessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn upsert_pending_approval_record(&self, session: &AgentSession) -> MidgardResult<()> {
        let Some(approval) = &session.pending_approval else {
            return Ok(());
        };
        if approval.approved.is_some() {
            return Ok(());
        }
        let mut approval_records = self
            .approval_records
            .lock()
            .map_err(|_| MidgardError::Storage("approval store poisoned".to_string()))?;
        let records = approval_records.entry(session.id).or_default();
        if !records.iter().any(|record| record.id == approval.id) {
            records.push(ApprovalRecord::pending(session.id, approval));
        }

        Ok(())
    }
}

pub struct MemoryAuthStore {
    users: Mutex<BTreeMap<Uuid, AuthUserRecord>>,
    sessions: Mutex<BTreeMap<String, AuthSession>>,
    audit_events: Mutex<Vec<NewAuthAuditEvent>>,
    roles: Mutex<BTreeMap<Uuid, RbacRole>>,
}

pub struct MemoryOrganizationStore {
    organizations: Mutex<BTreeMap<Uuid, Organization>>,
    memberships: Mutex<BTreeMap<Uuid, OrganizationMembership>>,
    workspaces: Mutex<BTreeMap<Uuid, Workspace>>,
    roles: Mutex<BTreeMap<Uuid, RbacRole>>,
}

impl MemoryOrganizationStore {
    pub fn new() -> Self {
        Self {
            organizations: Mutex::new(BTreeMap::new()),
            memberships: Mutex::new(BTreeMap::new()),
            workspaces: Mutex::new(BTreeMap::new()),
            roles: Mutex::new(BTreeMap::new()),
        }
    }

    fn organization_role_by_id(
        &self,
        organization_id: Uuid,
        id: Uuid,
    ) -> MidgardResult<Option<RbacRole>> {
        Ok(self
            .roles
            .lock()
            .map_err(|_| MidgardError::Storage("RBAC role store poisoned".to_string()))?
            .get(&id)
            .filter(|role| role.organization_id == Some(organization_id))
            .cloned())
    }

    fn organization_role_by_builtin_key(
        &self,
        organization_id: Uuid,
        builtin_key: &str,
    ) -> MidgardResult<Option<RbacRole>> {
        Ok(self
            .roles
            .lock()
            .map_err(|_| MidgardError::Storage("RBAC role store poisoned".to_string()))?
            .values()
            .find(|role| {
                role.organization_id == Some(organization_id)
                    && role.builtin_key.as_deref() == Some(builtin_key)
            })
            .cloned())
    }

    fn is_organization_owner_role_id(
        &self,
        organization_id: Uuid,
        role_id: Uuid,
    ) -> MidgardResult<bool> {
        Ok(self
            .organization_role_by_id(organization_id, role_id)?
            .is_some_and(|role| role.builtin_key.as_deref() == Some(ORG_OWNER_BUILTIN)))
    }

    fn active_organization_owner_count(
        &self,
        organization_id: Uuid,
        memberships: &BTreeMap<Uuid, OrganizationMembership>,
    ) -> MidgardResult<usize> {
        let roles = self
            .roles
            .lock()
            .map_err(|_| MidgardError::Storage("RBAC role store poisoned".to_string()))?;
        Ok(memberships
            .values()
            .filter(|membership| {
                membership.organization_id == organization_id
                    && membership.active
                    && roles
                        .get(&membership.role_id)
                        .is_some_and(|role| role.builtin_key.as_deref() == Some(ORG_OWNER_BUILTIN))
            })
            .count())
    }

    fn seed_builtin_organization_roles(&self, organization_id: Uuid) -> MidgardResult<()> {
        let now = utc_now_rfc3339();
        let mut roles = self
            .roles
            .lock()
            .map_err(|_| MidgardError::Storage("RBAC role store poisoned".to_string()))?;
        for definition in builtin_organization_roles() {
            if roles.values().any(|role| {
                role.organization_id == Some(organization_id)
                    && role.builtin_key.as_deref() == Some(definition.builtin_key)
            }) {
                continue;
            }
            let id = Uuid::new_v4();
            roles.insert(
                id,
                RbacRole {
                    id,
                    scope_kind: RbacScopeKind::Organization,
                    organization_id: Some(organization_id),
                    slug: definition.slug.to_string(),
                    name: definition.name.to_string(),
                    description: Some(definition.description.to_string()),
                    builtin_key: Some(definition.builtin_key.to_string()),
                    protected: definition.protected,
                    archived_at: None,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                    permissions: sorted_permissions(definition.permissions),
                },
            );
        }

        Ok(())
    }
}

impl Default for MemoryAuthStore {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for MemoryOrganizationStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryAuthStore {
    pub fn new() -> Self {
        let now = utc_now_rfc3339();
        let roles = builtin_system_roles()
            .into_iter()
            .map(|definition| {
                let id = definition
                    .id
                    .expect("system builtin roles must have stable ids");
                (
                    id,
                    RbacRole {
                        id,
                        scope_kind: RbacScopeKind::System,
                        organization_id: None,
                        slug: definition.slug.to_string(),
                        name: definition.name.to_string(),
                        description: Some(definition.description.to_string()),
                        builtin_key: Some(definition.builtin_key.to_string()),
                        protected: definition.protected,
                        archived_at: None,
                        created_at: now.clone(),
                        updated_at: now.clone(),
                        permissions: sorted_permissions(definition.permissions),
                    },
                )
            })
            .collect();

        Self {
            users: Mutex::new(BTreeMap::new()),
            sessions: Mutex::new(BTreeMap::new()),
            audit_events: Mutex::new(Vec::new()),
            roles: Mutex::new(roles),
        }
    }

    pub fn audit_event_count(&self) -> MidgardResult<usize> {
        Ok(self
            .audit_events
            .lock()
            .map_err(|_| MidgardError::Storage("auth audit store poisoned".to_string()))?
            .len())
    }

    fn system_role_by_id(&self, id: Uuid) -> MidgardResult<Option<RbacRole>> {
        Ok(self
            .roles
            .lock()
            .map_err(|_| MidgardError::Storage("RBAC role store poisoned".to_string()))?
            .get(&id)
            .cloned())
    }

    fn system_role_by_builtin_key(&self, builtin_key: &str) -> MidgardResult<Option<RbacRole>> {
        Ok(self
            .roles
            .lock()
            .map_err(|_| MidgardError::Storage("RBAC role store poisoned".to_string()))?
            .values()
            .find(|role| role.builtin_key.as_deref() == Some(builtin_key))
            .cloned())
    }

    fn is_system_owner_role_id(&self, role_id: Uuid) -> MidgardResult<bool> {
        Ok(self
            .system_role_by_id(role_id)?
            .is_some_and(|role| role.builtin_key.as_deref() == Some(SYSTEM_OWNER_BUILTIN)))
    }

    fn active_system_owner_count(
        &self,
        users: &BTreeMap<Uuid, AuthUserRecord>,
    ) -> MidgardResult<usize> {
        let roles = self
            .roles
            .lock()
            .map_err(|_| MidgardError::Storage("RBAC role store poisoned".to_string()))?;
        Ok(users
            .values()
            .filter(|record| {
                record.user.active
                    && roles.get(&record.user.system_role_id).is_some_and(|role| {
                        role.builtin_key.as_deref() == Some(SYSTEM_OWNER_BUILTIN)
                    })
            })
            .count())
    }
}

#[async_trait]
impl AgentSessionStore for MemoryAgentSessionStore {
    async fn create_session_in_workspace(
        &self,
        workspace_id: Uuid,
        goal: String,
    ) -> MidgardResult<AgentSession> {
        let session = AgentSession::new(goal);
        self.sessions
            .lock()
            .map_err(|_| MidgardError::Storage("session store poisoned".to_string()))?
            .insert(session.id, session.clone());
        self.session_workspaces
            .lock()
            .map_err(|_| MidgardError::Storage("session workspace store poisoned".to_string()))?
            .insert(session.id, workspace_id);

        Ok(session)
    }

    async fn append_user_message_in_workspace(
        &self,
        workspace_id: Uuid,
        id: Uuid,
        message: String,
    ) -> MidgardResult<AgentSession> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| MidgardError::Storage("session store poisoned".to_string()))?;
        let mut session_workspaces = self
            .session_workspaces
            .lock()
            .map_err(|_| MidgardError::Storage("session workspace store poisoned".to_string()))?;
        if matches!(session_workspaces.get(&id), Some(existing) if *existing != workspace_id) {
            return Err(MidgardError::Storage(format!(
                "session {id} does not belong to workspace {workspace_id}"
            )));
        }
        let session = sessions.entry(id).or_insert_with(|| {
            let mut session = AgentSession::new("resumed session");
            session.id = id;
            session
        });
        session_workspaces.entry(id).or_insert(workspace_id);

        session.messages.push(AgentMessage::user(message));

        Ok(session.clone())
    }

    async fn load_session(&self, id: Uuid) -> MidgardResult<Option<AgentSession>> {
        Ok(self
            .sessions
            .lock()
            .map_err(|_| MidgardError::Storage("session store poisoned".to_string()))?
            .get(&id)
            .cloned())
    }

    async fn load_session_in_workspace(
        &self,
        workspace_id: Uuid,
        id: Uuid,
    ) -> MidgardResult<Option<AgentSession>> {
        let belongs_to_workspace = self
            .session_workspaces
            .lock()
            .map_err(|_| MidgardError::Storage("session workspace store poisoned".to_string()))?
            .get(&id)
            .is_some_and(|existing| *existing == workspace_id);
        if !belongs_to_workspace {
            return Ok(None);
        }

        self.load_session(id).await
    }

    async fn save_session_in_workspace(
        &self,
        workspace_id: Uuid,
        session: AgentSession,
    ) -> MidgardResult<AgentSession> {
        self.upsert_pending_approval_record(&session)?;
        self.sessions
            .lock()
            .map_err(|_| MidgardError::Storage("session store poisoned".to_string()))?
            .insert(session.id, session.clone());
        self.session_workspaces
            .lock()
            .map_err(|_| MidgardError::Storage("session workspace store poisoned".to_string()))?
            .insert(session.id, workspace_id);

        Ok(session)
    }

    async fn list_approval_records(&self, session_id: Uuid) -> MidgardResult<Vec<ApprovalRecord>> {
        Ok(self
            .approval_records
            .lock()
            .map_err(|_| MidgardError::Storage("approval store poisoned".to_string()))?
            .get(&session_id)
            .cloned()
            .unwrap_or_default())
    }

    async fn record_approval_decision(
        &self,
        session_id: Uuid,
        approval: PendingApproval,
        decision: ApprovalDecision,
        actor: String,
        reason: Option<String>,
    ) -> MidgardResult<ApprovalRecord> {
        let mut approval_records = self
            .approval_records
            .lock()
            .map_err(|_| MidgardError::Storage("approval store poisoned".to_string()))?;
        let records = approval_records.entry(session_id).or_default();
        let index = match records.iter().position(|record| record.id == approval.id) {
            Some(index) => index,
            None => {
                records.push(ApprovalRecord::pending(session_id, &approval));
                records.len() - 1
            }
        };

        records[index].record_decision(decision, actor, reason);

        Ok(records[index].clone())
    }
}

#[async_trait]
impl AuthStore for MemoryAuthStore {
    async fn create_user(&self, user: NewUser) -> MidgardResult<AuthUser> {
        let email_lower = normalize_email(&user.email);
        if email_lower.is_empty() {
            return Err(MidgardError::Storage("user email is required".to_string()));
        }

        let system_role_id = match user.system_role_id {
            Some(id) => {
                let role = self
                    .system_role_by_id(id)?
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
                self.system_role_by_builtin_key(builtin_key)?
                    .ok_or_else(|| {
                        MidgardError::Storage(format!(
                            "builtin system role not found: {builtin_key}"
                        ))
                    })?
                    .id
            }
        };
        let mut users = self
            .users
            .lock()
            .map_err(|_| MidgardError::Storage("auth user store poisoned".to_string()))?;
        if users
            .values()
            .any(|record| record.user.email == email_lower)
        {
            return Err(MidgardError::Storage(format!(
                "user already exists: {email_lower}"
            )));
        }

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
        users.insert(
            auth_user.id,
            AuthUserRecord {
                user: auth_user.clone(),
                password_hash: user.password_hash,
            },
        );

        Ok(auth_user)
    }

    async fn list_users(&self) -> MidgardResult<Vec<AuthUser>> {
        let mut users = self
            .users
            .lock()
            .map_err(|_| MidgardError::Storage("auth user store poisoned".to_string()))?
            .values()
            .map(|record| record.user.clone())
            .collect::<Vec<_>>();
        users.sort_by(|left, right| left.email.cmp(&right.email));
        Ok(users)
    }

    async fn load_user_by_id(&self, id: Uuid) -> MidgardResult<Option<AuthUser>> {
        Ok(self
            .users
            .lock()
            .map_err(|_| MidgardError::Storage("auth user store poisoned".to_string()))?
            .get(&id)
            .map(|record| record.user.clone()))
    }

    async fn load_user_by_email(&self, email_lower: &str) -> MidgardResult<Option<AuthUserRecord>> {
        let email_lower = normalize_email(email_lower);
        Ok(self
            .users
            .lock()
            .map_err(|_| MidgardError::Storage("auth user store poisoned".to_string()))?
            .values()
            .find(|record| record.user.email == email_lower)
            .cloned())
    }

    async fn update_user(
        &self,
        id: Uuid,
        update: AuthUserUpdate,
    ) -> MidgardResult<Option<AuthUser>> {
        let mut users = self
            .users
            .lock()
            .map_err(|_| MidgardError::Storage("auth user store poisoned".to_string()))?;
        let Some(record) = users.get(&id) else {
            return Ok(None);
        };
        let next_system_role_id = match update.system_role_id {
            Some(role_id) => {
                let role = self
                    .system_role_by_id(role_id)?
                    .ok_or_else(|| MidgardError::Storage("system role not found".to_string()))?;
                if role.archived_at.is_some() {
                    return Err(MidgardError::Storage(
                        "archived system role cannot be assigned".to_string(),
                    ));
                }
                role_id
            }
            None => record.user.system_role_id,
        };
        let current_active = record.user.active;
        let current_system_role_id = record.user.system_role_id;
        let next_active = update.active.unwrap_or(current_active);
        let removes_owner = current_active
            && self.is_system_owner_role_id(current_system_role_id)?
            && (!next_active || !self.is_system_owner_role_id(next_system_role_id)?);
        if removes_owner && self.active_system_owner_count(&users)? <= 1 {
            return Err(MidgardError::Storage(
                "cannot remove or demote the last system owner".to_string(),
            ));
        }

        let record = users.get_mut(&id).expect("user id came from the same map");
        if let Some(display_name) = update.display_name {
            record.user.display_name = display_name.trim().to_string();
        }
        if let Some(role) = update.role {
            record.user.role = role;
        }
        record.user.system_role_id = next_system_role_id;
        if let Some(password_hash) = update.password_hash {
            record.password_hash = password_hash;
        }
        if let Some(active) = update.active {
            record.user.active = active;
        }
        record.user.updated_at = utc_now_rfc3339();

        Ok(Some(record.user.clone()))
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

        self.sessions
            .lock()
            .map_err(|_| MidgardError::Storage("auth session store poisoned".to_string()))?
            .insert(auth_session.token_hash.clone(), auth_session.clone());

        if let Some(record) = self
            .users
            .lock()
            .map_err(|_| MidgardError::Storage("auth user store poisoned".to_string()))?
            .get_mut(&auth_session.user_id)
        {
            record.user.last_login_at = Some(auth_session.created_at.clone());
            record.user.updated_at = auth_session.created_at.clone();
        }

        Ok(auth_session)
    }

    async fn load_user_by_session(
        &self,
        token_hash: &str,
        now: chrono::DateTime<chrono::Utc>,
    ) -> MidgardResult<Option<AuthUser>> {
        let session = self
            .sessions
            .lock()
            .map_err(|_| MidgardError::Storage("auth session store poisoned".to_string()))?
            .get(token_hash)
            .cloned();
        let Some(session) = session else {
            return Ok(None);
        };
        if session.revoked_at.is_some() || parse_rfc3339_utc(&session.expires_at)? <= now {
            return Ok(None);
        }

        let user = self
            .users
            .lock()
            .map_err(|_| MidgardError::Storage("auth user store poisoned".to_string()))?
            .get(&session.user_id)
            .map(|record| record.user.clone())
            .filter(|user| user.active);

        Ok(user)
    }

    async fn revoke_auth_session(&self, token_hash: &str, revoked_at: String) -> MidgardResult<()> {
        if let Some(session) = self
            .sessions
            .lock()
            .map_err(|_| MidgardError::Storage("auth session store poisoned".to_string()))?
            .get_mut(token_hash)
        {
            session.revoked_at = Some(revoked_at);
        }

        Ok(())
    }

    async fn record_auth_audit_event(&self, event: NewAuthAuditEvent) -> MidgardResult<()> {
        self.audit_events
            .lock()
            .map_err(|_| MidgardError::Storage("auth audit store poisoned".to_string()))?
            .push(event);

        Ok(())
    }

    async fn list_system_roles(&self) -> MidgardResult<Vec<RbacRole>> {
        let mut roles = self
            .roles
            .lock()
            .map_err(|_| MidgardError::Storage("RBAC role store poisoned".to_string()))?
            .values()
            .filter(|role| role.scope_kind == RbacScopeKind::System)
            .cloned()
            .collect::<Vec<_>>();
        roles.sort_by(|left, right| left.slug.cmp(&right.slug));
        Ok(roles)
    }

    async fn load_system_role(&self, id: Uuid) -> MidgardResult<Option<RbacRole>> {
        self.system_role_by_id(id)
    }

    async fn load_system_role_by_builtin_key(
        &self,
        builtin_key: &str,
    ) -> MidgardResult<Option<RbacRole>> {
        self.system_role_by_builtin_key(builtin_key)
    }

    async fn create_system_role(&self, role: NewRbacRole) -> MidgardResult<RbacRole> {
        if role.scope_kind != RbacScopeKind::System || role.organization_id.is_some() {
            return Err(MidgardError::Storage(
                "system role must use system scope".to_string(),
            ));
        }
        PermissionKey::validate_for_scope(&RbacScopeKind::System, &role.permissions)?;
        let slug = normalize_slug(&role.slug)?;
        let name = required_name(&role.name, "role name")?;
        let mut roles = self
            .roles
            .lock()
            .map_err(|_| MidgardError::Storage("RBAC role store poisoned".to_string()))?;
        if roles.values().any(|current| {
            current.scope_kind == RbacScopeKind::System
                && current.organization_id.is_none()
                && current.slug == slug
        }) {
            return Err(MidgardError::Storage(format!(
                "system role slug already exists: {slug}"
            )));
        }

        let now = utc_now_rfc3339();
        let created = RbacRole {
            id: Uuid::new_v4(),
            scope_kind: RbacScopeKind::System,
            organization_id: None,
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
        roles.insert(created.id, created.clone());
        Ok(created)
    }

    async fn update_system_role(
        &self,
        id: Uuid,
        update: RbacRoleUpdate,
    ) -> MidgardResult<Option<RbacRole>> {
        let mut roles = self
            .roles
            .lock()
            .map_err(|_| MidgardError::Storage("RBAC role store poisoned".to_string()))?;
        let Some(role) = roles.get_mut(&id) else {
            return Ok(None);
        };
        if role.scope_kind != RbacScopeKind::System {
            return Ok(None);
        }
        if update.archived == Some(true) && role.protected {
            return Err(MidgardError::Storage(
                "protected system role cannot be archived".to_string(),
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
        Ok(Some(role.clone()))
    }

    async fn replace_system_role_permissions(
        &self,
        id: Uuid,
        permissions: Vec<PermissionKey>,
    ) -> MidgardResult<Option<RbacRole>> {
        PermissionKey::validate_for_scope(&RbacScopeKind::System, &permissions)?;
        let mut roles = self
            .roles
            .lock()
            .map_err(|_| MidgardError::Storage("RBAC role store poisoned".to_string()))?;
        let Some(role) = roles.get_mut(&id) else {
            return Ok(None);
        };
        if role.scope_kind != RbacScopeKind::System {
            return Ok(None);
        }
        if role.archived_at.is_some() {
            return Err(MidgardError::Storage(
                "archived system role permissions cannot be updated".to_string(),
            ));
        }
        let permissions = sorted_permissions(permissions);
        if role.builtin_key.as_deref() == Some(SYSTEM_OWNER_BUILTIN) {
            require_all_permissions(&permissions, PermissionKey::system_permissions())?;
        }
        role.permissions = permissions;
        role.updated_at = utc_now_rfc3339();
        Ok(Some(role.clone()))
    }
}

#[async_trait]
impl OrganizationStore for MemoryOrganizationStore {
    async fn create_organization(
        &self,
        organization: NewOrganization,
    ) -> MidgardResult<Organization> {
        let slug = normalize_slug(&organization.slug)?;
        let name = required_name(&organization.name, "organization name")?;
        let mut organizations = self
            .organizations
            .lock()
            .map_err(|_| MidgardError::Storage("organization store poisoned".to_string()))?;
        if organizations.values().any(|org| org.slug == slug) {
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
        organizations.insert(created.id, created.clone());
        drop(organizations);
        self.seed_builtin_organization_roles(created.id)?;

        Ok(created)
    }

    async fn load_organization_by_slug(&self, slug: &str) -> MidgardResult<Option<Organization>> {
        let slug = normalize_slug(slug)?;
        Ok(self
            .organizations
            .lock()
            .map_err(|_| MidgardError::Storage("organization store poisoned".to_string()))?
            .values()
            .find(|organization| organization.slug == slug && organization.archived_at.is_none())
            .cloned())
    }

    async fn list_contexts_for_user(
        &self,
        user_id: Uuid,
    ) -> MidgardResult<Vec<OrganizationContext>> {
        let organizations = self
            .organizations
            .lock()
            .map_err(|_| MidgardError::Storage("organization store poisoned".to_string()))?
            .clone();
        let memberships = self
            .memberships
            .lock()
            .map_err(|_| {
                MidgardError::Storage("organization membership store poisoned".to_string())
            })?
            .clone();
        let workspaces = self
            .workspaces
            .lock()
            .map_err(|_| MidgardError::Storage("workspace store poisoned".to_string()))?
            .clone();
        let roles = self
            .roles
            .lock()
            .map_err(|_| MidgardError::Storage("RBAC role store poisoned".to_string()))?
            .clone();

        let mut contexts = memberships
            .values()
            .filter(|membership| membership.user_id == user_id && membership.active)
            .filter_map(|membership| {
                let organization = organizations.get(&membership.organization_id)?;
                if organization.archived_at.is_some() {
                    return None;
                }
                let mut organization_workspaces = workspaces
                    .values()
                    .filter(|workspace| {
                        workspace.organization_id == organization.id
                            && workspace.archived_at.is_none()
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                organization_workspaces.sort_by(|left, right| left.slug.cmp(&right.slug));

                Some(OrganizationContext {
                    organization: organization.clone(),
                    membership: membership.clone(),
                    workspaces: organization_workspaces,
                    permissions: roles
                        .get(&membership.role_id)
                        .filter(|role| role.archived_at.is_none())
                        .map(|role| role.permissions.clone())
                        .unwrap_or_default(),
                })
            })
            .collect::<Vec<_>>();
        contexts.sort_by(|left, right| left.organization.slug.cmp(&right.organization.slug));

        Ok(contexts)
    }

    async fn load_membership(
        &self,
        organization_id: Uuid,
        user_id: Uuid,
    ) -> MidgardResult<Option<OrganizationMembership>> {
        Ok(self
            .memberships
            .lock()
            .map_err(|_| {
                MidgardError::Storage("organization membership store poisoned".to_string())
            })?
            .values()
            .find(|membership| {
                membership.organization_id == organization_id
                    && membership.user_id == user_id
                    && membership.active
            })
            .cloned())
    }

    async fn list_memberships(
        &self,
        organization_id: Uuid,
    ) -> MidgardResult<Vec<OrganizationMembership>> {
        let mut memberships = self
            .memberships
            .lock()
            .map_err(|_| {
                MidgardError::Storage("organization membership store poisoned".to_string())
            })?
            .values()
            .filter(|membership| membership.organization_id == organization_id)
            .cloned()
            .collect::<Vec<_>>();
        memberships.sort_by_key(|membership| membership.user_id);
        Ok(memberships)
    }

    async fn create_membership(
        &self,
        membership: NewOrganizationMembership,
    ) -> MidgardResult<OrganizationMembership> {
        let role_id = match membership.role_id {
            Some(role_id) => {
                let role = self
                    .organization_role_by_id(membership.organization_id, role_id)?
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
                self.organization_role_by_builtin_key(membership.organization_id, builtin_key)?
                    .ok_or_else(|| {
                        MidgardError::Storage(format!(
                            "builtin organization role not found: {builtin_key}"
                        ))
                    })?
                    .id
            }
        };
        let mut memberships = self.memberships.lock().map_err(|_| {
            MidgardError::Storage("organization membership store poisoned".to_string())
        })?;
        if memberships.values().any(|current| {
            current.organization_id == membership.organization_id
                && current.user_id == membership.user_id
        }) {
            return Err(MidgardError::Storage(
                "organization membership already exists".to_string(),
            ));
        }

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
        memberships.insert(created.id, created.clone());

        Ok(created)
    }

    async fn update_membership(
        &self,
        organization_id: Uuid,
        user_id: Uuid,
        update: OrganizationMembershipUpdate,
    ) -> MidgardResult<Option<OrganizationMembership>> {
        let mut memberships = self.memberships.lock().map_err(|_| {
            MidgardError::Storage("organization membership store poisoned".to_string())
        })?;
        let Some(id) = memberships
            .values()
            .find(|membership| {
                membership.organization_id == organization_id && membership.user_id == user_id
            })
            .map(|membership| membership.id)
        else {
            return Ok(None);
        };

        let next_role_id = match update.role_id {
            Some(role_id) => {
                let role = self
                    .organization_role_by_id(organization_id, role_id)?
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
                    self.organization_role_by_builtin_key(organization_id, builtin_key)?
                        .ok_or_else(|| {
                            MidgardError::Storage(format!(
                                "builtin organization role not found: {builtin_key}"
                            ))
                        })?
                        .id
                } else {
                    memberships
                        .get(&id)
                        .expect("membership id came from the same map")
                        .role_id
                }
            }
        };
        let current = memberships
            .get(&id)
            .expect("membership id came from the same map");
        let current_active = current.active;
        let current_role_id = current.role_id;
        let next_active = update.active.unwrap_or(current_active);
        let removes_owner = current_active
            && self.is_organization_owner_role_id(organization_id, current_role_id)?
            && (!next_active
                || !self.is_organization_owner_role_id(organization_id, next_role_id)?);
        if removes_owner
            && self.active_organization_owner_count(organization_id, &memberships)? <= 1
        {
            return Err(MidgardError::Storage(
                "cannot remove or demote the last organization owner".to_string(),
            ));
        }

        let current = memberships
            .get_mut(&id)
            .expect("membership id came from the same map");
        if let Some(role) = update.role {
            current.role = role;
        }
        current.role_id = next_role_id;
        if let Some(active) = update.active {
            current.active = active;
        }
        current.updated_at = utc_now_rfc3339();

        Ok(Some(current.clone()))
    }

    async fn create_workspace(&self, workspace: NewWorkspace) -> MidgardResult<Workspace> {
        let slug = normalize_slug(&workspace.slug)?;
        let name = required_name(&workspace.name, "workspace name")?;
        let mut workspaces = self
            .workspaces
            .lock()
            .map_err(|_| MidgardError::Storage("workspace store poisoned".to_string()))?;
        if workspaces.values().any(|current| {
            current.organization_id == workspace.organization_id && current.slug == slug
        }) {
            return Err(MidgardError::Storage(format!(
                "workspace slug already exists: {slug}"
            )));
        }

        let now = utc_now_rfc3339();
        let created = Workspace {
            id: Uuid::new_v4(),
            organization_id: workspace.organization_id,
            slug,
            name,
            archived_at: None,
            created_at: now.clone(),
            updated_at: now,
        };
        workspaces.insert(created.id, created.clone());

        Ok(created)
    }

    async fn load_workspace_by_slug(
        &self,
        organization_id: Uuid,
        slug: &str,
    ) -> MidgardResult<Option<Workspace>> {
        let slug = normalize_slug(slug)?;
        Ok(self
            .workspaces
            .lock()
            .map_err(|_| MidgardError::Storage("workspace store poisoned".to_string()))?
            .values()
            .find(|workspace| {
                workspace.organization_id == organization_id
                    && workspace.slug == slug
                    && workspace.archived_at.is_none()
            })
            .cloned())
    }

    async fn list_workspaces(&self, organization_id: Uuid) -> MidgardResult<Vec<Workspace>> {
        let mut workspaces = self
            .workspaces
            .lock()
            .map_err(|_| MidgardError::Storage("workspace store poisoned".to_string()))?
            .values()
            .filter(|workspace| {
                workspace.organization_id == organization_id && workspace.archived_at.is_none()
            })
            .cloned()
            .collect::<Vec<_>>();
        workspaces.sort_by(|left, right| left.slug.cmp(&right.slug));
        Ok(workspaces)
    }

    async fn update_workspace(
        &self,
        organization_id: Uuid,
        slug: &str,
        update: WorkspaceUpdate,
    ) -> MidgardResult<Option<Workspace>> {
        let slug = normalize_slug(slug)?;
        let mut workspaces = self
            .workspaces
            .lock()
            .map_err(|_| MidgardError::Storage("workspace store poisoned".to_string()))?;
        let Some(workspace) = workspaces.values_mut().find(|workspace| {
            workspace.organization_id == organization_id && workspace.slug == slug
        }) else {
            return Ok(None);
        };

        if let Some(name) = update.name {
            workspace.name = required_name(&name, "workspace name")?;
        }
        if let Some(archived) = update.archived {
            workspace.archived_at = archived.then(utc_now_rfc3339);
        }
        workspace.updated_at = utc_now_rfc3339();

        Ok(Some(workspace.clone()))
    }

    async fn list_organization_roles(&self, organization_id: Uuid) -> MidgardResult<Vec<RbacRole>> {
        let mut roles = self
            .roles
            .lock()
            .map_err(|_| MidgardError::Storage("RBAC role store poisoned".to_string()))?
            .values()
            .filter(|role| {
                role.scope_kind == RbacScopeKind::Organization
                    && role.organization_id == Some(organization_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        roles.sort_by(|left, right| left.slug.cmp(&right.slug));
        Ok(roles)
    }

    async fn load_organization_role(
        &self,
        organization_id: Uuid,
        id: Uuid,
    ) -> MidgardResult<Option<RbacRole>> {
        self.organization_role_by_id(organization_id, id)
    }

    async fn load_organization_role_by_builtin_key(
        &self,
        organization_id: Uuid,
        builtin_key: &str,
    ) -> MidgardResult<Option<RbacRole>> {
        self.organization_role_by_builtin_key(organization_id, builtin_key)
    }

    async fn create_organization_role(&self, role: NewRbacRole) -> MidgardResult<RbacRole> {
        let Some(organization_id) = role.organization_id else {
            return Err(MidgardError::Storage(
                "organization role must include organization_id".to_string(),
            ));
        };
        if role.scope_kind != RbacScopeKind::Organization {
            return Err(MidgardError::Storage(
                "organization role must use organization scope".to_string(),
            ));
        }
        PermissionKey::validate_for_scope(&RbacScopeKind::Organization, &role.permissions)?;
        let slug = normalize_slug(&role.slug)?;
        let name = required_name(&role.name, "role name")?;
        let mut roles = self
            .roles
            .lock()
            .map_err(|_| MidgardError::Storage("RBAC role store poisoned".to_string()))?;
        if roles.values().any(|current| {
            current.scope_kind == RbacScopeKind::Organization
                && current.organization_id == Some(organization_id)
                && current.slug == slug
        }) {
            return Err(MidgardError::Storage(format!(
                "organization role slug already exists: {slug}"
            )));
        }

        let now = utc_now_rfc3339();
        let created = RbacRole {
            id: Uuid::new_v4(),
            scope_kind: RbacScopeKind::Organization,
            organization_id: Some(organization_id),
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
        roles.insert(created.id, created.clone());
        Ok(created)
    }

    async fn update_organization_role(
        &self,
        organization_id: Uuid,
        id: Uuid,
        update: RbacRoleUpdate,
    ) -> MidgardResult<Option<RbacRole>> {
        let mut roles = self
            .roles
            .lock()
            .map_err(|_| MidgardError::Storage("RBAC role store poisoned".to_string()))?;
        let Some(role) = roles.get_mut(&id) else {
            return Ok(None);
        };
        if role.scope_kind != RbacScopeKind::Organization
            || role.organization_id != Some(organization_id)
        {
            return Ok(None);
        }
        if update.archived == Some(true) && role.protected {
            return Err(MidgardError::Storage(
                "protected organization role cannot be archived".to_string(),
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
        Ok(Some(role.clone()))
    }

    async fn replace_organization_role_permissions(
        &self,
        organization_id: Uuid,
        id: Uuid,
        permissions: Vec<PermissionKey>,
    ) -> MidgardResult<Option<RbacRole>> {
        PermissionKey::validate_for_scope(&RbacScopeKind::Organization, &permissions)?;
        let mut roles = self
            .roles
            .lock()
            .map_err(|_| MidgardError::Storage("RBAC role store poisoned".to_string()))?;
        let Some(role) = roles.get_mut(&id) else {
            return Ok(None);
        };
        if role.scope_kind != RbacScopeKind::Organization
            || role.organization_id != Some(organization_id)
        {
            return Ok(None);
        }
        if role.archived_at.is_some() {
            return Err(MidgardError::Storage(
                "archived organization role permissions cannot be updated".to_string(),
            ));
        }
        let permissions = sorted_permissions(permissions);
        if role.builtin_key.as_deref() == Some(ORG_OWNER_BUILTIN) {
            require_all_permissions(&permissions, PermissionKey::organization_permissions())?;
        }
        role.permissions = permissions;
        role.updated_at = utc_now_rfc3339();
        Ok(Some(role.clone()))
    }
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

fn sorted_permissions(permissions: Vec<PermissionKey>) -> Vec<PermissionKey> {
    permissions
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{hash_password, session_token_hash, UserRole};
    use chrono::{Duration, Utc};
    use midgard_agent::{AgentRole, AgentToolCall, ApprovalStatus};
    use midgard_core::RiskLevel;

    #[tokio::test]
    async fn memory_store_creates_session_with_user_goal() {
        let store = MemoryAgentSessionStore::new();

        let session = store
            .create_session("inspect redis".to_string())
            .await
            .unwrap();

        assert_eq!(session.iteration_count, 0);
        assert_eq!(session.messages[0].role, AgentRole::User);
        assert_eq!(session.messages[0].content, "inspect redis");
        assert_eq!(store.load_session(session.id).await.unwrap(), Some(session));
    }

    #[tokio::test]
    async fn memory_store_appends_user_message_in_order() {
        let store = MemoryAgentSessionStore::new();
        let session = store
            .create_session("inspect redis".to_string())
            .await
            .unwrap();

        let session = store
            .append_user_message(session.id, "list pods".to_string())
            .await
            .unwrap();

        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[1].role, AgentRole::User);
        assert_eq!(session.messages[1].content, "list pods");
    }

    #[tokio::test]
    async fn memory_store_preserves_resumed_session_behavior_for_missing_id() {
        let store = MemoryAgentSessionStore::new();
        let id = Uuid::new_v4();

        let session = store
            .append_user_message(id, "continue".to_string())
            .await
            .unwrap();

        assert_eq!(session.messages[0].content, "resumed session");
        assert_eq!(session.messages[1].content, "continue");
    }

    #[tokio::test]
    async fn memory_store_saves_structured_tool_trace() {
        let store = MemoryAgentSessionStore::new();
        let mut session = AgentSession::new("inspect redis");
        session
            .messages
            .push(AgentMessage::assistant_with_tool_calls(
                "",
                vec![AgentToolCall::from_raw(
                    "call_1",
                    "redis_describe",
                    r#"{"namespace":"default","name":"cache"}"#,
                )],
            ));

        let saved = store.save_session(session).await.unwrap();
        let loaded = store.load_session(saved.id).await.unwrap().unwrap();

        assert_eq!(loaded.messages[1].tool_calls[0].name, "redis_describe");
        assert_eq!(
            loaded.messages[1].tool_calls[0].arguments["namespace"],
            "default"
        );
    }

    #[tokio::test]
    async fn pending_approval_save_creates_approval_record() {
        let store = MemoryAgentSessionStore::new();
        let mut session = AgentSession::new("restart redis");
        let approval = pending_restart_approval();
        session.pending_approval = Some(approval.clone());

        store.save_session(session.clone()).await.unwrap();
        let records = store.list_approval_records(session.id).await.unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].id, approval.id);
        assert_eq!(records[0].status, ApprovalStatus::Pending);
        assert_eq!(records[0].actor, None);
    }

    #[tokio::test]
    async fn repeated_pending_approval_save_does_not_duplicate_record() {
        let store = MemoryAgentSessionStore::new();
        let mut session = AgentSession::new("restart redis");
        session.pending_approval = Some(pending_restart_approval());

        store.save_session(session.clone()).await.unwrap();
        store.save_session(session.clone()).await.unwrap();
        let records = store.list_approval_records(session.id).await.unwrap();

        assert_eq!(records.len(), 1);
    }

    #[tokio::test]
    async fn approval_decision_updates_existing_record() {
        let store = MemoryAgentSessionStore::new();
        let mut session = AgentSession::new("restart redis");
        let approval = pending_restart_approval();
        session.pending_approval = Some(approval.clone());
        store.save_session(session.clone()).await.unwrap();

        let record = store
            .record_approval_decision(
                session.id,
                approval,
                ApprovalDecision::Approve,
                "operator@example.com".to_string(),
                Some("maintenance window".to_string()),
            )
            .await
            .unwrap();

        assert_eq!(record.status, ApprovalStatus::Approved);
        assert_eq!(record.actor.as_deref(), Some("operator@example.com"));
        assert_eq!(record.reason.as_deref(), Some("maintenance window"));
        assert!(record.decided_at.is_some());
    }

    #[tokio::test]
    async fn approval_history_remains_after_pending_approval_is_cleared() {
        let store = MemoryAgentSessionStore::new();
        let mut session = AgentSession::new("restart redis");
        let approval = pending_restart_approval();
        session.pending_approval = Some(approval.clone());
        store.save_session(session.clone()).await.unwrap();
        store
            .record_approval_decision(
                session.id,
                approval,
                ApprovalDecision::Reject,
                "operator@example.com".to_string(),
                None,
            )
            .await
            .unwrap();

        session.pending_approval = None;
        store.save_session(session.clone()).await.unwrap();
        let records = store.list_approval_records(session.id).await.unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, ApprovalStatus::Rejected);
    }

    fn pending_restart_approval() -> PendingApproval {
        PendingApproval::new(
            AgentToolCall::from_raw(
                "call_1",
                "redis_restart",
                r#"{"namespace":"default","name":"cache"}"#,
            ),
            RiskLevel::High,
        )
    }

    #[tokio::test]
    async fn memory_auth_store_creates_and_loads_user_by_email() {
        let store = MemoryAuthStore::new();
        let user = store
            .create_user(NewUser {
                email: "Operator@Example.com ".to_string(),
                display_name: "Operator".to_string(),
                role: UserRole::Operator,
                system_role_id: None,
                password_hash: hash_password("valid-password").unwrap(),
                active: true,
            })
            .await
            .unwrap();

        let loaded = store
            .load_user_by_email("operator@example.com")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(loaded.user, user);
        assert_eq!(loaded.user.email, "operator@example.com");
    }

    #[tokio::test]
    async fn memory_auth_store_rejects_duplicate_email() {
        let store = MemoryAuthStore::new();
        let first = NewUser {
            email: "operator@example.com".to_string(),
            display_name: "Operator".to_string(),
            role: UserRole::Operator,
            system_role_id: None,
            password_hash: hash_password("valid-password").unwrap(),
            active: true,
        };
        store.create_user(first.clone()).await.unwrap();

        let err = store.create_user(first).await.unwrap_err();

        assert!(matches!(err, MidgardError::Storage(_)));
        assert!(err.to_string().contains("user already exists"));
    }

    #[tokio::test]
    async fn memory_auth_store_rejects_revoked_and_expired_sessions() {
        let store = MemoryAuthStore::new();
        let user = store
            .create_user(NewUser {
                email: "operator@example.com".to_string(),
                display_name: "Operator".to_string(),
                role: UserRole::Operator,
                system_role_id: None,
                password_hash: hash_password("valid-password").unwrap(),
                active: true,
            })
            .await
            .unwrap();
        let token_hash = session_token_hash("session-token");
        let now = Utc::now();
        store
            .create_auth_session(NewAuthSession {
                user_id: user.id,
                token_hash: token_hash.clone(),
                created_at: now.to_rfc3339(),
                expires_at: (now + Duration::hours(1)).to_rfc3339(),
                user_agent: None,
                ip_address: None,
            })
            .await
            .unwrap();

        assert!(store
            .load_user_by_session(&token_hash, now)
            .await
            .unwrap()
            .is_some());

        store
            .revoke_auth_session(&token_hash, now.to_rfc3339())
            .await
            .unwrap();

        assert!(store
            .load_user_by_session(&token_hash, now)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn memory_organization_store_creates_context_for_owner() {
        let store = MemoryOrganizationStore::new();
        let user_id = Uuid::new_v4();
        let organization = store
            .create_organization(NewOrganization {
                slug: "platform-ops".to_string(),
                name: "Platform Ops".to_string(),
                created_by_user_id: user_id,
            })
            .await
            .unwrap();
        let membership = store
            .create_membership(NewOrganizationMembership {
                organization_id: organization.id,
                user_id,
                role: crate::OrganizationRole::Owner,
                role_id: None,
                active: true,
            })
            .await
            .unwrap();
        let workspace = store
            .create_workspace(NewWorkspace {
                organization_id: organization.id,
                slug: "operations".to_string(),
                name: "Operations".to_string(),
            })
            .await
            .unwrap();

        let contexts = store.list_contexts_for_user(user_id).await.unwrap();

        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0].organization, organization);
        assert_eq!(contexts[0].membership, membership);
        assert_eq!(contexts[0].workspaces, vec![workspace]);
    }

    #[tokio::test]
    async fn memory_organization_store_rejects_duplicate_membership() {
        let store = MemoryOrganizationStore::new();
        let user_id = Uuid::new_v4();
        let organization = store
            .create_organization(NewOrganization {
                slug: "platform-ops".to_string(),
                name: "Platform Ops".to_string(),
                created_by_user_id: user_id,
            })
            .await
            .unwrap();
        let membership = NewOrganizationMembership {
            organization_id: organization.id,
            user_id,
            role: crate::OrganizationRole::Operator,
            role_id: None,
            active: true,
        };

        store.create_membership(membership.clone()).await.unwrap();
        let err = store.create_membership(membership).await.unwrap_err();

        assert!(err.to_string().contains("membership already exists"));
    }

    #[tokio::test]
    async fn memory_organization_store_preserves_last_owner() {
        let store = MemoryOrganizationStore::new();
        let user_id = Uuid::new_v4();
        let organization = store
            .create_organization(NewOrganization {
                slug: "platform-ops".to_string(),
                name: "Platform Ops".to_string(),
                created_by_user_id: user_id,
            })
            .await
            .unwrap();
        store
            .create_membership(NewOrganizationMembership {
                organization_id: organization.id,
                user_id,
                role: crate::OrganizationRole::Owner,
                role_id: None,
                active: true,
            })
            .await
            .unwrap();

        let err = store
            .update_membership(
                organization.id,
                user_id,
                OrganizationMembershipUpdate {
                    role: Some(crate::OrganizationRole::Viewer),
                    role_id: None,
                    active: None,
                },
            )
            .await
            .unwrap_err();

        assert!(err.to_string().contains("last organization owner"));
    }
}
