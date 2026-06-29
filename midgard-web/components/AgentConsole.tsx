"use client";

import { type FormEvent, useState } from "react";

type ChatRole = "assistant" | "user" | "tool";
type TraceTone = "ready" | "pending" | "warn";

interface ChatMessage {
  id: string;
  role: ChatRole;
  eyebrow: string;
  content: string;
  meta?: string;
}

interface TraceStep {
  label: string;
  detail: string;
  tone: TraceTone;
}

const quickPrompts = [
  "Check Redis failover readiness",
  "Summarize middleware risk",
  "Plan a safe Kafka restart",
];

const initialMessages: ChatMessage[] = [
  {
    id: "welcome",
    role: "assistant",
    eyebrow: "Midgard agent",
    meta: "standing by",
    content:
      "Describe the operational outcome. I will keep the plan scoped to tools, risk, and approval boundaries.",
  },
  {
    id: "tool-catalog",
    role: "tool",
    eyebrow: "Available context",
    meta: "workspace context",
    content:
      "Plugins: Redis, Kafka, PostgreSQL. Mutating actions stay approval-gated.",
  },
];

const defaultTrace: TraceStep[] = [
  {
    label: "Intent parsed",
    detail: "Target, namespace, and operation type identified from the prompt.",
    tone: "ready",
  },
  {
    label: "Risk classified",
    detail: "Read-only inspection stays low risk. Mutations require approval.",
    tone: "ready",
  },
  {
    label: "Tool plan drafted",
    detail: "Plan is staged for operator review before any execution.",
    tone: "pending",
  },
];

function makeMessageId(prefix: string) {
  return `${prefix}-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function buildAgentReply(prompt: string) {
  const text = prompt.toLowerCase();

  if (text.includes("kafka")) {
    return "Kafka restart should be staged by broker, with partition leadership checked before and after each step. Mark the restart operation as high risk and request approval before any disruptive action.";
  }

  if (text.includes("risk")) {
    return "Current middleware risk is concentrated in write paths: failover, restart, scaling, and config changes. Read-only inspection can proceed without approval, but execution should expose the selected tool and arguments first.";
  }

  if (text.includes("redis")) {
    return "Redis should be inspected through workload health, replica lag, recent events, and configured persistence. Failover readiness depends on replica freshness and whether Sentinel or controller ownership is healthy.";
  }

  return "I would turn that goal into a short tool plan, classify the operational risk, and surface any approval requirement before execution.";
}

function buildTrace(prompt: string): TraceStep[] {
  const isMutation = /(restart|scale|failover|delete|apply|update)/i.test(prompt);

  return [
    {
      label: "Goal normalized",
      detail: "Converted the operator request into target, action, and success signal.",
      tone: "ready",
    },
    {
      label: isMutation ? "Approval required" : "Approval not required",
      detail: isMutation
        ? "The requested action can alter runtime state and must be reviewed."
        : "The request can be handled with read-only inspection tools.",
      tone: isMutation ? "warn" : "ready",
    },
    {
      label: "Execution held",
      detail: "This screen is intentionally disconnected from backend APIs.",
      tone: "pending",
    },
  ];
}

export function AgentConsole() {
  const [messages, setMessages] = useState<ChatMessage[]>(initialMessages);
  const [trace, setTrace] = useState<TraceStep[]>(defaultTrace);
  const [draft, setDraft] = useState(
    "Inspect Redis in the default namespace and report whether it is healthy.",
  );

  function handleSubmit(e: FormEvent<HTMLFormElement>) {
    e.preventDefault();

    const prompt = draft.trim();
    if (!prompt) return;

    setMessages((current) => [
      ...current,
      {
        id: makeMessageId("user"),
        role: "user",
        eyebrow: "Operator",
        meta: "now",
        content: prompt,
      },
      {
        id: makeMessageId("assistant"),
        role: "assistant",
        eyebrow: "Midgard agent",
        meta: "draft plan",
        content: buildAgentReply(prompt),
      },
    ]);
    setTrace(buildTrace(prompt));
    setDraft("");
  }

  return (
    <section className="workspace-panel agent-panel" aria-labelledby="agent-title">
      <div className="panel-header">
        <div>
          <p className="section-kicker">Agent chat</p>
          <h2 id="agent-title">Plan, inspect, and gate operations</h2>
        </div>
        <span className="badge badge-secondary">Draft mode</span>
      </div>

      <div className="quick-prompts" aria-label="Prompt shortcuts">
        {quickPrompts.map((prompt) => (
          <button
            className="button button-ghost prompt-chip"
            key={prompt}
            type="button"
            onClick={() => setDraft(prompt)}
          >
            {prompt}
          </button>
        ))}
      </div>

      <div className="chat-thread" role="log" aria-live="polite">
        {messages.map((message) => (
          <article className={`chat-message ${message.role}`} key={message.id}>
            <span className="message-avatar" aria-hidden="true">
              {message.role === "assistant"
                ? "A"
                : message.role === "tool"
                  ? "T"
                  : "U"}
            </span>
            <div>
              <div className="message-meta">
                <span>{message.eyebrow}</span>
                {message.meta && <small>{message.meta}</small>}
              </div>
              <p>{message.content}</p>
            </div>
          </article>
        ))}
      </div>

      <section className="trace-panel" aria-labelledby="trace-title">
        <div className="trace-header">
          <h3 id="trace-title">Execution trace</h3>
          <span className="badge badge-outline">3 steps</span>
        </div>
        <ol className="trace-list">
          {trace.map((step) => (
            <li className={`trace-step ${step.tone}`} key={step.label}>
              <span aria-hidden="true" />
              <div>
                <strong>{step.label}</strong>
                <p>{step.detail}</p>
              </div>
            </li>
          ))}
        </ol>
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
          onChange={(e) => setDraft(e.target.value)}
        />
        <div className="composer-actions">
          <span>Review before execution.</span>
          <button className="button button-primary" type="submit">
            Send
          </button>
        </div>
      </form>
    </section>
  );
}
