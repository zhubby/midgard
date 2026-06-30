"use client";

import { useEffect, useReducer, useState } from "react";
import { AgentConsole, type AgentTraceItem } from "@/components/AgentConsole";
import { MiddlewareDashboard } from "@/components/MiddlewareDashboard";
import { createSession, decideApproval, runAgent, sendMessage } from "@/lib/api";
import {
  connectWorkspaceEvents,
  type WorkspaceConnectionStatus,
} from "@/lib/events";
import type {
  AgentMessage,
  AgentRunStatus,
  AgentSession,
  AgentSessionSummary,
  ApprovalRecord,
  AuthUser,
  MiddlewareDashboardState,
  MiddlewareInstance,
  OrganizationContext,
  PendingApproval,
  PermissionKey,
  PluginResponse,
  ToolDefinition,
  Workspace,
  WorkspaceEvent,
  WorkspaceRuntimeConfigView,
} from "@/lib/types";

interface WorkspaceState {
  connectionStatus: WorkspaceConnectionStatus;
  session: AgentSession | null;
  activeSessionId: string | null;
  sessions: AgentSessionSummary[];
  messages: AgentMessage[];
  streamingAssistant: string;
  trace: AgentTraceItem[];
  tools: ToolDefinition[];
  plugins: PluginResponse[];
  runtimeConfig: WorkspaceRuntimeConfigView;
  middlewareInstances: MiddlewareInstance[];
  middleware: MiddlewareDashboardState;
  approvals: ApprovalRecord[];
  permissions: PermissionKey[];
  pendingApproval: PendingApproval | null;
  runStatus: AgentRunStatus | "idle";
  busy: boolean;
  error: string | null;
}

type WorkspaceAction =
  | { type: "connection"; status: WorkspaceConnectionStatus }
  | { type: "error"; message: string | null }
  | { type: "session_loaded"; session: AgentSession }
  | { type: "select_session"; sessionId: string | null }
  | { type: "new_session" }
  | { type: "run_busy"; busy: boolean }
  | { type: "event"; event: WorkspaceEvent };

const emptyMiddleware: MiddlewareDashboardState = {
  metrics: [],
  workloads: [],
  events: [],
};

const emptyRuntimeConfig: WorkspaceRuntimeConfigView = {
  status: "unconfigured",
};

const initialState: WorkspaceState = {
  connectionStatus: "connecting",
  session: null,
  activeSessionId: null,
  sessions: [],
  messages: [],
  streamingAssistant: "",
  trace: [],
  tools: [],
  plugins: [],
  runtimeConfig: emptyRuntimeConfig,
  middlewareInstances: [],
  middleware: emptyMiddleware,
  approvals: [],
  permissions: [],
  pendingApproval: null,
  runStatus: "idle",
  busy: false,
  error: null,
};

function messageKey(message: AgentMessage) {
  const toolCallIds = message.tool_calls?.map((call) => call.id).join(",") ?? "";
  return `${message.role}:${message.tool_call_id ?? ""}:${toolCallIds}:${message.content}`;
}

function appendMessage(messages: AgentMessage[], message: AgentMessage) {
  const key = messageKey(message);
  if (messages.some((current) => messageKey(current) === key)) {
    return messages;
  }

  return [...messages, message];
}

function upsertTrace(trace: AgentTraceItem[], item: AgentTraceItem) {
  const next = trace.filter((current) => current.id !== item.id);
  return [...next, item];
}

function upsertApproval(records: ApprovalRecord[], record: ApprovalRecord) {
  return [record, ...records.filter((current) => current.id !== record.id)];
}

function summarizeSession(session: AgentSession): AgentSessionSummary {
  const title =
    session.messages.find((message) => message.role === "user")?.content.trim() ||
    "Untitled session";
  return {
    id: session.id,
    title,
    status: session.status,
    message_count: session.messages.length,
    has_pending_approval: Boolean(session.pending_approval),
    last_error: session.last_error ?? null,
  };
}

function upsertSessionSummary(
  sessions: AgentSessionSummary[],
  summary: AgentSessionSummary,
) {
  return [
    summary,
    ...sessions.filter((session) => session.id !== summary.id),
  ];
}

function reduceWorkspace(
  state: WorkspaceState,
  action: WorkspaceAction,
): WorkspaceState {
  switch (action.type) {
    case "connection":
      return { ...state, connectionStatus: action.status };
    case "error":
      return { ...state, error: action.message };
    case "session_loaded":
      return {
        ...state,
        session: action.session,
        activeSessionId: action.session.id,
        sessions: upsertSessionSummary(state.sessions, summarizeSession(action.session)),
        messages: action.session.messages,
        pendingApproval: action.session.pending_approval ?? null,
        runStatus: action.session.status,
      };
    case "select_session":
      return {
        ...state,
        activeSessionId: action.sessionId,
        session: null,
        messages: [],
        streamingAssistant: "",
        trace: [],
        pendingApproval: null,
        runStatus: "idle",
      };
    case "new_session":
      return {
        ...state,
        activeSessionId: null,
        session: null,
        messages: [],
        streamingAssistant: "",
        trace: [],
        pendingApproval: null,
        runStatus: "idle",
        error: null,
      };
    case "run_busy":
      return { ...state, busy: action.busy };
    case "event":
      return reduceWorkspaceEvent(state, action.event);
  }
}

function reduceWorkspaceEvent(
  state: WorkspaceState,
  event: WorkspaceEvent,
): WorkspaceState {
  const payload = event.payload;

  switch (payload.kind) {
    case "connected":
      return {
        ...state,
        connectionStatus: "connected",
        session: payload.snapshot.session ?? null,
        activeSessionId:
          payload.snapshot.active_session_id ??
          payload.snapshot.session?.id ??
          state.activeSessionId,
        sessions: payload.snapshot.sessions,
        messages: payload.snapshot.session?.messages ?? [],
        tools: payload.snapshot.tools,
        plugins: payload.snapshot.plugins,
        runtimeConfig: payload.snapshot.runtime_config,
        middlewareInstances: payload.snapshot.middleware_instances,
        middleware: payload.snapshot.middleware,
        approvals: payload.snapshot.approvals,
        permissions: payload.snapshot.current_permissions,
        pendingApproval: payload.snapshot.session?.pending_approval ?? null,
        runStatus: payload.snapshot.session?.status ?? "idle",
        error: null,
      };
    case "heartbeat":
      return state;
    case "error":
      return { ...state, error: payload.message };
    case "agent_session_updated":
      if (
        state.activeSessionId &&
        payload.session.id !== state.activeSessionId
      ) {
        return {
          ...state,
          sessions: upsertSessionSummary(
            state.sessions,
            summarizeSession(payload.session),
          ),
        };
      }
      return {
        ...state,
        session: payload.session,
        activeSessionId: payload.session.id,
        sessions: upsertSessionSummary(
          state.sessions,
          summarizeSession(payload.session),
        ),
        messages: payload.session.messages,
        pendingApproval: payload.session.pending_approval ?? state.pendingApproval,
        runStatus: payload.session.status,
        busy: payload.session.status === "running",
      };
    case "agent_run_started":
      if (state.activeSessionId && payload.session_id !== state.activeSessionId) {
        return state;
      }
      return {
        ...state,
        runStatus: "running",
        busy: true,
        error: null,
        streamingAssistant: "",
        trace: upsertTrace(state.trace, {
          id: payload.run_id,
          label: "Run started",
          detail: `Session ${payload.session_id.slice(0, 8)}`,
          tone: "pending",
        }),
      };
    case "agent_message_delta":
      if (state.activeSessionId && payload.session_id !== state.activeSessionId) {
        return state;
      }
      return {
        ...state,
        streamingAssistant: state.streamingAssistant + payload.content,
      };
    case "agent_message_committed":
      if (state.activeSessionId && payload.session_id !== state.activeSessionId) {
        return state;
      }
      return {
        ...state,
        messages: appendMessage(state.messages, payload.message),
        streamingAssistant:
          payload.message.role === "assistant" ? "" : state.streamingAssistant,
      };
    case "tool_call_requested":
      if (state.activeSessionId && payload.session_id !== state.activeSessionId) {
        return state;
      }
      return {
        ...state,
        trace: upsertTrace(state.trace, {
          id: payload.tool_call.id,
          label: payload.tool_call.name,
          detail: "Tool call requested",
          tone: "pending",
        }),
      };
    case "tool_result_received":
      if (state.activeSessionId && payload.session_id !== state.activeSessionId) {
        return state;
      }
      return {
        ...state,
        trace: upsertTrace(state.trace, {
          id: payload.tool_call_id,
          label: payload.name,
          detail: payload.result.output,
          tone: payload.result.is_error ? "warn" : "ready",
        }),
      };
    case "approval_required":
      if (state.activeSessionId && payload.session_id !== state.activeSessionId) {
        return state;
      }
      return {
        ...state,
        pendingApproval: payload.approval,
        runStatus: "awaiting_approval",
        busy: false,
        trace: upsertTrace(state.trace, {
          id: payload.approval.id,
          label: "Approval required",
          detail: `${payload.approval.tool_call.name} is ${payload.approval.risk_level} risk`,
          tone: "warn",
        }),
      };
    case "approval_decided":
      return {
        ...state,
        session: payload.session,
        approvals: upsertApproval(state.approvals, payload.approval_record),
        pendingApproval:
          payload.approval_record.status === "pending"
            ? state.pendingApproval
            : null,
      };
    case "agent_run_completed":
      if (state.activeSessionId && payload.session_id !== state.activeSessionId) {
        return state;
      }
      return {
        ...state,
        runStatus: payload.status,
        busy: false,
        streamingAssistant: "",
        trace: upsertTrace(state.trace, {
          id: `complete-${event.event_id}`,
          label: "Run completed",
          detail: payload.output,
          tone: "ready",
        }),
      };
    case "agent_run_failed":
      if (state.activeSessionId && payload.session_id !== state.activeSessionId) {
        return state;
      }
      return {
        ...state,
        runStatus: "failed",
        busy: false,
        error: payload.error,
        trace: upsertTrace(state.trace, {
          id: `failed-${event.event_id}`,
          label: "Run failed",
          detail: payload.error,
          tone: "warn",
        }),
      };
    case "middleware_snapshot":
      return { ...state, middleware: payload.state };
    case "middleware_instance_upserted":
      return {
        ...state,
        middlewareInstances: [
          payload.instance,
          ...state.middlewareInstances.filter(
            (instance) => instance.id !== payload.instance.id,
          ),
        ],
      };
    case "middleware_instance_removed":
      return {
        ...state,
        middlewareInstances: state.middlewareInstances.filter(
          (instance) => instance.id !== payload.id,
        ),
      };
    case "middleware_workload_upserted":
      return {
        ...state,
        middleware: {
          ...state.middleware,
          workloads: [
            payload.workload,
            ...state.middleware.workloads.filter(
              (workload) => workload.id !== payload.workload.id,
            ),
          ],
        },
      };
    case "middleware_workload_removed":
      return {
        ...state,
        middleware: {
          ...state.middleware,
          workloads: state.middleware.workloads.filter(
            (workload) =>
              workload.namespace !== payload.namespace ||
              workload.name !== payload.name,
          ),
        },
      };
    case "middleware_metric_changed":
      return {
        ...state,
        middleware: {
          ...state.middleware,
          metrics: [
            payload.metric,
            ...state.middleware.metrics.filter(
              (metric) => metric.id !== payload.metric.id,
            ),
          ],
        },
      };
    case "middleware_event_observed":
      return {
        ...state,
        middleware: {
          ...state.middleware,
          events: [
            payload.event,
            ...state.middleware.events.filter(
              (timelineEvent) => timelineEvent.id !== payload.event.id,
            ),
          ].slice(0, 8),
        },
      };
    case "tool_catalog_updated":
      return { ...state, tools: payload.tools };
    case "plugin_catalog_updated":
      return { ...state, plugins: payload.plugins };
  }
}

interface WorkspaceShellProps {
  busyAuth: boolean;
  context: OrganizationContext;
  systemPermissions: PermissionKey[];
  workspace: Workspace;
  user: AuthUser;
  onLogout: () => void;
}

export function WorkspaceShell({
  busyAuth,
  context,
  systemPermissions,
  workspace,
  user,
  onLogout,
}: WorkspaceShellProps) {
  const [state, dispatch] = useReducer(reduceWorkspace, initialState);
  const [draft, setDraft] = useState(
    "Inspect Redis in the default namespace and report whether it is healthy.",
  );
  const orgSlug = context.organization.slug;
  const workspaceSlug = workspace.slug;
  const canOperate = state.permissions.includes("workspace.operate");
  const canManageOrgRoles = context.permissions.includes("org.roles.read");
  const canManageMembers = context.permissions.includes("org.members.read");
  const canReadSystemAdmin =
    systemPermissions.includes("system.users.read") ||
    systemPermissions.includes("system.roles.read");

  useEffect(() => {
    const connection = connectWorkspaceEvents({
      orgSlug,
      workspaceSlug,
      sessionId: state.activeSessionId,
      onEvent: (event) => dispatch({ type: "event", event }),
      onStatus: (status) => dispatch({ type: "connection", status }),
      onError: (message) => dispatch({ type: "error", message }),
    });

    return () => connection.close();
  }, [orgSlug, workspaceSlug, state.activeSessionId]);

  function handleNewSession() {
    dispatch({ type: "new_session" });
    setDraft("");
  }

  function handleSessionSelect(sessionId: string) {
    dispatch({
      type: "select_session",
      sessionId: sessionId || null,
    });
  }

  async function handleSend(prompt: string) {
    const message = prompt.trim();
    if (!message || state.busy || !canOperate) return;

    dispatch({ type: "run_busy", busy: true });
    dispatch({ type: "error", message: null });

    try {
      const session = state.session
        ? await sendMessage(orgSlug, workspaceSlug, state.session.id, message)
        : await createSession(orgSlug, workspaceSlug, message);
      dispatch({ type: "session_loaded", session });
      await runAgent(orgSlug, workspaceSlug, session.id);
      setDraft("");
    } catch (error) {
      dispatch({
        type: "error",
        message:
          error instanceof Error ? error.message : "Failed to start agent run.",
      });
      dispatch({ type: "run_busy", busy: false });
    }
  }

  async function handleApproval(decision: "approve" | "reject") {
    if (!state.session || !state.pendingApproval || !canOperate) return;

    dispatch({ type: "run_busy", busy: true });
    dispatch({ type: "error", message: null });

    try {
      const response = await decideApproval(
        orgSlug,
        workspaceSlug,
        state.session.id,
        decision,
        decision === "approve" ? "Approved from Midgard console" : undefined,
        true,
      );
      dispatch({ type: "session_loaded", session: response.session });
    } catch (error) {
      dispatch({
        type: "error",
        message:
          error instanceof Error ? error.message : "Failed to decide approval.",
      });
      dispatch({ type: "run_busy", busy: false });
    }
  }

  return (
    <main className="app-shell">
      <header className="app-header">
        <div className="brand-lockup">
          <div className="brand-mark" aria-hidden="true">
            M
          </div>
          <div>
            <p className="section-kicker">Midgard</p>
            <h1>{workspace.name}</h1>
            <p className="workspace-breadcrumb">
              {context.organization.name} / {workspace.slug}
            </p>
          </div>
        </div>

        <div className="header-actions" aria-label="Workspace state">
          <span className={`status-pill ${state.connectionStatus}`}>
            <span aria-hidden="true" />
            {state.connectionStatus}
          </span>
          <div className="user-chip" aria-label="Signed in user">
            <strong>{user.display_name || user.email}</strong>
            <span>{user.role}</span>
            <span>{context.membership.role}</span>
          </div>
          {canManageMembers && (
            <a className="button button-outline" href={`/orgs/${orgSlug}/settings/members`}>
              Members
            </a>
          )}
          {canManageOrgRoles && (
            <a className="button button-outline" href={`/orgs/${orgSlug}/settings/roles`}>
              Roles
            </a>
          )}
          {canReadSystemAdmin && (
            <a className="button button-outline" href="/admin/users">
              Admin
            </a>
          )}
          {context.permissions.includes("workspaces.manage") && (
            <a
              className="button button-outline"
              href={`/orgs/${orgSlug}/workspaces/${workspaceSlug}/settings`}
            >
              Workspace
            </a>
          )}
          <button
            className="button button-outline logout-button"
            disabled={busyAuth}
            type="button"
            onClick={onLogout}
          >
            Logout
          </button>
        </div>
      </header>

      <section className="workspace-grid" aria-label="Midgard operations workspace">
        <AgentConsole
          activeSessionId={state.activeSessionId}
          busy={state.busy}
          canOperate={canOperate}
          connectionStatus={state.connectionStatus}
          draft={draft}
          error={state.error}
          messages={state.messages}
          onApproval={handleApproval}
          onDraftChange={setDraft}
          onNewSession={handleNewSession}
          onSend={handleSend}
          onSessionSelect={handleSessionSelect}
          pendingApproval={state.pendingApproval}
          runStatus={state.runStatus}
          sessions={state.sessions}
          streamingAssistant={state.streamingAssistant}
          trace={state.trace}
        />
        <MiddlewareDashboard
          approvals={state.approvals}
          canManageWorkspace={context.permissions.includes("workspaces.manage")}
          instances={state.middlewareInstances}
          middleware={state.middleware}
          plugins={state.plugins}
          runtimeConfig={state.runtimeConfig}
          settingsHref={`/orgs/${orgSlug}/workspaces/${workspaceSlug}/settings`}
          tools={state.tools}
        />
      </section>
    </main>
  );
}
