//! VST3 MCP Host entry point.
//!
//! Starts an MCP server over stdio that exposes VST3 plugin hosting
//! tools: scan_plugins, load_plugin, process_audio, save_preset, load_preset.

mod server;

use rmcp::ServiceExt;
use server::AudioHost;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing to stderr (stdout is reserved for MCP JSON-RPC protocol)
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    tracing::info!("vst3-mcp-host starting");

    let host = AudioHost::new();
    let service = host.serve(rmcp::transport::io::stdio()).await?;

    service.waiting().await?;

    tracing::info!("vst3-mcp-host shutting down");
    Ok(())
}
