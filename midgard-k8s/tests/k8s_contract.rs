use midgard_k8s::{KubernetesClient, MockKubernetesClient};

#[tokio::test]
async fn mock_client_reports_cluster_health() {
    let client = MockKubernetesClient;

    let health = client.cluster_health().await.unwrap();

    assert!(health.ready);
    assert_eq!(health.context, "mock");
}

#[tokio::test]
async fn mock_client_lists_namespaces_and_workloads() {
    let client = MockKubernetesClient;

    let namespaces = client.list_namespaces().await.unwrap();
    let workloads = client.list_workloads("default").await.unwrap();

    assert!(namespaces.iter().any(|namespace| namespace == "default"));
    assert_eq!(workloads[0].namespace, "default");
}
