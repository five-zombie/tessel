use std::io::Cursor;

use image::{ImageFormat, RgbaImage};
use serde::{Deserialize, Serialize};
use tessel_core::{MeshTexture, Palette, PixelCanvas};

use crate::error::IoError;

/// Encodes a `MeshTexture` (RGBA8) into PNG bytes.
pub fn export_mesh_texture_png_bytes(tex: &MeshTexture) -> Result<Vec<u8>, IoError> {
    let img = RgbaImage::from_raw(tex.width, tex.height, tex.rgba.clone())
        .ok_or(IoError::InvalidDimensions(tex.width, tex.height))?;
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png)?;
    Ok(buf.into_inner())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpriteFrameRect {
    pub index: usize,
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub duration_ms: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpriteSheetMetadata {
    pub sheet_width: u32,
    pub sheet_height: u32,
    pub fps: u16,
    pub frames: Vec<SpriteFrameRect>,
}

const MAX_SPRITE_DIM: u32 = 512;
const MAX_SPRITE_FRAMES: usize = 128;

/// Encodes a single frame of `PixelCanvas` to PNG bytes.
pub fn export_png_bytes(
    canvas: &PixelCanvas,
    frame_idx: usize,
    palette: &Palette,
) -> Result<Vec<u8>, IoError> {
    if canvas.width == 0
        || canvas.height == 0
        || canvas.width > MAX_SPRITE_DIM
        || canvas.height > MAX_SPRITE_DIM
    {
        return Err(IoError::InvalidDimensions(canvas.width, canvas.height));
    }
    if frame_idx >= canvas.frames.len() {
        return Err(IoError::EmptyCanvas);
    }
    let rgba = canvas.composite_frame_rgba(frame_idx, palette);
    let img = RgbaImage::from_raw(canvas.width, canvas.height, rgba)
        .ok_or(IoError::InvalidDimensions(canvas.width, canvas.height))?;
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png)?;
    Ok(buf.into_inner())
}

/// Packs all frames horizontally into a PNG sprite sheet and returns `(png_bytes, metadata)`.
pub fn export_sprite_sheet_bytes(
    canvas: &PixelCanvas,
    palette: &Palette,
) -> Result<(Vec<u8>, SpriteSheetMetadata), IoError> {
    if canvas.width == 0
        || canvas.height == 0
        || canvas.width > MAX_SPRITE_DIM
        || canvas.height > MAX_SPRITE_DIM
    {
        return Err(IoError::InvalidDimensions(canvas.width, canvas.height));
    }
    let frame_count = canvas.frames.len();
    if frame_count == 0 {
        return Err(IoError::EmptyCanvas);
    }
    if frame_count > MAX_SPRITE_FRAMES {
        return Err(IoError::PayloadTooLarge(format!(
            "sprite sheet frame count {frame_count} exceeds limit {MAX_SPRITE_FRAMES}"
        )));
    }
    let sheet_w = canvas
        .width
        .checked_mul(frame_count as u32)
        .ok_or(IoError::InvalidDimensions(canvas.width, canvas.height))?;
    let sheet_h = canvas.height;
    let total_bytes = (sheet_w as usize)
        .checked_mul(sheet_h as usize)
        .and_then(|p| p.checked_mul(4))
        .ok_or(IoError::InvalidDimensions(sheet_w, sheet_h))?;
    let expected_frame_bytes = (canvas.width as usize)
        .checked_mul(canvas.height as usize)
        .and_then(|p| p.checked_mul(4))
        .ok_or(IoError::InvalidDimensions(canvas.width, canvas.height))?;
    let mut sheet = vec![0u8; total_bytes];
    let duration_ms = 1000u32 / u32::from(canvas.fps.max(1));
    let mut rects = Vec::with_capacity(frame_count);

    for f in 0..frame_count {
        let frame_rgba = canvas.composite_frame_rgba(f, palette);
        if frame_rgba.len() < expected_frame_bytes {
            return Err(IoError::InvalidDimensions(canvas.width, canvas.height));
        }
        let x_offset = (f as u32) * canvas.width;
        let row_bytes = (canvas.width as usize) * 4;
        for y in 0..canvas.height {
            let src_start = (y as usize) * row_bytes;
            let src_end = src_start + row_bytes;
            let dst_start = ((y as usize) * (sheet_w as usize) + (x_offset as usize)) * 4;
            let dst_end = dst_start + row_bytes;
            sheet[dst_start..dst_end].copy_from_slice(&frame_rgba[src_start..src_end]);
        }
        rects.push(SpriteFrameRect {
            index: f,
            x: x_offset,
            y: 0,
            width: canvas.width,
            height: canvas.height,
            duration_ms,
        });
    }

    let img = RgbaImage::from_raw(sheet_w, sheet_h, sheet)
        .ok_or(IoError::InvalidDimensions(sheet_w, sheet_h))?;
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png)?;

    let meta = SpriteSheetMetadata {
        sheet_width: sheet_w,
        sheet_height: sheet_h,
        fps: canvas.fps,
        frames: rects,
    };
    Ok((buf.into_inner(), meta))
}

/// Encodes all frames of `PixelCanvas` into an animated looping GIF.
pub fn export_gif_bytes(canvas: &PixelCanvas, palette: &Palette) -> Result<Vec<u8>, IoError> {
    if canvas.frames.is_empty() {
        return Err(IoError::EmptyCanvas);
    }
    let w = u16::try_from(canvas.width)
        .map_err(|_| IoError::InvalidDimensions(canvas.width, canvas.height))?;
    let h = u16::try_from(canvas.height)
        .map_err(|_| IoError::InvalidDimensions(canvas.width, canvas.height))?;
    let delay_cs = (100u16 / canvas.fps.max(1)).max(2);

    let mut out = Vec::new();
    {
        let mut encoder = gif::Encoder::new(&mut out, w, h, &[])?;
        encoder.set_repeat(gif::Repeat::Infinite)?;
        for f in 0..canvas.frames.len() {
            let mut rgba = canvas.composite_frame_rgba(f, palette);
            let mut frame = gif::Frame::from_rgba_speed(w, h, &mut rgba, 10);
            frame.delay = delay_cs;
            frame.dispose = gif::DisposalMethod::Background;
            encoder.write_frame(&frame)?;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tessel_core::{PaletteIndex, PixelCoord, PixelFrame};

    #[test]
    fn png_spritesheet_and_gif_export_succeed() {
        let palette = Palette::studio_default();
        let mut canvas = PixelCanvas::new(8, 8).unwrap();
        let amber = PaletteIndex::new(4).unwrap();
        canvas.frames[0].layers[0].set(PixelCoord::new(2, 2), Some(amber));
        let next_id = canvas.allocate_layer_id();
        let mut frame2 = PixelFrame::new(next_id, 8, 8);
        frame2.layers[0].set(PixelCoord::new(5, 5), Some(amber));
        canvas.frames.push(frame2);

        let png = export_png_bytes(&canvas, 0, &palette).unwrap();
        let decoded = image::load_from_memory(&png).unwrap();
        assert_eq!(decoded.width(), 8);
        assert_eq!(decoded.height(), 8);

        let (sheet_png, meta) = export_sprite_sheet_bytes(&canvas, &palette).unwrap();
        let decoded_sheet = image::load_from_memory(&sheet_png).unwrap();
        assert_eq!(decoded_sheet.width(), 16);
        assert_eq!(decoded_sheet.height(), 8);
        assert_eq!(meta.frames.len(), 2);

        let gif_bytes = export_gif_bytes(&canvas, &palette).unwrap();
        assert!(gif_bytes.starts_with(b"GIF89a"));
    }
}
