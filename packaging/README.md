# Packaging & launcher icons

Source of truth: [`assets/icons/tessel-icon.png`](../assets/icons/tessel-icon.png)
(512x512). Regenerate every derived asset with:

```sh
python3 packaging/generate_icons.py   # requires Pillow
```

| Target | Asset | Wiring |
|---|---|---|
| Linux | `assets/icons/hicolor/*/apps/tessel.png` + `packaging/linux/tessel.desktop` | Installed to `/usr/share/icons` + `/usr/share/applications` by the CD Linux job (tarball carries both for manual install) |
| Windows | `assets/icons/tessel.ico` (multi-size) | Embedded into `tessel.exe` by `crates/tessel-app/build.rs` via `embed-resource` (Windows-only dep, no-op elsewhere) |
| macOS | `packaging/macos/tessel.iconset/` | Converted to `tessel.icns` with `iconutil` and referenced via `CFBundleIconFile` by the CD macOS job |

The in-window/taskbar icon is separate: it is embedded at compile time and set
in `crates/tessel-app/src/app.rs` via `crates/tessel-app/src/icon.rs`.
