type Tone = "ready" | "warn" | "danger" | "neutral";

interface Metric {
  label: string;
  value: string;
  detail: string;
  tone: Tone;
}

interface Workload {
  name: string;
  kind: string;
  namespace: string;
  health: string;
  saturation: number;
  risk: string;
  tone: Tone;
}

interface ToolCapability {
  name: string;
  risk: string;
  approval: string;
}

const metrics: Metric[] = [
  {
    label: "Healthy workloads",
    value: "12/14",
    detail: "2 need attention",
    tone: "ready",
  },
  {
    label: "Approval queue",
    value: "2",
    detail: "high-risk actions",
    tone: "warn",
  },
  {
    label: "Registered tools",
    value: "18",
    detail: "7 gated",
    tone: "neutral",
  },
  {
    label: "Controller latency",
    value: "118ms",
    detail: "p95 mock sample",
    tone: "ready",
  },
];

const workloads: Workload[] = [
  {
    name: "redis-cache",
    kind: "Redis",
    namespace: "default",
    health: "Healthy",
    saturation: 41,
    risk: "Low",
    tone: "ready",
  },
  {
    name: "kafka-brokers",
    kind: "Kafka",
    namespace: "streaming",
    health: "Degraded",
    saturation: 73,
    risk: "High",
    tone: "warn",
  },
  {
    name: "postgres-primary",
    kind: "PostgreSQL",
    namespace: "data",
    health: "Watch",
    saturation: 62,
    risk: "Medium",
    tone: "neutral",
  },
];

const tools: ToolCapability[] = [
  {
    name: "inspect_workload",
    risk: "Low",
    approval: "No",
  },
  {
    name: "restart_middleware",
    risk: "High",
    approval: "Required",
  },
  {
    name: "scale_replicas",
    risk: "Medium",
    approval: "Required",
  },
  {
    name: "read_events",
    risk: "Low",
    approval: "No",
  },
];

const approvals = [
  {
    action: "Restart kafka-brokers",
    target: "streaming/kafka-brokers",
    age: "12m",
  },
  {
    action: "Fail over redis-cache",
    target: "default/redis-cache",
    age: "31m",
  },
];

export function MiddlewareDashboard() {
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
        <span className="badge badge-outline">staging</span>
      </div>

      <section className="metric-grid" aria-label="Middleware metrics">
        {metrics.map((metric) => (
          <article className={`metric-tile ${metric.tone}`} key={metric.label}>
            <span>{metric.label}</span>
            <strong>{metric.value}</strong>
            <p>{metric.detail}</p>
          </article>
        ))}
      </section>

      <section className="dashboard-section" aria-labelledby="workloads-title">
        <div className="section-row">
          <h3 id="workloads-title">Middleware fleet</h3>
          <button className="button button-ghost" type="button">
            Filter
          </button>
        </div>
        <div className="workload-list">
          {workloads.map((workload) => (
            <article className="workload-row" key={workload.name}>
              <div className="workload-main">
                <span className={`state-dot ${workload.tone}`} aria-hidden="true" />
                <div>
                  <strong>{workload.name}</strong>
                  <p>
                    {workload.kind} / {workload.namespace}
                  </p>
                </div>
              </div>
              <div className="workload-health">
                <span className={`badge badge-${workload.tone}`}>
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
        </div>
      </section>

      <div className="dashboard-columns">
        <section className="dashboard-section" aria-labelledby="tools-title">
          <div className="section-row">
            <h3 id="tools-title">Tool catalog</h3>
            <span className="subtle-count">{tools.length}</span>
          </div>
          <div className="tool-stack">
            {tools.map((tool) => (
              <article className="tool-row" key={tool.name}>
                <strong>{tool.name}</strong>
                <div>
                  <span>{tool.risk}</span>
                  <span>{tool.approval}</span>
                </div>
              </article>
            ))}
          </div>
        </section>

        <section className="dashboard-section" aria-labelledby="approvals-title">
          <div className="section-row">
            <h3 id="approvals-title">Approvals</h3>
            <span className="subtle-count">{approvals.length}</span>
          </div>
          <div className="approval-stack">
            {approvals.map((approval) => (
              <article className="approval-row" key={approval.action}>
                <div>
                  <strong>{approval.action}</strong>
                  <p>{approval.target}</p>
                </div>
                <span>{approval.age}</span>
              </article>
            ))}
          </div>
        </section>
      </div>
    </aside>
  );
}
