use std::cell::Cell;

use serde::{de, Deserialize, Deserializer, Serialize};

use crate::palette::PaletteIndex;

use super::canvas::MAX_PIXEL_CANVAS_DIM;
use super::nano::{LayerId, PixelCoord};

thread_local! {
    static PIXEL_DECODE_BUDGET: Cell<Option<usize>> = const { Cell::new(None) };
}

/// Executes `f` with a cumulative pixel deserialization budget across all `PixelLayer`s
/// decoded on the current thread, preventing multi-layer RLE OOM amplification bombs.
pub fn with_pixel_decode_budget<T>(max_pixels: usize, f: impl FnOnce() -> T) -> T {
    struct ResetGuard(Option<usize>);
    impl Drop for ResetGuard {
        fn drop(&mut self) {
            PIXEL_DECODE_BUDGET.with(|b| b.set(self.0));
        }
    }
    let prev = PIXEL_DECODE_BUDGET.with(|b| b.replace(Some(max_pixels)));
    let _guard = ResetGuard(prev);
    f()
}

fn consume_decode_budget(count: usize) -> Result<(), &'static str> {
    PIXEL_DECODE_BUDGET.with(|cell| {
        if let Some(remaining) = cell.get() {
            match remaining.checked_sub(count) {
                Some(next) => {
                    cell.set(Some(next));
                    Ok(())
                }
                None => Err("total decoded pixels across scene exceed budget"),
            }
        } else {
            Ok(())
        }
    })
}

/// Sub-Unit: Single indexed 2D pixel layer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PixelLayer {
    pub id: LayerId,
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    width: u32,
    height: u32,
    #[serde(with = "rle_pixels")]
    pixels: Vec<Option<PaletteIndex>>,
}

#[derive(Deserialize)]
struct RawPixelLayer {
    id: LayerId,
    name: String,
    visible: bool,
    opacity: f32,
    width: u32,
    height: u32,
    #[serde(with = "rle_pixels")]
    pixels: Vec<Option<PaletteIndex>>,
}

impl<'de> Deserialize<'de> for PixelLayer {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawPixelLayer::deserialize(deserializer)?;
        if raw.width == 0
            || raw.height == 0
            || raw.width > MAX_PIXEL_CANVAS_DIM
            || raw.height > MAX_PIXEL_CANVAS_DIM
        {
            return Err(de::Error::custom(format!(
                "pixel layer dimensions {}x{} outside 1..={MAX_PIXEL_CANVAS_DIM}",
                raw.width, raw.height
            )));
        }
        let expected = (raw.width as usize)
            .checked_mul(raw.height as usize)
            .ok_or_else(|| de::Error::custom("pixel layer dimension overflow"))?;
        if raw.pixels.len() != expected {
            return Err(de::Error::custom(format!(
                "pixel layer length {} does not match {}x{} ({expected})",
                raw.pixels.len(),
                raw.width,
                raw.height
            )));
        }
        Ok(Self {
            id: raw.id,
            name: raw.name,
            visible: raw.visible,
            opacity: raw.opacity,
            width: raw.width,
            height: raw.height,
            pixels: raw.pixels,
        })
    }
}

/// Compact deterministic RLE serde encoding (`{"rle":[[count,color],...]}`) with full
/// backward compatibility for legacy flat JSON arrays (`[null,1,2,...]`).
mod rle_pixels {
    use std::fmt;

    use serde::de::{self, MapAccess, SeqAccess, Visitor};
    use serde::ser::{SerializeMap, SerializeSeq};
    use serde::{Deserializer, Serialize, Serializer};

    use super::{consume_decode_budget, PaletteIndex, MAX_PIXEL_CANVAS_DIM};

    const MAX_LAYER_PIXELS: usize =
        (MAX_PIXEL_CANVAS_DIM as usize) * (MAX_PIXEL_CANVAS_DIM as usize);

    struct RleRuns<'a>(&'a [Option<PaletteIndex>]);

    impl Serialize for RleRuns<'_> {
        fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            let mut seq = serializer.serialize_seq(None)?;
            let mut iter = self.0.iter();
            if let Some(&first) = iter.next() {
                let mut current = first;
                let mut count: u32 = 1;
                for &cell in iter {
                    if cell == current && count < u32::MAX {
                        count += 1;
                    } else {
                        seq.serialize_element(&(count, current))?;
                        current = cell;
                        count = 1;
                    }
                }
                seq.serialize_element(&(count, current))?;
            }
            seq.end()
        }
    }

    pub fn serialize<S: Serializer>(
        pixels: &[Option<PaletteIndex>],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        map.serialize_entry("rle", &RleRuns(pixels))?;
        map.end()
    }

    struct BoundedRleRuns(Vec<Option<PaletteIndex>>);

    impl<'de> serde::Deserialize<'de> for BoundedRleRuns {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct RunsVisitor;

            impl<'de> Visitor<'de> for RunsVisitor {
                type Value = BoundedRleRuns;

                fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    f.write_str("a sequence of [count, color] RLE pixel runs")
                }

                fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                    let mut out = Vec::new();
                    let mut total: usize = 0;
                    while let Some((count, color)) =
                        seq.next_element::<(u32, Option<PaletteIndex>)>()?
                    {
                        if count == 0 {
                            return Err(de::Error::custom("RLE pixel run count must be >= 1"));
                        }
                        let run_len = count as usize;
                        total = total
                            .checked_add(run_len)
                            .filter(|&n| n <= MAX_LAYER_PIXELS)
                            .ok_or_else(|| {
                                de::Error::custom(format!(
                                    "RLE pixel count exceeds maximum {MAX_LAYER_PIXELS}"
                                ))
                            })?;
                        consume_decode_budget(run_len).map_err(de::Error::custom)?;
                        out.resize(total, color);
                    }
                    Ok(BoundedRleRuns(out))
                }
            }

            deserializer.deserialize_seq(RunsVisitor)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Option<PaletteIndex>>, D::Error> {
        struct PixelsVisitor;

        impl<'de> Visitor<'de> for PixelsVisitor {
            type Value = Vec<Option<PaletteIndex>>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an RLE object {\"rle\": ...} or legacy flat pixel array")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(cell) = seq.next_element::<Option<PaletteIndex>>()? {
                    if out.len() >= MAX_LAYER_PIXELS {
                        return Err(de::Error::custom(format!(
                            "pixel array length exceeds maximum {MAX_LAYER_PIXELS}"
                        )));
                    }
                    consume_decode_budget(1).map_err(de::Error::custom)?;
                    out.push(cell);
                }
                Ok(out)
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut decoded: Option<Vec<Option<PaletteIndex>>> = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key == "rle" {
                        if decoded.is_some() {
                            return Err(de::Error::duplicate_field("rle"));
                        }
                        let runs: BoundedRleRuns = map.next_value()?;
                        decoded = Some(runs.0);
                    } else {
                        return Err(de::Error::unknown_field(&key, &["rle"]));
                    }
                }
                decoded.ok_or_else(|| de::Error::missing_field("rle"))
            }
        }

        deserializer.deserialize_any(PixelsVisitor)
    }
}

impl PixelLayer {
    pub fn new(id: LayerId, name: impl Into<String>, width: u32, height: u32) -> Self {
        let len = (width as usize).saturating_mul(height as usize);
        Self {
            id,
            name: name.into(),
            visible: true,
            opacity: 1.0,
            width,
            height,
            pixels: vec![None; len],
        }
    }

    pub const fn width(&self) -> u32 {
        self.width
    }

    pub const fn height(&self) -> u32 {
        self.height
    }

    fn index(&self, coord: PixelCoord) -> Option<usize> {
        if coord.x < self.width && coord.y < self.height {
            Some((coord.y as usize) * (self.width as usize) + (coord.x as usize))
        } else {
            None
        }
    }

    pub fn get(&self, coord: PixelCoord) -> Option<PaletteIndex> {
        self.index(coord)
            .and_then(|idx| self.pixels.get(idx).copied().flatten())
    }

    pub fn set(
        &mut self,
        coord: PixelCoord,
        color: Option<PaletteIndex>,
    ) -> Option<Option<PaletteIndex>> {
        let idx = self.index(coord)?;
        let slot = self.pixels.get_mut(idx)?;
        let prev = *slot;
        *slot = color;
        Some(prev)
    }

    pub fn pixels(&self) -> &[Option<PaletteIndex>] {
        &self.pixels
    }

    pub(crate) fn pixels_mut(&mut self) -> &mut [Option<PaletteIndex>] {
        &mut self.pixels
    }

    /// Encodes this layer's pixel buffer into canonical run-length runs for compact undo snapshots.
    pub fn encode_rle(&self) -> Vec<(u32, Option<PaletteIndex>)> {
        let mut runs = Vec::new();
        let mut iter = self.pixels.iter();
        if let Some(&first) = iter.next() {
            let mut current = first;
            let mut count: u32 = 1;
            for &cell in iter {
                if cell == current && count < u32::MAX {
                    count += 1;
                } else {
                    runs.push((count, current));
                    current = cell;
                    count = 1;
                }
            }
            runs.push((count, current));
        }
        runs
    }

    /// Restores this layer's pixel buffer from canonical RLE runs; returns `false` if total length mismatches.
    pub fn restore_rle(&mut self, runs: &[(u32, Option<PaletteIndex>)]) -> bool {
        let mut total: usize = 0;
        for &(count, _) in runs {
            if count == 0 {
                return false;
            }
            let Some(next) = total.checked_add(count as usize) else {
                return false;
            };
            total = next;
        }
        if total != self.pixels.len() {
            return false;
        }
        let mut offset: usize = 0;
        for &(count, color) in runs {
            let end = offset + count as usize;
            self.pixels[offset..end].fill(color);
            offset = end;
        }
        true
    }

    /// Resizes the layer to `(new_width, new_height)`, preserving overlapping top-left pixels.
    pub fn resize(&mut self, new_width: u32, new_height: u32) {
        if self.width == new_width && self.height == new_height {
            return;
        }
        let len = (new_width as usize).saturating_mul(new_height as usize);
        let mut new_pixels = vec![None; len];
        let copy_w = self.width.min(new_width) as usize;
        let copy_h = self.height.min(new_height) as usize;
        let old_w = self.width as usize;
        let new_w = new_width as usize;
        for y in 0..copy_h {
            let old_start = y.saturating_mul(old_w);
            let new_start = y.saturating_mul(new_w);
            if let (Some(src), Some(dst)) = (
                self.pixels.get(old_start..old_start.saturating_add(copy_w)),
                new_pixels.get_mut(new_start..new_start.saturating_add(copy_w)),
            ) {
                dst.copy_from_slice(src);
            }
        }
        self.width = new_width;
        self.height = new_height;
        self.pixels = new_pixels;
    }
}
