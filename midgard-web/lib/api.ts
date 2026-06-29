import type {
  ApprovalRecord,
  AgentSession,
  ApprovalResponse,
  AuthUser,
  CreateOrganizationRequest,
  CreateWorkspaceRequest,
  OrganizationContext,
  PluginResponse,
  RunAccepted,
  ToolDefinition,
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

export function fetchCurrentUser(init?: RequestInit): Promise<AuthUser> {
  return request<AuthUser>("/api/auth/me", init);
}

export function login(email: string, password: string): Promise<AuthUser> {
  return request<AuthUser>("/api/auth/login", {
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
