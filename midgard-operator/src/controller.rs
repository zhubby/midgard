use std::collections::BTreeMap;
use std::fmt::Debug;

use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use kube::Client;
use kube::api::{Api, Patch, PatchParams, Resource};
use kube::core::NamespaceResourceScope;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::OperatorResult;

pub const DEFAULT_FIELD_MANAGER: &str = "midgard-operator";

#[derive(Clone)]
pub struct OperatorContext {
    pub client: Client,
    pub watch_namespaces: Vec<String>,
}

pub fn root_api<K>(client: Client, namespaces: &[String]) -> Api<K>
where
    K: Resource<DynamicType = (), Scope = NamespaceResourceScope>,
{
    if namespaces.len() == 1 {
        Api::namespaced(client, &namespaces[0])
    } else {
        Api::all(client)
    }
}

pub fn owner_reference<K>(owner: &K) -> Option<OwnerReference>
where
    K: Resource<DynamicType = ()>,
{
    owner.controller_owner_ref(&())
}

pub fn object_meta(
    name: impl Into<String>,
    namespace: impl Into<String>,
    labels: BTreeMap<String, String>,
    annotations: BTreeMap<String, String>,
    owner: Option<OwnerReference>,
) -> ObjectMeta {
    ObjectMeta {
        name: Some(name.into()),
        namespace: Some(namespace.into()),
        labels: (!labels.is_empty()).then_some(labels),
        annotations: (!annotations.is_empty()).then_some(annotations),
        owner_references: owner.map(|owner| vec![owner]),
        ..ObjectMeta::default()
    }
}

pub async fn apply<K>(api: &Api<K>, name: &str, obj: &K) -> OperatorResult<K>
where
    K: Clone + Debug + DeserializeOwned + Serialize,
{
    apply_with_manager(api, name, obj, DEFAULT_FIELD_MANAGER).await
}

pub async fn apply_with_manager<K>(
    api: &Api<K>,
    name: &str,
    obj: &K,
    field_manager: &str,
) -> OperatorResult<K>
where
    K: Clone + Debug + DeserializeOwned + Serialize,
{
    let pp = PatchParams::apply(field_manager).force();
    let mut patch = serde_json::to_value(obj)?;
    sanitize_apply_patch(&mut patch);
    Ok(api.patch(name, &pp, &Patch::Apply(&patch)).await?)
}

pub fn sanitize_apply_patch(value: &mut Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.remove("status");
    let Some(metadata) = object.get_mut("metadata").and_then(Value::as_object_mut) else {
        return;
    };
    for field in [
        "creationTimestamp",
        "deletionGracePeriodSeconds",
        "deletionTimestamp",
        "generation",
        "managedFields",
        "resourceVersion",
        "selfLink",
        "uid",
    ] {
        metadata.remove(field);
    }
}

pub async fn patch_status<K, S>(api: &Api<K>, name: &str, status: &S) -> OperatorResult<K>
where
    K: Clone + Debug + DeserializeOwned,
    S: Serialize + Debug,
{
    let pp = PatchParams::default();
    let patch = serde_json::json!({ "status": status });
    Ok(api.patch_status(name, &pp, &Patch::Merge(&patch)).await?)
}

pub fn label_selector(labels: &BTreeMap<String, String>) -> String {
    labels
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}
