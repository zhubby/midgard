use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::routing::get;

use crate::{OperatorError, OperatorResult};

pub fn start_probe_servers(
    health_probe_bind_address: Option<&str>,
    metrics_bind_address: Option<&str>,
    metrics_body: Option<String>,
    service_name: impl Into<String>,
) -> OperatorResult<()> {
    let service_name = service_name.into();
    if let Some(addr) = optional_probe_addr(health_probe_bind_address)? {
        let service_name = service_name.clone();
        tokio::spawn(async move {
            if let Err(err) = serve_health(addr, service_name.clone()).await {
                tracing::error!(%err, service = %service_name, "operator health server exited");
            }
        });
    }

    if let Some(addr) = optional_probe_addr(metrics_bind_address)? {
        let body = metrics_body.unwrap_or_default();
        let service_name = service_name.clone();
        tokio::spawn(async move {
            if let Err(err) = serve_metrics(addr, body, service_name.clone()).await {
                tracing::error!(%err, service = %service_name, "operator metrics server exited");
            }
        });
    }

    Ok(())
}

pub async fn serve_health(addr: SocketAddr, service_name: impl AsRef<str>) -> OperatorResult<()> {
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, service = %service_name.as_ref(), "operator health server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

pub async fn serve_metrics(
    addr: SocketAddr,
    metrics_body: String,
    service_name: impl AsRef<str>,
) -> OperatorResult<()> {
    let body = Arc::new(metrics_body);
    let app = Router::new().route(
        "/metrics",
        get({
            let body = body.clone();
            move || {
                let body = body.clone();
                async move { body.as_ref().clone() }
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, service = %service_name.as_ref(), "operator metrics server listening");
    axum::serve(listener, app).await?;
    Ok(())
}

pub fn optional_probe_addr(value: Option<&str>) -> OperatorResult<Option<SocketAddr>> {
    let Some(value) = value
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "0")
    else {
        return Ok(None);
    };
    let normalized = if let Some(port) = value.strip_prefix(':') {
        format!("0.0.0.0:{port}")
    } else {
        value.to_string()
    };
    normalized
        .parse()
        .map(Some)
        .map_err(|err| OperatorError::InvalidConfig(format!("invalid bind address {value}: {err}")))
}
