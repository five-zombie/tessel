use std::time::Instant;

use glam::Vec3;
use tessel_core::mesh::ops::{create_extruded_polygon, create_lathe_mesh, create_loft_mesh};
use tessel_core::pixel::ops::flood_fill_edits;
use tessel_core::voxel::greedy_mesh_volume;
use tessel_core::voxel::ops::{box_fill_edits, ellipsoid_fill_edits};
use tessel_core::{PaletteIndex, PixelCanvas, PixelCoord, VoxelCoord, VoxelVolume};

fn main() {
    let mat = match PaletteIndex::new(4) {
        Ok(idx) => idx,
        Err(err) => {
            eprintln!("invalid benchmark palette index: {err}");
            return;
        }
    };

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

    // 4. Greedy mesh 32x32x32 sparse checkerboard volume (worst-case face count)
    let mut vol_checker = VoxelVolume::new();
    for x in 0..32 {
        for y in 0..32 {
            for z in 0..32 {
                if (x + y + z) % 2 == 0 {
                    vol_checker.set(VoxelCoord::new(x, y, z), Some(mat));
                }
            }
        }
    }
    let t3 = Instant::now();
    let m_checker = greedy_mesh_volume(&vol_checker, 0.1);
    let d_checker = t3.elapsed();
    println!(
        "greedy_mesh_32x32x32_checker: {:?} ({} verts, {} tris)",
        d_checker,
        m_checker.vertices.len(),
        m_checker.indices.len() / 3
    );

    // 5. Voxel box_fill_edits (32x32x32 solid + hollow)
    let t4 = Instant::now();
    let box_solid = box_fill_edits(
        VoxelCoord::new(0, 0, 0),
        VoxelCoord::new(31, 31, 31),
        Some(mat),
        false,
    );
    let box_hollow = box_fill_edits(
        VoxelCoord::new(0, 0, 0),
        VoxelCoord::new(31, 31, 31),
        Some(mat),
        true,
    );
    let d_box = t4.elapsed();
    println!(
        "voxel_box_fill_32x32x32: {:?} (solid={}, hollow={})",
        d_box,
        box_solid.len(),
        box_hollow.len()
    );

    // 6. Voxel ellipsoid_fill_edits (radius 16)
    let t5 = Instant::now();
    let ellipsoid = ellipsoid_fill_edits(VoxelCoord::new(0, 0, 0), [16.0, 16.0, 16.0], Some(mat));
    let d_ellipsoid = t5.elapsed();
    println!(
        "voxel_ellipsoid_fill_r16: {:?} ({} edits)",
        d_ellipsoid,
        ellipsoid.len()
    );

    // 7. Pixel flood_fill_edits (256x256)
    if let Ok(canvas) = PixelCanvas::new(256, 256) {
        let layer = &canvas.frames[0].layers[0];
        let t6 = Instant::now();
        let flooded = flood_fill_edits(layer, PixelCoord::new(0, 0), Some(mat));
        let d_flood = t6.elapsed();
        println!(
            "pixel_flood_fill_256x256: {:?} ({} edits)",
            d_flood,
            flooded.len()
        );
    }

    // 8. Low-Poly re-mesh triangulation (`to_triangulated_flat`) x100
    if let Ok(sample_mesh) = create_lathe_mesh(&profile, 24, 360.0, true, mat) {
        let t7 = Instant::now();
        let mut tri_count = 0usize;
        for _ in 0..100 {
            let tri = sample_mesh.to_triangulated_flat();
            tri_count = tri.indices.len() / 3;
        }
        let d_tri = t7.elapsed();
        println!(
            "lowpoly_triangulated_flat_x100: {:?} ({} tris)",
            d_tri, tri_count
        );
    }
}
