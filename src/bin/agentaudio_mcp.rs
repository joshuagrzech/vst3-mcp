//! Installer/uninstaller for AgentAudio MCP router integration.
//!
//! This patches common MCP client config files on Linux:
//! - Claude Code: `~/.claude.json`
//! - Gemini CLI:  `~/.gemini/settings.json`
//! - Cursor:      `~/.config/cursor/mcp.json` (configured via stdio shim)

use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde_json::{Map, Value};

const DEFAULT_NAME: &str = "agentaudio-router";
const DEFAULT_ROUTER_BASE: &str = "http://127.0.0.1:38765";

#[derive(Clone, Copy, Debug)]
enum Action {
    Install,
    Uninstall,
    Status,
}

#[derive(Clone, Debug)]
struct Config {
    name: String,
    router_base: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (action, cfg) = parse_args()?;

    let home = home_dir().ok_or("HOME is not set")?;
    let targets = targets(&home);

    match action {
        Action::Install => {
            let mut results = Vec::new();
            for t in targets {
                results.push(apply_install(&t, &cfg));
            }
            print_results("install", &results);
        }
        Action::Uninstall => {
            let mut results = Vec::new();
            for t in targets {
                results.push(apply_uninstall(&t, &cfg));
            }
            print_results("uninstall", &results);
        }
        Action::Status => {
            let mut results = Vec::new();
            for t in targets {
                results.push(check_status(&t, &cfg));
            }
            print_results("status", &results);
        }
    }

    Ok(())
}

fn parse_args() -> Result<(Action, Config), String> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().ok_or_else(usage)?;
    let action = match cmd.as_str() {
        "install" => Action::Install,
        "uninstall" => Action::Uninstall,
        "status" => Action::Status,
        _ => return Err(usage()),
    };

    let mut name = DEFAULT_NAME.to_string();
    let mut router_base = std::env::var("AGENTAUDIO_MCP_ROUTERD")
        .ok()
        .unwrap_or_else(|| DEFAULT_ROUTER_BASE.to_string());

    while let Some(a) = args.next() {
        match a.as_str() {
            "--name" => {
                name = args
                    .next()
                    .ok_or_else(|| "Missing value for --name".to_string())?;
            }
            "--router" => {
                router_base = args
                    .next()
                    .ok_or_else(|| "Missing value for --router".to_string())?;
            }
            "--help" | "-h" => return Err(usage()),
            other => return Err(format!("Unknown arg '{other}'.\n\n{}", usage())),
        }
    }

    let router_base = router_base.trim().trim_end_matches('/').to_string();
    if router_base.is_empty() {
        return Err("router base URL is empty".to_string());
    }

    Ok((action, Config { name, router_base }))
}

fn usage() -> String {
    format!(
        "Usage:\n  agentaudio-mcp <install|uninstall|status> [--name NAME] [--router ROUTER_BASE]\n\nDefaults:\n  --name   {DEFAULT_NAME}\n  --router {DEFAULT_ROUTER_BASE}\n\nNotes:\n  - ROUTER_BASE should be like 'http://127.0.0.1:38765' (no /mcp).\n  - If AGENTAUDIO_MCP_ROUTERD is set, it is used as the default --router.\n"
    )
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[derive(Clone, Debug)]
struct Target {
    kind: TargetKind,
    path: PathBuf,
}

#[derive(Clone, Copy, Debug)]
enum TargetKind {
    ClaudeCode,
    GeminiCli,
    Cursor,
}

fn targets(home: &Path) -> Vec<Target> {
    vec![
        Target {
            kind: TargetKind::ClaudeCode,
            path: home.join(".claude.json"),
        },
        Target {
            kind: TargetKind::GeminiCli,
            path: home.join(".gemini").join("settings.json"),
        },
        Target {
            kind: TargetKind::Cursor,
            path: home.join(".config").join("cursor").join("mcp.json"),
        },
    ]
}

#[derive(Debug)]
struct Outcome {
    target: Target,
    changed: bool,
    present: bool,
    message: String,
}

fn print_results(op: &str, results: &[Outcome]) {
    let mut out = String::new();
    out.push_str(&format!("agentaudio-mcp {op} results:\n"));
    for r in results {
        out.push_str(&format!(
            "- {:?}: {} ({}, changed={}, present={})\n",
            r.target.kind,
            r.target.path.display(),
            r.message,
            r.changed,
            r.present
        ));
    }
    let _ = io::stdout().write_all(out.as_bytes());
}

fn check_status(target: &Target, cfg: &Config) -> Outcome {
    match load_json(&target.path) {
        Ok(mut root) => {
            let present = has_entry(&mut root, target.kind, cfg);
            Outcome {
                target: target.clone(),
                changed: false,
                present,
                message: if present {
                    "present".to_string()
                } else {
                    "not present".to_string()
                },
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Outcome {
            target: target.clone(),
            changed: false,
            present: false,
            message: "file not found".to_string(),
        },
        Err(e) => Outcome {
            target: target.clone(),
            changed: false,
            present: false,
            message: format!("error reading: {e}"),
        },
    }
}

fn apply_install(target: &Target, cfg: &Config) -> Outcome {
    let mut root = match load_json(&target.path) {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::NotFound => Value::Object(Map::new()),
        Err(e) => {
            return Outcome {
                target: target.clone(),
                changed: false,
                present: false,
                message: format!("error reading: {e}"),
            };
        }
    };

    let changed = upsert_entry(&mut root, target.kind, cfg);
    if changed {
        if let Err(e) = write_json_with_backup(&target.path, &root) {
            return Outcome {
                target: target.clone(),
                changed: false,
                present: false,
                message: format!("error writing: {e}"),
            };
        }
    }

    Outcome {
        target: target.clone(),
        changed,
        present: true,
        message: if changed {
            "installed".to_string()
        } else {
            "already installed".to_string()
        },
    }
}

fn apply_uninstall(target: &Target, cfg: &Config) -> Outcome {
    let mut root = match load_json(&target.path) {
        Ok(v) => v,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Outcome {
                target: target.clone(),
                changed: false,
                present: false,
                message: "file not found".to_string(),
            };
        }
        Err(e) => {
            return Outcome {
                target: target.clone(),
                changed: false,
                present: false,
                message: format!("error reading: {e}"),
            };
        }
    };

    let changed = remove_entry(&mut root, target.kind, cfg);
    let present = has_entry(&mut root, target.kind, cfg);
    if changed {
        if let Err(e) = write_json_with_backup(&target.path, &root) {
            return Outcome {
                target: target.clone(),
                changed: false,
                present,
                message: format!("error writing: {e}"),
            };
        }
    }

    Outcome {
        target: target.clone(),
        changed,
        present,
        message: if changed {
            "removed".to_string()
        } else {
            "not installed".to_string()
        },
    }
}

fn load_json(path: &Path) -> io::Result<Value> {
    let bytes = fs::read(path)?;
    let v: Value = serde_json::from_slice(&bytes)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(v)
}

fn ensure_parent_dir(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn write_json_with_backup(path: &Path, root: &Value) -> io::Result<()> {
    ensure_parent_dir(path)?;

    if path.exists() {
        let backup = backup_path(path);
        let _ = fs::copy(path, &backup);
    }

    let tmp = tmp_path(path);
    {
        let mut f = fs::File::create(&tmp)?;
        let bytes =
            serde_json::to_vec_pretty(root).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        f.write_all(&bytes)?;
        f.write_all(b"\n")?;
        f.sync_all()?;
    }
    fs::rename(tmp, path)?;
    Ok(())
}

fn backup_path(path: &Path) -> PathBuf {
    let ts = now_ms();
    let file = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("config.json");
    path.with_file_name(format!("{file}.bak-{ts}"))
}

fn tmp_path(path: &Path) -> PathBuf {
    let ts = now_ms();
    let file = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("config.json");
    path.with_file_name(format!("{file}.tmp-{ts}"))
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn mcp_servers_mut(root: &mut Value) -> &mut Map<String, Value> {
    if !root.is_object() {
        *root = Value::Object(Map::new());
    }
    let obj = root.as_object_mut().expect("root must be object");
    obj.entry("mcpServers".to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .expect("mcpServers must be object")
}

fn upsert_entry(root: &mut Value, kind: TargetKind, cfg: &Config) -> bool {
    let servers = mcp_servers_mut(root);
    let desired = desired_entry(kind, cfg);
    match servers.get(&cfg.name) {
        Some(existing) if *existing == desired => false,
        _ => {
            servers.insert(cfg.name.clone(), desired);
            true
        }
    }
}

fn remove_entry(root: &mut Value, _kind: TargetKind, cfg: &Config) -> bool {
    let servers = mcp_servers_mut(root);
    servers.remove(&cfg.name).is_some()
}

fn has_entry(root: &mut Value, _kind: TargetKind, cfg: &Config) -> bool {
    let servers = mcp_servers_mut(root);
    servers.contains_key(&cfg.name)
}

fn desired_entry(kind: TargetKind, cfg: &Config) -> Value {
    match kind {
        TargetKind::ClaudeCode => serde_json::json!({
            "type": "http",
            "url": format!("{}/mcp", cfg.router_base),
        }),
        TargetKind::GeminiCli => serde_json::json!({
            "httpUrl": format!("{}/mcp", cfg.router_base),
        }),
        TargetKind::Cursor => serde_json::json!({
            "command": "agentaudio-mcp-stdio",
            "args": [],
            "env": {
                "AGENTAUDIO_MCP_ROUTERD": cfg.router_base,
            }
        }),
    }
}
