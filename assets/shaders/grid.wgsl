struct CameraUniform {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> camera: CameraUniform;

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

fn grid_line(coord: vec2<f32>, scale: f32) -> f32 {
    let scaled = coord * scale;
    let deriv = fwidth(scaled);
    let grid = abs(fract(scaled - 0.5) - 0.5) / max(deriv, vec2<f32>(0.0001));
    let line = min(grid.x, grid.y);
    return 1.0 - min(line, 1.0);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let near_pt = unproject(in.ndc, 0.0);
    let far_pt = unproject(in.ndc, 1.0);
    let denom = far_pt.y - near_pt.y;
    if abs(denom) < 0.00001 {
        discard;
    }

    let t = -near_pt.y / denom;
    if t <= 0.0 {
        discard;
    }

    let world_pos = near_pt + t * (far_pt - near_pt);
    let dist = length(world_pos.xz - camera.camera_pos.xz);
    let fade = 1.0 - smoothstep(14.0, 48.0, dist);
    if fade <= 0.001 {
        discard;
    }

    let minor = grid_line(world_pos.xz, 1.0) * 0.22;
    let major = grid_line(world_pos.xz, 0.2) * 0.42;

    var color = vec3<f32>(0.24, 0.26, 0.31);
    var alpha = max(minor, major);

    let axis_width = max(fwidth(world_pos.xz), vec2<f32>(0.002)) * 1.4;
    if abs(world_pos.z) < axis_width.y {
        color = vec3<f32>(0.82, 0.34, 0.30);
        alpha = max(alpha, 0.78);
    }
    if abs(world_pos.x) < axis_width.x {
        color = vec3<f32>(0.30, 0.54, 0.85);
        alpha = max(alpha, 0.78);
    }

    let final_alpha = alpha * fade;
    if final_alpha <= 0.002 {
        discard;
    }

    return vec4<f32>(color, final_alpha);
}
