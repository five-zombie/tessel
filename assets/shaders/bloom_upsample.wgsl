struct BloomStepUniform {
    /// `[src_texel_x, src_texel_y, filter_radius, 0.0]`
    params: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> step: BloomStepUniform;

@group(0) @binding(1)
var src_tex: texture_2d<f32>;

@group(0) @binding(2)
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

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let rx = step.params.x * step.params.z;
    let ry = step.params.y * step.params.z;
    let uv = in.uv;

    // 9-tap tent filter (1, 2, 1 / 2, 4, 2 / 1, 2, 1) / 16
    let a = textureSample(src_tex, linear_sampler, uv + vec2<f32>(-rx, -ry)).rgb;
    let b = textureSample(src_tex, linear_sampler, uv + vec2<f32>(0.0, -ry)).rgb;
    let c = textureSample(src_tex, linear_sampler, uv + vec2<f32>(rx, -ry)).rgb;

    let d = textureSample(src_tex, linear_sampler, uv + vec2<f32>(-rx, 0.0)).rgb;
    let e = textureSample(src_tex, linear_sampler, uv).rgb;
    let f = textureSample(src_tex, linear_sampler, uv + vec2<f32>(rx, 0.0)).rgb;

    let g = textureSample(src_tex, linear_sampler, uv + vec2<f32>(-rx, ry)).rgb;
    let h = textureSample(src_tex, linear_sampler, uv + vec2<f32>(0.0, ry)).rgb;
    let i = textureSample(src_tex, linear_sampler, uv + vec2<f32>(rx, ry)).rgb;

    let tent = (a + c + g + i) * (1.0 / 16.0)
        + (b + d + f + h) * (2.0 / 16.0)
        + e * (4.0 / 16.0);

    return vec4<f32>(max(tent, vec3<f32>(0.0)), 1.0);
}
