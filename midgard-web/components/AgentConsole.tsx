"use client";

import { useState, FormEvent } from "react";
import type { AgentSession } from "@/lib/types";
import { createSession, sendMessage } from "@/lib/api";

export function AgentConsole({
  initialSession,
}: {
  initialSession: AgentSession | null;
}) {
  const [session, setSession] = useState<AgentSession | null>(initialSession);
  const [goal, setGoal] = useState(
    "Inspect Redis in the default namespace and report whether it is healthy.",
  );
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function handleRun(e: FormEvent) {
    e.preventDefault();
    if (!goal.trim()) return;

    setRunning(true);
    setError(null);
    try {
      const s = await createSession(goal);
      setSession(s);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to create session");
    } finally {
      setRunning(false);
    }
  }

  async function handleFollowUp(e: FormEvent) {
    e.preventDefault();
    if (!session || !goal.trim()) return;

    setRunning(true);
    setError(null);
    try {
      const s = await sendMessage(session.id, goal);
      setSession(s);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to send message");
    } finally {
      setRunning(false);
    }
  }

  return (
    <div className="panel console">
      <div className="sectionHeader">
        <p className="eyebrow">Agent Console</p>
        <h2>Describe the outcome. Inspect the trace.</h2>
      </div>

      {error && (
        <div className="errorBanner" role="alert">
          {error}
        </div>
      )}

      <form className="promptBox" onSubmit={session ? handleFollowUp : handleRun}>
        <label htmlFor="goal">Operations goal</label>
        <textarea
          id="goal"
          name="goal"
          value={goal}
          onChange={(e) => setGoal(e.target.value)}
          disabled={running}
        />
        <button type="submit" disabled={running}>
          {running
            ? "Running..."
            : session
              ? "Send message"
              : "Run agent"}
        </button>
      </form>

      {session && (
        <div className="sessionMessages" aria-live="polite">
          <p className="agentMeta">
            Session {session.id.slice(0, 8)}... &middot;{" "}
            {session.messages.length} message
            {session.messages.length !== 1 ? "s" : ""}
          </p>
          {session.messages.map((msg, i) => (
            <div key={i} className={`sessionMessage ${msg.role}`}>
              <span className="role">{msg.role}</span>
              <p>{msg.content}</p>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
