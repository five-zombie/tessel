struct ShadowMapUniform {
    light_view_proj: mat4x4<f32>,
    bias_params: vec4<f32>,
    npr_params: vec4<f32>,
    fog_color_density: vec4<f32>,
    fog_params: vec4<f32>,
    lookdev_tint: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> shadow_map: ShadowMapUniform;

struct VertexIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) palette_uv: vec2<f32>,
    @location(3) ao: f32,
    @location(4) tangent: vec4<f32>,
};

@vertex
fn vs_main(in: VertexIn) -> @builtin(position) vec4<f32> {
    return shadow_map.light_view_proj * vec4<f32>(in.position, 1.0);
}
