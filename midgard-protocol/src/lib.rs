use std::collections::BTreeMap;

use midgard_storage::{MiddlewareDesiredState, MiddlewareInstance, MiddlewareInstanceStatus};
use prost_types::{ListValue, NullValue, Struct, Value as ProstValue, value::Kind};
use serde_json::{Map, Number, Value};
use uuid::Uuid;

pub mod operator {
    #![allow(clippy::large_enum_variant)]

    tonic::include_proto!("midgard.operator.v1");
}

pub const OPERATOR_PROTOCOL_VERSION: u32 = 1;

pub use operator::{
    CommandType, DesiredState, MiddlewareResource, MiddlewareStatus, OperatorRegistration,
    OperatorToServer, ServerCommand, ServerToOperator, operator_control_client,
    operator_control_server,
};

impl From<&MiddlewareDesiredState> for DesiredState {
    fn from(value: &MiddlewareDesiredState) -> Self {
        match value {
            MiddlewareDesiredState::Enabled => DesiredState::Enabled,
            MiddlewareDesiredState::Disabled => DesiredState::Disabled,
        }
    }
}

impl From<&MiddlewareInstanceStatus> for MiddlewareStatus {
    fn from(value: &MiddlewareInstanceStatus) -> Self {
        match value {
            MiddlewareInstanceStatus::Pending => MiddlewareStatus::Pending,
            MiddlewareInstanceStatus::Running => MiddlewareStatus::Running,
            MiddlewareInstanceStatus::Degraded => MiddlewareStatus::Degraded,
            MiddlewareInstanceStatus::Stopped => MiddlewareStatus::Stopped,
        }
    }
}

impl From<DesiredState> for MiddlewareDesiredState {
    fn from(value: DesiredState) -> Self {
        match value {
            DesiredState::Disabled => MiddlewareDesiredState::Disabled,
            DesiredState::UnknownDesiredState | DesiredState::Enabled => {
                MiddlewareDesiredState::Enabled
            }
        }
    }
}

impl From<MiddlewareStatus> for MiddlewareInstanceStatus {
    fn from(value: MiddlewareStatus) -> Self {
        match value {
            MiddlewareStatus::Running => MiddlewareInstanceStatus::Running,
            MiddlewareStatus::Degraded => MiddlewareInstanceStatus::Degraded,
            MiddlewareStatus::Stopped => MiddlewareInstanceStatus::Stopped,
            MiddlewareStatus::UnknownMiddlewareStatus | MiddlewareStatus::Pending => {
                MiddlewareInstanceStatus::Pending
            }
        }
    }
}

impl From<&MiddlewareInstance> for MiddlewareResource {
    fn from(instance: &MiddlewareInstance) -> Self {
        Self {
            id: instance.id.to_string(),
            workspace_id: instance.workspace_id.to_string(),
            kind: instance.kind.clone(),
            name: instance.name.clone(),
            namespace: instance.namespace.clone(),
            desired_state: DesiredState::from(&instance.desired_state) as i32,
            status: MiddlewareStatus::from(&instance.status) as i32,
            config: Some(json_to_struct(&instance.config)),
            archived_at: instance.archived_at.clone().unwrap_or_default(),
            created_at: instance.created_at.clone(),
            updated_at: instance.updated_at.clone(),
        }
    }
}

impl TryFrom<MiddlewareResource> for MiddlewareInstance {
    type Error = String;

    fn try_from(resource: MiddlewareResource) -> Result<Self, Self::Error> {
        let id = Uuid::parse_str(&resource.id)
            .map_err(|err| format!("invalid middleware resource id: {err}"))?;
        let workspace_id = Uuid::parse_str(&resource.workspace_id)
            .map_err(|err| format!("invalid middleware workspace id: {err}"))?;
        let desired_state = DesiredState::try_from(resource.desired_state)
            .unwrap_or(DesiredState::UnknownDesiredState)
            .into();
        let status = MiddlewareStatus::try_from(resource.status)
            .unwrap_or(MiddlewareStatus::UnknownMiddlewareStatus)
            .into();

        Ok(Self {
            id,
            workspace_id,
            kind: resource.kind,
            name: resource.name,
            namespace: resource.namespace,
            desired_state,
            status,
            config: resource
                .config
                .map(struct_to_json)
                .unwrap_or_else(|| Value::Object(Map::new())),
            archived_at: empty_string_to_none(resource.archived_at),
            created_at: resource.created_at,
            updated_at: resource.updated_at,
        })
    }
}

pub fn json_to_struct(value: &Value) -> Struct {
    match json_to_prost_value(value).kind {
        Some(Kind::StructValue(value)) => value,
        _ => Struct {
            fields: BTreeMap::new(),
        },
    }
}

pub fn struct_to_json(value: Struct) -> Value {
    prost_value_to_json(ProstValue {
        kind: Some(Kind::StructValue(value)),
    })
}

fn json_to_prost_value(value: &Value) -> ProstValue {
    let kind = match value {
        Value::Null => Kind::NullValue(NullValue::NullValue as i32),
        Value::Bool(value) => Kind::BoolValue(*value),
        Value::Number(value) => Kind::NumberValue(value.as_f64().unwrap_or_default()),
        Value::String(value) => Kind::StringValue(value.clone()),
        Value::Array(values) => Kind::ListValue(ListValue {
            values: values.iter().map(json_to_prost_value).collect(),
        }),
        Value::Object(values) => Kind::StructValue(Struct {
            fields: values
                .iter()
                .map(|(key, value)| (key.clone(), json_to_prost_value(value)))
                .collect(),
        }),
    };

    ProstValue { kind: Some(kind) }
}

fn prost_value_to_json(value: ProstValue) -> Value {
    match value.kind {
        Some(Kind::NullValue(_)) | None => Value::Null,
        Some(Kind::BoolValue(value)) => Value::Bool(value),
        Some(Kind::NumberValue(value)) => json_number_from_f64(value),
        Some(Kind::StringValue(value)) => Value::String(value),
        Some(Kind::ListValue(value)) => {
            Value::Array(value.values.into_iter().map(prost_value_to_json).collect())
        }
        Some(Kind::StructValue(value)) => Value::Object(
            value
                .fields
                .into_iter()
                .map(|(key, value)| (key, prost_value_to_json(value)))
                .collect(),
        ),
    }
}

fn empty_string_to_none(value: String) -> Option<String> {
    if value.is_empty() { None } else { Some(value) }
}

fn json_number_from_f64(value: f64) -> Value {
    if value.is_finite() && value.fract() == 0.0 {
        let integer = value as i64;
        if integer as f64 == value {
            return Value::Number(Number::from(integer));
        }
    }

    Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or(Value::Null)
}
