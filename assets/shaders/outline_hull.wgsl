struct CameraUniform {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    light_dir: vec4<f32>,
};

struct ShadowMapUniform {
    light_view_proj: mat4x4<f32>,
    bias_params: vec4<f32>,
    npr_params: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(1) @binding(0)
var<uniform> shadow_map: ShadowMapUniform;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) palette_uv: vec2<f32>,
    @location(3) ao: f32,
    @location(4) tangent: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) dist: f32,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    let dist = length(camera.camera_pos.xyz - in.position);
    let width = shadow_map.npr_params.y * clamp(dist * 0.08, 0.45, 2.5);
    let n_len = length(in.normal);
    let n_dir = select(vec3<f32>(0.0), in.normal / n_len, n_len > 1e-5);
    let extruded = in.position + n_dir * width;
    out.clip_pos = camera.view_proj * vec4<f32>(extruded, 1.0);
    out.dist = dist;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let fog_dist = max(shadow_map.npr_params.z, 1.0);
    let fade = clamp(in.dist / fog_dist, 0.0, 0.75);
    let ink = mix(vec3<f32>(0.05, 0.045, 0.055), vec3<f32>(0.88, 0.85, 0.79), fade);
    return vec4<f32>(ink, 1.0);
}
