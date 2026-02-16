use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use eframe::egui;

#[derive(Clone, Debug)]
enum WorkerMsg {
    Log(String),
    Done(Result<(), String>),
}

#[derive(Default)]
struct InstallerApp {
    // UI state
    install_dir: String,
    router_base: String,
    install_plugin: bool,
    install_binaries: bool,
    enable_router_service: bool,
    configure_agents: bool,

    running: bool,
    logs: Vec<String>,
    worker_rx: Option<Receiver<WorkerMsg>>,
    last_result: Option<Result<(), String>>,
}

impl InstallerApp {
    fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let default_vst3 = if home.is_empty() {
            "~/.vst3".to_string()
        } else {
            format!("{home}/.vst3")
        };

        let router_base = std::env::var("AGENTAUDIO_MCP_ROUTERD")
            .ok()
            .unwrap_or_else(|| "http://127.0.0.1:38765".to_string());

        Self {
            install_dir: default_vst3,
            router_base,
            install_plugin: true,
            install_binaries: true,
            enable_router_service: true,
            configure_agents: true,
            ..Default::default()
        }
    }

    fn append_log(&mut self, s: impl Into<String>) {
        self.logs.push(s.into());
        if self.logs.len() > 2000 {
            let drain = self.logs.len().saturating_sub(2000);
            self.logs.drain(0..drain);
        }
    }

    fn start_install(&mut self) {
        if self.running {
            return;
        }
        self.running = true;
        self.last_result = None;
        self.append_log("Starting install…");

        let (tx, rx) = mpsc::channel::<WorkerMsg>();
        self.worker_rx = Some(rx);

        let opts = WorkerOpts {
            install_dir: self.install_dir.clone(),
            router_base: self.router_base.clone(),
            install_plugin: self.install_plugin,
            install_binaries: self.install_binaries,
            enable_router_service: self.enable_router_service,
            configure_agents: self.configure_agents,
        };

        thread::spawn(move || run_worker(opts, tx));
    }

    fn poll_worker(&mut self) {
        let mut drained: Vec<WorkerMsg> = Vec::new();
        if let Some(rx) = self.worker_rx.as_ref() {
            while let Ok(msg) = rx.try_recv() {
                drained.push(msg);
            }
        }

        for msg in drained {
            match msg {
                WorkerMsg::Log(line) => self.append_log(line),
                WorkerMsg::Done(res) => {
                    self.running = false;
                    self.last_result = Some(res.clone());
                    self.worker_rx = None;
                    self.append_log(match res {
                        Ok(()) => "Install complete.".to_string(),
                        Err(e) => format!("Install failed: {e}"),
                    });
                }
            }
        }
    }
}

impl eframe::App for InstallerApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_worker();

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("AgentAudio Installer");
            ui.add_space(6.0);

            ui.label("One-click build + install for the wrapper VST3 + MCP router tooling.");
            ui.add_space(10.0);

            ui.group(|ui| {
                ui.label("Install options");
                ui.checkbox(&mut self.install_plugin, "Build + install VST3 wrapper plugin");
                ui.checkbox(&mut self.install_binaries, "Build + install router binaries to ~/.local/bin");
                ui.checkbox(&mut self.enable_router_service, "Enable + start router service (systemd --user on Linux)");
                ui.checkbox(&mut self.configure_agents, "Configure Claude/Gemini/Cursor MCP settings");
            });

            ui.add_space(10.0);

            ui.group(|ui| {
                ui.label("Paths / settings");
                ui.horizontal(|ui| {
                    ui.label("VST3 install dir:");
                    ui.text_edit_singleline(&mut self.install_dir);
                });
                ui.horizontal(|ui| {
                    ui.label("Router base URL:");
                    ui.text_edit_singleline(&mut self.router_base);
                });
                ui.small("Router base should look like http://127.0.0.1:38765 (no /mcp).");
            });

            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let install_enabled = !self.running;
                if ui
                    .add_enabled(install_enabled, egui::Button::new("Install"))
                    .clicked()
                {
                    self.start_install();
                }

                if self.running {
                    ui.label("Running…");
                } else if let Some(res) = &self.last_result {
                    match res {
                        Ok(()) => ui.label("Status: OK"),
                        Err(_) => ui.label("Status: FAILED"),
                    };
                }
            });

            ui.add_space(10.0);
            ui.separator();
            ui.label("Log");

            egui::ScrollArea::vertical()
                .stick_to_bottom(true)
                .max_height(ui.available_height())
                .show(ui, |ui| {
                    for line in &self.logs {
                        ui.monospace(line);
                    }
                });
        });

        if self.running {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}

#[derive(Clone, Debug)]
struct WorkerOpts {
    install_dir: String,
    router_base: String,
    install_plugin: bool,
    install_binaries: bool,
    enable_router_service: bool,
    configure_agents: bool,
}

fn run_worker(opts: WorkerOpts, tx: Sender<WorkerMsg>) {
    let res = (|| -> Result<(), String> {
        validate_platform(&tx)?;

        let repo_root = std::env::current_dir().map_err(|e| e.to_string())?;
        tx.send(WorkerMsg::Log(format!(
            "Repo root: {}",
            repo_root.display()
        )))
        .ok();

        if opts.install_plugin {
            tx.send(WorkerMsg::Log("Building + bundling VST3 wrapper…".to_string()))
                .ok();
            let mut cmd = Command::new("bash");
            cmd.arg("./scripts/build-and-install-vst3.sh")
                .arg("release")
                .arg(expand_tilde(&opts.install_dir))
                .current_dir(&repo_root);
            run_cmd_stream(&tx, cmd)?;
        } else {
            tx.send(WorkerMsg::Log("Skipping VST3 wrapper install.".to_string()))
                .ok();
        }

        tx.send(WorkerMsg::Log("Building router + shims…".to_string()))
            .ok();
        let mut cmd = Command::new("cargo");
        cmd.arg("build")
            .arg("--release")
            .arg("-p")
            .arg("agentaudio-mcp-router")
            .arg("--bin")
            .arg("agentaudio-mcp-routerd")
            .current_dir(&repo_root);
        run_cmd_stream(&tx, cmd)?;

        let mut cmd = Command::new("cargo");
        cmd.arg("build")
            .arg("--release")
            .arg("--bin")
            .arg("agentaudio-mcp-stdio")
            .current_dir(&repo_root);
        run_cmd_stream(&tx, cmd)?;

        let mut cmd = Command::new("cargo");
        cmd.arg("build")
            .arg("--release")
            .arg("--bin")
            .arg("agentaudio-mcp")
            .current_dir(&repo_root);
        run_cmd_stream(&tx, cmd)?;

        let router_base = opts.router_base.trim().trim_end_matches('/').to_string();
        if router_base.is_empty() {
            return Err("Router base URL is empty".to_string());
        }

        let local_bin = expand_tilde("~/.local/bin");
        if opts.install_binaries {
            tx.send(WorkerMsg::Log(format!(
                "Installing binaries to {} …",
                local_bin.display()
            )))
            .ok();

            fs::create_dir_all(&local_bin).map_err(|e| e.to_string())?;

            let routerd_src = repo_root.join("target/release/agentaudio-mcp-routerd");
            let stdio_src = repo_root.join("target/release/agentaudio-mcp-stdio");
            let mcp_src = repo_root.join("target/release/agentaudio-mcp");

            copy_executable(&tx, &routerd_src, &local_bin.join("agentaudio-mcp-routerd"))?;
            copy_executable(&tx, &stdio_src, &local_bin.join("agentaudio-mcp-stdio"))?;
            copy_executable(&tx, &mcp_src, &local_bin.join("agentaudio-mcp"))?;
        } else {
            tx.send(WorkerMsg::Log("Skipping binary installation.".to_string()))
                .ok();
        }

        if opts.enable_router_service {
            tx.send(WorkerMsg::Log("Configuring router systemd user service…".to_string()))
                .ok();
            install_systemd_user_service(&tx, &local_bin, &router_base)?;
            let mut cmd = Command::new("systemctl");
            cmd.arg("--user").arg("daemon-reload");
            run_cmd_stream(&tx, cmd)?;

            let mut cmd = Command::new("systemctl");
            cmd.arg("--user")
                .arg("enable")
                .arg("--now")
                .arg("agentaudio-mcp-routerd.service");
            run_cmd_stream(&tx, cmd)?;
        } else {
            tx.send(WorkerMsg::Log("Skipping router service activation.".to_string()))
                .ok();
        }

        if opts.configure_agents {
            tx.send(WorkerMsg::Log("Patching MCP client configs…".to_string()))
                .ok();

            // Prefer installed binary, but fall back to repo target path.
            let mcp_bin = local_bin.join("agentaudio-mcp");
            let mut cmd = if mcp_bin.exists() {
                Command::new(mcp_bin)
            } else {
                Command::new(repo_root.join("target/release/agentaudio-mcp"))
            };
            cmd.arg("install").arg("--router").arg(&router_base);
            run_cmd_stream(&tx, cmd)?;
        } else {
            tx.send(WorkerMsg::Log("Skipping MCP client config.".to_string()))
                .ok();
        }

        Ok(())
    })();

    let _ = tx.send(WorkerMsg::Done(res));
}

fn validate_platform(tx: &Sender<WorkerMsg>) -> Result<(), String> {
    // Current repo scripts are Linux x86_64 oriented; this GUI targets "current platform" first.
    if cfg!(target_os = "linux") {
        let arch = std::env::consts::ARCH;
        tx.send(WorkerMsg::Log(format!("Detected platform: linux/{arch}")))
            .ok();
        Ok(())
    } else {
        Err("This installer currently supports Linux only (systemd + Linux VST3 bundle layout).".to_string())
    }
}

fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(s)
}

fn copy_executable(tx: &Sender<WorkerMsg>, from: &Path, to: &Path) -> Result<(), String> {
    if !from.exists() {
        return Err(format!("Expected build output not found: {}", from.display()));
    }
    let _ = fs::copy(from, to).map_err(|e| format!("Copy failed {} -> {}: {e}", from.display(), to.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(to).map_err(|e| e.to_string())?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(to, perms).map_err(|e| e.to_string())?;
    }
    tx.send(WorkerMsg::Log(format!(
        "Installed {}",
        to.display()
    )))
    .ok();
    Ok(())
}

fn install_systemd_user_service(
    tx: &Sender<WorkerMsg>,
    local_bin: &Path,
    router_base: &str,
) -> Result<(), String> {
    let home = std::env::var("HOME").map_err(|_| "HOME is not set".to_string())?;
    let service_dir = PathBuf::from(home)
        .join(".config/systemd/user");
    fs::create_dir_all(&service_dir).map_err(|e| e.to_string())?;

    let exec = local_bin.join("agentaudio-mcp-routerd");
    let bind = router_bind_from_base(router_base)?;
    let unit = format!(
        r#"[Unit]
Description=AgentAudio MCP Router Daemon
After=network.target

[Service]
Type=simple
ExecStart={}
Restart=on-failure
Environment=RUST_LOG=info
Environment=AGENTAUDIO_MCP_ROUTERD_BIND={}

[Install]
WantedBy=default.target
"#,
        exec.display(),
        bind
    );

    let path = service_dir.join("agentaudio-mcp-routerd.service");
    fs::write(&path, unit).map_err(|e| e.to_string())?;
    tx.send(WorkerMsg::Log(format!(
        "Wrote systemd unit: {}",
        path.display()
    )))
    .ok();

    // Keep router base consistent for anything reading it from env, but the daemon uses bind.
    tx.send(WorkerMsg::Log(format!(
        "Router base URL set to: {router_base}"
    )))
    .ok();
    Ok(())
}

fn router_bind_from_base(router_base: &str) -> Result<String, String> {
    let base = router_base.trim().trim_end_matches('/');
    let url = url::Url::parse(base).map_err(|e| format!("Invalid router base URL '{base}': {e}"))?;
    let host = url
        .host_str()
        .ok_or_else(|| format!("Router base URL has no host: '{base}'"))?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| format!("Router base URL has no port: '{base}'"))?;
    Ok(format!("{host}:{port}"))
}

fn run_cmd_stream(tx: &Sender<WorkerMsg>, mut cmd: Command) -> Result<(), String> {
    tx.send(WorkerMsg::Log(format!("$ {:?}", cmd))).ok();

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let (out_tx, out_rx) = mpsc::channel::<String>();

    let spawn_reader = |reader: Option<std::process::ChildStdout>,
                        out_tx: Sender<String>| {
        thread::spawn(move || {
            if let Some(r) = reader {
                let br = BufReader::new(r);
                for line in br.lines().flatten() {
                    let _ = out_tx.send(line);
                }
            }
        })
    };
    let spawn_reader_err = |reader: Option<std::process::ChildStderr>,
                            out_tx: Sender<String>| {
        thread::spawn(move || {
            if let Some(r) = reader {
                let br = BufReader::new(r);
                for line in br.lines().flatten() {
                    let _ = out_tx.send(line);
                }
            }
        })
    };

    let t1 = spawn_reader(stdout, out_tx.clone());
    let t2 = spawn_reader_err(stderr, out_tx.clone());

    loop {
        match out_rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(line) => {
                let _ = tx.send(WorkerMsg::Log(line));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Ok(Some(status)) = child.try_wait() {
                    let _ = t1.join();
                    let _ = t2.join();
                    if !status.success() {
                        return Err(format!("Command failed with status: {status}"));
                    }
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(())
}

fn main() -> Result<(), eframe::Error> {
    let native_options = eframe::NativeOptions::default();
    eframe::run_native(
        "AgentAudio Installer",
        native_options,
        Box::new(|_cc| Ok(Box::new(InstallerApp::new()))),
    )
}

