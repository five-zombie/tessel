use std::time::Instant;

use glam::Vec3;
use tessel_core::mesh::ops::{create_extruded_polygon, create_lathe_mesh, create_loft_mesh};
use tessel_core::voxel::greedy_mesh_volume;
use tessel_core::{PaletteIndex, VoxelCoord, VoxelVolume};

fn main() {
    let mat = PaletteIndex::new(4).expect("valid palette index");

    // 1. Greedy mesh 16x16x16 solid chunk
    let mut vol16 = VoxelVolume::new();
    for x in 0..16 {
        for y in 0..16 {
            for z in 0..16 {
                vol16.set(VoxelCoord::new(x, y, z), Some(mat));
            }
        }
    }
    let t0 = Instant::now();
    let m16 = greedy_mesh_volume(&vol16, 0.1);
    let d16 = t0.elapsed();
    println!(
        "greedy_mesh_16x16x16: {:?} ({} verts, {} tris)",
        d16,
        m16.vertices.len(),
        m16.indices.len() / 3
    );

    // 2. Greedy mesh 32x32x32 solid volume (32,768 voxels)
    let mut vol32 = VoxelVolume::new();
    for x in 0..32 {
        for y in 0..32 {
            for z in 0..32 {
                vol32.set(VoxelCoord::new(x, y, z), Some(mat));
            }
        }
    }
    let t1 = Instant::now();
    let m32 = greedy_mesh_volume(&vol32, 0.1);
    let d32 = t1.elapsed();
    println!(
        "greedy_mesh_32x32x32: {:?} ({} verts, {} tris)",
        d32,
        m32.vertices.len(),
        m32.indices.len() / 3
    );

    // 3. Procedural Low-Poly Triad (Lathe + Extruded Polygon + Loft) x100 iterations
    let profile = [[0.14, 0.0], [0.15, 0.08], [0.11, 0.16], [0.0, 0.19]];
    let poly = [
        [-0.03, 0.0],
        [0.03, 0.0],
        [0.045, -0.32],
        [0.0, -0.38],
        [-0.045, -0.32],
    ];
    let sections = vec![
        vec![
            Vec3::new(0.0, 0.0, 0.2),
            Vec3::new(0.25, 0.0, 0.0),
            Vec3::new(0.0, 0.0, -0.2),
            Vec3::new(-0.25, 0.0, 0.0),
        ],
        vec![
            Vec3::new(0.0, 0.5, 0.16),
            Vec3::new(0.20, 0.5, 0.0),
            Vec3::new(0.0, 0.5, -0.16),
            Vec3::new(-0.20, 0.5, 0.0),
        ],
    ];
    let t2 = Instant::now();
    for _ in 0..100 {
        let _ = create_lathe_mesh(&profile, 16, 360.0, true, mat);
        let _ = create_extruded_polygon(&poly, 0.015, mat);
        let _ = create_loft_mesh(&sections, true, mat);
    }
    let d_proc = t2.elapsed();
    println!("procedural_triad_x100: {:?}", d_proc);
}
