//! .vstpreset binary format read/write.
//!
//! Implements the Steinberg .vstpreset binary format:
//! - 48-byte header: magic "VST3", version, class_id (32 ASCII hex), chunk_list_offset
//! - Data area: processor state chunk ("Comp"), optional controller state chunk ("Cont")
//! - Chunk list at end: "List" magic, count, entries with chunk IDs, offsets, sizes
//!
//! All integers are little-endian.

use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::path::Path;

use crate::hosting::types::HostError;

/// Magic bytes for the .vstpreset format.
const VSTPRESET_MAGIC: &[u8; 4] = b"VST3";
/// Magic bytes for the chunk list.
const CHUNK_LIST_MAGIC: &[u8; 4] = b"List";
/// Chunk ID for component (processor) state.
const CHUNK_ID_COMP: &[u8; 4] = b"Comp";
/// Chunk ID for controller state.
const CHUNK_ID_CONT: &[u8; 4] = b"Cont";
/// Current format version.
const VSTPRESET_VERSION: i32 = 1;
/// Header size: 4 (magic) + 4 (version) + 32 (class_id) + 8 (chunk_list_offset) = 48.
const HEADER_SIZE: usize = 48;

/// Data loaded from a .vstpreset file.
#[derive(Debug, Clone)]
pub struct PresetData {
    /// The class ID as 32-byte ASCII hex.
    pub class_id: [u8; 32],
    /// Component (processor) state bytes.
    pub component_state: Vec<u8>,
    /// Optional controller state bytes.
    pub controller_state: Option<Vec<u8>>,
}

/// A chunk list entry.
struct ChunkEntry {
    id: [u8; 4],
    offset: i64,
    size: i64,
}

/// Save a preset to a .vstpreset file.
///
/// # Arguments
/// * `path` - Output file path
/// * `class_id` - 32-byte ASCII hex class ID
/// * `component_state` - Processor state bytes from IComponent::getState()
/// * `controller_state` - Optional controller state bytes from IEditController::getState()
pub fn save_preset(
    path: &Path,
    class_id: &[u8; 32],
    component_state: &[u8],
    controller_state: Option<&[u8]>,
) -> Result<(), HostError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| HostError::PresetError(format!("failed to create preset directory: {}", e)))?;
    }
    let file = std::fs::File::create(path)
        .map_err(|e| HostError::PresetError(format!("failed to create preset file: {}", e)))?;
    let mut writer = std::io::BufWriter::new(file);

    // Write header with placeholder for chunk_list_offset
    writer
        .write_all(VSTPRESET_MAGIC)
        .map_err(|e| HostError::PresetError(format!("write error: {}", e)))?;
    writer
        .write_all(&VSTPRESET_VERSION.to_le_bytes())
        .map_err(|e| HostError::PresetError(format!("write error: {}", e)))?;
    writer
        .write_all(class_id)
        .map_err(|e| HostError::PresetError(format!("write error: {}", e)))?;
    // Placeholder for chunk_list_offset (will patch later)
    let chunk_list_offset_pos = HEADER_SIZE - 8; // offset within file where we write the i64
    writer
        .write_all(&0i64.to_le_bytes())
        .map_err(|e| HostError::PresetError(format!("write error: {}", e)))?;

    // Track chunks
    let mut chunks = Vec::new();

    // Write component state chunk
    let comp_offset = HEADER_SIZE as i64;
    writer
        .write_all(component_state)
        .map_err(|e| HostError::PresetError(format!("write error: {}", e)))?;
    chunks.push(ChunkEntry {
        id: *CHUNK_ID_COMP,
        offset: comp_offset,
        size: component_state.len() as i64,
    });

    // Write controller state chunk (optional)
    if let Some(ctrl_state) = controller_state {
        let cont_offset = comp_offset + component_state.len() as i64;
        writer
            .write_all(ctrl_state)
            .map_err(|e| HostError::PresetError(format!("write error: {}", e)))?;
        chunks.push(ChunkEntry {
            id: *CHUNK_ID_CONT,
            offset: cont_offset,
            size: ctrl_state.len() as i64,
        });
    }

    // Record where the chunk list starts
    let chunk_list_offset = HEADER_SIZE as i64
        + component_state.len() as i64
        + controller_state.map_or(0, |s| s.len() as i64);

    // Write chunk list
    writer
        .write_all(CHUNK_LIST_MAGIC)
        .map_err(|e| HostError::PresetError(format!("write error: {}", e)))?;
    writer
        .write_all(&(chunks.len() as i32).to_le_bytes())
        .map_err(|e| HostError::PresetError(format!("write error: {}", e)))?;

    for chunk in &chunks {
        writer
            .write_all(&chunk.id)
            .map_err(|e| HostError::PresetError(format!("write error: {}", e)))?;
        writer
            .write_all(&chunk.offset.to_le_bytes())
            .map_err(|e| HostError::PresetError(format!("write error: {}", e)))?;
        writer
            .write_all(&chunk.size.to_le_bytes())
            .map_err(|e| HostError::PresetError(format!("write error: {}", e)))?;
    }

    // Flush before seeking
    writer
        .flush()
        .map_err(|e| HostError::PresetError(format!("flush error: {}", e)))?;

    // Patch chunk_list_offset in header
    let mut file = writer.into_inner()
        .map_err(|e| HostError::PresetError(format!("flush error: {}", e)))?;
    file.seek(SeekFrom::Start(chunk_list_offset_pos as u64))
        .map_err(|e| HostError::PresetError(format!("seek error: {}", e)))?;
    file.write_all(&chunk_list_offset.to_le_bytes())
        .map_err(|e| HostError::PresetError(format!("write error: {}", e)))?;

    Ok(())
}

/// Load a preset from a .vstpreset file.
pub fn load_preset(path: &Path) -> Result<PresetData, HostError> {
    let data = std::fs::read(path)
        .map_err(|e| HostError::PresetError(format!("failed to read preset file: {}", e)))?;

    if data.len() < HEADER_SIZE {
        return Err(HostError::PresetError(
            "preset file too small for header".to_string(),
        ));
    }

    let mut cursor = Cursor::new(&data);

    // Read and validate header
    let mut magic = [0u8; 4];
    cursor
        .read_exact(&mut magic)
        .map_err(|e| HostError::PresetError(format!("read error: {}", e)))?;
    if &magic != VSTPRESET_MAGIC {
        return Err(HostError::PresetError(format!(
            "invalid magic: expected VST3, got {:?}",
            magic
        )));
    }

    let mut version_bytes = [0u8; 4];
    cursor
        .read_exact(&mut version_bytes)
        .map_err(|e| HostError::PresetError(format!("read error: {}", e)))?;
    let version = i32::from_le_bytes(version_bytes);
    if version != VSTPRESET_VERSION {
        return Err(HostError::PresetError(format!(
            "unsupported version: {}",
            version
        )));
    }

    let mut class_id = [0u8; 32];
    cursor
        .read_exact(&mut class_id)
        .map_err(|e| HostError::PresetError(format!("read error: {}", e)))?;

    let mut offset_bytes = [0u8; 8];
    cursor
        .read_exact(&mut offset_bytes)
        .map_err(|e| HostError::PresetError(format!("read error: {}", e)))?;
    let chunk_list_offset = i64::from_le_bytes(offset_bytes);

    // Seek to chunk list
    cursor
        .seek(SeekFrom::Start(chunk_list_offset as u64))
        .map_err(|e| HostError::PresetError(format!("seek error: {}", e)))?;

    // Read chunk list header
    let mut list_magic = [0u8; 4];
    cursor
        .read_exact(&mut list_magic)
        .map_err(|e| HostError::PresetError(format!("read error: {}", e)))?;
    if &list_magic != CHUNK_LIST_MAGIC {
        return Err(HostError::PresetError(format!(
            "invalid chunk list magic: expected List, got {:?}",
            list_magic
        )));
    }

    let mut count_bytes = [0u8; 4];
    cursor
        .read_exact(&mut count_bytes)
        .map_err(|e| HostError::PresetError(format!("read error: {}", e)))?;
    let chunk_count = i32::from_le_bytes(count_bytes);

    // Read chunk entries
    let mut component_state = None;
    let mut controller_state = None;

    for _ in 0..chunk_count {
        let mut chunk_id = [0u8; 4];
        cursor
            .read_exact(&mut chunk_id)
            .map_err(|e| HostError::PresetError(format!("read error: {}", e)))?;

        let mut offset_bytes = [0u8; 8];
        cursor
            .read_exact(&mut offset_bytes)
            .map_err(|e| HostError::PresetError(format!("read error: {}", e)))?;
        let offset = i64::from_le_bytes(offset_bytes) as usize;

        let mut size_bytes = [0u8; 8];
        cursor
            .read_exact(&mut size_bytes)
            .map_err(|e| HostError::PresetError(format!("read error: {}", e)))?;
        let size = i64::from_le_bytes(size_bytes) as usize;

        // Validate bounds
        if offset + size > data.len() {
            return Err(HostError::PresetError(format!(
                "chunk {:?} exceeds file bounds (offset={}, size={}, file_size={})",
                chunk_id, offset, size, data.len()
            )));
        }

        let chunk_data = data[offset..offset + size].to_vec();

        if &chunk_id == CHUNK_ID_COMP {
            component_state = Some(chunk_data);
        } else if &chunk_id == CHUNK_ID_CONT {
            controller_state = Some(chunk_data);
        }
    }

    let component_state = component_state.ok_or_else(|| {
        HostError::PresetError("no component state chunk found in preset".to_string())
    })?;

    Ok(PresetData {
        class_id,
        component_state,
        controller_state,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_preset_round_trip() {
        let dir = std::env::temp_dir().join("vst3_mcp_test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.vstpreset");

        let class_id: [u8; 32] = *b"0123456789ABCDEF0123456789ABCDEF";
        let comp_state = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let ctrl_state = vec![10, 20, 30];

        // Save
        save_preset(&path, &class_id, &comp_state, Some(&ctrl_state)).unwrap();

        // Load
        let loaded = load_preset(&path).unwrap();

        assert_eq!(loaded.class_id, class_id);
        assert_eq!(loaded.component_state, comp_state);
        assert_eq!(loaded.controller_state, Some(ctrl_state));

        // Cleanup
        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn test_preset_round_trip_no_controller_state() {
        let dir = std::env::temp_dir().join("vst3_mcp_test2");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test_no_ctrl.vstpreset");

        let class_id: [u8; 32] = *b"AAAABBBBCCCCDDDDEEEEFFFFGGGGHHHH";
        let comp_state = vec![100, 200, 255, 0, 1];

        save_preset(&path, &class_id, &comp_state, None).unwrap();

        let loaded = load_preset(&path).unwrap();

        assert_eq!(loaded.class_id, class_id);
        assert_eq!(loaded.component_state, comp_state);
        assert_eq!(loaded.controller_state, None);

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn test_preset_invalid_magic() {
        let dir = std::env::temp_dir().join("vst3_mcp_test3");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad_magic.vstpreset");

        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(b"BAD!").unwrap();
        file.write_all(&[0u8; 44]).unwrap(); // Fill rest of "header"
        drop(file);

        let result = load_preset(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid magic"));

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn test_preset_file_too_small() {
        let dir = std::env::temp_dir().join("vst3_mcp_test4");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("too_small.vstpreset");

        std::fs::write(&path, b"VST3").unwrap();

        let result = load_preset(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too small"));

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&dir).ok();
    }

    #[test]
    fn test_preset_large_data() {
        let dir = std::env::temp_dir().join("vst3_mcp_test5");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("large.vstpreset");

        let class_id: [u8; 32] = *b"0123456789ABCDEF0123456789ABCDEF";
        let comp_state: Vec<u8> = (0..10000).map(|i| (i % 256) as u8).collect();
        let ctrl_state: Vec<u8> = (0..5000).map(|i| ((i * 7) % 256) as u8).collect();

        save_preset(&path, &class_id, &comp_state, Some(&ctrl_state)).unwrap();
        let loaded = load_preset(&path).unwrap();

        assert_eq!(loaded.component_state, comp_state);
        assert_eq!(loaded.controller_state, Some(ctrl_state));

        std::fs::remove_file(&path).ok();
        std::fs::remove_dir(&dir).ok();
    }
}
