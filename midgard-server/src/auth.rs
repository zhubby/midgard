use axum::{
    Json,
    extract::{FromRequestParts, Path, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{COOKIE, SET_COOKIE, USER_AGENT},
        request::Parts,
    },
    response::IntoResponse,
};
use chrono::{Duration, Utc};
use midgard_storage::{
    AuthUser, AuthUserUpdate, NewAuthAuditEvent, NewAuthSession, NewUser, PermissionKey, RbacRole,
    UserRole, generate_session_token, hash_password, normalize_email, session_token_hash,
    verify_password,
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use crate::{AppError, AppState};

#[derive(Clone, Debug)]
pub struct AuthSettings {
    pub cookie_name: String,
    pub cookie_secure: bool,
    pub cookie_same_site: String,
    pub session_ttl: Duration,
}

impl AuthSettings {
    pub fn new(
        session_ttl_hours: u64,
        cookie_name: impl Into<String>,
        cookie_secure: bool,
        cookie_same_site: impl Into<String>,
    ) -> Self {
        Self {
            cookie_name: cookie_name.into(),
            cookie_secure,
            cookie_same_site: cookie_same_site.into(),
            session_ttl: Duration::hours(session_ttl_hours.max(1) as i64),
        }
    }

    fn session_cookie(&self, token: &str) -> String {
        format!(
            "{}={}; Path=/; HttpOnly; SameSite={}; Max-Age={}{}",
            self.cookie_name,
            token,
            cookie_same_site(&self.cookie_same_site),
            self.session_ttl.num_seconds(),
            if self.cookie_secure { "; Secure" } else { "" }
        )
    }

    fn expired_cookie(&self) -> String {
        format!(
            "{}=; Path=/; HttpOnly; SameSite={}; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT{}",
            self.cookie_name,
            cookie_same_site(&self.cookie_same_site),
            if self.cookie_secure { "; Secure" } else { "" }
        )
    }
}

impl Default for AuthSettings {
    fn default() -> Self {
        Self::new(12, "midgard_session", false, "lax")
    }
}

#[derive(Clone, Debug, Deserialize, TS)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Clone, Debug, Deserialize, TS)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub display_name: Option<String>,
}

#[derive(Clone, Debug, Deserialize, TS)]
pub struct CreateAuthUserRequest {
    pub email: String,
    pub password: String,
    pub display_name: Option<String>,
    #[serde(default)]
    pub role: Option<UserRole>,
    #[serde(default)]
    #[ts(type = "string | null")]
    pub system_role_id: Option<Uuid>,
    #[serde(default = "default_active")]
    pub active: bool,
}

#[derive(Clone, Debug, Default, Deserialize, TS)]
pub struct UpdateAuthUserRequest {
    pub password: Option<String>,
    pub display_name: Option<String>,
    pub role: Option<UserRole>,
    #[serde(default)]
    #[ts(type = "string | null")]
    pub system_role_id: Option<Uuid>,
    pub active: Option<bool>,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct LogoutResponse {
    pub ok: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
pub struct AuthContext {
    pub user: AuthUser,
    pub system_role: RbacRole,
    pub system_permissions: Vec<PermissionKey>,
}

#[derive(Clone, Debug)]
pub(crate) struct AuthenticatedUser(pub AuthUser);

impl AuthenticatedUser {
    pub(crate) async fn require_system_permission(
        &self,
        state: &AppState,
        permission: PermissionKey,
    ) -> Result<RbacRole, AppError> {
        let role = state
            .auth
            .load_system_role(self.0.system_role_id)
            .await?
            .ok_or_else(|| AppError::Forbidden("system role is not available".to_string()))?;
        if role.has_permission(&permission) {
            return Ok(role);
        }

        Err(AppError::Forbidden(format!(
            "permission {} is required",
            permission.as_str()
        )))
    }

    pub(crate) fn actor(&self) -> String {
        self.0.email.clone()
    }
}

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = cookie_value(&parts.headers, &state.auth_settings.cookie_name)
            .ok_or_else(|| AppError::Unauthorized("authentication is required".to_string()))?;
        let token_hash = session_token_hash(&token);
        let user = state
            .auth
            .load_user_by_session(&token_hash, Utc::now())
            .await?
            .ok_or_else(|| AppError::Unauthorized("authentication is required".to_string()))?;

        Ok(Self(user))
    }
}

pub(crate) async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> Result<impl IntoResponse, AppError> {
    let email = normalize_email(&request.email);
    let ip_address = client_ip(&headers);
    let user_agent = header_string(&headers, USER_AGENT.as_str());
    let Some(record) = state.auth.load_user_by_email(&email).await? else {
        record_login_failure(&state, email, ip_address, user_agent).await?;
        return Err(invalid_credentials());
    };

    if !record.user.active || !verify_password(&request.password, &record.password_hash) {
        record_login_failure(&state, record.user.email, ip_address, user_agent).await?;
        return Err(invalid_credentials());
    }

    create_session_response(&state, &headers, record.user, "login_success", None).await
}

pub(crate) async fn register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<RegisterRequest>,
) -> Result<impl IntoResponse, AppError> {
    let email = normalize_email(&request.email);
    if email.is_empty() {
        return Err(AppError::BadRequest("email is required".to_string()));
    }
    if state.auth.load_user_by_email(&email).await?.is_some() {
        return Err(AppError::Conflict("user already exists".to_string()));
    }

    let is_initial_user = state.auth.list_users().await?.is_empty();
    let display_name = request
        .display_name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| email.clone());
    let role = if is_initial_user {
        UserRole::Admin
    } else {
        UserRole::Viewer
    };
    let created = state
        .auth
        .create_user(NewUser {
            email,
            display_name,
            role,
            system_role_id: None,
            password_hash: hash_password(&request.password)?,
            active: true,
        })
        .await
        .map_err(crate::storage_app_error)?;

    let (response_headers, context) = create_session_response(
        &state,
        &headers,
        created,
        "user_registered",
        Some(format!(r#"{{"initial_user":{is_initial_user}}}"#)),
    )
    .await?;

    Ok((StatusCode::CREATED, response_headers, context))
}

async fn create_session_response(
    state: &AppState,
    headers: &HeaderMap,
    user: AuthUser,
    event_type: &str,
    detail_json: Option<String>,
) -> Result<(HeaderMap, Json<AuthContext>), AppError> {
    let token = generate_session_token();
    let token_hash = session_token_hash(&token);
    let now = Utc::now();
    let ip_address = client_ip(headers);
    let user_agent = header_string(headers, USER_AGENT.as_str());
    state
        .auth
        .create_auth_session(NewAuthSession {
            user_id: user.id,
            token_hash,
            created_at: now.to_rfc3339(),
            expires_at: (now + state.auth_settings.session_ttl).to_rfc3339(),
            user_agent: user_agent.clone(),
            ip_address: ip_address.clone(),
        })
        .await?;
    state
        .auth
        .record_auth_audit_event(NewAuthAuditEvent {
            user_id: Some(user.id),
            event_type: event_type.to_string(),
            email_lower: Some(user.email.clone()),
            occurred_at: now.to_rfc3339(),
            ip_address,
            user_agent,
            detail_json,
        })
        .await?;

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        SET_COOKIE,
        HeaderValue::from_str(&state.auth_settings.session_cookie(&token))
            .map_err(|err| AppError::Internal(format!("build session cookie: {err}")))?,
    );

    let user = state.auth.load_user_by_id(user.id).await?.unwrap_or(user);

    let context = auth_context(state, user).await?;

    Ok((response_headers, Json(context)))
}

pub(crate) async fn logout(
    user: AuthenticatedUser,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, AppError> {
    if let Some(token) = cookie_value(&headers, &state.auth_settings.cookie_name) {
        state
            .auth
            .revoke_auth_session(&session_token_hash(&token), Utc::now().to_rfc3339())
            .await?;
    }
    state
        .auth
        .record_auth_audit_event(NewAuthAuditEvent {
            user_id: Some(user.0.id),
            event_type: "logout".to_string(),
            email_lower: Some(user.0.email),
            occurred_at: Utc::now().to_rfc3339(),
            ip_address: client_ip(&headers),
            user_agent: header_string(&headers, USER_AGENT.as_str()),
            detail_json: None,
        })
        .await?;

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        SET_COOKIE,
        HeaderValue::from_str(&state.auth_settings.expired_cookie())
            .map_err(|err| AppError::Internal(format!("build expired cookie: {err}")))?,
    );

    Ok((response_headers, Json(LogoutResponse { ok: true })))
}

pub(crate) async fn me(
    user: AuthenticatedUser,
    State(state): State<AppState>,
) -> Result<Json<AuthContext>, AppError> {
    Ok(Json(auth_context(&state, user.0).await?))
}

pub(crate) async fn list_users(
    user: AuthenticatedUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<AuthUser>>, AppError> {
    user.require_system_permission(&state, PermissionKey::SystemUsersRead)
        .await?;
    Ok(Json(state.auth.list_users().await?))
}

pub(crate) async fn create_user(
    user: AuthenticatedUser,
    State(state): State<AppState>,
    Json(request): Json<CreateAuthUserRequest>,
) -> Result<(StatusCode, Json<AuthUser>), AppError> {
    user.require_system_permission(&state, PermissionKey::SystemUsersManage)
        .await?;
    let email = normalize_email(&request.email);
    if email.is_empty() {
        return Err(AppError::BadRequest("email is required".to_string()));
    }
    if state.auth.load_user_by_email(&email).await?.is_some() {
        return Err(AppError::Conflict("user already exists".to_string()));
    }

    let display_name = request
        .display_name
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| email.clone());
    let created = state
        .auth
        .create_user(NewUser {
            email: email.clone(),
            display_name,
            role: request.role.unwrap_or(UserRole::Viewer),
            system_role_id: request.system_role_id,
            password_hash: hash_password(&request.password)?,
            active: request.active,
        })
        .await?;
    state
        .auth
        .record_auth_audit_event(NewAuthAuditEvent {
            user_id: Some(created.id),
            event_type: "user_created".to_string(),
            email_lower: Some(email),
            occurred_at: Utc::now().to_rfc3339(),
            ip_address: None,
            user_agent: None,
            detail_json: Some(format!(r#"{{"actor":"{}"}}"#, user.0.email)),
        })
        .await?;

    Ok((StatusCode::CREATED, Json(created)))
}

pub(crate) async fn update_user(
    user: AuthenticatedUser,
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(request): Json<UpdateAuthUserRequest>,
) -> Result<Json<AuthUser>, AppError> {
    user.require_system_permission(&state, PermissionKey::SystemUsersManage)
        .await?;
    let update = AuthUserUpdate {
        display_name: request.display_name,
        role: request.role,
        system_role_id: request.system_role_id,
        password_hash: request.password.as_deref().map(hash_password).transpose()?,
        active: request.active,
    };
    let updated = state
        .auth
        .update_user(id, update)
        .await?
        .ok_or_else(|| AppError::NotFound("user not found".to_string()))?;
    state
        .auth
        .record_auth_audit_event(NewAuthAuditEvent {
            user_id: Some(updated.id),
            event_type: "user_updated".to_string(),
            email_lower: Some(updated.email.clone()),
            occurred_at: Utc::now().to_rfc3339(),
            ip_address: None,
            user_agent: None,
            detail_json: Some(format!(r#"{{"actor":"{}"}}"#, user.0.email)),
        })
        .await?;

    Ok(Json(updated))
}

async fn record_login_failure(
    state: &AppState,
    email_lower: String,
    ip_address: Option<String>,
    user_agent: Option<String>,
) -> Result<(), AppError> {
    state
        .auth
        .record_auth_audit_event(NewAuthAuditEvent {
            user_id: None,
            event_type: "login_failed".to_string(),
            email_lower: if email_lower.is_empty() {
                None
            } else {
                Some(email_lower)
            },
            occurred_at: Utc::now().to_rfc3339(),
            ip_address,
            user_agent,
            detail_json: None,
        })
        .await?;

    Ok(())
}

fn invalid_credentials() -> AppError {
    AppError::Unauthorized("invalid email or password".to_string())
}

async fn auth_context(state: &AppState, user: AuthUser) -> Result<AuthContext, AppError> {
    let system_role = state
        .auth
        .load_system_role(user.system_role_id)
        .await?
        .ok_or_else(|| AppError::Forbidden("system role is not available".to_string()))?;
    let system_permissions = if system_role.archived_at.is_none() {
        system_role.permissions.clone()
    } else {
        Vec::new()
    };

    Ok(AuthContext {
        user,
        system_role,
        system_permissions,
    })
}

fn cookie_value(headers: &HeaderMap, cookie_name: &str) -> Option<String> {
    let cookie_header = headers.get(COOKIE)?.to_str().ok()?;
    cookie_header.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == cookie_name).then(|| value.to_string())
    })
}

fn header_string(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn client_ip(headers: &HeaderMap) -> Option<String> {
    header_string(headers, "x-forwarded-for")
        .and_then(|value| {
            value
                .split(',')
                .next()
                .map(|value| value.trim().to_string())
        })
        .filter(|value| !value.is_empty())
        .or_else(|| header_string(headers, "x-real-ip"))
}

fn cookie_same_site(value: &str) -> &'static str {
    match value.to_ascii_lowercase().as_str() {
        "strict" => "Strict",
        "none" => "None",
        _ => "Lax",
    }
}

fn default_active() -> bool {
    true
}
