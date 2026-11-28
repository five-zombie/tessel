use std::collections::VecDeque;

use crate::palette::PaletteIndex;

use super::layer::PixelLayer;
use super::nano::{PixelCoord, PixelEdit, Symmetry2D};

/// Expands a coordinate according to active X/Y symmetry flags within `(width, height)`.
pub fn expand_symmetry(
    coord: PixelCoord,
    width: u32,
    height: u32,
    symmetry: Symmetry2D,
) -> Vec<PixelCoord> {
    let mut pts = Vec::with_capacity(4);
    if coord.x >= width || coord.y >= height {
        return pts;
    }
    pts.push(coord);

    let mx = width - 1 - coord.x;
    let my = height - 1 - coord.y;

    if symmetry.x && mx != coord.x {
        pts.push(PixelCoord::new(mx, coord.y));
    }
    if symmetry.y && my != coord.y {
        pts.push(PixelCoord::new(coord.x, my));
    }
    if symmetry.x && symmetry.y && (mx != coord.x || my != coord.y) {
        let diag = PixelCoord::new(mx, my);
        if !pts.contains(&diag) {
            pts.push(diag);
        }
    }
    pts
}

/// Bresenham integer line algorithm between two pixel coordinates.
pub fn line_points(start: PixelCoord, end: PixelCoord) -> Vec<PixelCoord> {
    let mut x0 = start.x as i64;
    let mut y0 = start.y as i64;
    let x1 = end.x as i64;
    let y1 = end.y as i64;

    let dx = (x1 - x0).abs();
    let dy = -(y1 - y0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    let mut out = Vec::new();
    loop {
        if x0 >= 0 && y0 >= 0 {
            out.push(PixelCoord::new(x0 as u32, y0 as u32));
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
    out
}

/// Hollow rectangle outline points between two opposite corners.
pub fn rect_outline_points(start: PixelCoord, end: PixelCoord) -> Vec<PixelCoord> {
    let min_x = start.x.min(end.x);
    let max_x = start.x.max(end.x);
    let min_y = start.y.min(end.y);
    let max_y = start.y.max(end.y);

    let mut out = Vec::new();
    for x in min_x..=max_x {
        out.push(PixelCoord::new(x, min_y));
        if max_y != min_y {
            out.push(PixelCoord::new(x, max_y));
        }
    }
    for y in (min_y + 1)..max_y {
        out.push(PixelCoord::new(min_x, y));
        if max_x != min_x {
            out.push(PixelCoord::new(max_x, y));
        }
    }
    out
}

const MAX_FLOOD_FILL_DIM: u32 = super::MAX_PIXEL_CANVAS_DIM;

/// Contiguous 4-way flood fill returning `PixelEdit`s without mutating the layer directly.
pub fn flood_fill_edits(
    layer: &PixelLayer,
    start: PixelCoord,
    replacement: Option<PaletteIndex>,
) -> Vec<PixelEdit> {
    let w = layer.width();
    let h = layer.height();
    if w == 0
        || h == 0
        || w > MAX_FLOOD_FILL_DIM
        || h > MAX_FLOOD_FILL_DIM
        || start.x >= w
        || start.y >= h
    {
        return Vec::new();
    }
    let Some(total_pixels) = (w as usize).checked_mul(h as usize) else {
        return Vec::new();
    };
    let pixels = layer.pixels();
    if pixels.len() < total_pixels {
        return Vec::new();
    }

    let w_usize = w as usize;
    let start_idx = (start.y as usize) * w_usize + (start.x as usize);
    let target = pixels[start_idx];
    if target == replacement {
        return Vec::new();
    }

    let mut visited = vec![0u64; total_pixels.div_ceil(64)];
    let mut queue = VecDeque::new();
    let mut edits = Vec::new();

    visited[start_idx / 64] |= 1u64 << (start_idx % 64);
    queue.push_back(start);

    while let Some(c) = queue.pop_front() {
        edits.push(PixelEdit {
            coord: c,
            color: replacement,
        });

        let neighbors = [
            c.x.checked_sub(1).map(|nx| PixelCoord::new(nx, c.y)),
            (c.x + 1 < w).then(|| PixelCoord::new(c.x + 1, c.y)),
            c.y.checked_sub(1).map(|ny| PixelCoord::new(c.x, ny)),
            (c.y + 1 < h).then(|| PixelCoord::new(c.x, c.y + 1)),
        ];

        for next in neighbors.into_iter().flatten() {
            let idx = (next.y as usize) * w_usize + (next.x as usize);
            let word = idx / 64;
            let mask = 1u64 << (idx % 64);
            if (visited[word] & mask) == 0 && pixels[idx] == target {
                visited[word] |= mask;
                queue.push_back(next);
            }
        }
    }

    edits
}

/// Contiguous 4-way flood fill mutating `layer` in-place with a packed `u64` bitset (`<= 2 MiB` at 4K)
/// and zero intermediate `Vec<PixelEdit>` allocation. Returns the exact number of pixels painted.
pub fn flood_fill_layer_in_place(
    layer: &mut PixelLayer,
    start: PixelCoord,
    replacement: Option<PaletteIndex>,
) -> usize {
    let w = layer.width();
    let h = layer.height();
    if w == 0
        || h == 0
        || w > MAX_FLOOD_FILL_DIM
        || h > MAX_FLOOD_FILL_DIM
        || start.x >= w
        || start.y >= h
    {
        return 0;
    }
    let Some(total_pixels) = (w as usize).checked_mul(h as usize) else {
        return 0;
    };
    let pixels = layer.pixels_mut();
    if pixels.len() < total_pixels {
        return 0;
    }

    let w_usize = w as usize;
    let start_idx = (start.y as usize) * w_usize + (start.x as usize);
    let target = pixels[start_idx];
    if target == replacement {
        return 0;
    }

    let mut visited = vec![0u64; total_pixels.div_ceil(64)];
    let mut queue = VecDeque::new();
    let mut filled: usize = 0;

    visited[start_idx / 64] |= 1u64 << (start_idx % 64);
    queue.push_back(start);

    while let Some(c) = queue.pop_front() {
        let cur_idx = (c.y as usize) * w_usize + (c.x as usize);
        pixels[cur_idx] = replacement;
        filled += 1;

        let neighbors = [
            c.x.checked_sub(1).map(|nx| PixelCoord::new(nx, c.y)),
            (c.x + 1 < w).then(|| PixelCoord::new(c.x + 1, c.y)),
            c.y.checked_sub(1).map(|ny| PixelCoord::new(c.x, ny)),
            (c.y + 1 < h).then(|| PixelCoord::new(c.x, c.y + 1)),
        ];

        for next in neighbors.into_iter().flatten() {
            let idx = (next.y as usize) * w_usize + (next.x as usize);
            let word = idx / 64;
            let mask = 1u64 << (idx % 64);
            if (visited[word] & mask) == 0 && pixels[idx] == target {
                visited[word] |= mask;
                queue.push_back(next);
            }
        }
    }

    filled
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pixel::LayerId;

    #[test]
    fn symmetry_expands_four_quadrants() {
        let pts = expand_symmetry(
            PixelCoord::new(1, 2),
            16,
            16,
            Symmetry2D { x: true, y: true },
        );
        assert_eq!(pts.len(), 4);
        assert!(pts.contains(&PixelCoord::new(14, 2)));
        assert!(pts.contains(&PixelCoord::new(1, 13)));
        assert!(pts.contains(&PixelCoord::new(14, 13)));
    }

    #[test]
    fn flood_fill_stays_within_enclosed_region() {
        let mut layer = PixelLayer::new(LayerId(1), "L1", 5, 5);
        let wall = PaletteIndex::new(1).unwrap();
        let fill = PaletteIndex::new(4).unwrap();
        for p in rect_outline_points(PixelCoord::new(0, 0), PixelCoord::new(4, 4)) {
            layer.set(p, Some(wall));
        }
        let edits = flood_fill_edits(&layer, PixelCoord::new(2, 2), Some(fill));
        assert_eq!(edits.len(), 9);
    }

    #[test]
    fn flood_fill_non_square_crosses_u64_word_boundaries() {
        let mut layer = PixelLayer::new(LayerId(1), "L1", 17, 13);
        let wall = PaletteIndex::new(1).unwrap();
        let fill = PaletteIndex::new(2).unwrap();

        // Enclose a 3x3 island at top-right so 9 pixels (8 walls + 1 interior) are excluded.
        for p in rect_outline_points(PixelCoord::new(14, 0), PixelCoord::new(16, 2)) {
            layer.set(p, Some(wall));
        }

        let edits = flood_fill_edits(&layer, PixelCoord::new(0, 0), Some(fill));
        assert_eq!(edits.len(), 17 * 13 - 9);

        let coords: std::collections::HashSet<PixelCoord> = edits
            .iter()
            .map(|e| {
                assert_eq!(e.color, Some(fill));
                e.coord
            })
            .collect();
        assert_eq!(coords.len(), edits.len());

        // Verify coordinates straddling 64-bit word boundaries (63/64, 127/128, 191/192) are visited.
        for boundary_coord in [
            PixelCoord::new(12, 3),
            PixelCoord::new(13, 3),
            PixelCoord::new(8, 7),
            PixelCoord::new(9, 7),
            PixelCoord::new(4, 11),
            PixelCoord::new(5, 11),
        ] {
            assert!(coords.contains(&boundary_coord));
        }
        assert!(!coords.contains(&PixelCoord::new(15, 1)));
    }

    #[test]
    fn flood_fill_empty_when_target_equals_replacement_or_start_oob() {
        let mut layer = PixelLayer::new(LayerId(1), "L1", 5, 5);
        let color = PaletteIndex::new(3).unwrap();
        layer.set(PixelCoord::new(2, 2), Some(color));

        assert!(flood_fill_edits(&layer, PixelCoord::new(0, 0), None).is_empty());
        assert!(flood_fill_edits(&layer, PixelCoord::new(2, 2), Some(color)).is_empty());
        assert!(flood_fill_edits(&layer, PixelCoord::new(5, 0), Some(color)).is_empty());
        assert!(flood_fill_edits(&layer, PixelCoord::new(0, 5), Some(color)).is_empty());
        assert!(flood_fill_edits(&layer, PixelCoord::new(10, 10), Some(color)).is_empty());
    }
}
