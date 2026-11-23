# Tessel — Project Rules (GEMINI.md)

**Tessel** adalah aplikasi desktop lintas platform (Linux, Windows, macOS) berbasis **Pure Rust** (`wgpu`, `winit`, `egui`, `glam`, `tokio`) untuk membuat aset **Pixel, Voxel, Low-Poly, dan Karakter**, export **`.glb`** (serta format 2D/3D lainnya), dengan integrasi **Direct AI** dan **MCP**.

Acuan lengkap arsitektur, PRD, dan plan berada di `.knowledges/` (lihat `.knowledges/README.md` dan `.knowledges/docs/conventions.md`).

---

## Prinsip Rekayasa Wajib

### 1. SRP (Single Responsibility Principle) — WAJIB
- Setiap crate, file, `struct`, `trait`, dan fungsi hanya boleh memiliki **1 tanggung jawab tunggal** dan **1 alasan untuk berubah**.
- Terapkan pola **Anchoring Object Type** (`Unit` → `Sub-Unit` → `Nano-Unit`) untuk seluruh domain:
  - **Nano-Unit (`nano.rs`)**: Newtype / value object atomik (`NodeId`, `PaletteIndex`, `VoxelCoord`, `PixelCoord`).
  - **Sub-Unit (`<sub_unit>.rs`)**: Komponen internal terisolasi (`VoxelChunk`, `PixelLayer`, `Bone`).
  - **Unit Anchor (`<unit>.rs`)**: Agregat domain yang menjangkarkan Sub-Unit dan menjaga *invariants* (`VoxelVolume`, `PixelCanvas`, `LowPolyMesh`, `CharacterRig`).
  - **Transform & Mutasi**: Pisahkan algoritma murni di `ops.rs` dan mutasi state/undo di `commands.rs`.
- Perluas perilaku lintas crate menggunakan `trait` / adapter di crate konsumen (`tessel-render`, `tessel-io`, `tessel-ai`) tanpa mengacak-acak (*trashing*) file sumber di `tessel-core`.

### 2. KISS (Keep It Simple, Stupid) — WAJIB
- Pilih desain paling sederhana, datar (*flat*), dan eksplisit yang menyelesaikan masalah dengan bersih.
- Hindari hierarki abstraksi berlapis, *over-generic* yang tidak perlu, atau pola rumit yang menyulitkan pembacaan kode Rust.
- Manfaatkan fitur bawaan Rust (`struct`, `enum`, `trait`, `#[derive(...)]`) untuk meminimalkan boilerplate.

### 3. YAGNI (You Aren't Gonna Need It) — WAJIB
- Hanya tulis kode, tipe, trait, dan fitur yang **benar-benar dibutuhkan saat ini**.
- Dilarang membuat abstraksi spekulatif, hook kosong, config tak terpakai, atau fitur "untuk nanti".

---

## Aturan Reliabilitas Rust
- **Zero `unwrap()` / `expect()`** di production code; kembalikan `Result<T, E>` menggunakan `thiserror` (`unwrap()` hanya diizinkan di `#[cfg(test)]`).
- **Headless Core**: `tessel-core` dilarang bergantung pada `wgpu`, `winit`, `egui`, `tokio`, atau `reqwest`.
- **Single Command Bus**: Semua mutasi scene (UI, Direct AI, MCP) wajib melewati `Command` agar deterministik dan mendukung Undo/Redo tunggal.
- **Kualitas Build**: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, dan `cargo test --workspace` wajib hijau.
