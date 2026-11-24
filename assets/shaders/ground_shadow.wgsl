struct CameraUniform {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    light_dir: vec4<f32>,
};

struct ShadowUniform {
    // xyz = world-space ground footprint center (y = floor plane height), w = enabled (1.0 or 0.0)
    center_enabled: vec4<f32>,
    // xy = footprint half-extents (rx, rz), z = asset height, w = shadow opacity
    extents_height_opacity: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(0) @binding(1)
var<uniform> shadow: ShadowUniform;

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VertexOut {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let pos = positions[vid];
    var out: VertexOut;
    out.clip_pos = vec4<f32>(pos, 0.0, 1.0);
    out.ndc = pos;
    return out;
}

fn unproject(ndc: vec2<f32>, z: f32) -> vec3<f32> {
    let world = camera.inv_view_proj * vec4<f32>(ndc, z, 1.0);
    return world.xyz / world.w;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    if shadow.center_enabled.w < 0.5 {
        discard;
    }

    let near_pt = unproject(in.ndc, 0.0);
    let far_pt = unproject(in.ndc, 1.0);
    let denom = far_pt.y - near_pt.y;
    if abs(denom) < 0.00001 {
        discard;
    }

    let ground_y = shadow.center_enabled.y;
    let t = (ground_y - near_pt.y) / denom;
    if t <= 0.0 {
        discard;
    }

    let world_pos = near_pt + t * (far_pt - near_pt);
    let center_xz = shadow.center_enabled.xz;
    let rel = world_pos.xz - center_xz;
    let rx = max(shadow.extents_height_opacity.x, 0.25);
    let rz = max(shadow.extents_height_opacity.y, 0.25);
    let height = max(shadow.extents_height_opacity.z, 0.25);
    let opacity = shadow.extents_height_opacity.w;

    // Subtle studio floor light pool so soft shadows read crisply against dark background
    let floor_radius = max(rx, rz) * 4.2 + height * 0.75;
    let floor_dist = length(rel) / max(floor_radius, 0.5);
    let floor_falloff = (1.0 - smoothstep(0.0, 1.0, floor_dist));
    let floor_alpha = floor_falloff * floor_falloff * 0.22;

    // Tight elliptical contact AO shadow directly under the asset footprint
    let contact_d = length(vec2<f32>(rel.x / (rx * 1.12), rel.y / (rz * 1.12)));
    let contact_shadow = exp(-contact_d * contact_d * 2.4) * 0.70;

    // Directional cast shadow projected opposite the studio key light and stretched by sun elevation
    let light = normalize(camera.light_dir.xyz);
    let light_y = max(light.y, 0.18);
    let horiz = light.xz;
    let horiz_len = length(horiz);
    let proj_dir = select(vec2<f32>(1.0, 0.0), -horiz / horiz_len, horiz_len > 0.001);
    let perp_dir = vec2<f32>(-proj_dir.y, proj_dir.x);

    let shadow_len = clamp((horiz_len / light_y) * height, 0.0, height * 3.8);
    let shift = proj_dir * (shadow_len * 0.55);
    let cast_rel = rel - shift;

    let along = dot(cast_rel, proj_dir);
    let across = dot(cast_rel, perp_dir);
    let base_r = max(rx, rz);
    let r_along = base_r * 1.15 + shadow_len * 0.52;
    let r_across = base_r * 1.18;
    let cast_d = length(vec2<f32>(along / max(r_along, 0.2), across / max(r_across, 0.2)));
    let cast_shadow = exp(-cast_d * cast_d * 2.1) * 0.68;

    let shadow_mask = clamp(max(contact_shadow, cast_shadow) * opacity, 0.0, 0.92);

    // Blend subtle cool-slate studio pedestal into deep contact shadow
    let floor_color = vec3<f32>(0.065, 0.075, 0.095);
    let shadow_color = vec3<f32>(0.001, 0.0015, 0.0025);
    let color = mix(floor_color, shadow_color, shadow_mask);
    let final_alpha = clamp(max(floor_alpha, shadow_mask * 0.88), 0.0, 0.92);

    if final_alpha <= 0.002 {
        discard;
    }

    return vec4<f32>(color, final_alpha);
}
