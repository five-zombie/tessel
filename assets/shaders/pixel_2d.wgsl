struct PixelViewUniform {
    viewport_size: vec2<f32>,
    canvas_size: vec2<f32>,
    pan: vec2<f32>,
    zoom: f32,
    show_grid: f32,
};

@group(0) @binding(0)
var<uniform> view: PixelViewUniform;

@group(0) @binding(1)
var canvas_tex: texture_2d<f32>;

@group(0) @binding(2)
var canvas_sampler: sampler;

struct VertexOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) screen_uv: vec2<f32>,
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
    // Map NDC [-1, 1] to top-left screen UV [0, 1]
    out.screen_uv = vec2<f32>(pos.x * 0.5 + 0.5, 0.5 - pos.y * 0.5);
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let screen_px = in.screen_uv * view.viewport_size;
    let center = view.viewport_size * 0.5 + view.pan;
    let canvas_px = (screen_px - center) / max(view.zoom, 1.0) + view.canvas_size * 0.5;

    if canvas_px.x < 0.0 || canvas_px.y < 0.0 || canvas_px.x >= view.canvas_size.x || canvas_px.y >= view.canvas_size.y {
        // Subtle 1px amber-tinted frame border around canvas
        let outside_x = max(-canvas_px.x, canvas_px.x - view.canvas_size.x) * view.zoom;
        let outside_y = max(-canvas_px.y, canvas_px.y - view.canvas_size.y) * view.zoom;
        let border_dist = max(outside_x, outside_y);
        if border_dist <= 1.5 {
            return vec4<f32>(0.33, 0.26, 0.22, 1.0);
        }
        discard;
    }

    let cell = floor(canvas_px);
    let checker = ((i32(cell.x) + i32(cell.y)) & 1) == 0;
    let bg = select(vec3<f32>(0.09, 0.10, 0.13), vec3<f32>(0.13, 0.14, 0.18), checker);

    let tex_uv = (cell + vec2<f32>(0.5, 0.5)) / view.canvas_size;
    let sampled = textureSample(canvas_tex, canvas_sampler, tex_uv);
    var color = mix(bg, sampled.rgb, sampled.a);

    if view.show_grid > 0.5 && view.zoom >= 6.0 {
        let frac_px = fract(canvas_px);
        let edge_px = 1.0 / view.zoom;
        if frac_px.x < edge_px || frac_px.y < edge_px {
            color = mix(color, vec3<f32>(0.33, 0.26, 0.22), 0.35);
        }
    }

    return vec4<f32>(color, 1.0);
}
