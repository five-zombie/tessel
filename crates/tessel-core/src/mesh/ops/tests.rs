use super::primitives::point_in_ccw_triangle_2d;
use super::*;
use crate::mesh::{FaceId, PrimitiveKind};
use crate::palette::PaletteIndex;
use crate::voxel::VoxelVolume;
use glam::Vec3;

use crate::voxel::VoxelCoord;

#[test]
fn cube_extrude_adds_four_side_faces() {
    let mat = PaletteIndex::new(4).unwrap();
    let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    assert_eq!(cube.faces.len(), 6);
    let extruded = extrude_face(&cube, FaceId(4), 0.5).unwrap();
    assert_eq!(extruded.faces.len(), 10);
    assert_eq!(extruded.to_triangulated_flat().indices.len(), 60);
}

#[test]
fn voxel_to_lowpoly_welds_shared_vertices() {
    let mut vol = VoxelVolume::new();
    let c = PaletteIndex::new(4).unwrap();
    vol.set(VoxelCoord::new(0, 0, 0), Some(c));
    let mesh = voxel_to_lowpoly(&vol, 0.1);
    assert_eq!(mesh.vertices.len(), 8);
    assert_eq!(mesh.triangle_count(), 12);
}

#[test]
fn raycast_picks_front_face_of_cube() {
    let mat = PaletteIndex::new(4).unwrap();
    let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    let hit = raycast_mesh_face(&cube, Vec3::new(0.0, 0.5, 3.0), Vec3::new(0.0, 0.0, -1.0));
    assert_eq!(hit, Some(FaceId(0))); // +Z front face
}

#[test]
fn lathe_dome_has_outward_normals_and_pole_collapse() {
    let mat = PaletteIndex::new(3).unwrap();
    let profile = [[0.5, 0.0], [0.5, 0.3], [0.35, 0.55], [0.0, 0.65]];
    let dome = create_lathe_mesh(&profile, 12, 360.0, true, mat).unwrap();
    // 3 rings of 12 + 1 collapsed top pole = 37 vertices
    assert_eq!(dome.vertices.len(), 37);
    // 2 quad bands (24) + 1 top cap band of tris (12) + 1 bottom cap (1) = 37 faces
    assert_eq!(dome.faces.len(), 37);
    let tri = dome.to_triangulated_flat();
    assert!(tri.vertices[0].normal[2] > 0.5);
}

#[test]
fn lathe_partial_arc_visor_centers_on_front_z() {
    let mat = PaletteIndex::new(5).unwrap();
    let profile = [[0.4, 0.0], [0.4, 0.25]];
    let visor = create_lathe_mesh(&profile, 8, 180.0, true, mat).unwrap();
    assert_eq!(visor.faces.len(), 8);
    let (lo, hi) = visor.bounds();
    assert!(hi.z > 0.35 && lo.z >= -1e-4, "180° visor faces +Z front");
}

#[test]
fn extruded_polygon_normalizes_cw_input_and_caps_both_sides() {
    let mat = PaletteIndex::new(2).unwrap();
    // Deliberately clockwise square
    let cw = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
    let mesh = create_extruded_polygon(&cw, 0.2, mat).unwrap();
    assert_eq!(mesh.vertices.len(), 8);
    assert_eq!(mesh.faces.len(), 8); // 2 front tris + 2 back tris + 4 side quads
    let tri = mesh.to_triangulated_flat();
    // Front cap normal must point +Z
    assert!(tri.vertices[0].normal[2] > 0.99);
    // Reject degenerate collinear polygon
    assert!(create_extruded_polygon(&[[0.0, 0.0], [1.0, 0.0], [2.0, 0.0]], 0.2, mat).is_err());
}

#[test]
fn extruded_concave_polygon_ear_clips_without_bridging_concavity() {
    let mat = PaletteIndex::new(3).unwrap();
    // Concave C-shape polygon (CCW) with a deep notch at x in [0.4, 1.0], y in [0.3, 0.7]
    let c_shape = [
        [0.0, 0.0],
        [1.0, 0.0],
        [1.0, 0.3],
        [0.4, 0.3],
        [0.4, 0.7],
        [1.0, 0.7],
        [1.0, 1.0],
        [0.0, 1.0],
    ];
    let mesh = create_extruded_polygon(&c_shape, 0.2, mat).unwrap();
    let n = c_shape.len();
    assert_eq!(mesh.vertices.len(), n * 2);
    // Expected area of C-shape = 1.0 - (0.6 * 0.4) = 0.76
    let front_cap_area: f32 = mesh
        .faces
        .iter()
        .take(n - 2)
        .map(|f| {
            let a = mesh.vertices[f.vertices[0]];
            let b = mesh.vertices[f.vertices[1]];
            let c = mesh.vertices[f.vertices[2]];
            let area_z = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
            assert!(area_z > 0.0, "every front cap triangle must be CCW (+Z)");
            0.5 * area_z
        })
        .sum();
    assert!(
        (front_cap_area - 0.76).abs() < 1e-4,
        "front cap area {front_cap_area} must equal concave C-shape area 0.76"
    );
    // Verify no front cap triangle covers the concave notch center (0.75, 0.5)
    let notch = [0.75, 0.5];
    for f in mesh.faces.iter().take(n - 2) {
        let a = [
            mesh.vertices[f.vertices[0]].x,
            mesh.vertices[f.vertices[0]].y,
        ];
        let b = [
            mesh.vertices[f.vertices[1]].x,
            mesh.vertices[f.vertices[1]].y,
        ];
        let c = [
            mesh.vertices[f.vertices[2]].x,
            mesh.vertices[f.vertices[2]].y,
        ];
        assert!(
            !point_in_ccw_triangle_2d(notch, a, b, c, 1e-6),
            "ear-clipped cap triangle must not bridge concave notch"
        );
    }

    // Reject self-intersecting bowtie polygon with nonzero net area
    let self_intersecting = [[0.0, 0.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.8]];
    assert!(create_extruded_polygon(&self_intersecting, 0.2, mat).is_err());

    // Sub-millimeter concave polygon (scale 0.0005) still triangulates with exact area
    let tiny_scale = 0.0005f32;
    let tiny_c: Vec<[f32; 2]> = c_shape
        .iter()
        .map(|&[x, y]| [x * tiny_scale, y * tiny_scale])
        .collect();
    let tiny_mesh = create_extruded_polygon(&tiny_c, 0.01, mat).unwrap();
    let tiny_area: f32 = tiny_mesh
        .faces
        .iter()
        .take(n - 2)
        .map(|f| {
            let a = tiny_mesh.vertices[f.vertices[0]];
            let b = tiny_mesh.vertices[f.vertices[1]];
            let c = tiny_mesh.vertices[f.vertices[2]];
            0.5 * ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x))
        })
        .sum();
    let expected_tiny = 0.76 * tiny_scale * tiny_scale;
    assert!(
        (tiny_area - expected_tiny).abs() < expected_tiny * 1e-3,
        "tiny concave area {tiny_area} must match {expected_tiny}"
    );
}

#[test]
fn loft_mesh_connects_tapered_rings_with_outward_normals() {
    let mat = PaletteIndex::new(4).unwrap();
    let ring = |y: f32, r: f32| {
        vec![
            Vec3::new(0.0, y, r),
            Vec3::new(r, y, 0.0),
            Vec3::new(0.0, y, -r),
            Vec3::new(-r, y, 0.0),
        ]
    };
    let mesh = create_loft_mesh(
        &[ring(0.0, 0.5), ring(0.5, 0.35), ring(1.0, 0.2)],
        true,
        mat,
    )
    .unwrap();
    assert_eq!(mesh.vertices.len(), 12);
    assert_eq!(mesh.faces.len(), 10); // 8 side quads + 2 caps
    let tri = mesh.to_triangulated_flat();
    assert!(tri.vertices[0].normal[2] > 0.4 && tri.vertices[0].normal[0] > 0.4);
}

#[test]
fn smooth_subdivide_turns_cube_into_rounded_mesh_and_sphere() {
    let mat = PaletteIndex::new(4).unwrap();
    let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    let round1 = smooth_subdivide_mesh(&cube).unwrap();
    assert_eq!(round1.faces.len(), 24);
    assert_eq!(round1.vertices.len(), 26);

    let round2 = smooth_subdivide_mesh(&round1).unwrap();
    assert_eq!(round2.faces.len(), 96);
    assert_eq!(round2.to_triangulated_flat().indices.len(), 96 * 6);
}

#[test]
fn smooth_shading_averages_vertex_normals_across_shared_corners() {
    let mat = PaletteIndex::new(4).unwrap();
    let mut cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    let flat_n = cube.to_triangulated_flat().vertices[0].normal;
    cube.smooth_shading = true;
    let smooth_n = cube.to_triangulated_flat().vertices[0].normal;
    // Flat normal is axis-aligned [0, 0, 1]; smooth corner normal averages all 3 adjacent faces.
    assert!((flat_n[0]).abs() < 1e-4 && (flat_n[1]).abs() < 1e-4);
    assert!(smooth_n[0].abs() > 0.3 && smooth_n[1].abs() > 0.3 && smooth_n[2].abs() > 0.3);
}

#[test]
fn translate_face_moves_selected_face_vertices() {
    let mat = PaletteIndex::new(4).unwrap();
    let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    let before_z = cube.vertices[cube.faces[0].vertices[0]].z;
    let moved = translate_face(&cube, FaceId(0), Vec3::new(0.0, 0.0, 0.5)).unwrap();
    let after_z = moved.vertices[moved.faces[0].vertices[0]].z;
    assert!((after_z - before_z - 0.5).abs() < 1e-5);
}

#[test]
fn primitives_sphere_torus_capsule_have_valid_geometry() {
    let mat = PaletteIndex::new(4).unwrap();
    for kind in [
        PrimitiveKind::Sphere,
        PrimitiveKind::Torus,
        PrimitiveKind::Capsule,
    ] {
        let mesh = create_primitive(kind, 1.0, mat);
        assert!(!mesh.vertices.is_empty());
        assert!(!mesh.faces.is_empty());
        assert!(mesh.smooth_shading);
        assert!(mesh.triangle_count() > 20);
    }
}

#[test]
fn bevel_mesh_chamfers_cube_edges_and_corners() {
    let mat = PaletteIndex::new(4).unwrap();
    let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    let beveled = bevel_mesh(&cube, 0.2).unwrap();
    // 6 main faces + 12 edge chamfer quads + 8 corner triangles = 26 faces
    assert_eq!(beveled.faces.len(), 26);
    assert_eq!(beveled.vertices.len(), 24);
}

#[test]
fn curve_tube_and_decimate_produce_valid_lowpoly_meshes() {
    let mat = PaletteIndex::new(4).unwrap();
    let strap = create_curve_tube(
        &[
            Vec3::new(0.0, 0.2, 0.0),
            Vec3::new(0.2, 0.6, 0.1),
            Vec3::new(0.0, 1.0, 0.0),
        ],
        0.08,
        0.3,
        mat,
    )
    .unwrap();
    assert!(strap.faces.len() > 20);
    let decimated = decimate_mesh(&strap, 0.5).unwrap();
    assert!(!decimated.faces.is_empty());
    assert!(decimated.vertices.len() < strap.vertices.len());
}

#[test]
fn move_vertices_shifts_only_listed_corners() {
    let mat = PaletteIndex::new(4).unwrap();
    let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    let untouched = cube.vertices[1];
    let moved = move_mesh_vertices(&cube, &[0], Vec3::new(0.25, 0.0, 0.0)).unwrap();
    assert!((moved.vertices[0].x - (cube.vertices[0].x + 0.25)).abs() < 1e-6);
    assert_eq!(moved.vertices[1], untouched);
}

#[test]
fn move_vertices_rejects_bad_input() {
    let mat = PaletteIndex::new(4).unwrap();
    let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    assert!(move_mesh_vertices(&cube, &[], Vec3::ONE).is_err());
    assert!(move_mesh_vertices(&cube, &[999], Vec3::ONE).is_err());
    assert!(move_mesh_vertices(&cube, &[0], Vec3::new(f32::NAN, 0.0, 0.0)).is_err());
}

#[test]
fn set_mesh_vertex_places_corner_absolutely() {
    let mat = PaletteIndex::new(4).unwrap();
    let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    let target = Vec3::new(0.1, 0.2, 0.3);
    let moved = set_mesh_vertex(&cube, 2, target).unwrap();
    assert_eq!(moved.vertices[2], target);
    assert!(set_mesh_vertex(&cube, 999, target).is_err());
    assert!(set_mesh_vertex(&cube, 0, Vec3::new(f32::INFINITY, 0.0, 0.0)).is_err());
}

#[test]
fn set_face_uvs_stores_manual_unwrap() {
    let mat = PaletteIndex::new(4).unwrap();
    let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let unwrapped = set_face_uvs(&cube, FaceId(0), &uvs).unwrap();
    assert_eq!(unwrapped.faces[0].uvs, uvs);
    // Out-of-range values clamp into UV space instead of failing.
    let clamped = set_face_uvs(
        &cube,
        FaceId(0),
        &[[0.0, 0.0], [2.0, 0.0], [1.0, 1.0], [0.0, -1.0]],
    )
    .unwrap();
    assert_eq!(clamped.faces[0].uvs[1], [1.0, 0.0]);
    assert_eq!(clamped.faces[0].uvs[3], [0.0, 0.0]);
}

#[test]
fn set_face_uvs_rejects_bad_input() {
    let mat = PaletteIndex::new(4).unwrap();
    let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    assert!(set_face_uvs(&cube, FaceId(99), &[[0.0, 0.0]; 4]).is_err());
    assert!(set_face_uvs(&cube, FaceId(0), &[[0.0, 0.0]; 3]).is_err());
    assert!(set_face_uvs(&cube, FaceId(0), &[[f32::NAN, 0.0]; 4]).is_err());
}

#[test]
fn set_vertex_weights_normalizes_and_rejects_bad_input() {
    let mat = PaletteIndex::new(4).unwrap();
    let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    let painted = set_vertex_weights(&cube, &[0, 1], [2, 3, 0, 0], [3.0, 1.0, 0.0, 0.0]).unwrap();
    let w = painted.skin_weights[&0];
    assert_eq!(w.joints, [2, 3, 0, 0]);
    assert!((w.weights[0] - 0.75).abs() < 1e-6);
    assert!((w.weights[1] - 0.25).abs() < 1e-6);
    assert!((w.weights.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    assert_eq!(painted.skin_weights[&1], w);
    assert!(!painted.skin_weights.contains_key(&2));

    assert!(set_vertex_weights(&cube, &[], [0, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]).is_err());
    assert!(set_vertex_weights(&cube, &[99], [0, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]).is_err());
    assert!(set_vertex_weights(&cube, &[0], [0, 0, 0, 0], [0.0, 0.0, 0.0, 0.0]).is_err());
    assert!(set_vertex_weights(&cube, &[0], [0, 0, 0, 0], [-1.0, 1.0, 0.0, 0.0]).is_err());
    assert!(set_vertex_weights(&cube, &[0], [0, 0, 0, 0], [f32::NAN, 0.0, 0.0, 0.0]).is_err());
}

#[test]
fn clear_vertex_weights_clears_all_or_listed() {
    let mat = PaletteIndex::new(4).unwrap();
    let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    let painted =
        set_vertex_weights(&cube, &[0, 1, 2], [0, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]).unwrap();
    let partial = clear_vertex_weights(&painted, Some(&[0])).unwrap();
    assert!(!partial.skin_weights.contains_key(&0));
    assert!(partial.skin_weights.contains_key(&1));
    let cleared = clear_vertex_weights(&painted, None).unwrap();
    assert!(cleared.skin_weights.is_empty());
    assert!(clear_vertex_weights(&painted, Some(&[99])).is_err());
}

#[test]
fn morph_deltas_accumulate_and_clear() {
    let mat = PaletteIndex::new(4).unwrap();
    let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    let m1 = add_morph_delta(&cube, "Smile", &[0, 1], Vec3::new(0.0, 0.1, 0.0)).unwrap();
    assert_eq!(m1.morphs.len(), 1);
    assert_eq!(m1.morphs[0].name, "Smile");
    let m2 = add_morph_delta(&m1, "Smile", &[0], Vec3::new(0.0, 0.1, 0.0)).unwrap();
    assert_eq!(m2.morphs[0].deltas[&0], Vec3::new(0.0, 0.2, 0.0));
    assert_eq!(m2.morphs[0].deltas[&1], Vec3::new(0.0, 0.1, 0.0));
    // Cancelling deltas prune the vertex; empty targets are dropped.
    let m3 = add_morph_delta(&m2, "Smile", &[0, 1], Vec3::new(0.0, -0.1, 0.0)).unwrap();
    assert_eq!(m3.morphs.len(), 1);
    assert!(!m3.morphs[0].deltas.contains_key(&1));
    let cleared = clear_morphs(&m3, Some("Smile")).unwrap();
    assert!(cleared.morphs.is_empty());
    // Missing names are a no-op; None clears all.
    let kept = clear_morphs(&m3, Some("Frown")).unwrap();
    assert_eq!(kept.morphs.len(), 1);
    let all = clear_morphs(&m3, None).unwrap();
    assert!(all.morphs.is_empty());

    assert!(add_morph_delta(&cube, "", &[0], Vec3::ONE).is_err());
    assert!(add_morph_delta(&cube, "X", &[], Vec3::ONE).is_err());
    assert!(add_morph_delta(&cube, "X", &[99], Vec3::ONE).is_err());
    assert!(add_morph_delta(&cube, "X", &[0], Vec3::new(f32::NAN, 0.0, 0.0)).is_err());
}

#[test]
fn morphs_survive_move_but_not_rebuild() {
    let mat = PaletteIndex::new(4).unwrap();
    let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    let morphed = add_morph_delta(&cube, "Smile", &[0], Vec3::new(0.0, 0.1, 0.0)).unwrap();
    let moved = move_mesh_vertices(&morphed, &[0], Vec3::new(0.1, 0.0, 0.0)).unwrap();
    assert_eq!(moved.morphs.len(), 1);
    let beveled = bevel_mesh(&morphed, 0.2).unwrap();
    assert!(beveled.morphs.is_empty());
}

#[test]
fn skin_weights_survive_move_but_not_rebuild() {
    let mat = PaletteIndex::new(4).unwrap();
    let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    let painted = set_vertex_weights(&cube, &[0], [5, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]).unwrap();
    let moved = move_mesh_vertices(&painted, &[0], Vec3::new(0.1, 0.0, 0.0)).unwrap();
    assert_eq!(moved.skin_weights[&0].joints, [5, 0, 0, 0]);
    let beveled = bevel_mesh(&painted, 0.2).unwrap();
    assert!(beveled.skin_weights.is_empty());
}

#[test]
fn weight_paint_brush_blends_smoothly_and_normalizes() {
    let mat = PaletteIndex::new(4).unwrap();
    let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
    // Seed vertex 0 with joint 1 weight 1.0
    let seeded = set_vertex_weights(&cube, &[0], [1, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]).unwrap();
    let v0 = seeded.vertices[0];

    // Paint joint 3 directly on vertex 0 with strength 0.5 -> expect 50/50 blend between joint 1 and joint 3
    let painted = paint_vertex_weights_brush(&seeded, v0, 0.25, 3, 0.5, None)
        .unwrap()
        .expect("vertex 0 must be within radius");
    let w = painted.skin_weights[&0];
    let sum: f32 = w.weights.iter().sum();
    assert!((sum - 1.0).abs() < 1e-5, "weights must normalize to 1.0");
    assert!(w.joints.contains(&1) && w.joints.contains(&3));

    // Hit point far outside mesh returns Ok(None)
    let missed =
        paint_vertex_weights_brush(&seeded, Vec3::new(10.0, 10.0, 10.0), 0.25, 3, 0.5, None)
            .unwrap();
    assert!(missed.is_none());

    // Invalid radius/strength errors cleanly
    assert!(paint_vertex_weights_brush(&seeded, v0, 0.0, 3, 0.5, None).is_err());
    assert!(paint_vertex_weights_brush(&seeded, v0, 0.25, 3, 1.5, None).is_err());

    // Duplicate base joints are merged and NaN vertices are skipped safely
    let mut dup_mesh = set_vertex_weights(&cube, &[0], [1, 1, 2, 0], [0.4, 0.4, 0.2, 0.0]).unwrap();
    dup_mesh.vertices[1] = Vec3::new(f32::NAN, 0.0, 0.0);
    let dup_painted = paint_vertex_weights_brush(&dup_mesh, v0, 0.25, 3, 0.5, None)
        .unwrap()
        .unwrap();
    let dw = dup_painted.skin_weights[&0];
    assert_eq!(dw.joints[0], 3);
    assert_eq!(dw.joints[1], 1);
    assert_eq!(dw.joints[2], 2);
    assert!((dw.weights[0] - 0.5).abs() < 1e-4);
    assert!((dw.weights[1] - 0.4).abs() < 1e-4);
    assert!((dw.weights[2] - 0.1).abs() < 1e-4);
    assert!(!dup_painted.skin_weights.contains_key(&1));
}
