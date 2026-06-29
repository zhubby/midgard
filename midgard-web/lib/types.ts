export type RiskLevel = "low" | "medium" | "high" | "critical";

export interface PluginResponse {
  id: string;
  name: string;
  middleware_kind: string;
}

export interface ToolDefinition {
  name: string;
  description: string;
  parameters_schema: unknown;
  risk_level: RiskLevel;
  requires_approval: boolean;
}

export type AgentRole = "system" | "user" | "assistant" | "tool";

export interface AgentMessage {
  role: AgentRole;
  content: string;
}

export interface AgentSession {
  id: string;
  messages: AgentMessage[];
  iteration_count: number;
}
