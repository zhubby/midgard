const plugins = [
  {
    name: "Example Redis",
    kind: "redis",
    tools: ["redis_describe", "redis_restart"],
    risk: "High-risk restart requires approval",
  },
];

const toolTrace = [
  "list_namespaces -> default, midgard-system",
  "redis_describe -> Redis default/cache is ready",
  "complete_task -> success",
];

export default function Home() {
  return (
    <main className="shell">
      <nav className="topbar" aria-label="Primary navigation">
        <div>
          <p className="eyebrow">Midgard</p>
          <h1>Agent operations for Kubernetes middleware</h1>
        </div>
        <a className="primaryAction" href="#agent-console">
          Open console
        </a>
      </nav>

      <section className="hero" aria-labelledby="platform-status">
        <div>
          <p className="eyebrow">Platform status</p>
          <h2 id="platform-status">A ReAct agent with controller-grade tools.</h2>
          <p className="heroCopy">
            Midgard turns operational goals into auditable tool calls across Kubernetes
            middleware controllers. Every plugin declares its capabilities, risk, and
            approval boundary before the agent acts.
          </p>
        </div>
        <div className="statusGrid" aria-label="Current platform metrics">
          <Metric label="K8s context" value="mock" tone="ready" />
          <Metric label="Plugins" value="1" tone="ready" />
          <Metric label="Tools" value="3" tone="ready" />
          <Metric label="Approvals" value="armed" tone="warn" />
        </div>
      </section>

      <section className="workspace" id="agent-console" aria-label="Agent console">
        <div className="panel console">
          <div className="sectionHeader">
            <p className="eyebrow">Agent Console</p>
            <h2>Describe the outcome. Inspect the trace.</h2>
          </div>
          <form className="promptBox">
            <label htmlFor="goal">Operations goal</label>
            <textarea
              id="goal"
              name="goal"
              defaultValue="Inspect Redis in the default namespace and report whether it is healthy."
            />
            <button type="button">Run agent</button>
          </form>
        </div>

        <aside className="panel trace" aria-label="Tool trace">
          <p className="eyebrow">Tool trace</p>
          <ol>
            {toolTrace.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ol>
        </aside>
      </section>

      <section className="twoColumn" aria-label="Plugins and cluster">
        <div className="panel">
          <div className="sectionHeader">
            <p className="eyebrow">Plugin Catalog</p>
            <h2>Registered middleware capabilities</h2>
          </div>
          {plugins.map((plugin) => (
            <article className="plugin" key={plugin.name}>
              <div>
                <h3>{plugin.name}</h3>
                <p>{plugin.kind}</p>
              </div>
              <ul>
                {plugin.tools.map((tool) => (
                  <li key={tool}>{tool}</li>
                ))}
              </ul>
              <span>{plugin.risk}</span>
            </article>
          ))}
        </div>

        <div className="panel">
          <div className="sectionHeader">
            <p className="eyebrow">Cluster Overview</p>
            <h2>Namespaces and workloads</h2>
          </div>
          <div className="clusterRows">
            <ClusterRow namespace="default" workload="redis" state="Running" />
            <ClusterRow namespace="midgard-system" workload="midgard-server" state="Pending" />
          </div>
        </div>
      </section>
    </main>
  );
}

function Metric({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone: "ready" | "warn";
}) {
  return (
    <div className={`metric ${tone}`}>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function ClusterRow({
  namespace,
  workload,
  state,
}: {
  namespace: string;
  workload: string;
  state: string;
}) {
  return (
    <article className="clusterRow">
      <div>
        <strong>{workload}</strong>
        <span>{namespace}</span>
      </div>
      <p>{state}</p>
    </article>
  );
}
