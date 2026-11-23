use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{PaletteEntry, PaletteIndex, Rgba8};

/// Unit Anchor: Deterministic color & material palette.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Palette {
    entries: BTreeMap<PaletteIndex, PaletteEntry>,
}

impl Palette {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Curated default palette ("Studio Neutral") with deterministic ordering.
    pub fn studio_default() -> Self {
        let mut palette = Self::new();
        let presets = [
            (1, "Obsidian", Rgba8::rgb(26, 29, 36)),
            (2, "Slate", Rgba8::rgb(82, 90, 106)),
            (3, "Chalk", Rgba8::rgb(225, 226, 235)),
            (4, "Amber", Rgba8::rgb(229, 138, 60)),
            (5, "Terracotta", Rgba8::rgb(198, 92, 74)),
            (6, "Sage", Rgba8::rgb(112, 168, 128)),
            (7, "Cobalt", Rgba8::rgb(78, 134, 214)),
            (8, "Ochre", Rgba8::rgb(222, 184, 92)),
        ];
        for (raw, name, color) in presets {
            if let Ok(idx) = PaletteIndex::new(raw) {
                palette.set(idx, PaletteEntry::matte(name, color));
            }
        }
        palette
    }

    pub fn get(&self, index: PaletteIndex) -> Option<&PaletteEntry> {
        self.entries.get(&index)
    }

    pub fn set(&mut self, index: PaletteIndex, entry: PaletteEntry) -> Option<PaletteEntry> {
        self.entries.insert(index, entry)
    }

    pub fn remove(&mut self, index: PaletteIndex) -> Option<PaletteEntry> {
        self.entries.remove(&index)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&PaletteIndex, &PaletteEntry)> {
        self.entries.iter()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::studio_default()
    }
}
