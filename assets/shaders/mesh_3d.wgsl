struct CameraUniform {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    light_dir: vec4<f32>,
};

struct MaterialUniform {
    pbr: vec4<f32>,
    emissive: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

@group(0) @binding(1)
var palette_tex: texture_2d<f32>;

@group(0) @binding(2)
var palette_sampler: sampler;

@group(0) @binding(3)
var<uniform> material: MaterialUniform;

@group(0) @binding(4)
var normal_tex: texture_2d<f32>;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) palette_uv: vec2<f32>,
    @location(3) ao: f32,
    @location(4) tangent: vec4<f32>,
};

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) palette_uv: vec2<f32>,
    @location(3) ao: f32,
    @location(4) tangent: vec4<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> VertexOut {
    var out: VertexOut;
    out.clip_pos = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.world_pos = in.position;
    out.normal = in.normal;
    out.palette_uv = in.palette_uv;
    out.ao = in.ao;
    out.tangent = in.tangent;
    return out;
}

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let base_color = textureSample(palette_tex, palette_sampler, in.palette_uv).rgb;
    let raw_n = normalize(in.normal);
    let geom_n = select(-raw_n, raw_n, front_facing);
    let t_dir = normalize(in.tangent.xyz - geom_n * dot(geom_n, in.tangent.xyz));
    let b_dir = normalize(cross(geom_n, t_dir)) * in.tangent.w;
    let sampled_n = textureSample(normal_tex, palette_sampler, in.palette_uv).xyz * 2.0 - vec3<f32>(1.0);
    let n = normalize(t_dir * sampled_n.x + b_dir * sampled_n.y + geom_n * sampled_n.z);
    let key_dir = normalize(camera.light_dir.xyz);
    let fill_dir = normalize(vec3<f32>(-key_dir.x, 0.35, -key_dir.z));

    let ndl = max(dot(n, key_dir), 0.0);
    let fill = max(dot(n, fill_dir), 0.0) * 0.28;
    let hemi = mix(0.28, 0.46, n.y * 0.5 + 0.5);

    let view_dir = normalize(camera.camera_pos.xyz - in.world_pos);
    let rim = pow(1.0 - max(dot(n, view_dir), 0.0), 3.0) * 0.12;

    let metallic = clamp(material.pbr.x, 0.0, 1.0);
    let roughness = clamp(material.pbr.y, 0.05, 1.0);
    let half_vec = normalize(key_dir + view_dir);
    let spec_pow = mix(128.0, 4.0, roughness * roughness);
    let spec_strength = mix(0.04, 1.0, metallic) + (1.0 - roughness) * 0.45;
    let spec_color = mix(vec3<f32>(1.0), base_color, metallic);
    let specular = spec_color * pow(max(dot(n, half_vec), 0.0), spec_pow) * spec_strength;

    let diffuse = base_color * (1.0 - metallic * 0.65) * (hemi + ndl * 0.68 + fill);
    let lit = (diffuse + specular) * in.ao + vec3<f32>(rim) + material.emissive.rgb;
    return vec4<f32>(lit, 1.0);
}
