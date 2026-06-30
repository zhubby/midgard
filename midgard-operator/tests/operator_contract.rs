use std::time::Duration;

use k8s_openapi::api::core::v1::ConfigMap;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{Condition, ObjectMeta};
use midgard_core::{CapabilityDescriptor, RiskLevel};
use midgard_operator::conditions::{
    find_condition, remove_condition, remove_condition_if_reason, set_condition,
};
use midgard_operator::control::{capability_message, risk_level_label};
use midgard_operator::controller::sanitize_apply_patch;
use midgard_operator::finalizers::{add_finalizer, remove_finalizer};
use midgard_operator::lease::{LeaseDecision, decide_lease};
use midgard_operator::probe::optional_probe_addr;
use midgard_operator::traits::OperatorDefinition;
use midgard_protocol::operator::operator_to_server;
use serde_json::json;

#[test]
fn lease_decision_acquires_empty_holder() {
    assert_eq!(
        decide_lease("operator-a", None, None, 15, 100, 0),
        LeaseDecision::Acquire { transitions: 1 }
    );
}

#[test]
fn lease_decision_renews_same_holder() {
    assert_eq!(
        decide_lease("operator-a", Some("operator-a"), Some(95), 15, 100, 2),
        LeaseDecision::Renew
    );
}

#[test]
fn lease_decision_waits_for_active_other_holder() {
    assert_eq!(
        decide_lease("operator-a", Some("operator-b"), Some(95), 15, 100, 2),
        LeaseDecision::Wait {
            holder: "operator-b".to_string(),
        }
    );
}

#[test]
fn lease_decision_acquires_expired_other_holder() {
    assert_eq!(
        decide_lease("operator-a", Some("operator-b"), Some(80), 15, 100, 2),
        LeaseDecision::Acquire { transitions: 3 }
    );
}

#[test]
fn apply_patch_sanitizer_removes_status_and_server_owned_metadata() {
    let mut value = json!({
        "apiVersion": "v1",
        "kind": "Secret",
        "metadata": {
            "name": "example",
            "namespace": "default",
            "managedFields": [],
            "resourceVersion": "42",
            "uid": "abc",
            "creationTimestamp": "2026-06-11T00:00:00Z",
            "generation": 2
        },
        "status": { "ready": true },
        "data": {}
    });

    sanitize_apply_patch(&mut value);

    assert_eq!(value.get("status"), None);
    let metadata = value
        .get("metadata")
        .and_then(serde_json::Value::as_object)
        .expect("metadata should remain");
    assert_eq!(
        metadata.get("name").and_then(serde_json::Value::as_str),
        Some("example")
    );
    assert!(!metadata.contains_key("managedFields"));
    assert!(!metadata.contains_key("resourceVersion"));
    assert!(!metadata.contains_key("uid"));
    assert!(!metadata.contains_key("creationTimestamp"));
    assert!(!metadata.contains_key("generation"));
}

#[test]
fn condition_helpers_update_find_and_remove_conditions() {
    let mut conditions = Vec::<Condition>::new();

    set_condition(
        &mut conditions,
        7,
        "Ready",
        "Starting",
        "warming up",
        "False",
    );
    set_condition(&mut conditions, 8, "Ready", "Ready", "running", "True");
    set_condition(
        &mut conditions,
        8,
        "Progressing",
        "Complete",
        "done",
        "False",
    );

    assert_eq!(conditions.len(), 2);
    let ready = find_condition(&conditions, "Ready").expect("Ready condition");
    assert_eq!(ready.status, "True");
    assert_eq!(ready.reason, "Ready");
    assert_eq!(ready.observed_generation, Some(8));

    remove_condition_if_reason(&mut conditions, "Progressing", "Complete");
    assert!(find_condition(&conditions, "Progressing").is_none());

    remove_condition(&mut conditions, "Ready");
    assert!(conditions.is_empty());
}

#[test]
fn finalizer_helpers_are_idempotent() {
    let mut config_map = ConfigMap {
        metadata: ObjectMeta {
            name: Some("example".to_string()),
            namespace: Some("default".to_string()),
            finalizers: Some(vec!["midgard.io/cleanup".to_string()]),
            ..ObjectMeta::default()
        },
        ..ConfigMap::default()
    };

    let finalizers = add_finalizer(&config_map, "midgard.io/cleanup");
    assert_eq!(finalizers, vec!["midgard.io/cleanup".to_string()]);

    config_map.metadata.finalizers = Some(finalizers);
    let finalizers = add_finalizer(&config_map, "midgard.io/archive");
    assert_eq!(
        finalizers,
        vec![
            "midgard.io/cleanup".to_string(),
            "midgard.io/archive".to_string(),
        ]
    );

    config_map.metadata.finalizers = Some(finalizers);
    assert_eq!(
        remove_finalizer(&config_map, "midgard.io/cleanup"),
        vec!["midgard.io/archive".to_string()]
    );
}

#[test]
fn probe_address_parsing_accepts_colon_port_and_disables_empty_values() {
    assert_eq!(
        optional_probe_addr(Some(":8081"))
            .unwrap()
            .unwrap()
            .to_string(),
        "0.0.0.0:8081"
    );
    assert!(optional_probe_addr(Some("")).unwrap().is_none());
    assert!(optional_probe_addr(Some("0")).unwrap().is_none());
    assert!(optional_probe_addr(None).unwrap().is_none());
    assert!(optional_probe_addr(Some("not an address")).is_err());
}

#[test]
fn capability_builder_converts_typed_risk_levels_to_protocol_labels() {
    let message = capability_message(&TestDefinition);
    let Some(operator_to_server::Payload::CapabilityReport(report)) = message.payload else {
        panic!("expected capability report");
    };

    assert_eq!(report.operator_id, "operator-a");
    assert_eq!(report.capabilities[0].id, "cache.create");
    assert_eq!(report.capabilities[0].risk_level, "high");
    assert_eq!(risk_level_label(&RiskLevel::Critical), "critical");
}

struct TestDefinition;

impl OperatorDefinition for TestDefinition {
    fn operator_id(&self) -> String {
        "operator-a".to_string()
    }

    fn workspace_id(&self) -> &str {
        "workspace-a"
    }

    fn middleware_kind(&self) -> &str {
        "cache"
    }

    fn display_name(&self) -> &str {
        "Cache Operator"
    }

    fn supported_operations(&self) -> Vec<String> {
        vec!["create".to_string()]
    }

    fn capabilities(&self) -> Vec<CapabilityDescriptor> {
        vec![CapabilityDescriptor::new(
            "cache.create",
            "Create cache",
            RiskLevel::High,
        )]
    }

    fn heartbeat_interval(&self) -> Duration {
        Duration::from_secs(10)
    }
}
