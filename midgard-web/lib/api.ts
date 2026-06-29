import type {
  ApprovalRecord,
  AgentSession,
  ApprovalResponse,
  AuthUser,
  PluginResponse,
  RunAccepted,
  ToolDefinition,
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

export function fetchTools(): Promise<ToolDefinition[]> {
  return request<ToolDefinition[]>("/api/tools");
}

export function fetchPlugins(): Promise<PluginResponse[]> {
  return request<PluginResponse[]>("/api/plugins");
}

export function createSession(goal: string): Promise<AgentSession> {
  return request<AgentSession>("/api/agent/sessions", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ goal }),
  });
}

export function sendMessage(
  sessionId: string,
  message: string,
): Promise<AgentSession> {
  return request<AgentSession>(`/api/agent/sessions/${sessionId}/messages`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ message }),
  });
}

export function runAgent(sessionId: string): Promise<RunAccepted> {
  return request<RunAccepted>(`/api/agent/sessions/${sessionId}/runs`, {
    method: "POST",
  });
}

export function decideApproval(
  sessionId: string,
  decision: "approve" | "reject",
  reason?: string,
  resume = true,
): Promise<ApprovalResponse> {
  return request<ApprovalResponse>(`/api/agent/sessions/${sessionId}/approvals`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ decision, reason, resume }),
  });
}

export function fetchApprovals(sessionId: string): Promise<ApprovalRecord[]> {
  return request<ApprovalRecord[]>(`/api/agent/sessions/${sessionId}/approvals`);
}
