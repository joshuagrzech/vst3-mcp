//! VST3 module loading via dlopen.
//!
//! `VstModule` wraps the loading of a .vst3 bundle's shared library,
//! looks up the entry points (GetPluginFactory, InitDll, ExitDll),
//! and provides safe access to the plugin factory.

use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::path::{Path, PathBuf};

use libloading::{Library, Symbol};
use vst3::com_scrape_types::ComPtr;
use vst3::Steinberg::IPluginFactory;

use super::types::HostError;

type GetPluginFactoryFn = unsafe extern "system" fn() -> *mut c_void;
type InitDllFn = unsafe extern "system" fn() -> bool;
type ExitDllFn = unsafe extern "system" fn() -> bool;

/// A loaded VST3 module providing access to the plugin factory.
///
/// Handles dlopen of the platform-specific shared library inside a .vst3 bundle,
/// calls InitDll on load, and ExitDll on drop before unloading the library.
pub struct VstModule {
    factory: ManuallyDrop<ComPtr<IPluginFactory>>,
    _library: Library,
    _path: PathBuf,
}

impl VstModule {
    /// Load a VST3 module from a .vst3 bundle path.
    ///
    /// The bundle path should be the top-level `.vst3` directory.
    /// This function will locate the platform-specific shared library inside it.
    pub fn load(bundle_path: &Path) -> Result<Self, HostError> {
        let lib_path = find_library_path(bundle_path)?;

        // Safety: We are loading a shared library. The library must be a valid
        // VST3 plugin module.
        let library = unsafe { Library::new(&lib_path) }.map_err(|e| {
            HostError::ModuleLoadFailed(format!(
                "failed to load library {}: {}",
                lib_path.display(),
                e
            ))
        })?;

        // Call InitDll if available (optional per spec).
        unsafe {
            if let Ok(init_dll) = library.get::<InitDllFn>(b"InitDll\0") {
                init_dll();
            }
        }

        // Get the plugin factory.
        let factory = unsafe {
            let get_factory: Symbol<GetPluginFactoryFn> =
                library.get(b"GetPluginFactory\0").map_err(|e| {
                    HostError::ModuleLoadFailed(format!(
                        "GetPluginFactory not found in {}: {}",
                        lib_path.display(),
                        e
                    ))
                })?;

            let raw_factory = get_factory();
            if raw_factory.is_null() {
                return Err(HostError::ModuleLoadFailed(
                    "GetPluginFactory returned null".to_string(),
                ));
            }

            // The factory pointer is returned with a reference count already incremented,
            // so we take ownership via from_raw.
            ComPtr::<IPluginFactory>::from_raw(raw_factory as *mut IPluginFactory).ok_or_else(
                || HostError::ModuleLoadFailed("GetPluginFactory returned null pointer".to_string()),
            )?
        };

        Ok(VstModule {
            factory: ManuallyDrop::new(factory),
            _library: library,
            _path: bundle_path.to_path_buf(),
        })
    }

    /// Returns a reference to the plugin factory.
    pub fn factory(&self) -> &ComPtr<IPluginFactory> {
        &*self.factory
    }
}

impl Drop for VstModule {
    fn drop(&mut self) {
        // Drop factory COM pointer FIRST (releases reference to plugin code)
        unsafe {
            ManuallyDrop::drop(&mut self.factory);
        }
        // THEN call ExitDll
        unsafe {
            if let Ok(exit_dll) = self._library.get::<ExitDllFn>(b"ExitDll\0") {
                exit_dll();
            }
        }
        // Library handle (_library) dropped last when struct fields are dropped
    }
}

/// Find the platform-specific shared library inside a .vst3 bundle.
fn find_library_path(bundle_path: &Path) -> Result<PathBuf, HostError> {
    let bundle_name = bundle_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| {
            HostError::ModuleLoadFailed(format!(
                "invalid bundle path: {}",
                bundle_path.display()
            ))
        })?;

    #[cfg(target_os = "linux")]
    {
        // Try x86_64-linux first, then fallback paths
        let candidates = [
            bundle_path
                .join("Contents")
                .join("x86_64-linux")
                .join(format!("{}.so", bundle_name)),
            bundle_path
                .join("Contents")
                .join("aarch64-linux")
                .join(format!("{}.so", bundle_name)),
        ];

        for candidate in &candidates {
            if candidate.exists() {
                return Ok(candidate.clone());
            }
        }

        Err(HostError::ModuleLoadFailed(format!(
            "no Linux shared library found in bundle: {}",
            bundle_path.display()
        )))
    }

    #[cfg(target_os = "macos")]
    {
        let lib_path = bundle_path
            .join("Contents")
            .join("MacOS")
            .join(bundle_name);

        if lib_path.exists() {
            Ok(lib_path)
        } else {
            Err(HostError::ModuleLoadFailed(format!(
                "no macOS binary found in bundle: {}",
                bundle_path.display()
            )))
        }
    }

    #[cfg(target_os = "windows")]
    {
        let candidates = [
            bundle_path
                .join("Contents")
                .join("x86_64-win")
                .join(format!("{}.dll", bundle_name)),
            bundle_path
                .join("Contents")
                .join("x86-win")
                .join(format!("{}.dll", bundle_name)),
        ];

        for candidate in &candidates {
            if candidate.exists() {
                return Ok(candidate.clone());
            }
        }

        Err(HostError::ModuleLoadFailed(format!(
            "no Windows DLL found in bundle: {}",
            bundle_path.display()
        )))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        Err(HostError::ModuleLoadFailed(
            "unsupported platform".to_string(),
        ))
    }
}
