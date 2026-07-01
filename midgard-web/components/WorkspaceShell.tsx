"use client";

import {
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent,
  useEffect,
  useReducer,
  useRef,
  useState,
} from "react";
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

function userInitials(value: string) {
  const parts = value
    .replace(/@.*/, "")
    .trim()
    .split(/\s+/)
    .filter(Boolean);
  const initials = parts
    .slice(0, 2)
    .map((part) => part[0]?.toUpperCase())
    .join("");

  return initials || "U";
}

function sessionTabText(session: AgentSessionSummary, index: number) {
  if (session.has_pending_approval) return "!";
  const title = session.title.trim();
  if (!title) return String(index + 1);

  return String(index + 1);
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
  const [userMenuOpen, setUserMenuOpen] = useState(false);
  const [agentPanelWidth, setAgentPanelWidth] = useState(48);
  const mainGridRef = useRef<HTMLElement | null>(null);
  const orgSlug = context.organization.slug;
  const workspaceSlug = workspace.slug;
  const displayUser = user.display_name || user.email;
  const canOperate = state.permissions.includes("workspace.operate");
  const canManageOrgRoles = context.permissions.includes("org.roles.read");
  const canManageMembers = context.permissions.includes("org.members.read");
  const canManageWorkspace = context.permissions.includes("workspaces.manage");
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

  function clampAgentPanelWidth(width: number, containerWidth?: number) {
    const minAgentPercent = containerWidth
      ? Math.min(42, (320 / containerWidth) * 100)
      : 30;
    const maxAgentPercent = containerWidth
      ? Math.max(minAgentPercent, 100 - (360 / containerWidth) * 100)
      : 70;

    return Math.min(maxAgentPercent, Math.max(minAgentPercent, width));
  }

  function handlePanelResizePointerDown(
    event: PointerEvent<HTMLButtonElement>,
  ) {
    const grid = mainGridRef.current;
    if (!grid) return;

    event.preventDefault();
    const rect = grid.getBoundingClientRect();

    function updateWidth(clientX: number) {
      const nextWidth = ((clientX - rect.left) / rect.width) * 100;
      setAgentPanelWidth(clampAgentPanelWidth(nextWidth, rect.width));
    }

    function handlePointerMove(pointerEvent: globalThis.PointerEvent) {
      updateWidth(pointerEvent.clientX);
    }

    function handlePointerUp() {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("pointerup", handlePointerUp);
    }

    updateWidth(event.clientX);
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("pointerup", handlePointerUp);
  }

  function handlePanelResizeKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) {
      return;
    }

    event.preventDefault();
    const step = event.shiftKey ? 8 : 4;
    setAgentPanelWidth((width) => {
      if (event.key === "Home") return 32;
      if (event.key === "End") return 68;
      return clampAgentPanelWidth(
        width + (event.key === "ArrowLeft" ? -step : step),
      );
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
    <main className="workspace-shell">
      <aside className="workspace-sidebar" aria-label="Workspace navigation">
        <div className="workspace-sidebar-brand" title="Midgard">
          <div className="brand-mark" aria-hidden="true">
            M
          </div>
        </div>

        <div
          className="workspace-sidebar-workspace"
          title={`${context.organization.name} / ${workspace.slug}`}
        >
          <strong>{workspace.name.slice(0, 3)}</strong>
          <span>{workspace.slug.slice(0, 3)}</span>
        </div>

        <div className="workspace-sidebar-status" aria-label="Workspace state">
          <span
            aria-label={state.connectionStatus}
            className={`workspace-status-dot ${state.connectionStatus}`}
            title={state.connectionStatus}
          >
            <span aria-hidden="true" />
          </span>
        </div>

        <nav
          aria-label="Agent sessions"
          aria-orientation="vertical"
          className="workspace-session-rail"
          role="tablist"
        >
          <button
            aria-label="New session"
            className="session-rail-action"
            disabled={state.busy || !canOperate}
            title="New session"
            type="button"
            onClick={handleNewSession}
          >
            +
          </button>
          <button
            aria-label="Live state"
            aria-selected={!state.activeSessionId}
            className={`session-rail-tab ${
              !state.activeSessionId ? "active" : ""
            }`}
            disabled={state.busy}
            role="tab"
            title={`Live state / ${state.connectionStatus}`}
            type="button"
            onClick={() => handleSessionSelect("")}
          >
            L
          </button>
          {state.sessions.map((session, index) => (
            <button
              aria-label={session.title || `Session ${index + 1}`}
              aria-selected={state.activeSessionId === session.id}
              className={`session-rail-tab ${
                state.activeSessionId === session.id ? "active" : ""
              } ${session.has_pending_approval ? "warn" : ""}`}
              disabled={state.busy}
              key={session.id}
              role="tab"
              title={`${session.title || `Session ${index + 1}`} / ${
                session.status
              }`}
              type="button"
              onClick={() => handleSessionSelect(session.id)}
            >
              {sessionTabText(session, index)}
            </button>
          ))}
        </nav>

        <div className="workspace-user-menu">
          <button
            aria-expanded={userMenuOpen}
            aria-haspopup="menu"
            className={`user-menu-trigger ${userMenuOpen ? "active" : ""}`}
            title={displayUser}
            type="button"
            onClick={() => setUserMenuOpen((open) => !open)}
          >
            <span>{userInitials(displayUser)}</span>
          </button>

          {userMenuOpen && (
            <div className="user-menu-popover" role="menu">
              <div className="user-menu-header">
                <strong>{displayUser}</strong>
                <span>
                  {user.role} / {context.membership.role}
                </span>
              </div>
              <a className="user-menu-item" href="/organizations" role="menuitem">
                Organizations
              </a>
              {canManageMembers && (
                <a
                  className="user-menu-item"
                  href={`/orgs/${orgSlug}/settings/members`}
                  role="menuitem"
                >
                  Members
                </a>
              )}
              {canManageOrgRoles && (
                <a
                  className="user-menu-item"
                  href={`/orgs/${orgSlug}/settings/roles`}
                  role="menuitem"
                >
                  Roles
                </a>
              )}
              {canReadSystemAdmin && (
                <a className="user-menu-item" href="/admin/users" role="menuitem">
                  Admin
                </a>
              )}
              {canManageWorkspace && (
                <a
                  className="user-menu-item"
                  href={`/orgs/${orgSlug}/workspaces/${workspaceSlug}/settings`}
                  role="menuitem"
                >
                  Workspace
                </a>
              )}
              <button
                className="user-menu-item danger"
                disabled={busyAuth}
                role="menuitem"
                type="button"
                onClick={() => {
                  setUserMenuOpen(false);
                  onLogout();
                }}
              >
                Logout
              </button>
            </div>
          )}
        </div>
      </aside>

      <section
        className="workspace-main-grid"
        ref={mainGridRef}
        aria-label="Midgard operations workspace"
        style={
          {
            "--workspace-agent-width": `${agentPanelWidth}%`,
          } as CSSProperties
        }
      >
        <AgentConsole
          busy={state.busy}
          canOperate={canOperate}
          connectionStatus={state.connectionStatus}
          draft={draft}
          error={state.error}
          messages={state.messages}
          onApproval={handleApproval}
          onDraftChange={setDraft}
          onSend={handleSend}
          pendingApproval={state.pendingApproval}
          runStatus={state.runStatus}
          streamingAssistant={state.streamingAssistant}
          trace={state.trace}
        />
        <button
          aria-label="Resize agent and dashboard panels"
          aria-orientation="vertical"
          aria-valuemax={70}
          aria-valuemin={30}
          aria-valuenow={Math.round(agentPanelWidth)}
          className="workspace-resize-handle"
          role="separator"
          type="button"
          onKeyDown={handlePanelResizeKeyDown}
          onPointerDown={handlePanelResizePointerDown}
        >
          <span aria-hidden="true" />
        </button>
        <MiddlewareDashboard
          approvals={state.approvals}
          canManageWorkspace={canManageWorkspace}
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
