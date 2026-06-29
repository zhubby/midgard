import type { WorkspaceEvent, WorkspaceEventType } from "./types";

const API_BASE = process.env.NEXT_PUBLIC_API_URL ?? "http://localhost:8080";

const workspaceEventTypes: WorkspaceEventType[] = [
  "connected",
  "heartbeat",
  "error",
  "agent_session_updated",
  "agent_run_started",
  "agent_message_delta",
  "agent_message_committed",
  "tool_call_requested",
  "tool_result_received",
  "agent_run_completed",
  "agent_run_failed",
  "approval_required",
  "approval_decided",
  "middleware_snapshot",
  "middleware_workload_upserted",
  "middleware_workload_removed",
  "middleware_metric_changed",
  "middleware_event_observed",
  "tool_catalog_updated",
  "plugin_catalog_updated",
];

export type WorkspaceConnectionStatus =
  | "connecting"
  | "connected"
  | "disconnected";

export interface WorkspaceEventConnection {
  close: () => void;
}

export function connectWorkspaceEvents({
  sessionId,
  onEvent,
  onStatus,
  onError,
}: {
  sessionId?: string | null;
  onEvent: (event: WorkspaceEvent) => void;
  onStatus?: (status: WorkspaceConnectionStatus) => void;
  onError?: (message: string) => void;
}): WorkspaceEventConnection {
  const url = new URL("/api/workspace/events", API_BASE);
  if (sessionId) {
    url.searchParams.set("session_id", sessionId);
  }

  onStatus?.("connecting");
  const source = new EventSource(url, { withCredentials: true });

  source.onopen = () => onStatus?.("connected");
  source.onerror = () => {
    onStatus?.("disconnected");
  };

  for (const type of workspaceEventTypes) {
    source.addEventListener(type, (message) => {
      if (!("data" in message) || typeof message.data !== "string") {
        return;
      }

      try {
        onEvent(JSON.parse(message.data) as WorkspaceEvent);
      } catch (error) {
        onError?.(
          error instanceof Error
            ? `Invalid workspace event: ${error.message}`
            : "Invalid workspace event.",
        );
      }
    });
  }

  return {
    close: () => source.close(),
  };
}
