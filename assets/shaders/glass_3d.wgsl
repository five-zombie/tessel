struct CameraUniform {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    camera_pos: vec4<f32>,
    light_dir: vec4<f32>,
};

struct MaterialUniform {
    /// [metallic, roughness, receive_shadow, alpha_cutoff]
    pbr: vec4<f32>,
    /// [emissive_r, emissive_g, emissive_b, unlit_flag]
    emissive: vec4<f32>,
    /// [matcap_strength, has_face_sdf, has_hatching, hatching_scale]
    npr_slots: vec4<f32>,
    /// [light_x, light_y, light_z, has_face_sdf_light_override]
    face_sdf_light: vec4<f32>,
    /// [emissive_intensity, use_palette_mat_lut, transmission, ior]
    hdr_flags: vec4<f32>,
};

struct ShadowMapUniform {
    light_view_proj: mat4x4<f32>,
    bias_params: vec4<f32>,
    npr_params: vec4<f32>,
    fog_color_density: vec4<f32>,
    fog_params: vec4<f32>,
    lookdev_tint: vec4<f32>,
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

@group(0) @binding(5)
var matcap_tex: texture_2d<f32>;

@group(0) @binding(6)
var face_sdf_tex: texture_2d<f32>;

@group(0) @binding(7)
var hatching_tex: texture_2d<f32>;

@group(0) @binding(8)
var palette_mat_tex: texture_2d<f32>;

@group(1) @binding(0)
var<uniform> shadow_map: ShadowMapUniform;

@group(1) @binding(1)
var shadow_depth_tex: texture_depth_2d;

@group(1) @binding(2)
var shadow_sampler: sampler_comparison;

@group(2) @binding(0)
var opaque_copy_tex: texture_2d<f32>;

@group(2) @binding(1)
var opaque_sampler: sampler;

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
    let sampled_rgba = textureSample(palette_tex, palette_sampler, in.palette_uv);
    let base_color = sampled_rgba.rgb;

    let use_lut = material.hdr_flags.y > 0.5;
    let lut0 = textureSample(palette_mat_tex, palette_sampler, vec2<f32>(in.palette_uv.x, 0.25));
    let lut1 = textureSample(palette_mat_tex, palette_sampler, vec2<f32>(in.palette_uv.x, 0.75));

    let roughness = clamp(select(material.pbr.y, lut0.r, use_lut), 0.05, 1.0);
    let metallic = clamp(select(material.pbr.x, lut0.g, use_lut), 0.0, 1.0);
    let emissive_rgb = select(
        material.emissive.rgb * max(material.hdr_flags.x, 0.0),
        base_color * max(lut0.b, 0.0),
        use_lut,
    );
    let transmission = clamp(select(material.hdr_flags.z, lut0.a, use_lut), 0.0, 1.0);
    let ior = clamp(select(material.hdr_flags.w, lut1.r, use_lut), 1.0, 2.5);

    let raw_n_len = length(in.normal);
    let raw_n = select(vec3<f32>(0.0, 1.0, 0.0), in.normal / raw_n_len, raw_n_len > 1e-5);
    let geom_n = select(-raw_n, raw_n, front_facing);
    let t_raw = in.tangent.xyz - geom_n * dot(geom_n, in.tangent.xyz);
    let t_len = length(t_raw);
    let sampled_n = textureSample(normal_tex, palette_sampler, in.palette_uv).xyz * 2.0 - vec3<f32>(1.0);
    var n = geom_n;
    if (t_len > 1e-5) {
        let t_dir = t_raw / t_len;
        let b_raw = cross(geom_n, t_dir);
        let b_len = length(b_raw);
        if (b_len > 1e-5) {
            let b_dir = (b_raw / b_len) * in.tangent.w;
            let n_raw = t_dir * sampled_n.x + b_dir * sampled_n.y + geom_n * sampled_n.z;
            let n_len = length(n_raw);
            if (n_len > 1e-5) {
                n = n_raw / n_len;
            }
        }
    }

    let view_vec = camera.camera_pos.xyz - in.world_pos;
    let cam_dist = length(view_vec);
    let view_dir = view_vec / max(cam_dist, 1e-4);
    let key_dir = normalize(camera.light_dir.xyz);
    let h_raw = key_dir + view_dir;
    let h_len = length(h_raw);
    let half_vec = select(n, h_raw / h_len, h_len > 1e-5);

    // Build view-aligned screen basis for normal-driven screen-space refraction offset
    let world_up = vec3<f32>(0.0, 1.0, 0.0);
    let cam_right_raw = cross(world_up, view_dir);
    let cam_right = select(
        vec3<f32>(1.0, 0.0, 0.0),
        normalize(cam_right_raw),
        length(cam_right_raw) > 1e-4,
    );
    let cam_up = normalize(cross(view_dir, cam_right));

    let dims = vec2<f32>(textureDimensions(opaque_copy_tex));
    let screen_uv = in.clip_pos.xy / max(dims, vec2<f32>(1.0));
    let refract_scale = clamp((ior - 1.0) * 0.045 * transmission, 0.0, 0.08);
    let refract_offset = vec2<f32>(dot(n, cam_right), -dot(n, cam_up)) * refract_scale;
    let refract_uv = clamp(screen_uv + refract_offset, vec2<f32>(0.001), vec2<f32>(0.999));
    let bg_hdr = max(textureSample(opaque_copy_tex, opaque_sampler, refract_uv).rgb, vec3<f32>(0.0));

    // Schlick Fresnel reflection from dielectric IOR
    let r0_root = (ior - 1.0) / max(ior + 1.0, 1e-4);
    let f0 = clamp(r0_root * r0_root, 0.02, 0.98);
    let ndv = max(dot(n, view_dir), 0.0);
    let fresnel = clamp(f0 + (1.0 - f0) * pow(1.0 - ndv, 5.0), 0.0, 0.92);

    // Transmission tint + smooth diffuse fallback for partial transmission
    let key_tint = shadow_map.lookdev_tint.rgb;
    let amb_scale = shadow_map.lookdev_tint.w;
    let ndl = max(dot(n, key_dir), 0.0);
    let hemi = mix(0.28, 0.46, n.y * 0.5 + 0.5);
    let opaque_diffuse = base_color * (1.0 - metallic * 0.65) * (vec3<f32>(hemi * amb_scale) + key_tint * (ndl * 0.68));
    let glass_tint = mix(vec3<f32>(1.0), base_color, 0.35 + 0.40 * (1.0 - transmission));
    let transmitted = bg_hdr * glass_tint;
    let body = mix(opaque_diffuse, transmitted, transmission);

    let refl_dir = reflect(-view_dir, n);
    let sky_refl = mix(
        vec3<f32>(0.05, 0.08, 0.13),
        vec3<f32>(0.45, 0.60, 0.85),
        clamp(refl_dir.y * 0.5 + 0.5, 0.0, 1.0),
    ) * key_tint;
    let spec_pow = mix(256.0, 16.0, roughness * roughness);
    let spec_strength = mix(1.35, 0.25, roughness);
    let sun_spec = key_tint * pow(max(dot(n, half_vec), 0.0), spec_pow) * spec_strength;

    var lit = mix(body, sky_refl + base_color * 0.15, fresnel) + sun_spec + emissive_rgb;

    // Forward Distance & Height Fog (matching mesh_3d.wgsl, 4x MSAA safe)
    if (shadow_map.fog_params.x > 0.5) {
        let dist_over = max(cam_dist - shadow_map.fog_params.y, 0.0);
        let height_atten = exp(-max(in.world_pos.y, 0.0) * shadow_map.fog_params.z);
        let fog_amount = 1.0 - exp(-dist_over * shadow_map.fog_color_density.w * height_atten);
        lit = mix(lit, shadow_map.fog_color_density.rgb, clamp(fog_amount, 0.0, 1.0));
    }

    return vec4<f32>(lit, sampled_rgba.a);
}
