struct CameraUniform {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    light_dir: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(0) @binding(1)
var palette_tex: texture_2d<f32>;

@group(0) @binding(2)
var palette_sampler: sampler;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) palette_uv: vec2<f32>,
    @location(3) ao: f32,
};

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) palette_uv: vec2<f32>,
    @location(3) ao: f32,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_pos = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.world_pos = in.position;
    out.normal = in.normal;
    out.palette_uv = in.palette_uv;
    out.ao = in.ao;
    return out;
}

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let base_color = textureSample(palette_tex, palette_sampler, in.palette_uv).rgb;
    let raw_n = normalize(in.normal);
    let n = select(-raw_n, raw_n, front_facing);
    let key_dir = normalize(camera.light_dir.xyz);
    let fill_dir = normalize(vec3<f32>(-key_dir.x, 0.35, -key_dir.z));

    let ndl = max(dot(n, key_dir), 0.0);
    let fill = max(dot(n, fill_dir), 0.0) * 0.28;
    let hemi = mix(0.28, 0.46, n.y * 0.5 + 0.5);

    let view_dir = normalize(camera.camera_pos.xyz - in.world_pos);
    let rim = pow(1.0 - max(dot(n, view_dir), 0.0), 3.0) * 0.12;

    let lit = base_color * (hemi + ndl * 0.68 + fill) * in.ao + vec3<f32>(rim);
    return vec4<f32>(lit, 1.0);
}
