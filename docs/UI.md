# VST3 Host GUI

This project now includes a graphical user interface (GUI) built with [egui](https://github.com/emilk/egui) and [eframe](https://github.com/emilk/egui/tree/master/crates/eframe).

## Running the GUI

To start the VST3 Host GUI, run:

```bash
cargo run --bin vst3-gui
```

## Features

- **Scan Plugins:** click the "Scan Plugins" button to discover VST3 plugins on your system.
- **Plugin List:** Browse discovered plugins in the side panel.
- **Load Plugin:** Select a plugin and click "Load Plugin" to initialize it.
- **Open Editor:** Click "Open Editor" to open the plugin's native VST3 GUI in a separate window.
- **Unload Plugin:** improved resource management.

## Architecture

The GUI is implemented in `src/gui/app.rs` using the `eframe::App` trait.
It communicates with the `vst3-mcp-host` library logic (scanning, hosting) directly.

- **Main Thread:** Runs the `egui` event loop.
- **Scan Thread:** Background thread for scanning plugins.
- **Editor Thread:** Dedicated background thread for the VST3 plugin editor window (Linux/X11).

## dependencies

- `eframe` (0.31)
- `egui` (0.31)
- `winit` (0.30) - Used internally for plugin window management.
