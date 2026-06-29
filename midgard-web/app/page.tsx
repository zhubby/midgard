import { AgentConsole } from "@/components/AgentConsole";
import { MiddlewareDashboard } from "@/components/MiddlewareDashboard";

export default function Home() {
  return (
    <main className="app-shell">
      <header className="app-header">
        <div className="brand-lockup">
          <div className="brand-mark" aria-hidden="true">
            M
          </div>
          <div>
            <p className="section-kicker">Midgard</p>
            <h1>Agent-native middleware operations</h1>
          </div>
        </div>

        <div className="header-actions" aria-label="Workspace state">
          <span className="status-pill">
            <span aria-hidden="true" />
            Design draft
          </span>
        </div>
      </header>

      <section className="workspace-grid" aria-label="Midgard operations workspace">
        <AgentConsole />
        <MiddlewareDashboard />
      </section>
    </main>
  );
}
