"use client";

import { type FormEvent } from "react";
import {
  AlertTriangle,
  Braces,
  CheckCircle2,
  Send,
  Sparkles,
  Wrench,
  XCircle,
} from "lucide-react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import type {
  AgentMessage,
  AgentRunStatus,
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
  onSend: (prompt: string) => void;
  pendingApproval: PendingApproval | null;
  runStatus: AgentRunStatus | "idle";
  streamingAssistant: string;
  trace: AgentTraceItem[];
}

const quickPrompts = [
  "Check Redis failover readiness",
  "Summarize middleware risk",
  "Plan a safe Kafka restart",
];

const markdownComponents: Components = {
  a({ children, href, node, ...props }) {
    const opensNewTab = Boolean(href) && !href?.startsWith("#");

    return (
      <a
        {...props}
        href={href}
        rel={opensNewTab ? "noreferrer" : undefined}
        target={opensNewTab ? "_blank" : undefined}
      >
        {children}
      </a>
    );
  },
  table({ children, node, ...props }) {
    return (
      <div className="markdown-table-scroll">
        <table {...props}>{children}</table>
      </div>
    );
  },
};

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

function MarkdownContent({ content }: { content: string }) {
  return (
    <div className="markdown-content">
      <ReactMarkdown
        components={markdownComponents}
        remarkPlugins={[remarkGfm]}
        skipHtml
      >
        {content}
      </ReactMarkdown>
    </div>
  );
}

function toolResultStatus(traceItem?: AgentTraceItem) {
  if (traceItem?.tone === "warn") {
    return {
      label: "Needs attention",
      tone: "warn",
      icon: <AlertTriangle aria-hidden="true" />,
    };
  }

  if (traceItem?.tone === "ready") {
    return {
      label: "Completed",
      tone: "ready",
      icon: <CheckCircle2 aria-hidden="true" />,
    };
  }

  return {
    label: "Result",
    tone: "neutral",
    icon: <Wrench aria-hidden="true" />,
  };
}

function parseToolOutput(content: string) {
  const trimmed = content.trim();

  if (!trimmed) {
    return { kind: "empty" as const, value: "No output returned." };
  }

  try {
    const parsed: unknown = JSON.parse(trimmed);

    if (typeof parsed === "string") {
      return { kind: "text" as const, value: parsed };
    }

    return { kind: "json" as const, value: JSON.stringify(parsed, null, 2) };
  } catch {
    return { kind: "text" as const, value: content };
  }
}

function ToolResultCard({
  message,
  traceItem,
}: {
  message: AgentMessage;
  traceItem?: AgentTraceItem;
}) {
  const status = toolResultStatus(traceItem);
  const output = parseToolOutput(message.content);
  const callId = message.tool_call_id ?? "untracked";
  const shortCallId =
    callId.length > 16 ? `${callId.slice(0, 12)}...` : callId;

  return (
    <div
      aria-label={`${traceItem?.label ?? "Tool"} execution result`}
      className={`tool-result-card ${status.tone}`}
      role="group"
    >
      <div className="tool-result-header">
        <span className="tool-result-icon">{status.icon}</span>
        <div className="tool-result-heading">
          <span>Tool execution</span>
          <strong>{traceItem?.label ?? "Tool result"}</strong>
        </div>
        <span className={`tool-result-status ${status.tone}`}>
          {status.label}
        </span>
      </div>

      <div className="tool-result-meta-row">
        <span>Call id</span>
        <code title={callId}>{shortCallId}</code>
      </div>

      <div className={`tool-result-body ${output.kind}`}>
        {output.kind === "json" ? (
          <>
            <div className="tool-result-code-label">
              <Braces aria-hidden="true" />
              JSON output
            </div>
            <pre>
              <code>{output.value}</code>
            </pre>
          </>
        ) : (
          <p>{output.value}</p>
        )}
      </div>
    </div>
  );
}

function MessageContent({ message }: { message: AgentMessage }) {
  const content = message.content || "Tool call requested.";

  if (message.role === "assistant") {
    return <MarkdownContent content={content} />;
  }

  return <p>{content}</p>;
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
  onSend,
  pendingApproval,
  runStatus,
  streamingAssistant,
  trace,
}: AgentConsoleProps) {
  function handleSubmit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();
    onSend(draft);
  }

  return (
    <section className="workspace-panel agent-panel" aria-labelledby="agent-title">
      <div className="panel-header">
        <div>
          <p className="section-kicker">Agent chat</p>
          <h2 id="agent-title">Plan, inspect, and gate operations</h2>
        </div>
        <span className="badge badge-secondary">{runStatus}</span>
      </div>

      <div className="quick-prompts" aria-label="Prompt shortcuts">
        {quickPrompts.map((prompt) => (
          <button
            className="button button-secondary button-compact prompt-chip"
            disabled={busy || !canOperate}
            key={prompt}
            type="button"
            onClick={() => onDraftChange(prompt)}
          >
            <Sparkles aria-hidden="true" />
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

        {messages.map((message, index) => {
          const toolTraceItem = message.tool_call_id
            ? trace.find((step) => step.id === message.tool_call_id)
            : undefined;

          return (
            <article
              className={`chat-message ${message.role}`}
              key={`${message.role}-${message.tool_call_id ?? index}-${index}`}
            >
              <span className="message-avatar" aria-hidden="true">
                {messageAvatar(message.role)}
              </span>
              <div>
                {message.role === "tool" ? (
                  <ToolResultCard message={message} traceItem={toolTraceItem} />
                ) : (
                  <>
                    <div className="message-meta">
                      <span>{messageLabel(message)}</span>
                      <small>{messageMeta(message)}</small>
                    </div>
                    <MessageContent message={message} />
                  </>
                )}
              </div>
            </article>
          );
        })}

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
              <MarkdownContent content={streamingAssistant} />
            </div>
          </article>
        )}
      </div>

      {pendingApproval && (
        <section className="approval-region" aria-labelledby="approval-title">
          <article className="approval-card">
            <div>
              <p className="section-kicker">Approval required</p>
              <h3 id="approval-title">{pendingApproval.tool_call.name}</h3>
              <p>
                {pendingApproval.risk_level} risk action awaiting operator
                decision.
              </p>
            </div>
            <div className="approval-actions">
              <button
                className="button button-danger"
                disabled={busy || !canOperate}
                type="button"
                onClick={() => onApproval("reject")}
              >
                <XCircle aria-hidden="true" />
                Reject
              </button>
              <button
                className="button button-primary"
                disabled={busy || !canOperate}
                type="button"
                onClick={() => onApproval("approve")}
              >
                <CheckCircle2 aria-hidden="true" />
                Approve
              </button>
            </div>
          </article>
        </section>
      )}

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
            <Send aria-hidden="true" />
            {busy ? "Running" : canOperate ? "Send" : "Read only"}
          </button>
        </div>
      </form>
    </section>
  );
}
