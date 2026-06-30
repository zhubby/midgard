use std::sync::Arc;
use std::time::Duration;

use k8s_openapi::api::core::v1::Pod;
use kube::ResourceExt;
use kube::api::{Api, ListParams};
use kube::runtime::controller::Action;
use tracing::{debug, warn};

use crate::api::{
    CONDITION_CLUSTER_FORMED, CONDITION_DEGRADED, CONDITION_PROGRESSING, CONDITION_READY,
    CONDITION_SLOTS_ASSIGNED, ClusterState as ApiClusterState,
    VALKEY_NODE_CONDITION_LIVE_CONFIG_APPLIED, ValkeyCluster, ValkeyClusterStatus, ValkeyNode,
};
use crate::controller::config::{server_config_roll_hash, upsert_cluster_config_map};
use crate::controller::resources::{build_cluster_valkey_node, reconcile_pdb, upsert_service};
use crate::controller::users::{OPERATOR_USER, fetch_system_user_password, reconcile_users_acl};
use crate::controller::{
    Context, DEFAULT_PORT, LABEL_CLUSTER, LABEL_NODE_INDEX, LABEL_SHARD_INDEX, ROLE_PRIMARY, apply,
    find_condition, label_selector, node_role_and_shard, patch_status, remove_condition,
    remove_condition_if_reason, set_condition,
};
use crate::error::{Error, Result};
use crate::valkey::{self, ClusterState, NodeState, ShardState, SlotsRange};

const REQUEUE_FAST: Duration = Duration::from_secs(2);
const REQUEUE_HEALTH: Duration = Duration::from_secs(30);
const REBALANCE_SLOT_BATCH_SIZE: i32 = 400;

pub async fn reconcile(cluster: Arc<ValkeyCluster>, ctx: Arc<Context>) -> Result<Action> {
    let client = ctx.client.clone();
    let mut working = (*cluster).clone();
    let generation = working.metadata.generation.unwrap_or_default();

    upsert_service(client.clone(), &working)
        .await
        .inspect_err(|err| {
            condition_error(&mut working, CONDITION_READY, "ServiceError", err);
        })?;
    reconcile_pdb(client.clone(), &working)
        .await
        .inspect_err(|err| {
            condition_error(
                &mut working,
                CONDITION_READY,
                "PodDisruptionBudgetError",
                err,
            );
        })?;
    reconcile_users_acl(client.clone(), &working)
        .await
        .inspect_err(|err| {
            condition_error(&mut working, CONDITION_READY, "UsersACLError", err);
        })?;
    upsert_cluster_config_map(client.clone(), &working)
        .await
        .inspect_err(|err| {
            condition_error(&mut working, CONDITION_READY, "ConfigMapError", err);
        })?;

    let config_hash = server_config_roll_hash(&working);
    let nodes = list_cluster_nodes(client.clone(), &working).await?;
    if reconcile_valkey_nodes(client.clone(), &working, &nodes, &config_hash).await? {
        if let Some(result) = handle_pod_scheduling_issues(client.clone(), &mut working).await? {
            return Ok(result);
        }
        set_condition(
            &mut status_mut(&mut working).conditions,
            generation,
            CONDITION_READY,
            "UpdatingNodes",
            "Updating ValkeyNodes",
            "False",
        );
        set_condition(
            &mut status_mut(&mut working).conditions,
            generation,
            CONDITION_PROGRESSING,
            "UpdatingNodes",
            "Updating ValkeyNodes",
            "True",
        );
        update_status(client, &mut working, None).await?;
        return Ok(Action::requeue(REQUEUE_FAST));
    }
    if let Some(result) = handle_pod_scheduling_issues(client.clone(), &mut working).await? {
        return Ok(result);
    }

    let password = match fetch_system_user_password(
        client.clone(),
        OPERATOR_USER,
        &working.name_any(),
        &working.namespace().unwrap_or_default(),
    )
    .await
    {
        Ok(password) => password,
        Err(err) => {
            warn!(%err, "system user password unavailable; continuing without auth");
            String::new()
        }
    };
    let state = get_valkey_cluster_state(&working, &nodes, password).await;
    forget_stale_nodes(&working, &state, &nodes).await;

    let met = meet_isolated_nodes(&state).await?;
    if met > 0 {
        set_condition_pair(&mut working, "AddingNodes", "Introducing nodes to cluster");
        update_status(client, &mut working, Some(&state)).await?;
        return Ok(Action::requeue(REQUEUE_FAST));
    }

    if !state.pending_nodes.is_empty() {
        let assigned = assign_slots_to_pending_primaries(&working, &state, &nodes).await?;
        if assigned > 0 {
            set_condition_pair(&mut working, "AddingNodes", "Assigning slots to primaries");
            update_status(client, &mut working, Some(&state)).await?;
            return Ok(Action::requeue(REQUEUE_FAST));
        }
    }

    if !state.pending_nodes.is_empty() {
        let replicated = replicate_pending_replicas(&working, &state, &nodes).await?;
        if replicated > 0 {
            set_condition_pair(&mut working, "AddingNodes", "Attaching replicas");
            update_status(client, &mut working, Some(&state)).await?;
            return Ok(Action::requeue(REQUEUE_FAST));
        }
    }

    let all_shards = effective_shards(&state, &nodes);
    if let Some(result) = handle_scale_in(client.clone(), &mut working, &state, &nodes).await? {
        return Ok(result);
    }

    if all_shards.len() < working.spec.shards as usize {
        set_condition(
            &mut status_mut(&mut working).conditions,
            generation,
            CONDITION_READY,
            "MissingShards",
            "Waiting for all shards to be created",
            "False",
        );
        set_condition(
            &mut status_mut(&mut working).conditions,
            generation,
            CONDITION_PROGRESSING,
            "Reconciling",
            "Creating shards",
            "True",
        );
        set_condition(
            &mut status_mut(&mut working).conditions,
            generation,
            CONDITION_CLUSTER_FORMED,
            "MissingShards",
            "Waiting for shards",
            "False",
        );
        update_status(client, &mut working, Some(&state)).await?;
        return Ok(Action::requeue(REQUEUE_FAST));
    }

    for shard in &all_shards {
        if valkey::count_slots(&shard.slots) == 0 {
            continue;
        }
        if shard.nodes.len() < (1 + working.spec.replicas) as usize {
            set_condition(
                &mut status_mut(&mut working).conditions,
                generation,
                CONDITION_READY,
                "MissingReplicas",
                "Waiting for all replicas to be created",
                "False",
            );
            set_condition(
                &mut status_mut(&mut working).conditions,
                generation,
                CONDITION_PROGRESSING,
                "Reconciling",
                "Creating replicas",
                "True",
            );
            set_condition(
                &mut status_mut(&mut working).conditions,
                generation,
                CONDITION_CLUSTER_FORMED,
                "MissingReplicas",
                "Waiting for replicas",
                "False",
            );
            update_status(client, &mut working, Some(&state)).await?;
            return Ok(Action::requeue(REQUEUE_FAST));
        }
    }

    if !state.unassigned_slots().is_empty() {
        let assigned = assign_slots_to_existing_primaries(&state).await?;
        if assigned > 0 {
            set_condition_pair(&mut working, "RepairingSlots", "Assigning unassigned slots");
            update_status(client, &mut working, Some(&state)).await?;
            return Ok(Action::requeue(REQUEUE_FAST));
        }
        set_condition(
            &mut status_mut(&mut working).conditions,
            generation,
            CONDITION_SLOTS_ASSIGNED,
            "SlotsUnassigned",
            "Waiting for slots to be assigned",
            "False",
        );
        set_condition_pair(
            &mut working,
            "Reconciling",
            "Waiting for all slots to be assigned",
        );
        update_status(client, &mut working, Some(&state)).await?;
        return Ok(Action::requeue(REQUEUE_FAST));
    }

    for shard in &all_shards {
        for node in &shard.nodes {
            if !node.is_replication_in_sync() {
                set_condition_pair(
                    &mut working,
                    "Reconciling",
                    "Waiting for replicas to sync with primary",
                );
                update_status(client, &mut working, Some(&state)).await?;
                return Ok(Action::requeue(REQUEUE_FAST));
            }
        }
    }

    if rebalance_slots(&all_shards, working.spec.shards).await? {
        remove_condition(&mut status_mut(&mut working).conditions, CONDITION_DEGRADED);
        set_condition_pair(
            &mut working,
            "RebalancingSlots",
            "Rebalancing slots across primaries",
        );
        update_status(client, &mut working, Some(&state)).await?;
        return Ok(Action::requeue(REQUEUE_FAST));
    }

    let generation = working.metadata.generation.unwrap_or_default();
    set_condition(
        &mut status_mut(&mut working).conditions,
        generation,
        CONDITION_READY,
        "ClusterHealthy",
        "Cluster is healthy",
        "True",
    );
    set_condition(
        &mut status_mut(&mut working).conditions,
        generation,
        CONDITION_PROGRESSING,
        "ReconcileComplete",
        "No changes needed",
        "False",
    );
    remove_condition(&mut status_mut(&mut working).conditions, CONDITION_DEGRADED);
    set_condition(
        &mut status_mut(&mut working).conditions,
        generation,
        CONDITION_CLUSTER_FORMED,
        "TopologyComplete",
        "All nodes joined cluster",
        "True",
    );
    set_condition(
        &mut status_mut(&mut working).conditions,
        generation,
        CONDITION_SLOTS_ASSIGNED,
        "AllSlotsAssigned",
        "All slots assigned",
        "True",
    );
    update_status(client, &mut working, Some(&state)).await?;
    Ok(Action::requeue(REQUEUE_HEALTH))
}

pub fn error_policy(
    _cluster: Arc<ValkeyCluster>,
    error: &crate::error::Error,
    _ctx: Arc<Context>,
) -> Action {
    warn!(%error, "ValkeyCluster reconcile failed");
    Action::requeue(Duration::from_secs(10))
}

fn condition_error(cluster: &mut ValkeyCluster, cond_type: &str, reason: &str, err: &Error) {
    let generation = cluster.metadata.generation.unwrap_or_default();
    set_condition(
        &mut status_mut(cluster).conditions,
        generation,
        cond_type,
        reason,
        &err.to_string(),
        "False",
    );
}

fn set_condition_pair(cluster: &mut ValkeyCluster, reason: &str, message: &str) {
    let generation = cluster.metadata.generation.unwrap_or_default();
    set_condition(
        &mut status_mut(cluster).conditions,
        generation,
        CONDITION_READY,
        reason,
        message,
        "False",
    );
    set_condition(
        &mut status_mut(cluster).conditions,
        generation,
        CONDITION_PROGRESSING,
        reason,
        message,
        "True",
    );
}

fn status_mut(cluster: &mut ValkeyCluster) -> &mut ValkeyClusterStatus {
    cluster
        .status
        .get_or_insert_with(ValkeyClusterStatus::default)
}

async fn list_cluster_nodes(
    client: kube::Client,
    cluster: &ValkeyCluster,
) -> Result<Vec<ValkeyNode>> {
    let namespace = cluster.namespace().unwrap_or_default();
    let api = Api::<ValkeyNode>::namespaced(client, &namespace);
    let lp = ListParams::default().labels(&format!("{LABEL_CLUSTER}={}", cluster.name_any()));
    Ok(api.list(&lp).await?.items)
}

async fn reconcile_valkey_nodes(
    client: kube::Client,
    cluster: &ValkeyCluster,
    nodes: &[ValkeyNode],
    config_hash: &str,
) -> Result<bool> {
    let nodes_per_shard = 1 + cluster.spec.replicas;
    let namespace = cluster.namespace().unwrap_or_default();
    let api = Api::<ValkeyNode>::namespaced(client, &namespace);
    for shard_index in 0..cluster.spec.shards {
        for node_index in (0..nodes_per_shard).rev() {
            let mut desired = build_cluster_valkey_node(cluster, shard_index, node_index);
            desired.spec.server_config_hash = config_hash.to_string();
            let name = desired.name_any();
            let current = api.get_opt(&name).await?;
            let Some(current) = current else {
                apply(&api, &name, &desired).await?;
                return Ok(true);
            };
            if node_requires_roll(&current, &desired) {
                apply(&api, &name, &desired).await?;
                return Ok(true);
            }
            let status = current.status.clone().unwrap_or_default();
            if status.observed_generation > 0
                && current.metadata.generation.unwrap_or_default() != status.observed_generation
            {
                return Ok(true);
            }
            if !status.ready {
                return Ok(true);
            }
            if find_condition(
                &status.conditions,
                VALKEY_NODE_CONDITION_LIVE_CONFIG_APPLIED,
            )
            .is_some_and(|condition| condition.status == "False")
            {
                return Ok(true);
            }
        }
    }
    let _ = nodes;
    Ok(false)
}

fn node_requires_roll(current: &ValkeyNode, desired: &ValkeyNode) -> bool {
    if current
        .status
        .as_ref()
        .map(|status| status.pod_ip.is_empty())
        .unwrap_or(true)
    {
        return false;
    }
    let mut current_spec = serde_json::to_value(&current.spec).unwrap_or(ValueOrNull::null());
    let mut desired_spec = serde_json::to_value(&desired.spec).unwrap_or(ValueOrNull::null());
    if let Some(obj) = current_spec.as_object_mut() {
        obj.remove("config");
    }
    if let Some(obj) = desired_spec.as_object_mut() {
        obj.remove("config");
    }
    current_spec != desired_spec
}

struct ValueOrNull;

impl ValueOrNull {
    fn null() -> serde_json::Value {
        serde_json::Value::Null
    }
}

async fn get_valkey_cluster_state(
    cluster: &ValkeyCluster,
    nodes: &[ValkeyNode],
    password: String,
) -> ClusterState {
    let addresses = nodes
        .iter()
        .filter_map(|node| node.status.as_ref().map(|status| status.pod_ip.clone()))
        .filter(|ip| !ip.is_empty())
        .collect::<Vec<_>>();
    valkey::get_cluster_state(
        &addresses,
        DEFAULT_PORT as u16,
        (!password.is_empty()).then_some(OPERATOR_USER.to_string()),
        (!password.is_empty()).then_some(password),
        cluster.spec.tls.is_some(),
    )
    .await
}

async fn meet_isolated_nodes(state: &ClusterState) -> Result<usize> {
    let mut isolated = state
        .pending_nodes
        .iter()
        .filter(|node| node.is_isolated())
        .cloned()
        .collect::<Vec<_>>();
    if isolated.is_empty() {
        return Ok(0);
    }
    let target = find_meet_target(state, &isolated)
        .ok_or_else(|| Error::InvalidState("no meet target found".to_string()))?;
    let current_epoch = target.current_epoch();
    for (idx, node) in isolated.iter().enumerate() {
        let epoch = (current_epoch + idx as i64 + 1).to_string();
        let args = vec!["CLUSTER".to_string(), "SET-CONFIG-EPOCH".to_string(), epoch];
        if let Err(err) = node.client.query_owned::<String>(&args).await {
            debug!(address = %node.address, %err, "CLUSTER SET-CONFIG-EPOCH skipped");
        }
    }
    if target.address == isolated[0].address {
        isolated.remove(0);
    }
    let mut met = 0;
    for node in isolated {
        node.client
            .query_owned::<String>(&[
                "CLUSTER".to_string(),
                "MEET".to_string(),
                target.address.clone(),
                target.port.to_string(),
            ])
            .await?;
        target
            .client
            .query_owned::<String>(&[
                "CLUSTER".to_string(),
                "MEET".to_string(),
                node.address.clone(),
                node.port.to_string(),
            ])
            .await?;
        met += 1;
    }
    Ok(met)
}

fn find_meet_target(state: &ClusterState, isolated: &[NodeState]) -> Option<NodeState> {
    state
        .shards
        .iter()
        .find_map(|shard| shard.primary().cloned())
        .or_else(|| {
            state
                .pending_nodes
                .iter()
                .find(|node| !node.is_isolated())
                .cloned()
        })
        .or_else(|| isolated.first().cloned())
}

async fn assign_slots_to_pending_primaries(
    cluster: &ValkeyCluster,
    state: &ClusterState,
    nodes: &[ValkeyNode],
) -> Result<usize> {
    let is_single = cluster.spec.shards == 1 && cluster.spec.replicas == 0;
    let mut primaries = Vec::new();
    for node in &state.pending_nodes {
        if node.is_isolated() && !is_single {
            continue;
        }
        let (role, shard_index) = node_role_and_shard(&node.address, nodes);
        if role != ROLE_PRIMARY {
            continue;
        }
        if shard_exists_in_topology(state, shard_index, nodes) {
            continue;
        }
        primaries.push(node.clone());
    }
    if primaries.is_empty() {
        return Ok(0);
    }
    let slots = state.unassigned_slots();
    if slots.is_empty() {
        return Ok(0);
    }
    let slot_plans = split_slots_evenly(slots, primaries.len());
    let mut assigned = 0;
    for (node, node_ranges) in primaries.into_iter().zip(slot_plans) {
        if node_ranges.is_empty() {
            continue;
        }
        add_slot_ranges(&node, &node_ranges).await?;
        assigned += 1;
    }
    Ok(assigned)
}

async fn assign_slots_to_existing_primaries(state: &ClusterState) -> Result<usize> {
    let slots = state.unassigned_slots();
    if slots.is_empty() {
        return Ok(0);
    }
    let primaries = state
        .shards
        .iter()
        .filter_map(ShardState::primary)
        .filter(|node| {
            !node
                .flags
                .iter()
                .any(|flag| flag == "fail" || flag == "pfail")
        })
        .cloned()
        .collect::<Vec<_>>();
    if primaries.is_empty() {
        return Ok(0);
    }

    let slot_plans = split_slots_evenly(slots, primaries.len());
    let mut assigned = 0;
    for (node, node_ranges) in primaries.into_iter().zip(slot_plans) {
        if node_ranges.is_empty() {
            continue;
        }
        add_slot_ranges(&node, &node_ranges).await?;
        assigned += 1;
    }
    Ok(assigned)
}

async fn add_slot_ranges(node: &NodeState, ranges: &[SlotsRange]) -> Result<()> {
    let mut args = vec!["CLUSTER".to_string(), "ADDSLOTSRANGE".to_string()];
    for range in ranges {
        args.push(range.start.to_string());
        args.push(range.end.to_string());
    }
    node.client.query_owned::<String>(&args).await?;
    Ok(())
}

fn split_slots_evenly(slots: Vec<SlotsRange>, primary_count: usize) -> Vec<Vec<SlotsRange>> {
    if primary_count == 0 {
        return Vec::new();
    }
    let mut slots = slots;
    let mut unassigned = valkey::count_slots(&slots);
    let mut output = Vec::with_capacity(primary_count);
    for assigned in 0..primary_count {
        let remaining_primaries = (primary_count - assigned) as i32;
        let target = (unassigned + remaining_primaries - 1) / remaining_primaries;
        let mut node_ranges = Vec::new();
        let mut remaining = target;
        while remaining > 0 && !slots.is_empty() {
            let take = std::cmp::min(slots[0].end - slots[0].start + 1, remaining);
            let start = slots[0].start;
            let end = start + take - 1;
            node_ranges.push(SlotsRange { start, end });
            remaining -= take;
            unassigned -= take;
            if take == slots[0].end - slots[0].start + 1 {
                slots.remove(0);
            } else {
                slots[0].start = end + 1;
            }
        }
        output.push(node_ranges);
    }
    output
}

async fn replicate_pending_replicas(
    cluster: &ValkeyCluster,
    state: &ClusterState,
    nodes: &[ValkeyNode],
) -> Result<usize> {
    let mut replicated = 0;
    for node in &state.pending_nodes {
        let (role, shard_index) = node_role_and_shard(&node.address, nodes);
        if role == ROLE_PRIMARY && !shard_exists_in_topology(state, shard_index, nodes) {
            continue;
        }
        match replicate_to_shard_primary(cluster, state, node, shard_index, nodes).await {
            Ok(()) => replicated += 1,
            Err(Error::InvalidState(message)) if message.contains("primary not yet") => continue,
            Err(err) => return Err(err),
        }
    }
    Ok(replicated)
}

async fn replicate_to_shard_primary(
    _cluster: &ValkeyCluster,
    state: &ClusterState,
    node: &NodeState,
    shard_index: i32,
    nodes: &[ValkeyNode],
) -> Result<()> {
    let (primary_id, _) = find_shard_primary(state, shard_index, nodes);
    if primary_id.is_empty() {
        return Err(Error::InvalidState(format!(
            "shard {shard_index}: primary not yet in cluster state"
        )));
    }
    if let Err(err) = node
        .client
        .query_owned::<String>(&[
            "CLUSTER".to_string(),
            "REPLICATE".to_string(),
            primary_id.clone(),
        ])
        .await
    {
        if err.to_string().contains("Unknown node") {
            return Err(Error::InvalidState(format!(
                "shard {shard_index}: primary not yet known to replica"
            )));
        }
        return Err(err.into());
    }
    Ok(())
}

async fn forget_stale_nodes(cluster: &ValkeyCluster, state: &ClusterState, nodes: &[ValkeyNode]) {
    let _ = cluster;
    for shard in &state.shards {
        for node in &shard.nodes {
            for failing in node.failing_nodes() {
                if nodes.iter().any(|n| {
                    n.status
                        .as_ref()
                        .is_some_and(|status| status.pod_ip == failing.address)
                }) {
                    continue;
                }
                if state.has_replica_of(&failing.id) {
                    continue;
                }
                if let Err(err) = node
                    .client
                    .query_owned::<String>(&[
                        "CLUSTER".to_string(),
                        "FORGET".to_string(),
                        failing.id.clone(),
                    ])
                    .await
                {
                    warn!(%err, address = %failing.address, "CLUSTER FORGET failed");
                }
            }
        }
    }
}

fn shard_exists_in_topology(state: &ClusterState, shard_index: i32, nodes: &[ValkeyNode]) -> bool {
    for node in nodes {
        let labels = node.metadata.labels.clone().unwrap_or_default();
        if labels
            .get(LABEL_SHARD_INDEX)
            .and_then(|s| s.parse::<i32>().ok())
            != Some(shard_index)
        {
            continue;
        }
        let Some(pod_ip) = node
            .status
            .as_ref()
            .map(|status| status.pod_ip.clone())
            .filter(|ip| !ip.is_empty())
        else {
            continue;
        };
        if state.shards.iter().any(|shard| {
            shard
                .nodes
                .iter()
                .any(|state_node| state_node.address == pod_ip)
        }) {
            return true;
        }
    }
    false
}

fn find_shard_primary(
    state: &ClusterState,
    shard_index: i32,
    nodes: &[ValkeyNode],
) -> (String, String) {
    for node in nodes {
        let labels = node.metadata.labels.clone().unwrap_or_default();
        if labels
            .get(LABEL_SHARD_INDEX)
            .and_then(|s| s.parse::<i32>().ok())
            != Some(shard_index)
        {
            continue;
        }
        let Some(pod_ip) = node
            .status
            .as_ref()
            .map(|status| status.pod_ip.clone())
            .filter(|ip| !ip.is_empty())
        else {
            continue;
        };
        for shard in &state.shards {
            if valkey::count_slots(&shard.slots) == 0 {
                continue;
            }
            if let Some(primary) = shard.primary()
                && primary.address == pod_ip
            {
                return (primary.id.clone(), pod_ip);
            }
        }
    }
    (String::new(), String::new())
}

fn effective_shards(state: &ClusterState, nodes: &[ValkeyNode]) -> Vec<ShardState> {
    let mut shards = state.shards.clone();
    for node in &state.pending_nodes {
        let (role, _) = node_role_and_shard(&node.address, nodes);
        if role == ROLE_PRIMARY {
            shards.push(ShardState {
                id: node.shard_id.clone(),
                primary_id: node.id.clone(),
                slots: Vec::new(),
                nodes: vec![node.clone()],
            });
        }
    }
    shards
}

async fn rebalance_slots(shards: &[ShardState], expected_shards: i32) -> Result<bool> {
    let Some(slot_move) =
        valkey::plan_rebalance_move(shards, expected_shards, REBALANCE_SLOT_BATCH_SIZE)?
    else {
        return Ok(false);
    };
    if valkey::slot_migration_in_progress(&slot_move.src).await? {
        return Ok(true);
    }
    if !slot_move.src.cluster_nodes.contains(&slot_move.dst.id) {
        return Ok(true);
    }
    let ranges = valkey::slots_to_ranges(&slot_move.slots);
    if let Err(err) = valkey::migrate_slots_atomic(&slot_move.src, &slot_move.dst, &ranges).await {
        if valkey::is_slots_not_served_by_node(&err) {
            return Ok(true);
        }
        return Err(err);
    }
    Ok(true)
}

async fn handle_scale_in(
    client: kube::Client,
    cluster: &mut ValkeyCluster,
    state: &ClusterState,
    nodes: &[ValkeyNode],
) -> Result<Option<Action>> {
    if state.shards.len() > cluster.spec.shards as usize
        && drain_excess_shards(client.clone(), cluster, state, nodes).await?
    {
        remove_condition(&mut status_mut(cluster).conditions, CONDITION_DEGRADED);
        set_condition_pair(
            cluster,
            "RebalancingSlots",
            "Rebalancing slots for scale-in",
        );
        update_status(client, cluster, Some(state)).await?;
        return Ok(Some(Action::requeue(REQUEUE_FAST)));
    }
    if delete_excess_valkey_nodes(client.clone(), cluster).await? {
        return Ok(Some(Action::requeue(REQUEUE_FAST)));
    }
    Ok(None)
}

async fn drain_excess_shards(
    client: kube::Client,
    cluster: &ValkeyCluster,
    state: &ClusterState,
    nodes: &[ValkeyNode],
) -> Result<bool> {
    let expected = cluster.spec.shards;
    let mut remaining = Vec::new();
    let mut draining = Vec::new();
    for shard in &state.shards {
        let index = shard_index_from_state(shard, nodes);
        if index >= 0 && index < expected {
            remaining.push(shard.clone());
        } else {
            draining.push(shard.clone());
        }
    }
    if draining.is_empty() {
        return Ok(false);
    }
    for shard in &draining {
        let Some(slot_move) =
            valkey::plan_drain_move(shard, &remaining, REBALANCE_SLOT_BATCH_SIZE)?
        else {
            continue;
        };
        if valkey::slot_migration_in_progress(&slot_move.src).await? {
            return Ok(true);
        }
        if !slot_move.src.cluster_nodes.contains(&slot_move.dst.id) {
            return Ok(true);
        }
        let ranges = valkey::slots_to_ranges(&slot_move.slots);
        if let Err(err) =
            valkey::migrate_slots_atomic(&slot_move.src, &slot_move.dst, &ranges).await
        {
            if valkey::is_slots_not_served_by_node(&err) {
                return Ok(true);
            }
            return Err(err);
        }
        return Ok(true);
    }
    let namespace = cluster.namespace().unwrap_or_default();
    let api = Api::<ValkeyNode>::namespaced(client, &namespace);
    for shard in &draining {
        let shard_index = shard_index_from_state(shard, nodes);
        if shard_index < 0 {
            continue;
        }
        let nodes_per_shard = 1 + cluster.spec.replicas;
        for node_index in 0..nodes_per_shard {
            let name =
                crate::controller::valkey_node_name(&cluster.name_any(), shard_index, node_index);
            if api.get_opt(&name).await?.is_some() {
                api.delete(&name, &Default::default()).await?;
            }
        }
    }
    Ok(true)
}

async fn delete_excess_valkey_nodes(client: kube::Client, cluster: &ValkeyCluster) -> Result<bool> {
    let nodes = list_cluster_nodes(client.clone(), cluster).await?;
    let namespace = cluster.namespace().unwrap_or_default();
    let api = Api::<ValkeyNode>::namespaced(client, &namespace);
    let nodes_per_shard = 1 + cluster.spec.replicas;
    let mut deleted = false;
    for node in nodes {
        let labels = node.metadata.labels.clone().unwrap_or_default();
        let shard_index = labels
            .get(LABEL_SHARD_INDEX)
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(-1);
        let node_index = labels
            .get(LABEL_NODE_INDEX)
            .and_then(|value| value.parse::<i32>().ok())
            .unwrap_or(-1);
        if shard_index >= cluster.spec.shards || node_index >= nodes_per_shard {
            api.delete(&node.name_any(), &Default::default()).await?;
            deleted = true;
        }
    }
    Ok(deleted)
}

fn shard_index_from_state(shard: &ShardState, nodes: &[ValkeyNode]) -> i32 {
    if let Some(primary) = shard.primary() {
        let (_, shard_index) = node_role_and_shard(&primary.address, nodes);
        if shard_index >= 0 {
            return shard_index;
        }
    }
    for node in &shard.nodes {
        let (_, shard_index) = node_role_and_shard(&node.address, nodes);
        if shard_index >= 0 {
            return shard_index;
        }
    }
    -1
}

async fn handle_pod_scheduling_issues(
    client: kube::Client,
    cluster: &mut ValkeyCluster,
) -> Result<Option<Action>> {
    let namespace = cluster.namespace().unwrap_or_default();
    let pods = Api::<Pod>::namespaced(client.clone(), &namespace);
    let labels = label_selector(&std::collections::BTreeMap::from([(
        LABEL_CLUSTER.to_string(),
        cluster.name_any(),
    )]));
    let pod_list = pods.list(&ListParams::default().labels(&labels)).await?;
    for pod in pod_list.items {
        if pod.metadata.deletion_timestamp.is_some() {
            continue;
        }
        let Some(status) = &pod.status else {
            continue;
        };
        let Some(conditions) = &status.conditions else {
            continue;
        };
        if let Some(condition) = conditions.iter().find(|condition| {
            condition.type_ == "PodScheduled"
                && condition.status == "False"
                && condition.reason.as_deref() == Some("Unschedulable")
        }) {
            let detail = condition.message.clone().unwrap_or_default();
            let message = if detail.is_empty() {
                format!("Pod {} is unschedulable", pod.name_any())
            } else {
                format!("Pod {} is unschedulable: {}", pod.name_any(), detail)
            };
            let generation = cluster.metadata.generation.unwrap_or_default();
            set_condition(
                &mut status_mut(cluster).conditions,
                generation,
                CONDITION_DEGRADED,
                "PodUnschedulable",
                &message,
                "True",
            );
            set_condition(
                &mut status_mut(cluster).conditions,
                generation,
                CONDITION_READY,
                "PodUnschedulable",
                &message,
                "False",
            );
            set_condition(
                &mut status_mut(cluster).conditions,
                generation,
                CONDITION_PROGRESSING,
                "Reconciling",
                "Waiting for unschedulable pods to be scheduled",
                "True",
            );
            update_status(client, cluster, None).await?;
            return Ok(Some(Action::requeue(Duration::from_secs(10))));
        }
    }
    remove_condition_if_reason(
        &mut status_mut(cluster).conditions,
        CONDITION_DEGRADED,
        "PodUnschedulable",
    );
    remove_condition_if_reason(
        &mut status_mut(cluster).conditions,
        CONDITION_READY,
        "PodUnschedulable",
    );
    Ok(None)
}

async fn update_status(
    client: kube::Client,
    cluster: &mut ValkeyCluster,
    state: Option<&ClusterState>,
) -> Result<()> {
    let mut status = cluster.status.clone().unwrap_or_default();
    if let Some(state) = state {
        status.ready_shards = count_ready_shards(state, cluster);
        status.shards = state.shards.len() as i32;
    }
    let ready = find_condition(&status.conditions, CONDITION_READY);
    let progressing = find_condition(&status.conditions, CONDITION_PROGRESSING);
    let degraded = find_condition(&status.conditions, CONDITION_DEGRADED);
    if let Some(condition) = degraded.filter(|condition| condition.status == "True") {
        status.state = ApiClusterState::Degraded;
        status.reason = condition.reason.clone();
        status.message = condition.message.clone();
    } else if let Some(condition) = ready.filter(|condition| condition.status == "True") {
        status.state = ApiClusterState::Ready;
        status.reason = condition.reason.clone();
        status.message = condition.message.clone();
    } else if let Some(condition) = progressing.filter(|condition| condition.status == "True") {
        status.state = ApiClusterState::Reconciling;
        status.reason = condition.reason.clone();
        status.message = condition.message.clone();
    } else if let Some(condition) = ready.filter(|condition| condition.status == "False") {
        status.state = ApiClusterState::Failed;
        status.reason = condition.reason.clone();
        status.message = condition.message.clone();
    }
    let namespace = cluster.namespace().unwrap_or_default();
    let api = Api::<ValkeyCluster>::namespaced(client, &namespace);
    patch_status(&api, &cluster.name_any(), &status).await?;
    cluster.status = Some(status);
    Ok(())
}

fn count_ready_shards(state: &ClusterState, cluster: &ValkeyCluster) -> i32 {
    let required_nodes = 1 + cluster.spec.replicas;
    state
        .shards
        .iter()
        .filter(|shard| {
            shard.nodes.len() >= required_nodes as usize
                && shard.primary().is_some()
                && shard.nodes.iter().all(|node| {
                    !node
                        .flags
                        .iter()
                        .any(|flag| flag == "fail" || flag == "pfail")
                        && node.is_replication_in_sync()
                })
        })
        .count() as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_slots_evenly_covers_full_slot_range_for_three_primaries() {
        let plans = split_slots_evenly(
            vec![SlotsRange {
                start: 0,
                end: valkey::TOTAL_SLOTS - 1,
            }],
            3,
        );

        assert_eq!(plans.len(), 3);
        assert_eq!(
            plans,
            vec![
                vec![SlotsRange {
                    start: 0,
                    end: 5461,
                }],
                vec![SlotsRange {
                    start: 5462,
                    end: 10922,
                }],
                vec![SlotsRange {
                    start: 10923,
                    end: 16383,
                }],
            ]
        );
        assert_eq!(
            plans
                .iter()
                .map(|ranges| valkey::count_slots(ranges))
                .sum::<i32>(),
            valkey::TOTAL_SLOTS
        );
    }

    #[test]
    fn split_slots_evenly_preserves_gaps_between_unassigned_ranges() {
        let plans = split_slots_evenly(
            vec![
                SlotsRange { start: 0, end: 2 },
                SlotsRange { start: 10, end: 14 },
            ],
            3,
        );

        assert_eq!(
            plans,
            vec![
                vec![SlotsRange { start: 0, end: 2 }],
                vec![SlotsRange { start: 10, end: 12 }],
                vec![SlotsRange { start: 13, end: 14 }],
            ]
        );
        assert_eq!(
            plans
                .iter()
                .map(|ranges| valkey::count_slots(ranges))
                .sum::<i32>(),
            8
        );
    }

    #[test]
    fn split_slots_evenly_handles_zero_primaries_and_more_primaries_than_slots() {
        assert!(split_slots_evenly(vec![SlotsRange { start: 5, end: 6 }], 0).is_empty());

        let plans = split_slots_evenly(vec![SlotsRange { start: 5, end: 6 }], 3);

        assert_eq!(
            plans,
            vec![
                vec![SlotsRange { start: 5, end: 5 }],
                vec![SlotsRange { start: 6, end: 6 }],
                Vec::new(),
            ]
        );
    }
}
