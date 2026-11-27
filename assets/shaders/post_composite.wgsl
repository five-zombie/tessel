struct PostProcessUniform {
    /// `[exposure, bloom_intensity, aces_enabled (1.0/0.0), bypass_flag (1.0/0.0)]`
    tone: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> params: PostProcessUniform;

@group(0) @binding(1)
var hdr_tex: texture_2d<f32>;

@group(0) @binding(2)
var bloom_tex: texture_2d<f32>;

@group(0) @binding(3)
var linear_sampler: sampler;

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VertexOut {
    let x = f32((vid << 1u) & 2u);
    let y = f32(vid & 2u);
    var out: VertexOut;
    out.clip_pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
    out.uv = vec2<f32>(x, y);
    return out;
}

fn aces_film(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3<f32>(0.0), vec3<f32>(1.0));
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let hdr_rgba = textureSample(hdr_tex, linear_sampler, in.uv);
    if (params.tone.w > 0.5) {
        // Bypass Exposure, Bloom, and ACES for NPR Sumi-e & analysis modes to preserve pure ink/paper.
        return vec4<f32>(clamp(hdr_rgba.rgb, vec3<f32>(0.0), vec3<f32>(1.0)), hdr_rgba.a);
    }

    let bloom_rgb = textureSample(bloom_tex, linear_sampler, in.uv).rgb;
    var color = max(hdr_rgba.rgb, vec3<f32>(0.0)) + max(bloom_rgb, vec3<f32>(0.0)) * params.tone.y;
    color = color * params.tone.x;

    if (params.tone.z > 0.5) {
        color = aces_film(color);
    } else {
        color = clamp(color, vec3<f32>(0.0), vec3<f32>(1.0));
    }

    return vec4<f32>(color, hdr_rgba.a);
}
