#[tokio::main]
async fn main() -> anyhow::Result<()> {
    midgard_cli::run().await
}
