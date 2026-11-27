struct BloomStepUniform {
    /// `[src_texel_x, src_texel_y, threshold, prefilter_flag (1.0 for mip 0, 0.0 for mips 1..4)]`
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

fn sample_safe(uv: vec2<f32>) -> vec3<f32> {
    return clamp(textureSample(src_tex, linear_sampler, uv).rgb, vec3<f32>(0.0), vec3<f32>(65000.0));
}

fn soft_threshold(c: vec3<f32>, threshold: f32) -> vec3<f32> {
    let br = max(c.r, max(c.g, c.b));
    let knee = max(threshold * 0.5, 1e-4);
    let soft = clamp(br - threshold + knee, 0.0, 2.0 * knee);
    let contrib = max(br - threshold, (soft * soft) / (4.0 * knee));
    let scale = contrib / max(br, 1e-5);
    return c * scale;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let dx = step.params.x;
    let dy = step.params.y;
    let uv = in.uv;

    // 13-tap energy-conserving Dual-Kawase / Jimenez downsample filter
    let a = sample_safe(uv + vec2<f32>(-2.0 * dx, -2.0 * dy));
    let b = sample_safe(uv + vec2<f32>(0.0, -2.0 * dy));
    let c = sample_safe(uv + vec2<f32>(2.0 * dx, -2.0 * dy));

    let d = sample_safe(uv + vec2<f32>(-2.0 * dx, 0.0));
    let e = sample_safe(uv);
    let f = sample_safe(uv + vec2<f32>(2.0 * dx, 0.0));

    let g = sample_safe(uv + vec2<f32>(-2.0 * dx, 2.0 * dy));
    let h = sample_safe(uv + vec2<f32>(0.0, 2.0 * dy));
    let i = sample_safe(uv + vec2<f32>(2.0 * dx, 2.0 * dy));

    let j = sample_safe(uv + vec2<f32>(-dx, -dy));
    let k = sample_safe(uv + vec2<f32>(dx, -dy));
    let l = sample_safe(uv + vec2<f32>(-dx, dy));
    let m = sample_safe(uv + vec2<f32>(dx, dy));

    var color = e * 0.125
        + (j + k + l + m) * 0.125
        + (b + d + f + h) * 0.0625
        + (a + c + g + i) * 0.03125;

    if (step.params.w > 0.5) {
        color = soft_threshold(color, max(step.params.z, 0.0));
    }

    return vec4<f32>(max(color, vec3<f32>(0.0)), 1.0);
}
