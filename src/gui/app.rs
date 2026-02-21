use std::sync::{Arc, Mutex};
use std::thread;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::{egui, App, Frame, CreationContext};
use egui::{CentralPanel, SidePanel, TopBottomPanel, ScrollArea, RichText};

use crate::hosting::scanner;
use crate::hosting::types::PluginInfo;
use crate::hosting::module::VstModule;
use crate::hosting::plugin::PluginInstance;
use crate::hosting::host_app::{HostApp as VstHostApp, ComponentHandler};

pub struct Vst3GuiApp {
    plugins: Vec<PluginInfo>,
    selected_plugin: Option<PluginInfo>,
    
    // Loaded plugin state
    loaded_plugin_info: Option<PluginInfo>,
    loaded_plugin_instance: Arc<Mutex<Option<PluginInstance>>>,
    loaded_module: Arc<Mutex<Option<Arc<VstModule>>>>,
    
    // Editor window management
    editor_thread: Option<thread::JoinHandle<()>>,
    editor_close_signal: Arc<AtomicBool>,
    editor_opened_rx: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    editor_status_msg: String,
    
    // Scanning state
    scan_rx: Option<std::sync::mpsc::Receiver<Result<Vec<PluginInfo>, String>>>,
    is_scanning: bool,
    
    // Status
    status_msg: String,
}

impl Vst3GuiApp {
    pub fn new(_cc: &CreationContext) -> Self {
        Self {
            plugins: Vec::new(),
            selected_plugin: None,
            loaded_plugin_info: None,
            loaded_plugin_instance: Arc::new(Mutex::new(None)),
            loaded_module: Arc::new(Mutex::new(None)),
            editor_thread: None,
            editor_close_signal: Arc::new(AtomicBool::new(false)),
            editor_opened_rx: None,
            editor_status_msg: String::new(),
            scan_rx: None,
            is_scanning: false,
            status_msg: "Ready.".to_owned(),
        }
    }
    
    fn scan_plugins(&mut self) {
        if self.is_scanning { return; }
        
        self.is_scanning = true;
        self.status_msg = "Scanning plugins...".to_owned();
        self.plugins.clear();
        
        let (tx, rx) = std::sync::mpsc::channel();
        self.scan_rx = Some(rx);
        
        thread::spawn(move || {
            let result = scanner::scan_plugins(None)
                .map_err(|e| e.to_string());
            let _ = tx.send(result);
        });
    }

    fn load_plugin(&mut self, info: PluginInfo) {
        self.unload_plugin();
        
        self.status_msg = format!("Loading {}...", info.name);
        
        // Blocking load for simplicity in V1
        // In a real app, do this async/threaded too.
        match self.load_plugin_inner(&info) {
            Ok(_) => {
                self.status_msg = format!("Loaded {}", info.name);
                self.loaded_plugin_info = Some(info.clone());
                self.selected_plugin = Some(info);
            },
            Err(e) => {
                self.status_msg = format!("Failed to load {}: {}", info.name, e);
                self.unload_plugin(); // Cleanup if partial failure
            }
        }
    }
    
    fn load_plugin_inner(&self, info: &PluginInfo) -> Result<(), String> {
        let module = VstModule::load(&info.path)
            .map_err(|e| format!("Module load failed: {}", e))?;
        let module = Arc::new(module);
        
        // Parse UID
        let tuid = hex_to_tuid(&info.uid)?;
        
        let host_app = VstHostApp::new();
        let handler = ComponentHandler::new();
        
        let mut instance = PluginInstance::from_factory(
            Arc::clone(&module), 
            &tuid, 
            host_app, 
            handler
        ).map_err(|e| format!("Instance creation failed: {}", e))?;
        
        instance.setup(44100.0, 512)
            .map_err(|e| format!("Setup failed: {}", e))?;
            
        instance.activate()
             .map_err(|e| format!("Activate failed: {}", e))?;
             
        instance.start_processing()
             .map_err(|e| format!("Start processing failed: {}", e))?;

        // Store
        {
            let mut m = self.loaded_module.lock().unwrap();
            *m = Some(module);
        }
        {
            let mut p = self.loaded_plugin_instance.lock().unwrap();
            *p = Some(instance);
        }
        
        Ok(())
    }
    
    fn unload_plugin(&mut self) {
        // Close editor first
        self.close_editor();
        
        // Drop plugin
        {
            let mut p = self.loaded_plugin_instance.lock().unwrap();
            *p = None;
        }
        {
            let mut m = self.loaded_module.lock().unwrap();
            *m = None;
        }
        self.loaded_plugin_info = None;
    }
    
    fn open_editor(&mut self) {
        if self.editor_thread.is_some() {
            self.close_editor();
        }
        
        let Some(info) = &self.loaded_plugin_info else { return; };
        let plugin_arc = Arc::clone(&self.loaded_plugin_instance);
        let close_signal = Arc::clone(&self.editor_close_signal);
        
        // Reset close signal
        close_signal.store(false, Ordering::Relaxed);
        
        let (tx, rx) = std::sync::mpsc::channel();
        self.editor_opened_rx = Some(rx);
        let plugin_name = info.name.clone();
        
        let handle = thread::spawn(move || {
             let _ = crate::gui::open_editor_window(
                 plugin_arc, 
                 plugin_name, 
                 tx, 
                 close_signal
             );
        });
        
        self.editor_thread = Some(handle);
        self.editor_status_msg = "Opening editor...".to_owned();
    }
    
    fn close_editor(&mut self) {
        if let Some(handle) = self.editor_thread.take() {
            self.editor_close_signal.store(true, Ordering::Relaxed);
            let _ = handle.join();
            self.editor_status_msg = "Editor closed.".to_owned();
        }
        self.editor_opened_rx = None;
    }
}

impl App for Vst3GuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        // Handle scan results
        if let Some(rx) = &self.scan_rx {
             if let Ok(result) = rx.try_recv() {
                 self.is_scanning = false;
                 self.scan_rx = None;
                 match result {
                     Ok(plugins) => {
                         self.plugins = plugins;
                         self.status_msg = format!("Found {} plugins.", self.plugins.len());
                     },
                     Err(e) => {
                         self.status_msg = format!("Scan error: {}", e);
                     }
                 }
             }
        }
        
        // Handle editor open status
        if let Some(rx) = &self.editor_opened_rx {
            if let Ok(res) = rx.try_recv() {
                match res {
                    Ok(_) => self.editor_status_msg = "Editor open.".to_owned(),
                    Err(e) => {
                        self.editor_status_msg = format!("Editor failed: {}", e);
                        // If it failed immediately, join the thread
                        if let Some(handle) = self.editor_thread.take() {
                            let _ = handle.join();
                        }
                    }
                }
            }
        }
    
        TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
             ui.label(&self.status_msg);
             if !self.editor_status_msg.is_empty() {
                 ui.separator();
                 ui.label(&self.editor_status_msg);
             }
        });
        
        SidePanel::left("plugin_list").show(ctx, |ui| {
            ui.heading("Plugins");
            if ui.button("Scan Plugins").clicked() {
                self.scan_plugins();
            }
            if self.is_scanning {
                ui.spinner();
            }
            
            ui.separator();
            
            ScrollArea::vertical().show(ui, |ui| {
                for plugin in &self.plugins {
                    let selected = self.selected_plugin.as_ref().map_or(false, |p| p.uid == plugin.uid);
                    if ui.selectable_label(selected, &plugin.name).clicked() {
                        self.selected_plugin = Some(plugin.clone());
                    }
                }
            });
        });
        
        CentralPanel::default().show(ctx, |ui| {
            if let Some(plugin) = &self.selected_plugin {
                ui.heading(&plugin.name);
                ui.label(format!("Vendor: {}", plugin.vendor));
                ui.label(format!("Version: {}", plugin.version));
                ui.label(format!("Path: {}", plugin.path.display()));
                
                ui.separator();
                
                if self.loaded_plugin_info.as_ref().map_or(false, |p| p.uid == plugin.uid) {
                    ui.label(RichText::new("Loaded").color(egui::Color32::GREEN));
                    
                    if ui.button("Open Editor").clicked() {
                        self.open_editor();
                    }
                    if ui.button("Close Editor").clicked() {
                        self.close_editor();
                    }
                    if ui.button("Unload Plugin").clicked() {
                        self.unload_plugin();
                    }
                    
                    // Show some params maybe?
                    // (Omitted for brevity, but easy to add)
                    
                } else {
                    if ui.button("Load Plugin").clicked() {
                        self.load_plugin(plugin.clone());
                    }
                }
            } else {
                ui.label("Select a plugin to load.");
            }
        });
    }
}

// Helper to copy hex_to_tuid logic
fn hex_to_tuid(hex: &str) -> Result<[i8; 16], String> {
    if hex.len() != 32 {
        return Err(format!("UID must be 32 hex characters, got {}", hex.len()));
    }

    let mut tuid = [0i8; 16];
    for i in 0..16 {
        let byte_str = &hex[i * 2..i * 2 + 2];
        let byte = u8::from_str_radix(byte_str, 16)
            .map_err(|e| format!("Invalid hex byte '{}': {}", byte_str, e))?;
        tuid[i] = byte as i8;
    }

    Ok(tuid)
}

/// Entry point to run the GUI
pub fn run() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0]),
        ..Default::default()
    };
    
    eframe::run_native(
        "VST3 Host",
        options,
        Box::new(|cc| Ok(Box::new(Vst3GuiApp::new(cc)))),
    )
}
