use ankhimate_mcp::server::AnkhimateServer;
use rmcp::{ServiceExt, transport::stdio};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = AnkhimateServer::new().serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
