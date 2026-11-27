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
};

struct ShadowMapUniform {
    light_view_proj: mat4x4<f32>,
    bias_params: vec4<f32>,
    /// [shading_mode (0=PBR, 1=NPR, 2=Silhouette, 3=Luminance), outline_width, fog_dist, paper_grain]
    npr_params: vec4<f32>,
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

@group(1) @binding(0)
var<uniform> shadow_map: ShadowMapUniform;

@group(1) @binding(1)
var shadow_depth_tex: texture_depth_2d;

@group(1) @binding(2)
var shadow_sampler: sampler_comparison;

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

fn sample_shadow_pcf(world_pos: vec3<f32>, geom_n: vec3<f32>, ndl: f32) -> f32 {
    if (shadow_map.bias_params.w < 0.5 || material.pbr.z < 0.5 || ndl <= 0.0) {
        return 1.0;
    }
    let normal_bias = shadow_map.bias_params.x * (1.0 - ndl * 0.65);
    let offset_pos = world_pos + geom_n * normal_bias;
    let ls = shadow_map.light_view_proj * vec4<f32>(offset_pos, 1.0);
    let proj = ls.xyz / ls.w;
    let uv = vec2<f32>(proj.x * 0.5 + 0.5, 0.5 - proj.y * 0.5);
    let in_bounds = uv.x >= 0.0 && uv.x <= 1.0 && uv.y >= 0.0 && uv.y <= 1.0 && proj.z >= 0.0 && proj.z <= 1.0;
    let depth_cmp = clamp(proj.z - shadow_map.bias_params.y, 0.0, 1.0);
    let texel = 1.0 / 2048.0;
    var vis = 0.0;
    for (var dy: i32 = -1; dy <= 1; dy = dy + 1) {
        for (var dx: i32 = -1; dx <= 1; dx = dx + 1) {
            let sample_uv = clamp(uv + vec2<f32>(f32(dx), f32(dy)) * texel, vec2<f32>(0.0), vec2<f32>(1.0));
            vis = vis + textureSampleCompare(shadow_depth_tex, shadow_sampler, sample_uv, depth_cmp);
        }
    }
    let pcf = vis / 9.0;
    return select(1.0, mix(1.0 - shadow_map.bias_params.z, 1.0, pcf), in_bounds);
}

fn triplanar_paper_grain(world_pos: vec3<f32>) -> f32 {
    let p = world_pos * 24.0;
    let g1 = fract(sin(dot(floor(p.xy), vec2<f32>(12.9898, 78.233))) * 43758.5453);
    let g2 = fract(sin(dot(floor(p.zy), vec2<f32>(39.346, 11.135))) * 24634.6345);
    return (g1 + g2) * 0.5 - 0.5;
}

@fragment
fn fs_main(in: VertexOut, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    let sampled_rgba = textureSample(palette_tex, palette_sampler, in.palette_uv);
    let alpha_cutoff = material.pbr.w;
    if (alpha_cutoff > 0.0 && sampled_rgba.a < alpha_cutoff) {
        discard;
    }
    let base_color = sampled_rgba.rgb;
    let shading_mode = i32(round(shadow_map.npr_params.x));

    if (shading_mode == 2) {
        // Silhouette readability check mode
        return vec4<f32>(vec3<f32>(0.06, 0.055, 0.065), sampled_rgba.a);
    }
    if (material.emissive.w > 0.5) {
        // KHR_materials_unlit
        return vec4<f32>(base_color + material.emissive.rgb, sampled_rgba.a);
    }

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
    let key_dir = normalize(camera.light_dir.xyz);
    let fill_dir = normalize(vec3<f32>(-key_dir.x, 0.35, -key_dir.z));

    let ndl = max(dot(n, key_dir), 0.0);
    let shadow_vis = sample_shadow_pcf(in.world_pos, geom_n, ndl);
    let view_vec = camera.camera_pos.xyz - in.world_pos;
    let cam_dist = length(view_vec);
    let view_dir = view_vec / max(cam_dist, 1e-4);
    let h_raw = key_dir + view_dir;
    let h_len = length(h_raw);
    let half_vec = select(n, h_raw / h_len, h_len > 1e-5);

    var lit: vec3<f32>;
    if (shading_mode == 1) {
        // NPR / Sumi-e Ink-Wash Preview: stepped 3-band ink ramp, optional Face SDF shadow map,
        // optional multi-level hatching ramp, optional MatCap, dark Fresnel ink rim,
        // stylized stepped specular, object-space washi paper grain, and atmospheric depth ink-fade.
        var lit_tone = ndl * shadow_vis;
        if (material.npr_slots.y > 0.5) {
            // Face SDF Shadow Map with optional light-direction override (anime / sumi-e face lighting)
            let override_len = length(material.face_sdf_light.xyz);
            let sdf_light = select(
                key_dir,
                material.face_sdf_light.xyz / max(override_len, 1e-5),
                material.face_sdf_light.w > 0.5 && override_len > 1e-5,
            );
            let l_xz = normalize(vec2<f32>(sdf_light.x, sdf_light.z) + vec2<f32>(1e-5, 0.0));
            let flip_u = clamp(select(in.palette_uv.x, 1.0 - in.palette_uv.x, l_xz.x < 0.0), 0.0, 1.0);
            let sdf_sample = textureSample(face_sdf_tex, palette_sampler, vec2<f32>(flip_u, clamp(in.palette_uv.y, 0.0, 1.0))).r;
            let sdf_threshold = clamp(1.0 - (l_xz.y * 0.5 + 0.5), 0.0, 1.0);
            let sdf_lit = select(0.15, 1.0, sdf_sample >= sdf_threshold);
            lit_tone = sdf_lit * shadow_vis;
        }
        let band = select(select(0.32, 0.68, lit_tone > 0.22), 1.0, lit_tone > 0.58);
        let dark_fresnel = pow(1.0 - max(dot(n, view_dir), 0.0), 2.8) * 0.45;
        let spec_step = select(0.0, 0.22, pow(max(dot(n, half_vec), 0.0), 32.0) * shadow_vis > 0.45);
        let grain = triplanar_paper_grain(in.world_pos) * shadow_map.npr_params.w;
        let ink_tint = vec3<f32>(0.08, 0.075, 0.09);
        let paper_tint = vec3<f32>(0.93, 0.90, 0.84);
        var sumi = base_color * band * in.ao + vec3<f32>(spec_step + grain) + material.emissive.rgb;

        if (material.npr_slots.z > 0.5) {
            // Multi-level hatching ramp: R=light hatch, G=mid hatch, B=dense shadow hatch
            let hatch_uv = fract(in.palette_uv * max(material.npr_slots.w, 0.1));
            let hatch_rgb = textureSample(hatching_tex, palette_sampler, hatch_uv).rgb;
            let shade_density = clamp(1.0 - lit_tone, 0.0, 1.0);
            let hatch_mask = select(
                select(hatch_rgb.r, hatch_rgb.g, shade_density > 0.35),
                hatch_rgb.b,
                shade_density > 0.70,
            );
            sumi = mix(sumi, sumi * hatch_mask, shade_density * 0.65);
        }

        if (material.npr_slots.x > 0.001) {
            // Custom MatCap spherical environment lookup in view-aligned basis
            let cam_up = vec3<f32>(0.0, 1.0, 0.0);
            let cam_right_raw = cross(cam_up, view_dir);
            let cam_right = select(
                vec3<f32>(1.0, 0.0, 0.0),
                normalize(cam_right_raw),
                length(cam_right_raw) > 1e-4,
            );
            let cam_true_up = normalize(cross(view_dir, cam_right));
            let mc_u = clamp(dot(n, cam_right) * 0.5 + 0.5, 0.0, 1.0);
            let mc_v = clamp(0.5 - dot(n, cam_true_up) * 0.5, 0.0, 1.0);
            let matcap_col = textureSample(matcap_tex, palette_sampler, vec2<f32>(mc_u, mc_v)).rgb;
            sumi = mix(sumi, sumi * matcap_col * 1.4, clamp(material.npr_slots.x, 0.0, 1.0));
        }

        sumi = mix(sumi, ink_tint, dark_fresnel);
        let fog_dist = max(shadow_map.npr_params.z, 1.0);
        let fog_factor = clamp((cam_dist - fog_dist * 0.35) / fog_dist, 0.0, 0.55);
        lit = mix(sumi, paper_tint, fog_factor);
    } else {
        let fill = max(dot(n, fill_dir), 0.0) * 0.28;
        let hemi = mix(0.28, 0.46, n.y * 0.5 + 0.5);
        let rim = pow(1.0 - max(dot(n, view_dir), 0.0), 3.0) * 0.12;

        let metallic = clamp(material.pbr.x, 0.0, 1.0);
        let roughness = clamp(material.pbr.y, 0.05, 1.0);
        let spec_pow = mix(128.0, 4.0, roughness * roughness);
        let spec_strength = mix(0.04, 1.0, metallic) + (1.0 - roughness) * 0.45;
        let spec_color = mix(vec3<f32>(1.0), base_color, metallic);
        let specular = spec_color * pow(max(dot(n, half_vec), 0.0), spec_pow) * spec_strength * shadow_vis;

        let diffuse = base_color * (1.0 - metallic * 0.65) * (hemi + ndl * 0.68 * shadow_vis + fill);
        lit = (diffuse + specular) * in.ao + vec3<f32>(rim) + material.emissive.rgb;
    }

    if (shading_mode == 3) {
        let luma = dot(lit, vec3<f32>(0.2126, 0.7152, 0.0722));
        return vec4<f32>(vec3<f32>(luma), sampled_rgba.a);
    }

    return vec4<f32>(lit, sampled_rgba.a);
}
