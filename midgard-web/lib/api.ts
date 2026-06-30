import type {
  ApprovalRecord,
  AgentSession,
  AgentSessionSummary,
  ApprovalResponse,
  AddOrganizationMemberRequest,
  AuthContext,
  AuthUser,
  CreateAuthUserRequest,
  CreateMiddlewareInstanceRequest,
  CreateOrganizationRequest,
  CreateRbacRoleRequest,
  CreateWorkspaceRequest,
  MiddlewareInstance,
  OrganizationContext,
  OrganizationMemberView,
  OrganizationMembership,
  PermissionCatalogItem,
  PluginResponse,
  ReplaceRolePermissionsRequest,
  RbacRole,
  RunAccepted,
  ToolDefinition,
  UpdateAuthUserRequest,
  UpdateMiddlewareInstanceRequest,
  UpdateOrganizationMemberRequest,
  UpdateRbacRoleRequest,
  UpdateWorkspaceRequest,
  Workspace,
} from "./types";

const API_BASE = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, {
    ...init,
    credentials: "include",
  });

  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new Error(`API ${res.status}: ${body || res.statusText}`);
  }

  return res.json() as Promise<T>;
}

export function fetchCurrentUser(init?: RequestInit): Promise<AuthContext> {
  return request<AuthContext>("/api/auth/me", init);
}

export function login(email: string, password: string): Promise<AuthContext> {
  return request<AuthContext>("/api/auth/login", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ email, password }),
  });
}

export function logout(): Promise<{ ok: boolean }> {
  return request<{ ok: boolean }>("/api/auth/logout", {
    method: "POST",
  });
}

export function fetchOrganizationContexts(): Promise<OrganizationContext[]> {
  return request<OrganizationContext[]>("/api/orgs");
}

export function fetchPermissionCatalog(): Promise<PermissionCatalogItem[]> {
  return request<PermissionCatalogItem[]>("/api/permissions/catalog");
}

export function fetchUsers(): Promise<AuthUser[]> {
  return request<AuthUser[]>("/api/auth/users");
}

export function createUser(payload: CreateAuthUserRequest): Promise<AuthUser> {
  return request<AuthUser>("/api/auth/users", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

export function updateUser(
  userId: string,
  payload: UpdateAuthUserRequest,
): Promise<AuthUser> {
  return request<AuthUser>(`/api/auth/users/${userId}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

export function fetchSystemRoles(): Promise<RbacRole[]> {
  return request<RbacRole[]>("/api/rbac/system/roles");
}

export function createSystemRole(
  payload: CreateRbacRoleRequest,
): Promise<RbacRole> {
  return request<RbacRole>("/api/rbac/system/roles", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

export function updateSystemRole(
  roleId: string,
  payload: UpdateRbacRoleRequest,
): Promise<RbacRole> {
  return request<RbacRole>(`/api/rbac/system/roles/${roleId}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

export function replaceSystemRolePermissions(
  roleId: string,
  payload: ReplaceRolePermissionsRequest,
): Promise<RbacRole> {
  return request<RbacRole>(`/api/rbac/system/roles/${roleId}/permissions`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

export function createOrganization(
  payload: CreateOrganizationRequest,
): Promise<OrganizationContext> {
  return request<OrganizationContext>("/api/orgs", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

export function fetchOrganizationContext(
  orgSlug: string,
): Promise<OrganizationContext> {
  return request<OrganizationContext>(`/api/orgs/${orgSlug}`);
}

export function fetchOrganizationRoles(orgSlug: string): Promise<RbacRole[]> {
  return request<RbacRole[]>(`/api/orgs/${orgSlug}/roles`);
}

export function fetchOrganizationMembers(
  orgSlug: string,
): Promise<OrganizationMemberView[]> {
  return request<OrganizationMemberView[]>(`/api/orgs/${orgSlug}/members`);
}

export function createOrganizationRole(
  orgSlug: string,
  payload: CreateRbacRoleRequest,
): Promise<RbacRole> {
  return request<RbacRole>(`/api/orgs/${orgSlug}/roles`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

export function updateOrganizationRole(
  orgSlug: string,
  roleId: string,
  payload: UpdateRbacRoleRequest,
): Promise<RbacRole> {
  return request<RbacRole>(`/api/orgs/${orgSlug}/roles/${roleId}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

export function replaceOrganizationRolePermissions(
  orgSlug: string,
  roleId: string,
  payload: ReplaceRolePermissionsRequest,
): Promise<RbacRole> {
  return request<RbacRole>(`/api/orgs/${orgSlug}/roles/${roleId}/permissions`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

export function updateOrganizationMember(
  orgSlug: string,
  userId: string,
  payload: UpdateOrganizationMemberRequest,
): Promise<OrganizationMembership> {
  return request<OrganizationMembership>(`/api/orgs/${orgSlug}/members/${userId}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

export function addOrganizationMember(
  orgSlug: string,
  payload: AddOrganizationMemberRequest,
): Promise<OrganizationMembership> {
  return request<OrganizationMembership>(`/api/orgs/${orgSlug}/members`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

export function createWorkspace(
  orgSlug: string,
  payload: CreateWorkspaceRequest,
): Promise<Workspace> {
  return request<Workspace>(`/api/orgs/${orgSlug}/workspaces`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

export function fetchWorkspace(
  orgSlug: string,
  workspaceSlug: string,
): Promise<Workspace> {
  return request<Workspace>(`/api/orgs/${orgSlug}/workspaces/${workspaceSlug}`);
}

export function updateWorkspace(
  orgSlug: string,
  workspaceSlug: string,
  payload: UpdateWorkspaceRequest,
): Promise<Workspace> {
  return request<Workspace>(`/api/orgs/${orgSlug}/workspaces/${workspaceSlug}`, {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(payload),
  });
}

function workspaceApiPath(
  orgSlug: string,
  workspaceSlug: string,
  suffix: string,
) {
  return `/api/orgs/${orgSlug}/workspaces/${workspaceSlug}${suffix}`;
}

export function fetchTools(
  orgSlug: string,
  workspaceSlug: string,
): Promise<ToolDefinition[]> {
  return request<ToolDefinition[]>(
    workspaceApiPath(orgSlug, workspaceSlug, "/tools"),
  );
}

export function fetchPlugins(
  orgSlug: string,
  workspaceSlug: string,
): Promise<PluginResponse[]> {
  return request<PluginResponse[]>(
    workspaceApiPath(orgSlug, workspaceSlug, "/plugins"),
  );
}

export function fetchSessions(
  orgSlug: string,
  workspaceSlug: string,
): Promise<AgentSessionSummary[]> {
  return request<AgentSessionSummary[]>(
    workspaceApiPath(orgSlug, workspaceSlug, "/agent/sessions"),
  );
}

export function fetchMiddlewareInstances(
  orgSlug: string,
  workspaceSlug: string,
): Promise<MiddlewareInstance[]> {
  return request<MiddlewareInstance[]>(
    workspaceApiPath(orgSlug, workspaceSlug, "/middleware"),
  );
}

export function createMiddlewareInstance(
  orgSlug: string,
  workspaceSlug: string,
  payload: CreateMiddlewareInstanceRequest,
): Promise<MiddlewareInstance> {
  return request<MiddlewareInstance>(
    workspaceApiPath(orgSlug, workspaceSlug, "/middleware"),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    },
  );
}

export function updateMiddlewareInstance(
  orgSlug: string,
  workspaceSlug: string,
  id: string,
  payload: UpdateMiddlewareInstanceRequest,
): Promise<MiddlewareInstance> {
  return request<MiddlewareInstance>(
    workspaceApiPath(orgSlug, workspaceSlug, `/middleware/${id}`),
    {
      method: "PATCH",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    },
  );
}

export function createSession(
  orgSlug: string,
  workspaceSlug: string,
  goal: string,
): Promise<AgentSession> {
  return request<AgentSession>(
    workspaceApiPath(orgSlug, workspaceSlug, "/agent/sessions"),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ goal }),
    },
  );
}

export function sendMessage(
  orgSlug: string,
  workspaceSlug: string,
  sessionId: string,
  message: string,
): Promise<AgentSession> {
  return request<AgentSession>(
    workspaceApiPath(
      orgSlug,
      workspaceSlug,
      `/agent/sessions/${sessionId}/messages`,
    ),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ message }),
    },
  );
}

export function runAgent(
  orgSlug: string,
  workspaceSlug: string,
  sessionId: string,
): Promise<RunAccepted> {
  return request<RunAccepted>(
    workspaceApiPath(
      orgSlug,
      workspaceSlug,
      `/agent/sessions/${sessionId}/runs`,
    ),
    {
      method: "POST",
    },
  );
}

export function decideApproval(
  orgSlug: string,
  workspaceSlug: string,
  sessionId: string,
  decision: "approve" | "reject",
  reason?: string,
  resume = true,
): Promise<ApprovalResponse> {
  return request<ApprovalResponse>(
    workspaceApiPath(
      orgSlug,
      workspaceSlug,
      `/agent/sessions/${sessionId}/approvals`,
    ),
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ decision, reason, resume }),
    },
  );
}

export function fetchApprovals(
  orgSlug: string,
  workspaceSlug: string,
  sessionId: string,
): Promise<ApprovalRecord[]> {
  return request<ApprovalRecord[]>(
    workspaceApiPath(
      orgSlug,
      workspaceSlug,
      `/agent/sessions/${sessionId}/approvals`,
    ),
  );
}
