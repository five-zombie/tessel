# Tessel

A cross-platform desktop studio (**Linux, Windows, macOS**) built in **Pure Rust** (`wgpu`, `winit`, `egui`, `glam`, `tokio`) for creating **Pixel, Voxel, Low-Poly, and Character** assets, exporting **`.glb`** and game-ready 2D/3D formats, with built-in **Direct AI** and **MCP (Model Context Protocol)** support.

---

## Features

- **4 Integrated Creative Modes**:
  - **Pixel**: Multi-layer 2D sprite & texture editor with frame-by-frame animation, onion skinning, flood fill, X/Y symmetry, and `.png` / sprite sheet / `.gif` export.
  - **Voxel**: Sparse chunked 3D voxel editor with Greedy Meshing, per-vertex Ambient Occlusion, 3D DDA raycast picking, X/Y/Z mirror symmetry, `.vox` import/export, and **Pixel → Voxel** conversion.
  - **Low-Poly**: Lightweight flat-shaded 3D mesh editing with primitives, Möller–Trumbore face picking, face extrusion, and **Voxel → Low-Poly** conversion.
  - **Character**: Modular biped skeleton rigging, direct bone picking, pose controls, and keyframe animation playback (*Idle*, *Walk*).
- **Production Export Pipeline**:
  - `.glb` (glTF 2.0 Binary with embedded palette texture, bone hierarchy, and animations), `.obj`, `.stl`, `.vox`, `.png`, `.gif`, and atomic `.tessel` project archives.
  - One-click engine presets for **Godot 4**, **Unity**, **Unreal Engine**, **Web / Three.js**, and **3D Print**.
- **Direct AI & MCP Integration**:
  - **Direct AI** panel (OpenAI, Ollama, Gemini, Claude) with autonomous tool-calling loop and single-click undo.
  - **MCP Server** (`stdio` CLI mode and local `127.0.0.1` HTTP mode with bearer token authentication) wired to the same deterministic Command Bus.

---

## Workspace Structure

```text
tessel/
├── assets/shaders/         # WGSL shaders (grid, pixel_2d, mesh_3d, overlay)
└── crates/
    ├── tessel-core/        # Headless domain engine, meshing, rigging & Command/Undo Bus
    ├── tessel-io/          # .glb, .obj, .stl, .vox, .png, .gif & .tessel project I/O
    ├── tessel-render/      # wgpu 2D/3D viewport & overlay pipeline
    ├── tessel-ai/          # Direct AI client & MCP Server (stdio + local HTTP)
    └── tessel-app/         # Desktop GUI (winit + egui) & CLI entrypoint
```

---

## Quick Start

### Run Desktop App
```bash
cargo run -p tessel-app
```

- `1` / `2` / `3` / `4`: Switch between Pixel, Voxel, Low-Poly, and Character modes
- `Ctrl/Cmd + K`: Open Command Palette
- `Ctrl/Cmd + E`: Open Export Dialog (`.glb` & multi-format)
- `Ctrl/Cmd + S` / `Ctrl/Cmd + O`: Save / Open `.tessel` project
- `Ctrl/Cmd + Z` / `Ctrl/Cmd + Y`: Undo / Redo

### Run Headless MCP Server (`stdio`)
```bash
cargo run -p tessel-app -- mcp --export-dir ./exports
```

### Verify Build & Tests
```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

---

## AI Model Notes

- This project is built with **Gemini Neon** and **Muse Spark**.
- For generating **Low-Poly** assets via MCP, the **Gemini Neon** model delivers better results overall. **Muse Spark** works well for inanimate objects and props (such as backpacks or other hard-surface items), but is not recommended for living/organic subjects such as animals (e.g., cats) or human characters.

---

## Third-Party Notices

- **Phosphor Icons** (toolbar glyphs, via the `egui-phosphor` crate) — Copyright Helena Zhang and Tobias Fried, [MIT License](https://github.com/phosphor-icons/home/blob/main/LICENSE). Glyph names and codepoints © their authors.

---

## License

Copyright 2026 **The Five Zombie Org**

Licensed under the [Apache License, Version 2.0](./LICENSE).
