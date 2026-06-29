use std::net::SocketAddr;

use midgard_server::app;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let address = SocketAddr::from(([0, 0, 0, 0], 8080));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .expect("bind midgard server");

    tracing::info!(%address, "midgard server listening");
    axum::serve(listener, app()).await.expect("serve midgard api");
}
