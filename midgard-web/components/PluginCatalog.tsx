import type { PluginResponse } from "@/lib/types";

export function PluginCatalog({ plugins }: { plugins: PluginResponse[] }) {
  return (
    <div className="panel">
      <div className="sectionHeader">
        <p className="eyebrow">Plugin Catalog</p>
        <h2>Registered middleware capabilities</h2>
      </div>
      {plugins.length === 0 && <p className="muted">No plugins registered.</p>}
      {plugins.map((plugin) => (
        <article className="plugin" key={plugin.id}>
          <div>
            <h3>{plugin.name}</h3>
            <p>{plugin.middleware_kind}</p>
          </div>
        </article>
      ))}
    </div>
  );
}
