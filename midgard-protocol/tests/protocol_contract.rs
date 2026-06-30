use midgard_protocol::{
    DesiredState, MiddlewareResource, MiddlewareStatus, OPERATOR_PROTOCOL_VERSION, json_to_struct,
    struct_to_json,
};
use midgard_storage::{MiddlewareDesiredState, MiddlewareInstance, MiddlewareInstanceStatus};
use serde_json::json;
use uuid::Uuid;

#[test]
fn protocol_version_is_stable_for_v1() {
    assert_eq!(OPERATOR_PROTOCOL_VERSION, 1);
}

#[test]
fn enum_conversions_match_storage_values() {
    assert_eq!(
        MiddlewareDesiredState::from(DesiredState::Enabled),
        MiddlewareDesiredState::Enabled
    );
    assert_eq!(
        MiddlewareDesiredState::from(DesiredState::Disabled),
        MiddlewareDesiredState::Disabled
    );
    assert_eq!(
        MiddlewareInstanceStatus::from(MiddlewareStatus::Running),
        MiddlewareInstanceStatus::Running
    );
    assert_eq!(
        MiddlewareInstanceStatus::from(MiddlewareStatus::Stopped),
        MiddlewareInstanceStatus::Stopped
    );
}

#[test]
fn json_config_round_trips_through_protobuf_struct() {
    let config = json!({
        "replicas": 3,
        "memory": "512Mi",
        "features": ["persistence", "tls"],
        "limits": {
            "cpu": 1.5,
            "eviction": null
        },
        "enabled": true
    });

    let round_trip = struct_to_json(json_to_struct(&config));

    assert_eq!(round_trip, config);
}

#[test]
fn middleware_resource_round_trips_storage_instance() {
    let instance = MiddlewareInstance {
        id: Uuid::new_v4(),
        workspace_id: Uuid::new_v4(),
        kind: "redis".to_string(),
        name: "cache".to_string(),
        namespace: "data".to_string(),
        desired_state: MiddlewareDesiredState::Enabled,
        status: MiddlewareInstanceStatus::Running,
        config: json!({"memory": "512Mi"}),
        archived_at: None,
        created_at: "2026-06-30T00:00:00Z".to_string(),
        updated_at: "2026-06-30T00:01:00Z".to_string(),
    };

    let resource = MiddlewareResource::from(&instance);
    let decoded = MiddlewareInstance::try_from(resource).unwrap();

    assert_eq!(decoded, instance);
}
