use std::fmt::Debug;

use kube::api::{Api, Patch, PatchParams, Resource};
use kube::core::NamespaceResourceScope;
use kube::{Client, ResourceExt};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{OperatorError, OperatorResult};

pub fn add_finalizer<K>(resource: &K, finalizer: &str) -> Vec<String>
where
    K: ResourceExt,
{
    let mut finalizers = resource.finalizers().to_vec();
    if !finalizers.iter().any(|item| item == finalizer) {
        finalizers.push(finalizer.to_string());
    }
    finalizers
}

pub fn remove_finalizer<K>(resource: &K, finalizer: &str) -> Vec<String>
where
    K: ResourceExt,
{
    resource
        .finalizers()
        .iter()
        .filter(|item| *item != finalizer)
        .cloned()
        .collect()
}

pub async fn patch_finalizers<K>(
    client: Client,
    resource: &K,
    finalizers: Vec<String>,
) -> OperatorResult<K>
where
    K: Clone
        + Debug
        + DeserializeOwned
        + Resource<DynamicType = (), Scope = NamespaceResourceScope>
        + Serialize,
{
    let namespace = resource.namespace().ok_or_else(|| {
        OperatorError::InvalidState(format!(
            "resource {} has no namespace for finalizer patch",
            resource.name_any()
        ))
    })?;
    let api = Api::<K>::namespaced(client, &namespace);
    let patch = serde_json::json!({ "metadata": { "finalizers": finalizers } });
    Ok(api
        .patch(
            &resource.name_any(),
            &PatchParams::default(),
            &Patch::Merge(&patch),
        )
        .await?)
}
