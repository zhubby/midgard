import type {
  ApprovalRecord,
  MiddlewareInstance,
  MiddlewareDashboardState,
  MiddlewareMetric,
  MiddlewareWorkload,
  PluginResponse,
  ToolDefinition,
  WorkspaceRuntimeConfigView,
} from "@/lib/types";
import { Settings2 } from "lucide-react";

interface MiddlewareDashboardProps {
  approvals: ApprovalRecord[];
  canManageWorkspace: boolean;
  instances: MiddlewareInstance[];
  middleware: MiddlewareDashboardState;
  plugins: PluginResponse[];
  runtimeConfig: WorkspaceRuntimeConfigView;
  settingsHref: string;
  tools: ToolDefinition[];
  workspaceId: string;
}

function metricTone(metric: MiddlewareMetric) {
  return metric.tone;
}

function workloadTone(workload: MiddlewareWorkload) {
  return workload.tone;
}

export function MiddlewareDashboard({
  approvals,
  canManageWorkspace,
  instances,
  middleware,
  plugins,
  runtimeConfig,
  settingsHref,
  tools,
  workspaceId,
}: MiddlewareDashboardProps) {
  const gatedTools = tools.filter((tool) => tool.requires_approval).length;
  const runtimeLabel = runtimeConfig.mode ?? "unconfigured";
  const runtimeDetail =
    runtimeConfig.mode === "docker"
      ? runtimeConfig.docker?.endpoint_host ?? "docker endpoint saved"
      : runtimeConfig.mode === "kubernetes"
        ? runtimeConfig.kubernetes?.context_name ??
          runtimeConfig.kubernetes?.cluster_server_host ??
          "kubeconfig saved"
        : "open workspace settings";

  return (
    <aside
      className="workspace-panel dashboard-panel"
      aria-labelledby="dashboard-title"
    >
      <div className="panel-header">
        <div>
          <p className="section-kicker">Middleware dashboard</p>
          <h2 id="dashboard-title">Health, risk, and tool readiness</h2>
        </div>
        <span className="badge badge-outline">
          {plugins.length} plugin{plugins.length === 1 ? "" : "s"}
        </span>
      </div>

      <section className="runtime-summary" aria-label="Workspace runtime">
        <div>
          <span>Runtime</span>
          <strong>{runtimeLabel}</strong>
          <p>{runtimeDetail}</p>
        </div>
        <div>
          <span>Status</span>
          <strong>{runtimeConfig.status}</strong>
          <p>{runtimeConfig.updated_at ?? "not configured"}</p>
        </div>
        <div className="workspace-uuid">
          <span>Workspace UUID</span>
          <strong title={workspaceId}>{workspaceId}</strong>
          <p>Use for operator startup</p>
        </div>
        {canManageWorkspace && (
          <a className="button button-outline" href={settingsHref}>
            <Settings2 aria-hidden="true" />
            Settings
          </a>
        )}
      </section>

      <section className="metric-grid" aria-label="Middleware metrics">
        {middleware.metrics.map((metric) => (
          <article className={`metric-tile ${metricTone(metric)}`} key={metric.id}>
            <span>{metric.label}</span>
            <strong>{metric.value}</strong>
            <p>{metric.detail}</p>
          </article>
        ))}
        {middleware.metrics.length === 0 && (
          <article className="metric-tile neutral">
            <span>Workspace</span>
            <strong>--</strong>
            <p>Waiting for middleware snapshot</p>
          </article>
        )}
      </section>

      <section className="dashboard-section" aria-labelledby="workloads-title">
        <div className="section-row">
          <h3 id="workloads-title">Middleware fleet</h3>
          <span className="subtle-count">{middleware.workloads.length}</span>
        </div>
        <div className="workload-list">
          {middleware.workloads.map((workload) => (
            <article className="workload-row" key={workload.id}>
              <div className="workload-main">
                <span
                  className={`state-dot ${workloadTone(workload)}`}
                  aria-hidden="true"
                />
                <div>
                  <strong>{workload.name}</strong>
                  <p>
                    {workload.kind} / {workload.namespace}
                  </p>
                </div>
              </div>
              <div className="workload-health">
                <span className={`badge badge-${workloadTone(workload)}`}>
                  {workload.health}
                </span>
                <div
                  className="load-bar"
                  aria-label={`${workload.name} saturation ${workload.saturation}%`}
                >
                  <span style={{ width: `${workload.saturation}%` }} />
                </div>
              </div>
              <span className="risk-label">{workload.risk}</span>
            </article>
          ))}
          {middleware.workloads.length === 0 && (
            <article className="empty-row">No workloads observed.</article>
          )}
        </div>
      </section>

      <section className="dashboard-section" aria-labelledby="instances-title">
        <div className="section-row">
          <h3 id="instances-title">Middleware instances</h3>
          <span className="subtle-count">{instances.length}</span>
        </div>
        <div className="instance-list">
          {instances.map((instance) => (
            <article className="instance-row" key={instance.id}>
              <div>
                <strong>{instance.name}</strong>
                <p>
                  {instance.kind} / {instance.namespace}
                </p>
              </div>
              <div>
                <span className="badge badge-outline">
                  {instance.desired_state}
                </span>
                <span className={`badge badge-${instance.status === "degraded" ? "warn" : instance.status === "running" ? "ready" : "neutral"}`}>
                  {instance.status}
                </span>
              </div>
            </article>
          ))}
          {instances.length === 0 && (
            <article className="empty-row">No middleware instances registered.</article>
          )}
        </div>
      </section>

      <div className="dashboard-columns">
        <section className="dashboard-section" aria-labelledby="tools-title">
          <div className="section-row">
            <h3 id="tools-title">Tool catalog</h3>
            <span className="subtle-count">
              {tools.length} / {gatedTools} gated
            </span>
          </div>
          <div className="tool-stack">
            {tools.map((tool) => (
              <article className="tool-row" key={tool.name}>
                <strong>{tool.name}</strong>
                <div>
                  <span>{tool.risk_level}</span>
                  <span>{tool.requires_approval ? "Required" : "No approval"}</span>
                </div>
              </article>
            ))}
            {tools.length === 0 && (
              <article className="empty-row">Waiting for tool catalog.</article>
            )}
          </div>
        </section>

        <section className="dashboard-section" aria-labelledby="approvals-title">
          <div className="section-row">
            <h3 id="approvals-title">Approvals</h3>
            <span className="subtle-count">{approvals.length}</span>
          </div>
          <div className="approval-stack">
            {approvals.map((approval) => (
              <article className="approval-row" key={approval.id}>
                <div>
                  <strong>{approval.tool_call.name}</strong>
                  <p>
                    {approval.risk_level} / {approval.status}
                  </p>
                </div>
                <span>{approval.requested_at}</span>
              </article>
            ))}
            {approvals.length === 0 && (
              <article className="empty-row">No approval records.</article>
            )}
          </div>
        </section>
      </div>
    </aside>
  );
}
