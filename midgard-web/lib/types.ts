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
export type AgentRunStatus =
  | "running"
  | "completed"
  | "awaiting_approval"
  | "responded"
  | "max_iterations"
  | "failed";

export interface AgentToolCall {
  id: string;
  name: string;
  arguments: unknown;
  raw_arguments: string;
}

export interface AgentMessage {
  role: AgentRole;
  content: string;
  tool_calls?: AgentToolCall[];
  tool_call_id?: string;
}

export interface PendingApproval {
  id: string;
  tool_call: AgentToolCall;
  risk_level: RiskLevel;
  approved?: boolean;
}

export type ApprovalStatus = "pending" | "approved" | "rejected";

export interface ApprovalRecord {
  id: string;
  session_id: string;
  tool_call: AgentToolCall;
  risk_level: RiskLevel;
  status: ApprovalStatus;
  requested_at: string;
  decided_at?: string;
  actor?: string;
  reason?: string;
}

export interface AgentSession {
  id: string;
  messages: AgentMessage[];
  iteration_count: number;
  status: AgentRunStatus;
  pending_approval?: PendingApproval;
  last_error?: string;
}

export interface AgentRunEvent {
  type: string;
  [key: string]: unknown;
}

export interface AgentRunResponse {
  status: AgentRunStatus;
  pending_approval?: PendingApproval;
  events: AgentRunEvent[];
  session: AgentSession;
}

export interface ApprovalResponse {
  approval_record: ApprovalRecord;
  session: AgentSession;
}
