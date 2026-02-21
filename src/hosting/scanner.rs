//! Plugin discovery: scan OS-specific paths for .vst3 bundles
//! and extract metadata (name, vendor, UID, category, version).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use tracing::{debug, warn};
use vst3::Steinberg::kResultOk;
use vst3::Steinberg::IPluginFactory2;
use vst3::Steinberg::IPluginFactoryTrait;
use vst3::Steinberg::IPluginFactory2Trait;
use vst3::Steinberg::PClassInfo;
use vst3::Steinberg::PClassInfo2;
use vst3::Steinberg::PFactoryInfo;
use vst3::com_scrape_types::ComPtr;

use super::module::VstModule;
use super::types::{HostError, PluginInfo};

/// Return the default OS-specific paths where VST3 plugins are installed.
pub fn default_scan_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    #[cfg(target_os = "linux")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(PathBuf::from(home).join(".vst3"));
        }
        paths.push(PathBuf::from("/usr/lib/vst3"));
        paths.push(PathBuf::from("/usr/local/lib/vst3"));
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            paths.push(PathBuf::from(home).join("Library/Audio/Plug-Ins/VST3"));
        }
        paths.push(PathBuf::from("/Library/Audio/Plug-Ins/VST3"));
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            paths.push(PathBuf::from(local).join("Programs/Common/VST3"));
        }
        paths.push(PathBuf::from("C:/Program Files/Common Files/VST3"));
    }

    paths
}

/// Scan for VST3 plugins and return metadata for each discovered plugin.
///
/// If `custom_path` is provided, only that directory is scanned.
/// Otherwise, the default OS-specific paths are scanned.
pub fn scan_plugins(custom_path: Option<&str>) -> Result<Vec<PluginInfo>, HostError> {
    let paths = match custom_path {
        Some(p) => vec![PathBuf::from(p)],
        None => default_scan_paths(),
    };

    let mut plugins = Vec::new();

    for scan_path in &paths {
        if !scan_path.exists() {
            debug!("scan path does not exist, skipping: {}", scan_path.display());
            continue;
        }

        debug!("scanning for plugins in: {}", scan_path.display());
        scan_directory(scan_path, &mut plugins);
    }

    Ok(plugins)
}

/// Scan for VST3 plugins with optional out-of-process crash isolation.
///
/// When `scanner_binary` is provided, the slow path (binary loading) runs
/// in a separate child process. If the child crashes, the host survives and
/// reports the failure. The fast path (moduleinfo.json) always runs in-process
/// since it is just file I/O with zero crash risk.
///
/// When `scanner_binary` is None, falls back to fully in-process scanning
/// (same behavior as `scan_plugins`).
pub fn scan_plugins_safe(
    custom_path: Option<&str>,
    scanner_binary: Option<&Path>,
) -> Result<Vec<PluginInfo>, HostError> {
    let paths = match custom_path {
        Some(p) => vec![PathBuf::from(p)],
        None => default_scan_paths(),
    };

    let mut plugins = Vec::new();

    for scan_path in &paths {
        if !scan_path.exists() {
            debug!("scan path does not exist, skipping: {}", scan_path.display());
            continue;
        }

        debug!("scanning for plugins in: {}", scan_path.display());
        match scanner_binary {
            Some(binary) => scan_directory_safe(scan_path, &mut plugins, binary),
            None => scan_directory(scan_path, &mut plugins),
        }
    }

    Ok(plugins)
}

/// Scan a single .vst3 bundle out-of-process by spawning the scanner binary.
///
/// The scanner binary loads the module and queries the factory in a child
/// process. If the child crashes (non-zero exit, signal), this returns a
/// `HostError::ScanError` instead of crashing the host.
pub fn scan_bundle_out_of_process(
    scanner_path: &Path,
    bundle_path: &Path,
    timeout: Duration,
) -> Result<Vec<PluginInfo>, HostError> {
    let child = Command::new(scanner_path)
        .arg(bundle_path.as_os_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            HostError::ScanError(format!(
                "failed to spawn scanner for {}: {}",
                bundle_path.display(),
                e
            ))
        })?;

    // Implement timeout using a thread that kills the child if it takes too long
    let timeout_ms = timeout.as_millis() as u64;
    let child_id = child.id();
    let (tx, rx) = std::sync::mpsc::channel();

    let timer_thread = std::thread::spawn(move || {
        match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
            Ok(()) => {
                // Child finished before timeout -- nothing to do
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                // Timeout: try to kill the child process
                #[cfg(unix)]
                {
                    unsafe {
                        libc::kill(child_id as i32, libc::SIGKILL);
                    }
                }
                #[cfg(not(unix))]
                {
                    // On non-unix, we cannot easily kill by PID here.
                    // The wait_with_output below will eventually return.
                    let _ = child_id;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                // Sender dropped -- child finished
            }
        }
    });

    let output = child.wait_with_output().map_err(|e| {
        let _ = tx.send(());
        HostError::ScanError(format!(
            "scanner process failed for {}: {}",
            bundle_path.display(),
            e
        ))
    })?;

    // Signal the timer thread that we are done
    let _ = tx.send(());
    let _ = timer_thread.join();

    if !output.status.success() {
        return Err(HostError::ScanError(format!(
            "scanner crashed or failed for {}: exit={:?}, stderr={}",
            bundle_path.display(),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr),
        )));
    }

    let plugins: Vec<PluginInfo> = serde_json::from_slice(&output.stdout).map_err(|e| {
        HostError::ScanError(format!(
            "invalid scanner output for {}: {}",
            bundle_path.display(),
            e
        ))
    })?;

    Ok(plugins)
}

/// Default timeout per bundle when scanning out-of-process.
pub const DEFAULT_SCAN_TIMEOUT: Duration = Duration::from_secs(10);

/// Recursively walk a directory looking for .vst3 bundles, using out-of-process
/// scanning for the binary load path.
fn scan_directory_safe(dir: &Path, plugins: &mut Vec<PluginInfo>, scanner_binary: &Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            warn!("failed to read directory {}: {}", dir.display(), e);
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("failed to read directory entry: {}", e);
                continue;
            }
        };

        let path = entry.path();

        if path.is_dir() {
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("vst3"))
            {
                // Found a .vst3 bundle
                debug!("found VST3 bundle: {}", path.display());
                match scan_bundle_safe(&path, scanner_binary) {
                    Ok(mut bundle_plugins) => plugins.append(&mut bundle_plugins),
                    Err(e) => {
                        warn!("failed to scan bundle {}: {}", path.display(), e);
                    }
                }
            } else {
                // Recurse into subdirectories
                scan_directory_safe(&path, plugins, scanner_binary);
            }
        }
    }
}

/// Scan a single .vst3 bundle with crash isolation.
///
/// Fast path (moduleinfo.json) runs in-process. Slow path (binary load)
/// runs out-of-process via the scanner binary.
fn scan_bundle_safe(
    bundle_path: &Path,
    scanner_binary: &Path,
) -> Result<Vec<PluginInfo>, HostError> {
    // Fast path: try moduleinfo.json (safe in-process -- just file I/O)
    let moduleinfo_path = bundle_path
        .join("Contents")
        .join("Resources")
        .join("moduleinfo.json");

    if moduleinfo_path.exists() {
        match scan_moduleinfo(&moduleinfo_path, bundle_path) {
            Ok(plugins) if !plugins.is_empty() => {
                debug!(
                    "extracted {} plugins from moduleinfo.json for {}",
                    plugins.len(),
                    bundle_path.display()
                );
                return Ok(plugins);
            }
            Ok(_) => {
                debug!(
                    "moduleinfo.json had no audio processor classes, falling back to out-of-process binary scan"
                );
            }
            Err(e) => {
                debug!(
                    "failed to parse moduleinfo.json for {}: {}, falling back to out-of-process binary scan",
                    bundle_path.display(),
                    e
                );
            }
        }
    }

    // Slow path: out-of-process binary scan
    scan_bundle_out_of_process(scanner_binary, bundle_path, DEFAULT_SCAN_TIMEOUT)
}

/// Recursively walk a directory looking for .vst3 bundles.
fn scan_directory(dir: &Path, plugins: &mut Vec<PluginInfo>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            warn!("failed to read directory {}: {}", dir.display(), e);
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("failed to read directory entry: {}", e);
                continue;
            }
        };

        let path = entry.path();

        if path.is_dir() {
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("vst3"))
            {
                // Found a .vst3 bundle
                debug!("found VST3 bundle: {}", path.display());
                match scan_bundle(&path) {
                    Ok(mut bundle_plugins) => plugins.append(&mut bundle_plugins),
                    Err(e) => {
                        warn!("failed to scan bundle {}: {}", path.display(), e);
                    }
                }
            } else {
                // Recurse into subdirectories
                scan_directory(&path, plugins);
            }
        }
    }
}

/// Scan a single .vst3 bundle for plugin class info.
///
/// Use this when the user manually selects a .vst3 path instead of scanning
/// all system paths. Fast path: moduleinfo.json; slow path: load the module.
pub fn scan_single_bundle(bundle_path: &Path) -> Result<Vec<PluginInfo>, HostError> {
    scan_bundle(bundle_path)
}

/// Scan a single .vst3 bundle for plugin class info (internal).
fn scan_bundle(bundle_path: &Path) -> Result<Vec<PluginInfo>, HostError> {
    // Fast path: try moduleinfo.json
    let moduleinfo_path = bundle_path
        .join("Contents")
        .join("Resources")
        .join("moduleinfo.json");

    if moduleinfo_path.exists() {
        match scan_moduleinfo(&moduleinfo_path, bundle_path) {
            Ok(plugins) if !plugins.is_empty() => {
                debug!(
                    "extracted {} plugins from moduleinfo.json for {}",
                    plugins.len(),
                    bundle_path.display()
                );
                return Ok(plugins);
            }
            Ok(_) => {
                debug!(
                    "moduleinfo.json had no audio processor classes, falling back to binary scan"
                );
            }
            Err(e) => {
                debug!(
                    "failed to parse moduleinfo.json for {}: {}, falling back to binary scan",
                    bundle_path.display(),
                    e
                );
            }
        }
    }

    // Slow path: load the module
    scan_bundle_binary(bundle_path)
}

/// Parse moduleinfo.json to extract plugin metadata without loading the binary.
fn scan_moduleinfo(
    moduleinfo_path: &Path,
    bundle_path: &Path,
) -> Result<Vec<PluginInfo>, HostError> {
    let content = std::fs::read_to_string(moduleinfo_path)
        .map_err(|e| HostError::ScanError(format!("failed to read moduleinfo.json: {}", e)))?;

    let json: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| HostError::ScanError(format!("failed to parse moduleinfo.json: {}", e)))?;

    let mut plugins = Vec::new();

    // moduleinfo.json structure: { "Classes": [ { "CID": "...", "Name": "...", ... } ] }
    if let Some(classes) = json.get("Classes").and_then(|c| c.as_array()) {
        // Also try to extract factory info for vendor
        let factory_vendor = json
            .get("Factory Info")
            .and_then(|f| f.get("Vendor"))
            .and_then(|v| v.as_str())
            .unwrap_or("");

        for class in classes {
            let category = class
                .get("Category")
                .and_then(|c| c.as_str())
                .unwrap_or("");

            let sub_categories = class
                .get("Sub Categories")
                .and_then(|s| s.as_str())
                .unwrap_or("");

            // Filter for audio processor classes
            if !is_audio_processor_class(category, sub_categories) {
                continue;
            }

            let name = class
                .get("Name")
                .and_then(|n| n.as_str())
                .unwrap_or("Unknown")
                .to_string();

            let vendor = class
                .get("Vendor")
                .and_then(|v| v.as_str())
                .unwrap_or(factory_vendor)
                .to_string();

            let uid = class
                .get("CID")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();

            let version = class
                .get("Version")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let category_str = if sub_categories.is_empty() {
                category.to_string()
            } else {
                sub_categories.to_string()
            };

            plugins.push(PluginInfo {
                name,
                vendor,
                uid,
                category: category_str,
                version,
                path: bundle_path.to_path_buf(),
            });
        }
    }

    Ok(plugins)
}

/// Load a .vst3 module and query the factory for class info.
pub fn scan_bundle_binary(bundle_path: &Path) -> Result<Vec<PluginInfo>, HostError> {
    let module = VstModule::load(bundle_path)?;
    let factory = module.factory();

    let mut plugins = Vec::new();

    // Get factory info for vendor name
    let factory_vendor = unsafe {
        let mut info = std::mem::zeroed::<PFactoryInfo>();
        if factory.getFactoryInfo(&mut info) == kResultOk {
            cstr_from_char8_array(&info.vendor).unwrap_or_default()
        } else {
            String::new()
        }
    };

    let class_count = unsafe { factory.countClasses() };
    debug!(
        "factory reports {} classes in {}",
        class_count,
        bundle_path.display()
    );

    // Try IPluginFactory2 for richer class info
    let factory2: Option<ComPtr<IPluginFactory2>> = factory.cast();

    for i in 0..class_count {
        if let Some(ref f2) = factory2 {
            // Use PClassInfo2 for more detail
            let mut info = unsafe { std::mem::zeroed::<PClassInfo2>() };
            let result = unsafe { f2.getClassInfo2(i, &mut info) };

            if result == kResultOk {
                let category = cstr_from_char8_array(&info.category).unwrap_or_default();
                let sub_categories =
                    cstr_from_char8_array(&info.subCategories).unwrap_or_default();

                if !is_audio_processor_class(&category, &sub_categories) {
                    continue;
                }

                let name = cstr_from_char8_array(&info.name).unwrap_or_default();
                let vendor = {
                    let v = cstr_from_char8_array(&info.vendor).unwrap_or_default();
                    if v.is_empty() {
                        factory_vendor.clone()
                    } else {
                        v
                    }
                };
                let version = cstr_from_char8_array(&info.version).unwrap_or_default();
                let uid = tuid_to_hex_string(&info.cid);

                let category_str = if sub_categories.is_empty() {
                    category
                } else {
                    sub_categories
                };

                plugins.push(PluginInfo {
                    name,
                    vendor,
                    uid,
                    category: category_str,
                    version,
                    path: bundle_path.to_path_buf(),
                });
                continue;
            }
        }

        // Fallback to PClassInfo
        let mut info = unsafe { std::mem::zeroed::<PClassInfo>() };
        let result = unsafe { factory.getClassInfo(i, &mut info) };

        if result != kResultOk {
            warn!("failed to get class info for index {} in {}", i, bundle_path.display());
            continue;
        }

        let category = cstr_from_char8_array(&info.category).unwrap_or_default();

        // With PClassInfo, we only have basic category info
        if !is_audio_processor_class(&category, "") {
            continue;
        }

        let name = cstr_from_char8_array(&info.name).unwrap_or_default();
        let uid = tuid_to_hex_string(&info.cid);

        plugins.push(PluginInfo {
            name,
            vendor: factory_vendor.clone(),
            uid,
            category,
            version: String::new(),
            path: bundle_path.to_path_buf(),
        });
    }

    Ok(plugins)
}

/// Check if a class is an audio processor based on its category and subcategories.
fn is_audio_processor_class(category: &str, sub_categories: &str) -> bool {
    // The standard VST3 category for audio processors
    if category == "Audio Module Class" {
        return true;
    }

    // Check subcategories for common audio processor indicators
    let sub_lower = sub_categories.to_lowercase();
    if sub_lower.contains("fx")
        || sub_lower.contains("instrument")
        || sub_lower.contains("audio")
    {
        return true;
    }

    false
}

/// Convert a hex string (32 hex chars) back to a TUID (16-byte array).
///
/// Returns None if the string is not exactly 32 valid hex characters.
pub fn hex_string_to_tuid(hex: &str) -> Option<[std::ffi::c_char; 16]> {
    if hex.len() != 32 {
        return None;
    }
    let mut tuid = [0i8; 16];
    for i in 0..16 {
        let byte_str = &hex[i * 2..i * 2 + 2];
        let byte = u8::from_str_radix(byte_str, 16).ok()?;
        tuid[i] = byte as i8;
    }
    Some(tuid)
}

/// Convert a TUID (16-byte array) to a hex string.
fn tuid_to_hex_string(tuid: &[std::ffi::c_char; 16]) -> String {
    tuid.iter()
        .map(|b| format!("{:02X}", *b as u8))
        .collect()
}

/// Extract a Rust String from a null-terminated char8 array.
fn cstr_from_char8_array(arr: &[std::ffi::c_char]) -> Option<String> {
    // Find the null terminator
    let len = arr.iter().position(|&c| c == 0).unwrap_or(arr.len());
    let bytes: Vec<u8> = arr[..len].iter().map(|&c| c as u8).collect();
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_scan_paths_not_empty() {
        let paths = default_scan_paths();
        assert!(!paths.is_empty(), "should return at least one scan path");
    }

    #[test]
    fn test_default_scan_paths_platform_specific() {
        let paths = default_scan_paths();

        #[cfg(target_os = "linux")]
        {
            // Should include system paths
            assert!(
                paths.iter().any(|p| p == &PathBuf::from("/usr/lib/vst3")),
                "should include /usr/lib/vst3 on Linux"
            );
        }

        #[cfg(target_os = "macos")]
        {
            assert!(
                paths
                    .iter()
                    .any(|p| p == &PathBuf::from("/Library/Audio/Plug-Ins/VST3")),
                "should include system VST3 path on macOS"
            );
        }

        #[cfg(target_os = "windows")]
        {
            assert!(
                paths
                    .iter()
                    .any(|p| p == &PathBuf::from("C:/Program Files/Common Files/VST3")),
                "should include system VST3 path on Windows"
            );
        }
    }

    #[test]
    fn test_tuid_to_hex_string() {
        let tuid: [std::ffi::c_char; 16] = [
            0x01, 0x23, 0x45, 0x67, 0x89u8 as i8, 0xABu8 as i8, 0xCDu8 as i8, 0xEFu8 as i8,
            0x01, 0x23, 0x45, 0x67, 0x89u8 as i8, 0xABu8 as i8, 0xCDu8 as i8, 0xEFu8 as i8,
        ];
        let hex = tuid_to_hex_string(&tuid);
        assert_eq!(hex, "0123456789ABCDEF0123456789ABCDEF");
    }

    #[test]
    fn test_cstr_from_char8_array() {
        let arr: [std::ffi::c_char; 8] = [b'H' as i8, b'e' as i8, b'l' as i8, b'l' as i8, b'o' as i8, 0, 0, 0];
        let s = cstr_from_char8_array(&arr);
        assert_eq!(s, Some("Hello".to_string()));
    }

    #[test]
    fn test_cstr_from_char8_array_empty() {
        let arr: [std::ffi::c_char; 4] = [0, 0, 0, 0];
        let s = cstr_from_char8_array(&arr);
        assert_eq!(s, Some(String::new()));
    }

    #[test]
    fn test_is_audio_processor_class() {
        assert!(is_audio_processor_class("Audio Module Class", ""));
        assert!(is_audio_processor_class("", "Fx|EQ"));
        assert!(is_audio_processor_class("", "Instrument|Synth"));
        assert!(!is_audio_processor_class("Component Controller Class", ""));
        assert!(!is_audio_processor_class("", ""));
    }

    #[test]
    fn test_scan_nonexistent_path() {
        let result = scan_plugins(Some("/nonexistent/path/for/testing"));
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
