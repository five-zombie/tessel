use tessel_core::{Palette, PaletteEntry, PaletteIndex, Rgba8, VoxelCoord, VoxelVolume};

use crate::error::IoError;

/// Exports a `VoxelVolume` + `Palette` into MagicaVoxel `.vox` binary format (version 150).
pub fn export_vox_bytes(volume: &VoxelVolume, palette: &Palette) -> Result<Vec<u8>, IoError> {
    let Some((min_b, max_b)) = volume.bounds() else {
        return Err(IoError::EmptyCanvas);
    };
    let dx = i64::from(max_b.x) - i64::from(min_b.x) + 1;
    let dy = i64::from(max_b.y) - i64::from(min_b.y) + 1;
    let dz = i64::from(max_b.z) - i64::from(min_b.z) + 1;
    if dx <= 0 || dy <= 0 || dz <= 0 || dx > 256 || dy > 256 || dz > 256 {
        return Err(IoError::InvalidDimensions(
            dx.clamp(0, u32::MAX as i64) as u32,
            dy.clamp(0, u32::MAX as i64) as u32,
        ));
    }
    let size_x = dx as u32;
    let size_y = dy as u32;
    let size_z = dz as u32;

    // MagicaVoxel uses Z-up; map Tessel (X, Y-up, Z) -> MagicaVoxel (X, Z, Y)
    let mut size_chunk = Vec::with_capacity(12);
    size_chunk.extend_from_slice(&size_x.to_le_bytes());
    size_chunk.extend_from_slice(&size_z.to_le_bytes());
    size_chunk.extend_from_slice(&size_y.to_le_bytes());

    let voxels = volume.iter_voxels();
    let mut xyzi_chunk = Vec::with_capacity(4 + voxels.len() * 4);
    xyzi_chunk.extend_from_slice(&(voxels.len() as u32).to_le_bytes());
    for (coord, color) in voxels {
        let x = (coord.x - min_b.x) as u8;
        let y = (coord.z - min_b.z) as u8;
        let z = (coord.y - min_b.y) as u8;
        xyzi_chunk.push(x);
        xyzi_chunk.push(y);
        xyzi_chunk.push(z);
        xyzi_chunk.push(color.get());
    }

    let mut rgba_chunk = vec![0u8; 256 * 4];
    for raw in 1u8..=255 {
        if let Ok(idx) = PaletteIndex::new(raw) {
            if let Some(entry) = palette.get(idx) {
                let base = (usize::from(raw) - 1) * 4;
                rgba_chunk[base] = entry.color.r;
                rgba_chunk[base + 1] = entry.color.g;
                rgba_chunk[base + 2] = entry.color.b;
                rgba_chunk[base + 3] = entry.color.a;
            }
        }
    }

    let mut children = Vec::new();
    write_chunk(&mut children, b"SIZE", &size_chunk);
    write_chunk(&mut children, b"XYZI", &xyzi_chunk);
    write_chunk(&mut children, b"RGBA", &rgba_chunk);

    let mut out = Vec::with_capacity(20 + children.len());
    out.extend_from_slice(b"VOX ");
    out.extend_from_slice(&150u32.to_le_bytes());
    out.extend_from_slice(b"MAIN");
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(children.len() as u32).to_le_bytes());
    out.extend_from_slice(&children);
    Ok(out)
}

const MAX_VOX_FILE_BYTES: usize = 64 * 1024 * 1024;
const MAX_VOX_VOXELS: usize = 2_000_000;

/// Reads a MagicaVoxel `.vox` binary buffer into `(VoxelVolume, Palette)`.
pub fn import_vox_bytes(bytes: &[u8]) -> Result<(VoxelVolume, Palette), IoError> {
    if bytes.len() > MAX_VOX_FILE_BYTES {
        return Err(IoError::PayloadTooLarge(
            "VOX file exceeds 64 MiB safety limit".into(),
        ));
    }
    if bytes.len() < 20 || &bytes[0..4] != b"VOX " {
        return Err(IoError::InvalidFormat("not a valid VOX header".into()));
    }
    let mut volume = VoxelVolume::new();
    let mut palette = Palette::studio_default();
    let mut total_voxels = 0usize;

    let mut offset = 8usize;
    if &bytes[offset..offset + 4] != b"MAIN" {
        return Err(IoError::InvalidFormat("missing MAIN chunk".into()));
    }
    let main_content = read_u32_le(bytes, offset + 4)? as usize;
    let main_children = read_u32_le(bytes, offset + 8)? as usize;
    offset = offset
        .checked_add(12)
        .and_then(|o| o.checked_add(main_content))
        .ok_or_else(|| IoError::InvalidFormat("VOX MAIN chunk offset overflow".into()))?;
    let end = offset.saturating_add(main_children).min(bytes.len());

    while offset.saturating_add(12) <= end {
        let id = &bytes[offset..offset + 4];
        let content_len = read_u32_le(bytes, offset + 4)? as usize;
        let children_len = read_u32_le(bytes, offset + 8)? as usize;
        let content_start = offset + 12;
        let Some(content_end) = content_start.checked_add(content_len) else {
            break;
        };
        if content_end > end {
            break;
        }
        let content = &bytes[content_start..content_end];

        match id {
            b"XYZI" if content.len() >= 4 => {
                let raw_count = read_u32_le(content, 0)? as usize;
                let max_count = (content.len() - 4) / 4;
                let count = raw_count.min(max_count);
                if total_voxels.saturating_add(count) > MAX_VOX_VOXELS {
                    return Err(IoError::PayloadTooLarge(format!(
                        "VOX voxel count exceeds limit {MAX_VOX_VOXELS}"
                    )));
                }
                for i in 0..count {
                    let Some(base) = i.checked_mul(4).and_then(|o| o.checked_add(4)) else {
                        break;
                    };
                    let Some(entry) = content.get(base..base + 4) else {
                        break;
                    };
                    let vx = i32::from(entry[0]);
                    let vz = i32::from(entry[1]);
                    let vy = i32::from(entry[2]);
                    if let Ok(idx) = PaletteIndex::new(entry[3]) {
                        volume.set(VoxelCoord::new(vx, vy, vz), Some(idx));
                        total_voxels += 1;
                    }
                }
            }
            b"RGBA" if content.len() >= 256 * 4 => {
                let mut loaded = Palette::new();
                for raw in 1u8..=255 {
                    let base = (usize::from(raw) - 1) * 4;
                    let a = content[base + 3];
                    if a > 0 {
                        if let Ok(idx) = PaletteIndex::new(raw) {
                            loaded.set(
                                idx,
                                PaletteEntry::matte(
                                    format!("Color {raw}"),
                                    Rgba8::rgba(
                                        content[base],
                                        content[base + 1],
                                        content[base + 2],
                                        a,
                                    ),
                                ),
                            );
                        }
                    }
                }
                if !loaded.is_empty() {
                    palette = loaded;
                }
            }
            _ => {}
        }

        let Some(next_offset) = content_end.checked_add(children_len) else {
            break;
        };
        offset = next_offset;
    }

    Ok((volume, palette))
}

fn write_chunk(dst: &mut Vec<u8>, id: &[u8; 4], content: &[u8]) {
    dst.extend_from_slice(id);
    dst.extend_from_slice(&(content.len() as u32).to_le_bytes());
    dst.extend_from_slice(&0u32.to_le_bytes());
    dst.extend_from_slice(content);
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, IoError> {
    let slice = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| IoError::InvalidFormat("unexpected EOF in VOX".into()))?;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vox_export_and_import_roundtrip() {
        let palette = Palette::studio_default();
        let mut vol = VoxelVolume::new();
        let amber = PaletteIndex::new(4).unwrap();
        let cobalt = PaletteIndex::new(7).unwrap();
        vol.set(VoxelCoord::new(0, 0, 0), Some(amber));
        vol.set(VoxelCoord::new(2, 3, 1), Some(cobalt));

        let bytes = export_vox_bytes(&vol, &palette).unwrap();
        let (loaded_vol, loaded_pal) = import_vox_bytes(&bytes).unwrap();

        assert_eq!(loaded_vol.voxel_count(), 2);
        assert_eq!(loaded_vol.get(VoxelCoord::new(0, 0, 0)), Some(amber));
        assert_eq!(loaded_vol.get(VoxelCoord::new(2, 3, 1)), Some(cobalt));
        assert_eq!(
            loaded_pal.get(amber).unwrap().color,
            palette.get(amber).unwrap().color
        );
    }
}
