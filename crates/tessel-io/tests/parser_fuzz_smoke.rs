//! Deterministic fuzz-harness tests for untrusted file parsers (`.tessel`, `.glb`, `.obj`, `.vox`).
//! Verifies zero panics across malformed, truncated, boundary, and mutated inputs (Phase J4).

use tessel_core::PaletteIndex;
use tessel_io::{decode_tessel_project, import_glb_bytes, import_obj_str, import_vox_bytes};

struct XorShift64(u64);

impl XorShift64 {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn fill_bytes(&mut self, buf: &mut [u8]) {
        for b in buf.iter_mut() {
            *b = (self.next_u64() & 0xff) as u8;
        }
    }
}

#[test]
fn fuzz_untrusted_tessel_glb_vox_obj_parsers_never_panic() {
    let mut rng = XorShift64(0x5EED_CAFE_BEEF_1234);
    let default_mat = PaletteIndex::DEFAULT_ACTIVE;

    // 1. Seed corpora (headers, magic bytes, degenerate structures)
    let binary_seeds: Vec<Vec<u8>> = vec![
        vec![],
        vec![0u8; 4],
        b"glTF\x02\x00\x00\x00\xff\xff\xff\xff".to_vec(),
        b"glTF\x02\x00\x00\x00\x14\x00\x00\x00\x04\x00\x00\x00JSON{}".to_vec(),
        b"VOX \x96\x00\x00\x00MAIN\x00\x00\x00\x00\x00\x00\x00\x00".to_vec(),
        b"VOX \x96\x00\x00\x00MAIN\xff\xff\xff\x7f\xff\xff\xff\x7fSIZE\x0c\x00\x00\x00\x00\x00\x00\x00\xff\xff\x00\x00\xff\xff\x00\x00\xff\xff\x00\x00".to_vec(),
        b"PK\x03\x04\x14\x00\x00\x00\x00\x00\x00\x00\x00\x00".to_vec(),
    ];

    for seed in &binary_seeds {
        let _ = decode_tessel_project(seed);
        let _ = import_glb_bytes(seed, default_mat);
        let _ = import_vox_bytes(seed);
    }

    // 2. Random & mutated binary payloads (256 iterations)
    for i in 0..256usize {
        let len = (rng.next_u64() as usize) % 2048;
        let mut payload = vec![0u8; len];
        rng.fill_bytes(&mut payload);

        // Occasionally inject valid magic headers to penetrate deeper parser branches
        if i % 4 == 0 && payload.len() >= 8 {
            payload[0..4].copy_from_slice(b"VOX ");
            payload[4..8].copy_from_slice(&150u32.to_le_bytes());
        } else if i % 4 == 1 && payload.len() >= 12 {
            payload[0..4].copy_from_slice(b"glTF");
            payload[4..8].copy_from_slice(&2u32.to_le_bytes());
            let total = payload.len() as u32;
            payload[8..12].copy_from_slice(&total.to_le_bytes());
        } else if i % 4 == 2 && payload.len() >= 4 {
            payload[0..4].copy_from_slice(b"PK\x03\x04");
        }

        let _ = decode_tessel_project(&payload);
        let _ = import_glb_bytes(&payload, default_mat);
        let _ = import_vox_bytes(&payload);
    }

    // 3. Adversarial OBJ inputs (negative indices, NaN/Inf, huge faces, UTF-8 garbage)
    let obj_seeds = [
        "",
        "v nan inf -inf\nf 1 2 3\n",
        "v 0 0 0\nv 1 0 0\nv 0 1 0\nf 0 0 0\nf -999999 -1 -2\nf 999999/1/1 2/2/2 3/3/3\n",
        "f 1// 2// 3//\nvt 0.5\nvn 0 0\n",
        "v 1e308 1e308 1e308\nv -1e308 0 0\nv 0 1e308 0\nf 1 2 3 4 5 6 7 8 9 10\n",
    ];
    for s in obj_seeds {
        let _ = import_obj_str(s, default_mat);
    }

    for _ in 0..128 {
        let len = (rng.next_u64() as usize) % 512;
        let mut bytes = vec![0u8; len];
        rng.fill_bytes(&mut bytes);
        let text = String::from_utf8_lossy(&bytes);
        let _ = import_obj_str(&text, default_mat);
    }
}
