import type { AgentSession, PluginResponse, ToolDefinition } from "./types";

const API_BASE = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080";

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(`${API_BASE}${path}`, init);

  if (!res.ok) {
    const body = await res.text().catch(() => "");
    throw new Error(`API ${res.status}: ${body || res.statusText}`);
  }

  return res.json() as Promise<T>;
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
