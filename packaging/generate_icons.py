#!/usr/bin/env python3
"""Regenerate all launcher icon assets from assets/icons/tessel-icon.png.

Outputs (all paths relative to the repo root):
  assets/icons/hicolor/<size>/apps/tessel.png   Linux hicolor theme icons
  assets/icons/tessel.ico                       Windows multi-size icon
  packaging/macos/tessel.iconset/*.png          macOS iconset (iconutil -> .icns)

Requires: Pillow (pip install pillow). Run: python3 packaging/generate_icons.py
"""
import sys
from pathlib import Path

try:
    from PIL import Image
except ImportError:
    sys.exit("Pillow is required: pip install pillow")

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "assets" / "icons" / "tessel-icon.png"

HICOLOR_SIZES = (16, 24, 32, 48, 64, 128, 256, 512)
ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)
# (filename, pixel size) pairs required by macOS iconutil.
ICONSET = (
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
)


def main() -> None:
    if not SRC.is_file():
        sys.exit(f"source icon missing: {SRC}")
    with Image.open(SRC) as src:
        base = src.convert("RGBA")

        for size in HICOLOR_SIZES:
            out = ROOT / "assets" / "icons" / "hicolor" / str(size) / "apps" / "tessel.png"
            out.parent.mkdir(parents=True, exist_ok=True)
            base.resize((size, size), Image.LANCZOS).save(out)
            print(f"wrote {out.relative_to(ROOT)}")

        ico = ROOT / "assets" / "icons" / "tessel.ico"
        base.save(ico, sizes=[(s, s) for s in ICO_SIZES])
        print(f"wrote {ico.relative_to(ROOT)}")

        iconset = ROOT / "packaging" / "macos" / "tessel.iconset"
        iconset.mkdir(parents=True, exist_ok=True)
        for name, size in ICONSET:
            # Source is 512px; keep it untouched for the 512 entry,
            # resample for everything else (including the 1024px @2x upscale).
            img = base if (size, name) == (512, "icon_512x512.png") else base.resize(
                (size, size), Image.LANCZOS
            )
            out = iconset / name
            img.save(out)
            print(f"wrote {out.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
