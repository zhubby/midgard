import { AgentConsole } from "@/components/AgentConsole";
import { ClusterOverview } from "@/components/ClusterOverview";
import { Metric } from "@/components/Metric";
import { PluginCatalog } from "@/components/PluginCatalog";
import { fetchPlugins, fetchTools } from "@/lib/api";

export default async function Home() {
  const [plugins, tools] = await Promise.all([
    fetchPlugins().catch(() => []),
    fetchTools().catch(() => []),
  ]);

  const approvalArmed = tools.some((t) => t.requires_approval);

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
          <Metric label="Plugins" value={String(plugins.length)} tone="ready" />
          <Metric label="Tools" value={String(tools.length)} tone="ready" />
          <Metric
            label="Approvals"
            value={approvalArmed ? "armed" : "idle"}
            tone={approvalArmed ? "warn" : "ready"}
          />
        </div>
      </section>

      <section className="workspace" id="agent-console" aria-label="Agent console">
        <AgentConsole initialSession={null} />

        <aside className="panel" aria-label="Available tools">
          <p className="eyebrow">Available tools</p>
          {tools.length === 0 && (
            <p className="muted">No tools registered.</p>
          )}
          <ol className="toolList">
            {tools.map((tool) => (
              <li key={tool.name}>
                <strong>{tool.name}</strong>
                <span>{tool.description}</span>
              </li>
            ))}
          </ol>
        </aside>
      </section>

      <section className="twoColumn" aria-label="Plugins and cluster">
        <PluginCatalog plugins={plugins} />
        <ClusterOverview />
      </section>
    </main>
  );
}
