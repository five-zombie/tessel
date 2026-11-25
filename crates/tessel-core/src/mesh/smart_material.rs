use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::palette::Rgba8;

use super::bake::{bake_mesh_maps, BakeMapKind, BakeParams};
use super::lowpoly::LowPolyMesh;
use super::texture::{MeshTexture, DEFAULT_MESH_TEX_SIZE};

/// Parameters for procedural Smart Material Lite texture generation
/// (base color + curvature-driven edge wear + AO/concavity cavity dirt).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmartMaterialParams {
    pub base_color: Rgba8,
    pub edge_wear_color: Rgba8,
    pub cavity_dirt_color: Rgba8,
    /// Blend strength of convex edge wear (`0.0..=1.0`).
    pub edge_strength: f32,
    /// Normalized curvature threshold above which edge wear begins (`0.5..=0.95`).
    pub edge_threshold: f32,
    /// Blend strength of AO/concave cavity dirt (`0.0..=1.0`).
    pub cavity_strength: f32,
    /// AO threshold below which cavity dirt accumulates (`0.05..=1.0`).
    pub cavity_threshold: f32,
    /// Multiplicative AO darkening factor baked into albedo (`0.0..=1.0`).
    pub ao_darken: f32,
    pub resolution: u32,
    pub samples: u32,
    pub dilation_px: u32,
}

impl SmartMaterialParams {
    /// Builds default smart-material parameters derived from `base_color`
    /// (auto-lightened edge wear, auto-darkened cavity dirt).
    pub fn from_base(base_color: Rgba8) -> Self {
        let edge_wear_color = Rgba8::rgba(
            base_color.r.saturating_add(55),
            base_color.g.saturating_add(55),
            base_color.b.saturating_add(55),
            base_color.a,
        );
        let cavity_dirt_color = Rgba8::rgba(
            (f32::from(base_color.r) * 0.35).round() as u8,
            (f32::from(base_color.g) * 0.32).round() as u8,
            (f32::from(base_color.b) * 0.28).round() as u8,
            base_color.a,
        );
        Self {
            base_color,
            edge_wear_color,
            cavity_dirt_color,
            edge_strength: 0.65,
            edge_threshold: 0.58,
            cavity_strength: 0.6,
            cavity_threshold: 0.85,
            ao_darken: 0.25,
            resolution: DEFAULT_MESH_TEX_SIZE,
            samples: 16,
            dilation_px: 4,
        }
    }
}

/// Output of `generate_smart_material_texture`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmartMaterialOutput {
    pub albedo: MeshTexture,
    pub ao_map: Option<MeshTexture>,
    pub covered_texels: usize,
}

/// Deterministically generates a Smart Material Lite RGBA texture by combining
/// `base_color`, curvature-driven edge wear, and AO/concavity cavity dirt.
pub fn generate_smart_material_texture(
    mesh: &LowPolyMesh,
    params: &SmartMaterialParams,
) -> Result<SmartMaterialOutput, CoreError> {
    let bake_params = BakeParams {
        resolution: params.resolution.clamp(16, 1024),
        samples: params.samples.clamp(4, 64),
        max_distance: 0.0,
        bias: 1e-3,
        dilation_px: params.dilation_px.clamp(0, 16),
        maps: vec![BakeMapKind::Ao, BakeMapKind::Curvature],
    };
    let baked = bake_mesh_maps(mesh, &bake_params)?;
    let ao_tex = baked.ao.as_ref().ok_or(CoreError::OutOfBounds)?;
    let curv_tex = baked.curvature.as_ref().ok_or(CoreError::OutOfBounds)?;

    let w = ao_tex.width;
    let h = ao_tex.height;
    let pixel_count = (w as usize).saturating_mul(h as usize);
    let mut rgba = Vec::with_capacity(pixel_count * 4);

    let base = to_rgb_f32(params.base_color);
    let edge = to_rgb_f32(params.edge_wear_color);
    let dirt = to_rgb_f32(params.cavity_dirt_color);

    let finite_clamp = |v: f32, min: f32, max: f32, fallback: f32| -> f32 {
        if v.is_finite() {
            v.clamp(min, max)
        } else {
            fallback
        }
    };
    let edge_strength = finite_clamp(params.edge_strength, 0.0, 1.0, 0.65);
    let edge_threshold = finite_clamp(params.edge_threshold, 0.5, 0.95, 0.58);
    let edge_denom = (1.0 - edge_threshold).max(1e-4);

    let cavity_strength = finite_clamp(params.cavity_strength, 0.0, 1.0, 0.6);
    let cavity_threshold = finite_clamp(params.cavity_threshold, 0.05, 1.0, 0.85);
    let ao_darken = finite_clamp(params.ao_darken, 0.0, 1.0, 0.25);

    for i in 0..pixel_count {
        let ao = f32::from(ao_tex.rgba[i * 4]) / 255.0;
        let curv = f32::from(curv_tex.rgba[i * 4]) / 255.0;

        let mut rgb = base;

        // Cavity dirt from low AO + concave curvature (< 0.45).
        let ao_dirt = ((cavity_threshold - ao) / cavity_threshold).clamp(0.0, 1.0);
        let concave_dirt = ((0.45 - curv) / 0.45).clamp(0.0, 1.0) * 0.5;
        let dirt_mask = ((ao_dirt + concave_dirt) * cavity_strength).clamp(0.0, 1.0);
        rgb = lerp_rgb(rgb, dirt, dirt_mask);

        // Convex edge wear from curvature > edge_threshold.
        let edge_mask = (((curv - edge_threshold) / edge_denom).clamp(0.0, 1.0) * edge_strength)
            .clamp(0.0, 1.0);
        rgb = lerp_rgb(rgb, edge, edge_mask);

        // Subtle multiplicative AO darkening.
        let ao_mul = 1.0 - ao_darken * (1.0 - ao);
        rgba.push(float_to_u8(rgb[0] * ao_mul));
        rgba.push(float_to_u8(rgb[1] * ao_mul));
        rgba.push(float_to_u8(rgb[2] * ao_mul));
        rgba.push(params.base_color.a);
    }

    Ok(SmartMaterialOutput {
        albedo: MeshTexture {
            width: w,
            height: h,
            rgba,
        },
        ao_map: baked.ao,
        covered_texels: baked.covered_texels,
    })
}

fn to_rgb_f32(c: Rgba8) -> [f32; 3] {
    [
        f32::from(c.r) / 255.0,
        f32::from(c.g) / 255.0,
        f32::from(c.b) / 255.0,
    ]
}

fn lerp_rgb(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn float_to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use glam::Vec3;

    use super::*;
    use crate::mesh::face::Face;
    use crate::mesh::nano::PrimitiveKind;
    use crate::mesh::ops::create_primitive;
    use crate::palette::PaletteIndex;

    fn avg_channel(tex: &MeshTexture, ch: usize) -> f32 {
        let sum: u64 = tex
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|px| u64::from(px[ch]))
            .sum();
        sum as f32 / (tex.width as f32 * tex.height as f32 * 255.0)
    }

    #[test]
    fn smart_material_deterministic_and_valid_dimensions() {
        let mat = PaletteIndex::new(3).unwrap();
        let cube = create_primitive(PrimitiveKind::Cube, 1.0, mat);
        let mut params = SmartMaterialParams::from_base(Rgba8::rgb(120, 110, 100));
        params.resolution = 32;
        params.samples = 8;
        let a = generate_smart_material_texture(&cube, &params).unwrap();
        let b = generate_smart_material_texture(&cube, &params).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.albedo.width, 32);
        assert_eq!(a.albedo.height, 32);
        assert!(a.ao_map.is_some());
        assert!(a.covered_texels > 0);
    }

    #[test]
    fn smart_material_applies_edge_wear_on_convex_ridge() {
        let mat = PaletteIndex::new(3).unwrap();
        let plane = create_primitive(PrimitiveKind::Plane, 2.0, mat);
        let mut ridge = LowPolyMesh::new();
        ridge.vertices = vec![
            Vec3::new(-1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(-1.0, 1.0, 0.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(-1.0, 0.0, 1.0),
        ];
        ridge.faces = vec![
            Face::new(vec![0, 3, 2, 1], mat),
            Face::new(vec![3, 5, 4, 2], mat),
        ];
        let params = SmartMaterialParams {
            base_color: Rgba8::rgb(40, 40, 40),
            edge_wear_color: Rgba8::rgb(255, 255, 255),
            cavity_dirt_color: Rgba8::rgb(0, 0, 0),
            edge_strength: 1.0,
            edge_threshold: 0.55,
            cavity_strength: 0.0,
            cavity_threshold: 0.85,
            ao_darken: 0.0,
            resolution: 32,
            samples: 8,
            dilation_px: 0,
        };
        let plane_out = generate_smart_material_texture(&plane, &params).unwrap();
        let ridge_out = generate_smart_material_texture(&ridge, &params).unwrap();
        let plane_r = avg_channel(&plane_out.albedo, 0);
        let ridge_r = avg_channel(&ridge_out.albedo, 0);
        assert!(
            ridge_r > plane_r + 0.02,
            "expected convex ridge ({ridge_r:.3}) brighter from edge wear than flat plane ({plane_r:.3})"
        );
    }

    #[test]
    fn smart_material_applies_cavity_dirt_in_concave_corner() {
        let mat = PaletteIndex::new(3).unwrap();
        let plane = create_primitive(PrimitiveKind::Plane, 2.0, mat);
        let mut l_corner = LowPolyMesh::new();
        l_corner.vertices = vec![
            Vec3::new(-1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, -1.0),
            Vec3::new(1.0, 0.0, 1.0),
            Vec3::new(-1.0, 0.0, 1.0),
            Vec3::new(-1.0, 2.0, 1.0),
            Vec3::new(1.0, 2.0, 1.0),
        ];
        l_corner.faces = vec![
            Face::new(vec![0, 3, 2, 1], mat),
            Face::new(vec![3, 4, 5, 2], mat),
        ];
        let params = SmartMaterialParams {
            base_color: Rgba8::rgb(220, 220, 220),
            edge_wear_color: Rgba8::rgb(255, 255, 255),
            cavity_dirt_color: Rgba8::rgb(20, 15, 10),
            edge_strength: 0.0,
            edge_threshold: 0.6,
            cavity_strength: 1.0,
            cavity_threshold: 0.9,
            ao_darken: 0.2,
            resolution: 32,
            samples: 16,
            dilation_px: 0,
        };
        let plane_out = generate_smart_material_texture(&plane, &params).unwrap();
        let corner_out = generate_smart_material_texture(&l_corner, &params).unwrap();
        let plane_r = avg_channel(&plane_out.albedo, 0);
        let corner_r = avg_channel(&corner_out.albedo, 0);
        assert!(
            corner_r < plane_r - 0.03,
            "expected concave corner ({corner_r:.3}) darker from cavity dirt than flat plane ({plane_r:.3})"
        );
    }
}
