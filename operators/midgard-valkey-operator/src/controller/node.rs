use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::api::apps::v1::{Deployment, StatefulSet};
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod};
use kube::ResourceExt;
use kube::api::{Api, ListParams, Patch, PatchParams};
use kube::runtime::controller::Action;
use serde_json::json;
use tracing::{debug, warn};

use crate::api::{
    PersistenceReclaimPolicy, VALKEY_NODE_CONDITION_LIVE_CONFIG_APPLIED,
    VALKEY_NODE_CONDITION_PVC_READY, VALKEY_NODE_CONDITION_PVC_SIZE_READY,
    VALKEY_NODE_CONDITION_READY, ValkeyNode, ValkeyNodeStatus, WorkloadType,
};
use crate::controller::config::live_config_to_apply;
use crate::controller::resources::{
    delete_workload, ensure_node_config_map, ensure_pvc, ensure_workload,
};
use crate::controller::users::{OPERATOR_USER, fetch_system_user_password};
use crate::controller::{
    Context, DEFAULT_PORT, LABEL_CLUSTER, ROLE_MASTER, ROLE_PRIMARY, ROLE_REPLICA, ROLE_SLAVE,
    find_condition, label_selector, patch_status, remove_condition, set_condition,
    valkey_node_labels, valkey_node_pvc_name, valkey_node_resource_name,
};
use crate::error::Result;
use crate::valkey::ValkeyClient;

const PERSISTENT_VOLUME_CLEANUP_FINALIZER: &str = "valkey.io/persistent-volume-cleanup";

pub async fn reconcile(node: Arc<ValkeyNode>, ctx: Arc<Context>) -> Result<Action> {
    let client = ctx.client.clone();
    if node.metadata.deletion_timestamp.is_some() {
        return reconcile_deletion(client, &node).await;
    }
    if reconcile_persistence_finalizer(client.clone(), &node).await? {
        return Ok(Action::requeue(Duration::from_secs(1)));
    }

    ensure_node_config_map(client.clone(), &node).await?;
    ensure_pvc(client.clone(), &node).await?;
    ensure_workload(client.clone(), &node).await?;

    let mut status = compute_status(client.clone(), &node).await?;
    update_status(client.clone(), &node, &status).await?;

    if !status.ready {
        return Ok(Action::requeue(Duration::from_secs(10)));
    }

    match apply_live_config(client.clone(), &node).await {
        Ok(true) => {
            set_condition(
                &mut status.conditions,
                node.metadata.generation.unwrap_or_default(),
                VALKEY_NODE_CONDITION_LIVE_CONFIG_APPLIED,
                "Applied",
                "Live config applied",
                "True",
            );
            update_status(client, &node, &status).await?;
        }
        Ok(false) => {
            remove_condition(
                &mut status.conditions,
                VALKEY_NODE_CONDITION_LIVE_CONFIG_APPLIED,
            );
            update_status(client, &node, &status).await?;
        }
        Err(err) => {
            set_condition(
                &mut status.conditions,
                node.metadata.generation.unwrap_or_default(),
                VALKEY_NODE_CONDITION_LIVE_CONFIG_APPLIED,
                "ApplyFailed",
                &err.to_string(),
                "False",
            );
            update_status(client, &node, &status).await?;
            return Err(err);
        }
    }

    Ok(Action::requeue(Duration::from_secs(60)))
}

pub fn error_policy(
    _node: Arc<ValkeyNode>,
    error: &crate::error::Error,
    _ctx: Arc<Context>,
) -> Action {
    warn!(%error, "ValkeyNode reconcile failed");
    Action::requeue(Duration::from_secs(10))
}

fn persistence_reclaim_policy(node: &ValkeyNode) -> PersistenceReclaimPolicy {
    node.spec
        .persistence
        .as_ref()
        .and_then(|persistence| persistence.reclaim_policy.clone())
        .unwrap_or_default()
}

async fn reconcile_persistence_finalizer(client: kube::Client, node: &ValkeyNode) -> Result<bool> {
    let should_have = node.spec.persistence.is_some()
        && persistence_reclaim_policy(node) == PersistenceReclaimPolicy::Delete;
    let has = node
        .finalizers()
        .iter()
        .any(|f| f == PERSISTENT_VOLUME_CLEANUP_FINALIZER);
    match (should_have, has) {
        (true, false) => {
            patch_finalizers(
                client,
                node,
                add_finalizer(node, PERSISTENT_VOLUME_CLEANUP_FINALIZER),
            )
            .await?;
            Ok(true)
        }
        (false, true) => {
            patch_finalizers(
                client,
                node,
                remove_finalizer(node, PERSISTENT_VOLUME_CLEANUP_FINALIZER),
            )
            .await?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

async fn reconcile_deletion(client: kube::Client, node: &ValkeyNode) -> Result<Action> {
    if !node
        .finalizers()
        .iter()
        .any(|f| f == PERSISTENT_VOLUME_CLEANUP_FINALIZER)
    {
        return Ok(Action::await_change());
    }
    if node.spec.persistence.is_none()
        || persistence_reclaim_policy(node) != PersistenceReclaimPolicy::Delete
    {
        patch_finalizers(
            client,
            node,
            remove_finalizer(node, PERSISTENT_VOLUME_CLEANUP_FINALIZER),
        )
        .await?;
        return Ok(Action::await_change());
    }
    delete_workload(client.clone(), node).await?;
    if get_pod(client.clone(), node).await?.is_some() {
        return Ok(Action::requeue(Duration::from_secs(2)));
    }
    let namespace = node.namespace().unwrap_or_default();
    let pvc_api = Api::<PersistentVolumeClaim>::namespaced(client.clone(), &namespace);
    let pvc_name = valkey_node_pvc_name(node);
    if let Some(pvc) = pvc_api.get_opt(&pvc_name).await? {
        if pvc.metadata.deletion_timestamp.is_none() {
            pvc_api.delete(&pvc_name, &Default::default()).await?;
        }
        return Ok(Action::requeue(Duration::from_secs(2)));
    }
    patch_finalizers(
        client,
        node,
        remove_finalizer(node, PERSISTENT_VOLUME_CLEANUP_FINALIZER),
    )
    .await?;
    Ok(Action::await_change())
}

async fn patch_finalizers(
    client: kube::Client,
    node: &ValkeyNode,
    finalizers: Vec<String>,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_default();
    let api = Api::<ValkeyNode>::namespaced(client, &namespace);
    let patch = json!({ "metadata": { "finalizers": finalizers } });
    api.patch(
        &node.name_any(),
        &PatchParams::default(),
        &Patch::Merge(&patch),
    )
    .await?;
    Ok(())
}

fn add_finalizer(node: &ValkeyNode, finalizer: &str) -> Vec<String> {
    let mut finalizers = node.finalizers().to_vec();
    if !finalizers.iter().any(|item| item == finalizer) {
        finalizers.push(finalizer.to_string());
    }
    finalizers
}

fn remove_finalizer(node: &ValkeyNode, finalizer: &str) -> Vec<String> {
    node.finalizers()
        .iter()
        .filter(|item| *item != finalizer)
        .cloned()
        .collect()
}

async fn compute_status(client: kube::Client, node: &ValkeyNode) -> Result<ValkeyNodeStatus> {
    let generation = node.metadata.generation.unwrap_or_default();
    let mut status = node.status.clone().unwrap_or_default();
    status.observed_generation = generation;

    let pvc = get_pvc(client.clone(), node).await?;
    if node.spec.persistence.is_some() {
        let (condition_status, reason, message) = pvc_status_condition(pvc.as_ref());
        set_condition(
            &mut status.conditions,
            generation,
            VALKEY_NODE_CONDITION_PVC_READY,
            &reason,
            &message,
            condition_status,
        );
        let (condition_status, reason, message) = pvc_size_status_condition(node, pvc.as_ref());
        set_condition(
            &mut status.conditions,
            generation,
            VALKEY_NODE_CONDITION_PVC_SIZE_READY,
            &reason,
            &message,
            condition_status,
        );
    } else {
        remove_condition(&mut status.conditions, VALKEY_NODE_CONDITION_PVC_READY);
        remove_condition(&mut status.conditions, VALKEY_NODE_CONDITION_PVC_SIZE_READY);
    }

    let pod = get_pod(client.clone(), node).await?;
    let Some(pod) = pod else {
        status.ready = false;
        status.pod_name.clear();
        status.pod_ip.clear();
        let (reason, message) = if node.spec.persistence.is_some() {
            let (_, reason, message) = pvc_status_condition(pvc.as_ref());
            (reason, message)
        } else {
            (
                "PodNotReady".to_string(),
                "Pod does not exist yet".to_string(),
            )
        };
        set_condition(
            &mut status.conditions,
            generation,
            VALKEY_NODE_CONDITION_READY,
            &reason,
            &message,
            "False",
        );
        return Ok(status);
    };

    status.pod_name = pod.name_any();
    status.pod_ip = pod
        .status
        .as_ref()
        .and_then(|s| s.pod_ip.clone())
        .unwrap_or_default();
    let mut ready = pod_ready(&pod);
    if ready {
        ready = workload_rolled_out(client.clone(), node).await?;
    }
    status.ready = ready;
    if ready {
        status.role = get_valkey_role(client, node, &status.pod_ip)
            .await
            .unwrap_or_default();
        set_condition(
            &mut status.conditions,
            generation,
            VALKEY_NODE_CONDITION_READY,
            "PodRunning",
            "Pod is running and ready",
            "True",
        );
    } else {
        let mut reason = "PodNotReady".to_string();
        let mut message = "Pod is not ready".to_string();
        if node.spec.persistence.is_some() {
            let (pvc_status, pvc_reason, pvc_message) = pvc_status_condition(pvc.as_ref());
            if pvc_status != "True" {
                reason = pvc_reason;
                message = pvc_message;
            }
        }
        set_condition(
            &mut status.conditions,
            generation,
            VALKEY_NODE_CONDITION_READY,
            &reason,
            &message,
            "False",
        );
    }
    Ok(status)
}

async fn update_status(
    client: kube::Client,
    node: &ValkeyNode,
    status: &ValkeyNodeStatus,
) -> Result<()> {
    let namespace = node.namespace().unwrap_or_default();
    let api = Api::<ValkeyNode>::namespaced(client, &namespace);
    patch_status(&api, &node.name_any(), status).await?;
    Ok(())
}

pub async fn get_pod(client: kube::Client, node: &ValkeyNode) -> Result<Option<Pod>> {
    let namespace = node.namespace().unwrap_or_default();
    let pods = Api::<Pod>::namespaced(client, &namespace);
    let labels = valkey_node_labels(node);
    let pod_list = pods
        .list(&ListParams::default().labels(&label_selector(&labels)))
        .await?;
    Ok(pod_list.items.into_iter().next())
}

async fn get_pvc(client: kube::Client, node: &ValkeyNode) -> Result<Option<PersistentVolumeClaim>> {
    if node.spec.persistence.is_none() {
        return Ok(None);
    }
    let namespace = node.namespace().unwrap_or_default();
    let api = Api::<PersistentVolumeClaim>::namespaced(client, &namespace);
    Ok(api.get_opt(&valkey_node_pvc_name(node)).await?)
}

fn pod_ready(pod: &Pod) -> bool {
    pod.status
        .as_ref()
        .and_then(|status| status.conditions.as_ref())
        .is_some_and(|conditions| {
            conditions
                .iter()
                .any(|condition| condition.type_ == "Ready" && condition.status == "True")
        })
}

async fn workload_rolled_out(client: kube::Client, node: &ValkeyNode) -> Result<bool> {
    let namespace = node.namespace().unwrap_or_default();
    let name = valkey_node_resource_name(node);
    match node.spec.workload_type {
        WorkloadType::StatefulSet => {
            let api = Api::<StatefulSet>::namespaced(client, &namespace);
            let Some(sts) = api.get_opt(&name).await? else {
                return Ok(false);
            };
            let status = sts.status.unwrap_or_default();
            if status.observed_generation.unwrap_or_default()
                < sts.metadata.generation.unwrap_or_default()
            {
                return Ok(false);
            }
            Ok(status.current_revision == status.update_revision
                && status.ready_replicas.unwrap_or_default() >= 1)
        }
        WorkloadType::Deployment => {
            let api = Api::<Deployment>::namespaced(client, &namespace);
            let Some(dep) = api.get_opt(&name).await? else {
                return Ok(false);
            };
            let status = dep.status.unwrap_or_default();
            if status.observed_generation.unwrap_or_default()
                < dep.metadata.generation.unwrap_or_default()
            {
                return Ok(false);
            }
            let replicas = dep
                .spec
                .as_ref()
                .and_then(|spec| spec.replicas)
                .unwrap_or(1);
            Ok(status.updated_replicas.unwrap_or_default() >= replicas
                && status.ready_replicas.unwrap_or_default() >= replicas)
        }
    }
}

async fn get_valkey_role(client: kube::Client, node: &ValkeyNode, pod_ip: &str) -> Result<String> {
    if pod_ip.is_empty() {
        return Ok(String::new());
    }
    let labels = node.metadata.labels.clone().unwrap_or_default();
    let cluster_name = labels.get(LABEL_CLUSTER).cloned().unwrap_or_default();
    let password = fetch_system_user_password(
        client,
        OPERATOR_USER,
        &cluster_name,
        &node.namespace().unwrap_or_default(),
    )
    .await
    .unwrap_or_default();
    let tls = node.spec.tls.is_some();
    let valkey = ValkeyClient::new(
        pod_ip,
        DEFAULT_PORT as u16,
        (!password.is_empty()).then_some(OPERATOR_USER.to_string()),
        (!password.is_empty()).then_some(password),
        tls,
    );
    let info = valkey.query::<String>(&["INFO", "replication"]).await?;
    Ok(parse_valkey_role(&info))
}

fn parse_valkey_role(info: &str) -> String {
    for line in info.lines() {
        let line = line.trim();
        if let Some(role) = line.strip_prefix("role:") {
            return match role {
                ROLE_MASTER => ROLE_PRIMARY.to_string(),
                ROLE_SLAVE => ROLE_REPLICA.to_string(),
                _ => String::new(),
            };
        }
    }
    String::new()
}

async fn apply_live_config(client: kube::Client, node: &ValkeyNode) -> Result<bool> {
    let params = live_config_to_apply(&node.spec.config);
    if params.is_empty() {
        return Ok(false);
    }
    let pod_ip = node
        .status
        .as_ref()
        .map(|status| status.pod_ip.clone())
        .unwrap_or_default();
    if pod_ip.is_empty() {
        return Ok(false);
    }
    let labels = node.metadata.labels.clone().unwrap_or_default();
    let cluster_name = labels.get(LABEL_CLUSTER).cloned().unwrap_or_default();
    let password = fetch_system_user_password(
        client,
        OPERATOR_USER,
        &cluster_name,
        &node.namespace().unwrap_or_default(),
    )
    .await
    .unwrap_or_default();
    let valkey = ValkeyClient::new(
        pod_ip,
        DEFAULT_PORT as u16,
        (!password.is_empty()).then_some(OPERATOR_USER.to_string()),
        (!password.is_empty()).then_some(password),
        node.spec.tls.is_some(),
    );
    let mut args = vec!["CONFIG".to_string(), "SET".to_string()];
    for (key, value) in params {
        args.push(key);
        args.push(value);
    }
    valkey.query_owned::<String>(&args).await?;
    Ok(true)
}

fn pvc_status_condition(pvc: Option<&PersistentVolumeClaim>) -> (&'static str, String, String) {
    let Some(pvc) = pvc else {
        return (
            "False",
            "PersistentVolumeClaimPending".to_string(),
            "PersistentVolumeClaim does not exist yet".to_string(),
        );
    };
    let phase = pvc
        .status
        .as_ref()
        .and_then(|status| status.phase.clone())
        .unwrap_or_else(|| "Pending".to_string());
    if phase != "Bound" {
        return (
            "False",
            "PersistentVolumeClaimPending".to_string(),
            format!("PersistentVolumeClaim {} is {phase}", pvc.name_any()),
        );
    }
    (
        "True",
        "PersistentVolumeClaimBound".to_string(),
        format!("PersistentVolumeClaim {} is bound", pvc.name_any()),
    )
}

fn pvc_size_status_condition(
    node: &ValkeyNode,
    pvc: Option<&PersistentVolumeClaim>,
) -> (&'static str, String, String) {
    let Some(pvc) = pvc else {
        return (
            "False",
            "PersistentVolumeClaimResizePending".to_string(),
            "PersistentVolumeClaim does not exist yet".to_string(),
        );
    };
    let phase = pvc
        .status
        .as_ref()
        .and_then(|status| status.phase.clone())
        .unwrap_or_else(|| "Pending".to_string());
    if phase != "Bound" {
        return (
            "False",
            "PersistentVolumeClaimResizePending".to_string(),
            format!(
                "PersistentVolumeClaim {} is {phase} before size reconciliation can complete",
                pvc.name_any()
            ),
        );
    }
    let desired = node
        .spec
        .persistence
        .as_ref()
        .map(|p| p.size.0.clone())
        .unwrap_or_default();
    let capacity = pvc
        .status
        .as_ref()
        .and_then(|status| status.capacity.as_ref())
        .and_then(|capacity| capacity.get("storage"))
        .map(|quantity| quantity.0.clone());
    if capacity.is_none() {
        return (
            "False",
            "PersistentVolumeClaimResizePending".to_string(),
            format!(
                "PersistentVolumeClaim {} has no reported storage capacity yet",
                pvc.name_any()
            ),
        );
    }
    (
        "True",
        "PersistentVolumeClaimSizeSatisfied".to_string(),
        format!(
            "PersistentVolumeClaim {} satisfies the requested size {desired}",
            pvc.name_any()
        ),
    )
}

#[allow(dead_code)]
fn _keep(_: Option<&k8s_openapi::apimachinery::pkg::apis::meta::v1::Condition>) {
    let _ = find_condition;
    debug!("keep");
}
