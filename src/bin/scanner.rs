//! Out-of-process VST3 scanner binary.
//!
//! Accepts a single argument: the path to a .vst3 bundle.
//! Loads the module, queries the factory, and outputs PluginInfo
//! as JSON on stdout. Errors go to stderr.
//!
//! Exit codes:
//!   0 = success (JSON on stdout)
//!   1 = error (message on stderr)

use std::path::Path;
use std::process;

use anyhow::{Context, Result};
use tracing_subscriber::EnvFilter;

use vst3_mcp_host::hosting::scanner::scan_bundle_binary;

fn main() {
    // Initialize tracing to stderr (stdout is reserved for JSON output)
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    if let Err(e) = run() {
        eprintln!("scan error: {:#}", e);
        process::exit(1);
    }
}

fn run() -> Result<()> {
    let bundle_path = std::env::args()
        .nth(1)
        .context("Usage: agent-audio-scanner <bundle_path>")?;

    let path = Path::new(&bundle_path);

    let plugins = scan_bundle_binary(path)
        .map_err(|e| anyhow::anyhow!("{}", e))
        .with_context(|| format!("failed to scan bundle: {}", path.display()))?;

    let json = serde_json::to_string(&plugins).context("failed to serialize plugin info")?;

    println!("{}", json);

    Ok(())
}
