export function ClusterOverview() {
  return (
    <div className="panel">
      <div className="sectionHeader">
        <p className="eyebrow">Cluster Overview</p>
        <h2>Namespaces and workloads</h2>
      </div>
      <div className="clusterRows">
        <ClusterRow namespace="default" workload="redis" state="Running" />
        <ClusterRow
          namespace="midgard-system"
          workload="midgard-server"
          state="Pending"
        />
      </div>
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
