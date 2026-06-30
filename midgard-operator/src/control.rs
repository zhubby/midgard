use std::time::{Duration, SystemTime, UNIX_EPOCH};

use midgard_core::{CapabilityDescriptor, RiskLevel};
use midgard_protocol::OPERATOR_PROTOCOL_VERSION;
use midgard_protocol::operator::{
    CapabilityReport, OperatorCapability, OperatorHeartbeat, OperatorRegistration,
    OperatorToServer, operator_to_server,
};
use uuid::Uuid;

use crate::traits::OperatorDefinition;

pub fn registration_message(definition: &impl OperatorDefinition) -> OperatorToServer {
    OperatorToServer {
        request_id: Uuid::new_v4().to_string(),
        payload: Some(operator_to_server::Payload::Registration(
            OperatorRegistration {
                protocol_version: OPERATOR_PROTOCOL_VERSION,
                operator_id: definition.operator_id(),
                workspace_id: definition.workspace_id().to_string(),
                middleware_kind: definition.middleware_kind().to_string(),
                display_name: definition.display_name().to_string(),
                supported_operations: definition.supported_operations(),
            },
        )),
    }
}

pub fn capability_message(definition: &impl OperatorDefinition) -> OperatorToServer {
    OperatorToServer {
        request_id: Uuid::new_v4().to_string(),
        payload: Some(operator_to_server::Payload::CapabilityReport(
            CapabilityReport {
                operator_id: definition.operator_id(),
                capabilities: definition
                    .capabilities()
                    .iter()
                    .map(operator_capability_from_descriptor)
                    .collect(),
            },
        )),
    }
}

pub fn heartbeat_message(definition: &impl OperatorDefinition) -> OperatorToServer {
    OperatorToServer {
        request_id: Uuid::new_v4().to_string(),
        payload: Some(operator_to_server::Payload::Heartbeat(OperatorHeartbeat {
            operator_id: definition.operator_id(),
            observed_at_unix_ms: unix_time_millis(),
        })),
    }
}

pub fn operator_capability_from_descriptor(
    descriptor: &CapabilityDescriptor,
) -> OperatorCapability {
    OperatorCapability {
        id: descriptor.id.clone(),
        name: descriptor.name.clone(),
        risk_level: risk_level_label(&descriptor.risk_level).to_string(),
    }
}

pub fn risk_level_label(risk_level: &RiskLevel) -> &'static str {
    match risk_level {
        RiskLevel::Low => "low",
        RiskLevel::Medium => "medium",
        RiskLevel::High => "high",
        RiskLevel::Critical => "critical",
    }
}

fn unix_time_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as i64
}
