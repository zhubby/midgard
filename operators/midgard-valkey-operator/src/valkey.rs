use std::collections::BTreeMap;

use redis::{FromRedisValue, Value};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::{Error, Result};

pub const TOTAL_SLOTS: i32 = 16_384;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlotsRange {
    pub start: i32,
    pub end: i32,
}

impl std::fmt::Display for SlotsRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.start == self.end {
            write!(f, "{}", self.start)
        } else {
            write!(f, "{}-{}", self.start, self.end)
        }
    }
}

pub fn format_slots_ranges(ranges: &[SlotsRange]) -> String {
    ranges
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

#[derive(Debug, Clone)]
pub struct ValkeyClient {
    address: String,
    port: u16,
    username: Option<String>,
    password: Option<String>,
    tls: bool,
}

impl ValkeyClient {
    pub fn new(
        address: impl Into<String>,
        port: u16,
        username: Option<String>,
        password: Option<String>,
        tls: bool,
    ) -> Self {
        Self {
            address: address.into(),
            port,
            username,
            password,
            tls,
        }
    }

    fn uri(&self) -> String {
        let scheme = if self.tls { "rediss" } else { "redis" };
        let auth = match (&self.username, &self.password) {
            (Some(username), Some(password)) if !username.is_empty() => {
                format!("{username}:{password}@")
            }
            (_, Some(password)) if !password.is_empty() => format!(":{password}@"),
            _ => String::new(),
        };
        if self.tls {
            format!("{scheme}://{auth}{}:{}/?insecure", self.address, self.port)
        } else {
            format!("{scheme}://{auth}{}:{}/", self.address, self.port)
        }
    }

    async fn connection(&self) -> redis::RedisResult<redis::aio::MultiplexedConnection> {
        let client = redis::Client::open(self.uri())?;
        client.get_multiplexed_async_connection().await
    }

    pub async fn query<T>(&self, args: &[&str]) -> redis::RedisResult<T>
    where
        T: FromRedisValue,
    {
        let mut con = self.connection().await?;
        let mut cmd = redis::cmd(args[0]);
        for arg in &args[1..] {
            cmd.arg(arg);
        }
        cmd.query_async(&mut con).await
    }

    pub async fn query_owned<T>(&self, args: &[String]) -> redis::RedisResult<T>
    where
        T: FromRedisValue,
    {
        let mut con = self.connection().await?;
        let mut cmd = redis::cmd(&args[0]);
        for arg in &args[1..] {
            cmd.arg(arg);
        }
        cmd.query_async(&mut con).await
    }
}

#[derive(Debug, Clone)]
pub struct NodeState {
    pub client: ValkeyClient,
    pub address: String,
    pub port: u16,
    pub id: String,
    pub flags: Vec<String>,
    pub shard_id: String,
    pub info: BTreeMap<String, String>,
    pub cluster_info: BTreeMap<String, String>,
    pub cluster_nodes: String,
}

#[derive(Debug, Clone, Default)]
pub struct ShardState {
    pub id: String,
    pub primary_id: String,
    pub slots: Vec<SlotsRange>,
    pub nodes: Vec<NodeState>,
}

#[derive(Debug, Clone, Default)]
pub struct ClusterState {
    pub shards: Vec<ShardState>,
    pub pending_nodes: Vec<NodeState>,
}

impl ClusterState {
    pub fn unassigned_slots(&self) -> Vec<SlotsRange> {
        let mut remaining = vec![SlotsRange {
            start: 0,
            end: TOTAL_SLOTS - 1,
        }];
        for shard in &self.shards {
            for slot in &shard.slots {
                let mut next = Vec::new();
                for base in remaining {
                    next.extend(subtract_slots_range(base, *slot));
                }
                remaining = next;
            }
        }
        remaining
    }

    pub fn find_shard_for_address(&self, address: &str) -> Option<&ShardState> {
        self.shards
            .iter()
            .find(|shard| shard.nodes.iter().any(|node| node.address == address))
    }

    pub fn has_replica_of(&self, node_id: &str) -> bool {
        self.shards.iter().any(|shard| {
            shard
                .nodes
                .iter()
                .any(|node| node.primary_id_from_self() == node_id)
        })
    }
}

impl ShardState {
    pub fn primary(&self) -> Option<&NodeState> {
        self.nodes.iter().find(|node| node.id == self.primary_id)
    }

    pub fn synced_replicas(&self) -> Vec<NodeState> {
        self.nodes
            .iter()
            .filter(|node| {
                node.id != self.primary_id
                    && !node
                        .flags
                        .iter()
                        .any(|flag| flag == "fail" || flag == "pfail")
                    && node.info.get("master_link_status").map(String::as_str) == Some("up")
            })
            .cloned()
            .collect()
    }
}

impl NodeState {
    pub fn is_primary(&self) -> bool {
        self.flags.iter().any(|flag| flag == "master")
    }

    pub fn current_epoch(&self) -> i64 {
        self.cluster_info
            .get("cluster_current_epoch")
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0)
    }

    pub fn is_isolated(&self) -> bool {
        self.cluster_info
            .get("cluster_known_nodes")
            .and_then(|v| v.parse::<i32>().ok())
            .is_some_and(|count| count <= 1)
    }

    pub fn is_replication_in_sync(&self) -> bool {
        self.is_primary() || self.info.get("master_link_status").map(String::as_str) == Some("up")
    }

    pub fn slots(&self) -> Vec<String> {
        for line in self.cluster_nodes.lines() {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 8 {
                continue;
            }
            let flags = fields[2].split(',').collect::<Vec<_>>();
            if flags.contains(&"myself") && flags.contains(&"master") {
                return fields[8..].iter().map(|s| (*s).to_string()).collect();
            }
        }
        Vec::new()
    }

    pub fn primary_id_from_self(&self) -> String {
        for line in self.cluster_nodes.lines() {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() >= 8 && fields[2].contains("myself") {
                return fields[3].to_string();
            }
        }
        String::new()
    }

    pub fn failing_nodes(&self) -> Vec<NodeState> {
        let mut nodes = Vec::new();
        for line in self.cluster_nodes.lines() {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 8 {
                continue;
            }
            let flags = fields[2].split(',').collect::<Vec<_>>();
            if flags.contains(&"myself") || !(flags.contains(&"fail") || flags.contains(&"noaddr"))
            {
                continue;
            }
            if let Some(address) = fields[1].rsplit_once(':').map(|(host, _)| host.to_string()) {
                nodes.push(NodeState {
                    client: self.client.clone(),
                    address,
                    port: self.port,
                    id: fields[0].to_string(),
                    flags: flags.into_iter().map(str::to_string).collect(),
                    shard_id: String::new(),
                    info: BTreeMap::new(),
                    cluster_info: BTreeMap::new(),
                    cluster_nodes: String::new(),
                });
            }
        }
        nodes
    }
}

pub async fn get_cluster_state(
    addresses: &[String],
    port: u16,
    username: Option<String>,
    password: Option<String>,
    tls: bool,
) -> ClusterState {
    let mut state = ClusterState::default();
    for address in addresses {
        let Some(node) =
            get_node_state(address, port, username.clone(), password.clone(), tls).await
        else {
            continue;
        };
        if node.is_primary() && node.slots().is_empty() {
            state.pending_nodes.push(node);
            continue;
        }
        let shard_id = node.shard_id.clone();
        if let Some(shard) = state.shards.iter_mut().find(|shard| shard.id == shard_id) {
            if node.is_primary() {
                shard.slots = parse_slots_ranges(&node.slots()).unwrap_or_default();
                shard.primary_id = node.id.clone();
            }
            shard.nodes.push(node);
        } else {
            let mut shard = ShardState {
                id: shard_id,
                nodes: vec![node.clone()],
                ..ShardState::default()
            };
            if node.is_primary() {
                shard.slots = parse_slots_ranges(&node.slots()).unwrap_or_default();
                shard.primary_id = node.id.clone();
            }
            state.shards.push(shard);
        }
    }
    state
}

async fn get_node_state(
    address: &str,
    port: u16,
    username: Option<String>,
    password: Option<String>,
    tls: bool,
) -> Option<NodeState> {
    let client = ValkeyClient::new(address, port, username, password, tls);
    let id = match client.query::<String>(&["CLUSTER", "MYID"]).await {
        Ok(id) => id,
        Err(err) => {
            warn!(%address, %err, "failed to query CLUSTER MYID");
            return None;
        }
    };
    let shard_id = match client.query::<String>(&["CLUSTER", "MYSHARDID"]).await {
        Ok(id) => id,
        Err(err) => {
            warn!(%address, %err, "failed to query CLUSTER MYSHARDID");
            String::new()
        }
    };
    let info = client
        .query::<String>(&["INFO"])
        .await
        .map(|s| info_string_to_map(&s))
        .unwrap_or_default();
    let cluster_info = client
        .query::<String>(&["CLUSTER", "INFO"])
        .await
        .map(|s| info_string_to_map(&s))
        .unwrap_or_default();
    let cluster_nodes = client
        .query::<String>(&["CLUSTER", "NODES"])
        .await
        .unwrap_or_default()
        .trim_start_matches("txt:")
        .to_string();

    let flags = cluster_nodes
        .lines()
        .find_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() >= 8 && fields[2].contains("myself") {
                Some(fields[2].split(',').map(str::to_string).collect())
            } else {
                None
            }
        })
        .unwrap_or_default();

    Some(NodeState {
        client,
        address: address.to_string(),
        port,
        id,
        flags,
        shard_id,
        info,
        cluster_info,
        cluster_nodes,
    })
}

pub fn info_string_to_map(input: &str) -> BTreeMap<String, String> {
    input
        .trim_start_matches("txt:")
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            line.split_once(':')
                .map(|(key, value)| (key.to_string(), value.to_string()))
        })
        .collect()
}

pub fn parse_slots_ranges(parts: &[String]) -> Result<Vec<SlotsRange>> {
    parts
        .iter()
        .filter(|part| !part.starts_with('['))
        .map(|part| parse_slots_range(part))
        .collect()
}

pub fn parse_slots_range(part: &str) -> Result<SlotsRange> {
    if let Some((start, end)) = part.split_once('-') {
        let start = start
            .parse::<i32>()
            .map_err(|err| Error::InvalidState(err.to_string()))?;
        let end = end
            .parse::<i32>()
            .map_err(|err| Error::InvalidState(err.to_string()))?;
        if start > end {
            return Err(Error::InvalidState(format!("invalid slot range {part}")));
        }
        return Ok(SlotsRange { start, end });
    }
    let slot = part
        .parse::<i32>()
        .map_err(|err| Error::InvalidState(err.to_string()))?;
    Ok(SlotsRange {
        start: slot,
        end: slot,
    })
}

pub fn subtract_slots_range(base: SlotsRange, remove: SlotsRange) -> Vec<SlotsRange> {
    if remove.end < base.start || remove.start > base.end {
        return vec![base];
    }
    let mut result = Vec::new();
    if remove.start > base.start {
        result.push(SlotsRange {
            start: base.start,
            end: remove.start - 1,
        });
    }
    if remove.end < base.end {
        result.push(SlotsRange {
            start: remove.end + 1,
            end: base.end,
        });
    }
    result
}

#[derive(Debug, Clone)]
pub struct SlotMove {
    pub src: NodeState,
    pub dst: NodeState,
    pub slots: Vec<i32>,
}

#[derive(Debug)]
struct PrimarySlots {
    node: NodeState,
    ranges: Vec<SlotsRange>,
    num_slots: i32,
    target_num_slots: i32,
}

pub fn plan_rebalance_move(
    shards: &[ShardState],
    expected_shards: i32,
    max_slots: i32,
) -> Result<Option<SlotMove>> {
    if expected_shards <= 0 || max_slots <= 0 || shards.len() != expected_shards as usize {
        return Ok(None);
    }
    let mut allocations = Vec::new();
    for shard in shards {
        let primary = shard.primary().ok_or_else(|| {
            Error::InvalidState(format!("primary missing for shard {}", shard.id))
        })?;
        let num_slots = count_slots(&shard.slots);
        allocations.push(PrimarySlots {
            node: primary.clone(),
            ranges: shard.slots.clone(),
            num_slots,
            target_num_slots: 0,
        });
    }
    allocations.sort_by(|a, b| a.node.address.cmp(&b.node.address));

    let per_shard = TOTAL_SLOTS / expected_shards;
    let remainder = TOTAL_SLOTS % expected_shards;
    let mut src = None;
    let mut dst = None;
    for (idx, alloc) in allocations.iter_mut().enumerate() {
        alloc.target_num_slots = per_shard + i32::from((idx as i32) < remainder);
        if src.is_none() && alloc.num_slots - alloc.target_num_slots > 1 {
            src = Some(idx);
        }
        if dst.is_none() && alloc.target_num_slots - alloc.num_slots > 1 {
            dst = Some(idx);
        }
    }
    let (Some(src), Some(dst)) = (src, dst) else {
        return Ok(None);
    };
    let src_alloc = &allocations[src];
    let dst_alloc = &allocations[dst];
    let num_to_move = std::cmp::min(
        std::cmp::min(
            src_alloc.num_slots - src_alloc.target_num_slots,
            dst_alloc.target_num_slots - dst_alloc.num_slots,
        ),
        max_slots,
    );
    Ok(Some(SlotMove {
        src: src_alloc.node.clone(),
        dst: dst_alloc.node.clone(),
        slots: take_slots_from_ranges(&src_alloc.ranges, num_to_move),
    }))
}

pub fn plan_drain_move(
    src: &ShardState,
    dsts: &[ShardState],
    max_slots: i32,
) -> Result<Option<SlotMove>> {
    if max_slots <= 0 || dsts.is_empty() {
        return Ok(None);
    }
    let src_count = count_slots(&src.slots);
    if src_count == 0 {
        return Ok(None);
    }
    let src_primary = src.primary().ok_or_else(|| {
        Error::InvalidState(format!("primary missing for draining shard {}", src.id))
    })?;
    let dst_primary = dsts.iter().find_map(ShardState::primary).ok_or_else(|| {
        Error::InvalidState(format!(
            "no valid destination for draining shard {}",
            src.id
        ))
    })?;

    Ok(Some(SlotMove {
        src: src_primary.clone(),
        dst: dst_primary.clone(),
        slots: take_slots_from_ranges(&src.slots, std::cmp::min(src_count, max_slots)),
    }))
}

pub fn count_slots(ranges: &[SlotsRange]) -> i32 {
    ranges.iter().map(|slot| slot.end - slot.start + 1).sum()
}

pub fn slots_to_ranges(slots: &[i32]) -> Vec<SlotsRange> {
    if slots.is_empty() {
        return Vec::new();
    }
    let mut ordered = slots.to_vec();
    ordered.sort_unstable();
    let mut ranges = Vec::new();
    let mut start = ordered[0];
    let mut prev = ordered[0];
    for slot in ordered.into_iter().skip(1) {
        if slot == prev + 1 {
            prev = slot;
            continue;
        }
        ranges.push(SlotsRange { start, end: prev });
        start = slot;
        prev = slot;
    }
    ranges.push(SlotsRange { start, end: prev });
    ranges
}

fn take_slots_from_ranges(ranges: &[SlotsRange], slots_to_move: i32) -> Vec<i32> {
    let mut out = Vec::new();
    if slots_to_move <= 0 {
        return out;
    }
    for slot_range in ranges {
        let mut slot = slot_range.start;
        while slot <= slot_range.end && out.len() < slots_to_move as usize {
            out.push(slot);
            slot += 1;
        }
        if out.len() == slots_to_move as usize {
            break;
        }
    }
    out
}

pub async fn slot_migration_in_progress(src: &NodeState) -> Result<bool> {
    let value = src
        .client
        .query::<Value>(&["CLUSTER", "GETSLOTMIGRATIONS"])
        .await
        .map_err(wrap_unsupported_err)?;
    let Value::Array(migrations) = value else {
        return Ok(false);
    };
    for migration in migrations {
        let Some(state) = migration_state(&migration) else {
            debug!(address = %src.address, "unable to parse migration state; treating as in progress");
            return Ok(true);
        };
        if !is_slot_migration_terminal(&state) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub async fn migrate_slots_atomic(
    src: &NodeState,
    dst: &NodeState,
    ranges: &[SlotsRange],
) -> Result<()> {
    let mut args = vec!["CLUSTER".to_string(), "MIGRATESLOTS".to_string()];
    for range in ranges {
        args.extend([
            "SLOTSRANGE".to_string(),
            range.start.to_string(),
            range.end.to_string(),
            "NODE".to_string(),
            dst.id.clone(),
        ]);
    }
    src.client
        .query_owned::<String>(&args)
        .await
        .map(|_| ())
        .map_err(wrap_unsupported_err)
        .map_err(Into::into)
}

pub fn is_slots_not_served_by_node(err: &Error) -> bool {
    err.to_string()
        .to_lowercase()
        .contains("slots are not served by this node")
}

fn wrap_unsupported_err(err: redis::RedisError) -> redis::RedisError {
    let msg = err.to_string().to_lowercase();
    if msg.contains("unknown command")
        || msg.contains("unknown subcommand")
        || msg.contains("wrong number of arguments")
    {
        redis::RedisError::from((
            redis::ErrorKind::Extension,
            "Valkey command unsupported",
            format!(
                "{err}; please upgrade to Valkey 9.0.0 or later for atomic slot migration support"
            ),
        ))
    } else {
        err
    }
}

fn migration_state(value: &Value) -> Option<String> {
    match value {
        Value::Map(entries) => entries.iter().find_map(|(key, value)| {
            if value_to_string(key).as_deref() == Some("state") {
                value_to_string(value).map(|s| s.to_lowercase())
            } else {
                None
            }
        }),
        Value::Array(items) => items.chunks(2).find_map(|chunk| {
            if chunk.len() == 2 && value_to_string(&chunk[0]).as_deref() == Some("state") {
                value_to_string(&chunk[1]).map(|s| s.to_lowercase())
            } else {
                None
            }
        }),
        _ => None,
    }
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::SimpleString(s) => Some(s.clone()),
        Value::BulkString(bytes) => String::from_utf8(bytes.clone()).ok(),
        Value::VerbatimString { text, .. } => Some(text.clone()),
        Value::Int(i) => Some(i.to_string()),
        Value::Okay => Some("OK".to_string()),
        _ => None,
    }
}

fn is_slot_migration_terminal(state: &str) -> bool {
    matches!(state, "success" | "failed" | "canceled" | "cancelled")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(address: &str, id: &str, flags: &[&str]) -> NodeState {
        NodeState {
            client: ValkeyClient::new(address, 6379, None, None, false),
            address: address.to_string(),
            port: 6379,
            id: id.to_string(),
            flags: flags.iter().map(|flag| (*flag).to_string()).collect(),
            shard_id: format!("shard-{id}"),
            info: BTreeMap::new(),
            cluster_info: BTreeMap::new(),
            cluster_nodes: String::new(),
        }
    }

    fn primary_shard(address: &str, node_id: &str, slots: Vec<SlotsRange>) -> ShardState {
        ShardState {
            id: format!("shard-{node_id}"),
            primary_id: node_id.to_string(),
            slots,
            nodes: vec![node(address, node_id, &["master"])],
        }
    }

    #[test]
    fn parse_slots_range_accepts_ranges_and_single_slots() {
        assert_eq!(
            parse_slots_range("0-16383").unwrap(),
            SlotsRange {
                start: 0,
                end: 16383
            }
        );
        assert_eq!(
            parse_slots_range("5").unwrap(),
            SlotsRange { start: 5, end: 5 }
        );
        assert!(parse_slots_range("10-5").is_err());
    }

    #[test]
    fn parse_slots_ranges_skips_migrating_and_importing_entries() {
        let parts = vec![
            "0-5460".to_string(),
            "[5461->-abc123]".to_string(),
            "[5462-<-def456]".to_string(),
        ];

        assert_eq!(
            parse_slots_ranges(&parts).unwrap(),
            vec![SlotsRange {
                start: 0,
                end: 5460
            }]
        );
        assert!(
            parse_slots_ranges(&["[5461->-abc123]".to_string()])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn subtract_slots_range_handles_prefix_suffix_middle_and_full_overlap() {
        assert_eq!(
            subtract_slots_range(
                SlotsRange {
                    start: 0,
                    end: 16383
                },
                SlotsRange {
                    start: 10,
                    end: 16380
                }
            ),
            vec![
                SlotsRange { start: 0, end: 9 },
                SlotsRange {
                    start: 16381,
                    end: 16383
                }
            ]
        );
        assert_eq!(
            subtract_slots_range(
                SlotsRange { start: 0, end: 10 },
                SlotsRange { start: 5, end: 10 }
            ),
            vec![SlotsRange { start: 0, end: 4 }]
        );
        assert_eq!(
            subtract_slots_range(
                SlotsRange { start: 0, end: 10 },
                SlotsRange { start: 0, end: 9 }
            ),
            vec![SlotsRange { start: 10, end: 10 }]
        );
        assert!(
            subtract_slots_range(
                SlotsRange { start: 0, end: 10 },
                SlotsRange { start: 0, end: 10 }
            )
            .is_empty()
        );
    }

    #[test]
    fn unassigned_slots_returns_gaps_across_shards() {
        let state = ClusterState {
            shards: vec![
                ShardState {
                    slots: vec![
                        SlotsRange {
                            start: 100,
                            end: 200,
                        },
                        SlotsRange {
                            start: 300,
                            end: 400,
                        },
                    ],
                    ..ShardState::default()
                },
                ShardState {
                    slots: vec![SlotsRange {
                        start: 700,
                        end: 800,
                    }],
                    ..ShardState::default()
                },
                ShardState {
                    slots: vec![SlotsRange {
                        start: 500,
                        end: 600,
                    }],
                    ..ShardState::default()
                },
            ],
            pending_nodes: Vec::new(),
        };

        assert_eq!(
            state.unassigned_slots(),
            vec![
                SlotsRange { start: 0, end: 99 },
                SlotsRange {
                    start: 201,
                    end: 299
                },
                SlotsRange {
                    start: 401,
                    end: 499
                },
                SlotsRange {
                    start: 601,
                    end: 699
                },
                SlotsRange {
                    start: 801,
                    end: 16383
                }
            ]
        );
    }

    #[test]
    fn find_shard_for_address_matches_primary_or_replica() {
        let mut replica = node("10.0.0.2", "node-2", &["slave"]);
        replica.shard_id = "shard-1".to_string();
        let state = ClusterState {
            shards: vec![
                ShardState {
                    id: "shard-1".to_string(),
                    primary_id: "node-1".to_string(),
                    nodes: vec![node("10.0.0.1", "node-1", &["master", "myself"]), replica],
                    ..ShardState::default()
                },
                primary_shard("10.0.0.3", "node-3", Vec::new()),
            ],
            pending_nodes: Vec::new(),
        };

        assert_eq!(
            state.find_shard_for_address("10.0.0.1").unwrap().id,
            "shard-1"
        );
        assert_eq!(
            state.find_shard_for_address("10.0.0.2").unwrap().id,
            "shard-1"
        );
        assert!(state.find_shard_for_address("10.0.0.99").is_none());
    }

    #[test]
    fn synced_replicas_filters_unsynced_and_failing_nodes() {
        let primary = node("10.0.0.1", "primary-id", &["myself", "master"]);
        let mut synced = node("10.0.0.2", "replica-1-id", &["slave"]);
        synced
            .info
            .insert("master_link_status".to_string(), "up".to_string());
        let mut unsynced = node("10.0.0.3", "replica-2-id", &["slave"]);
        unsynced
            .info
            .insert("master_link_status".to_string(), "down".to_string());
        let mut failing = node("10.0.0.4", "replica-3-id", &["slave", "fail"]);
        failing
            .info
            .insert("master_link_status".to_string(), "up".to_string());
        let mut pfailing = node("10.0.0.5", "replica-4-id", &["slave", "pfail"]);
        pfailing
            .info
            .insert("master_link_status".to_string(), "up".to_string());
        let shard = ShardState {
            id: "shard-0".to_string(),
            primary_id: "primary-id".to_string(),
            slots: vec![SlotsRange {
                start: 0,
                end: 5461,
            }],
            nodes: vec![primary, synced, unsynced, failing, pfailing],
        };

        let replicas = shard.synced_replicas();

        assert_eq!(replicas.len(), 1);
        assert_eq!(replicas[0].id, "replica-1-id");
    }

    #[test]
    fn plan_rebalance_move_moves_batch_from_overfull_to_empty_shard() {
        let shards = vec![
            primary_shard(
                "10.0.0.1",
                "node-1",
                vec![SlotsRange {
                    start: 0,
                    end: 8191,
                }],
            ),
            primary_shard(
                "10.0.0.2",
                "node-2",
                vec![SlotsRange {
                    start: 8192,
                    end: 16383,
                }],
            ),
            primary_shard("10.0.0.3", "node-3", Vec::new()),
        ];

        let move_plan = plan_rebalance_move(&shards, 3, 20).unwrap().unwrap();

        assert_eq!(move_plan.src.address, "10.0.0.1");
        assert_eq!(move_plan.dst.address, "10.0.0.3");
        assert_eq!(move_plan.slots, (0..20).collect::<Vec<_>>());
    }

    #[test]
    fn plan_rebalance_move_returns_none_when_balanced_or_mismatch_or_zero_batch() {
        let balanced = vec![
            primary_shard(
                "10.0.0.1",
                "node-1",
                vec![SlotsRange {
                    start: 0,
                    end: 8191,
                }],
            ),
            primary_shard(
                "10.0.0.2",
                "node-2",
                vec![SlotsRange {
                    start: 8192,
                    end: 16383,
                }],
            ),
        ];

        assert!(plan_rebalance_move(&balanced, 2, 10).unwrap().is_none());
        assert!(plan_rebalance_move(&balanced, 3, 10).unwrap().is_none());
        assert!(plan_rebalance_move(&balanced, 2, 0).unwrap().is_none());
    }

    #[test]
    fn plan_drain_move_picks_first_valid_destination_and_limits_batch_size() {
        let src = primary_shard(
            "10.0.0.3",
            "node-3",
            vec![SlotsRange {
                start: 10923,
                end: 16383,
            }],
        );
        let dsts = vec![
            primary_shard(
                "10.0.0.1",
                "node-1",
                vec![SlotsRange {
                    start: 0,
                    end: 5460,
                }],
            ),
            primary_shard(
                "10.0.0.2",
                "node-2",
                vec![SlotsRange {
                    start: 5461,
                    end: 10922,
                }],
            ),
        ];

        let move_plan = plan_drain_move(&src, &dsts, 100).unwrap().unwrap();

        assert_eq!(move_plan.src.address, "10.0.0.3");
        assert_eq!(move_plan.dst.address, "10.0.0.1");
        assert_eq!(move_plan.slots.len(), 100);
        assert_eq!(move_plan.slots[0], 10923);
    }

    #[test]
    fn plan_drain_move_returns_none_for_empty_source_or_zero_batch() {
        let src = primary_shard("10.0.0.3", "node-3", Vec::new());
        let dsts = vec![primary_shard(
            "10.0.0.1",
            "node-1",
            vec![SlotsRange {
                start: 0,
                end: 16383,
            }],
        )];

        assert!(plan_drain_move(&src, &dsts, 100).unwrap().is_none());
        assert!(plan_drain_move(&src, &dsts, 0).unwrap().is_none());
    }
}
