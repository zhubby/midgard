"use client";

import { type FormEvent } from "react";
import type {
  AgentMessage,
  AgentRunStatus,
  AgentSessionSummary,
  PendingApproval,
} from "@/lib/types";
import type { WorkspaceConnectionStatus } from "@/lib/events";

type TraceTone = "ready" | "pending" | "warn";

export interface AgentTraceItem {
  id: string;
  label: string;
  detail: string;
  tone: TraceTone;
}

interface AgentConsoleProps {
  busy: boolean;
  canOperate: boolean;
  connectionStatus: WorkspaceConnectionStatus;
  draft: string;
  error: string | null;
  messages: AgentMessage[];
  onApproval: (decision: "approve" | "reject") => void;
  onDraftChange: (draft: string) => void;
  onNewSession: () => void;
  onSend: (prompt: string) => void;
  onSessionSelect: (sessionId: string) => void;
  pendingApproval: PendingApproval | null;
  runStatus: AgentRunStatus | "idle";
  activeSessionId: string | null;
  sessions: AgentSessionSummary[];
  streamingAssistant: string;
  trace: AgentTraceItem[];
}

const quickPrompts = [
  "Check Redis failover readiness",
  "Summarize middleware risk",
  "Plan a safe Kafka restart",
];

function messageLabel(message: AgentMessage) {
  switch (message.role) {
    case "assistant":
      return "Midgard agent";
    case "tool":
      return message.tool_call_id ? "Tool result" : "Tool";
    case "system":
      return "System";
    case "user":
      return "Operator";
  }
}

function messageMeta(message: AgentMessage) {
  if (message.tool_calls && message.tool_calls.length > 0) {
    return `${message.tool_calls.length} tool call${
      message.tool_calls.length === 1 ? "" : "s"
    }`;
  }
  if (message.tool_call_id) return message.tool_call_id.slice(0, 8);
  return message.role === "assistant" ? "committed" : "message";
}

function messageAvatar(role: AgentMessage["role"]) {
  switch (role) {
    case "assistant":
      return "A";
    case "tool":
      return "T";
    case "system":
      return "S";
    case "user":
      return "U";
  }
}

export function AgentConsole({
  busy,
  canOperate,
  connectionStatus,
  draft,
  error,
  messages,
  onApproval,
  onDraftChange,
  onNewSession,
  onSend,
  onSessionSelect,
  pendingApproval,
  runStatus,
  activeSessionId,
  sessions,
  streamingAssistant,
  trace,
}: AgentConsoleProps) {
  function handleSubmit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    onSend(draft);
  }

  const traceItems =
    trace.length > 0
      ? trace
      : [
          {
            id: "waiting",
            label: "Waiting for events",
            detail: "Submit an operations goal to start a live agent run.",
            tone: "pending" as const,
          },
        ];

  return (
    <section className="workspace-panel agent-panel" aria-labelledby="agent-title">
      <div className="panel-header">
        <div>
          <p className="section-kicker">Agent chat</p>
          <h2 id="agent-title">Plan, inspect, and gate operations</h2>
        </div>
        <span className="badge badge-secondary">{runStatus}</span>
      </div>

      <div className="session-strip" aria-label="Agent sessions">
        <label className="sr-only" htmlFor="agent-session-select">
          Agent session
        </label>
        <select
          id="agent-session-select"
          value={activeSessionId ?? ""}
          onChange={(event) => onSessionSelect(event.target.value)}
        >
          <option value="">Workspace live state</option>
          {sessions.map((session) => (
            <option key={session.id} value={session.id}>
              {session.title.slice(0, 64)} · {session.status}
            </option>
          ))}
        </select>
        <button
          className="button button-outline"
          disabled={busy || !canOperate}
          type="button"
          onClick={onNewSession}
        >
          New session
        </button>
      </div>

      <div className="quick-prompts" aria-label="Prompt shortcuts">
        {quickPrompts.map((prompt) => (
          <button
            className="button button-ghost prompt-chip"
            disabled={busy || !canOperate}
            key={prompt}
            type="button"
            onClick={() => onDraftChange(prompt)}
          >
            {prompt}
          </button>
        ))}
      </div>

      {error && (
        <div className="inline-alert" role="alert">
          {error}
        </div>
      )}
      {!canOperate && (
        <div className="inline-alert" role="status">
          This role can read workspace state but cannot run agent operations.
        </div>
      )}

      <div className="chat-thread" role="log" aria-live="polite">
        {messages.length === 0 && !streamingAssistant && (
          <article className="chat-message assistant">
            <span className="message-avatar" aria-hidden="true">
              A
            </span>
            <div>
              <div className="message-meta">
                <span>Midgard agent</span>
                <small>{connectionStatus}</small>
              </div>
              <p>
                Describe an operational outcome. The live trace will show model
                output, tool calls, approval gates, and completion events.
              </p>
            </div>
          </article>
        )}

        {messages.map((message, index) => (
          <article
            className={`chat-message ${message.role}`}
            key={`${message.role}-${message.tool_call_id ?? index}-${index}`}
          >
            <span className="message-avatar" aria-hidden="true">
              {messageAvatar(message.role)}
            </span>
            <div>
              <div className="message-meta">
                <span>{messageLabel(message)}</span>
                <small>{messageMeta(message)}</small>
              </div>
              <p>{message.content || "Tool call requested."}</p>
            </div>
          </article>
        ))}

        {streamingAssistant && (
          <article className="chat-message assistant streaming">
            <span className="message-avatar" aria-hidden="true">
              A
            </span>
            <div>
              <div className="message-meta">
                <span>Midgard agent</span>
                <small>streaming</small>
              </div>
              <p>{streamingAssistant}</p>
            </div>
          </article>
        )}
      </div>

      <section className="trace-panel" aria-labelledby="trace-title">
        <div className="trace-header">
          <h3 id="trace-title">Execution trace</h3>
          <span className="badge badge-outline">{traceItems.length} steps</span>
        </div>
        <ol className="trace-list">
          {traceItems.map((step) => (
            <li className={`trace-step ${step.tone}`} key={step.id}>
              <span aria-hidden="true" />
              <div>
                <strong>{step.label}</strong>
                <p>{step.detail}</p>
              </div>
            </li>
          ))}
        </ol>

        {pendingApproval && (
          <article className="approval-card">
            <div>
              <p className="section-kicker">Approval required</p>
              <h3>{pendingApproval.tool_call.name}</h3>
              <p>
                {pendingApproval.risk_level} risk action awaiting operator
                decision.
              </p>
            </div>
            <div className="approval-actions">
              <button
                className="button button-outline"
                disabled={busy || !canOperate}
                type="button"
                onClick={() => onApproval("reject")}
              >
                Reject
              </button>
              <button
                className="button button-primary"
                disabled={busy || !canOperate}
                type="button"
                onClick={() => onApproval("approve")}
              >
                Approve
              </button>
            </div>
          </article>
        )}
      </section>

      <form className="composer" onSubmit={handleSubmit}>
        <label className="sr-only" htmlFor="agent-message">
          Message to agent
        </label>
        <textarea
          id="agent-message"
          name="message"
          placeholder="Ask the agent to inspect, summarize, or plan a middleware operation..."
          rows={4}
          value={draft}
          disabled={busy || !canOperate}
          onChange={(e) => onDraftChange(e.target.value)}
        />
        <div className="composer-actions">
          <span>
            {connectionStatus === "connected"
              ? "Live events connected."
              : "Waiting for workspace events."}
          </span>
          <button
            className="button button-primary"
            disabled={busy || !canOperate}
            type="submit"
          >
            {busy ? "Running" : canOperate ? "Send" : "Read only"}
          </button>
        </div>
      </form>
    </section>
  );
}
