use std::time::Duration;

use glam::Vec3;
use tessel_core::{
    AppMode, Face, LowPolyMesh, MeshTexture, NodeKind, PaletteIndex, PixelCanvas, PixelCoord,
    SceneNode, SceneState, Transform, ViewportShadingMode, VoxelCoord, VoxelVolume,
};
use tessel_render::{OrbitCamera, PixelCamera2D, ViewportRenderer};

fn request_test_device() -> Option<(wgpu::Device, wgpu::Queue, wgpu::AdapterInfo)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .or_else(|| {
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: true,
        }))
    })?;

    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("tessel-render-gpu-smoke"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
        },
        None,
    ))
    .ok()?;
    Some((device, queue, info))
}

#[test]
fn smoke_nonfiltering_sampler_3d_pipelines_and_readback() {
    let Some((device, queue, info)) = request_test_device() else {
        eprintln!("skipping GPU smoke test: no compatible GPU or software adapter available");
        return;
    };
    eprintln!(
        "GPU smoke adapter: {} ({:?}, {:?})",
        info.name, info.backend, info.device_type
    );

    device.push_error_scope(wgpu::ErrorFilter::Validation);

    let mut viewport = ViewportRenderer::new(&device, 64, 64);
    let mut scene = SceneState::new();
    scene.active_mode = AppMode::Voxel;

    // Populate a voxel so the default palette texture + NonFiltering sampler path is drawn.
    let vox_id = scene.allocate_node_id();
    scene.nodes.insert(
        vox_id,
        SceneNode {
            id: vox_id,
            name: "SmokeVoxel".into(),
            kind: NodeKind::VoxelVolume,
            visible: true,
            parent: None,
            transform: Transform::default(),
        },
    );
    let mut vol = VoxelVolume::new();
    vol.set(
        VoxelCoord::new(0, 0, 0),
        Some(PaletteIndex::new(1).unwrap()),
    );
    scene.voxel_volumes.insert(vox_id, vol);

    // Add a custom-textured + normal-mapped lowpoly mesh to exercise custom bind groups
    // with NonFiltering sampler + textureSample on both palette_tex and normal_tex.
    let mesh_id = scene.allocate_node_id();
    scene.nodes.insert(
        mesh_id,
        SceneNode {
            id: mesh_id,
            name: "SmokeMesh".into(),
            kind: NodeKind::LowPolyMesh,
            visible: true,
            parent: None,
            transform: Transform::default(),
        },
    );
    let mut mesh = LowPolyMesh::new();
    mesh.vertices = vec![
        Vec3::new(-0.5, 0.0, 0.5),
        Vec3::new(0.5, 0.0, 0.5),
        Vec3::new(0.5, 1.0, 0.5),
        Vec3::new(-0.5, 1.0, 0.5),
    ];
    mesh.faces = vec![Face::with_uvs(
        vec![0, 1, 2, 3],
        PaletteIndex::new(2).unwrap(),
        vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
    )];
    mesh.texture = Some(MeshTexture {
        width: 2,
        height: 2,
        rgba: vec![
            255, 64, 64, 255, 64, 255, 64, 255, 64, 64, 255, 255, 255, 255, 64, 255,
        ],
    });
    mesh.normal_map = Some(MeshTexture {
        width: 2,
        height: 2,
        rgba: vec![
            128, 128, 255, 255, 128, 128, 255, 255, 128, 128, 255, 255, 128, 128, 255, 255,
        ],
    });
    scene.lowpoly_meshes.insert(mesh_id, mesh);
    scene.mark_modified();

    viewport.sync_scene(&device, &queue, &scene, None, None);

    let mut cam = OrbitCamera::new();
    cam.distance = 4.0;
    let cam_2d = PixelCamera2D::new();

    for mode in [ViewportShadingMode::Pbr, ViewportShadingMode::NprSumie] {
        scene.shading_mode = mode;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("tessel-smoke-3d-encoder"),
        });
        viewport.render(&queue, &mut encoder, &scene, &cam, &cam_2d, 1.0, false);
        queue.submit(Some(encoder.finish()));

        let (w, h, rgba) = viewport
            .capture_rgba_bytes_with_timeout(&device, &queue, Duration::from_secs(5))
            .expect("3D viewport RGBA readback must succeed");
        assert_eq!((w, h), (64, 64));
        assert_eq!(rgba.len(), 64 * 64 * 4);
        assert!(
            rgba.iter().any(|&b| b != 0),
            "rendered frame should not be all zeros"
        );
    }

    let png = viewport
        .capture_png_bytes(&device, &queue)
        .expect("PNG readback must succeed");
    assert!(png.starts_with(&[137, 80, 78, 71, 13, 10, 26, 10]));

    let validation_err = pollster::block_on(device.pop_error_scope());
    assert!(
        validation_err.is_none(),
        "unexpected wgpu validation error on 3D NonFiltering sampler pipeline: {validation_err:?}"
    );
}

#[test]
fn smoke_nonfiltering_sampler_pixel_2d_pipeline_and_readback() {
    let Some((device, queue, _info)) = request_test_device() else {
        eprintln!(
            "skipping Pixel 2D GPU smoke test: no compatible GPU or software adapter available"
        );
        return;
    };

    device.push_error_scope(wgpu::ErrorFilter::Validation);

    let mut viewport = ViewportRenderer::new(&device, 64, 64);
    let mut scene = SceneState::new();
    scene.active_mode = AppMode::Pixel;

    let pix_id = scene.allocate_node_id();
    scene.nodes.insert(
        pix_id,
        SceneNode {
            id: pix_id,
            name: "SmokePixel".into(),
            kind: NodeKind::PixelCanvas,
            visible: true,
            parent: None,
            transform: Transform::default(),
        },
    );
    let mut canvas = PixelCanvas::new(16, 16).unwrap();
    if let Some(frame) = canvas.current_frame_mut() {
        if let Some(layer) = frame.layers.first_mut() {
            layer.set(PixelCoord::new(0, 0), Some(PaletteIndex::new(1).unwrap()));
        }
    }
    scene.pixel_canvases.insert(pix_id, canvas);
    scene.selected_node = Some(pix_id);
    scene.mark_modified();

    viewport.sync_scene(&device, &queue, &scene, None, None);

    let cam_3d = OrbitCamera::new();
    let cam_2d = PixelCamera2D::new();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tessel-smoke-pixel-2d-encoder"),
    });
    viewport.render(&queue, &mut encoder, &scene, &cam_3d, &cam_2d, 1.0, false);
    queue.submit(Some(encoder.finish()));

    let (w, h, rgba) = viewport
        .capture_rgba_bytes(&device, &queue)
        .expect("Pixel 2D viewport RGBA readback must succeed");
    assert_eq!((w, h), (64, 64));
    assert_eq!(rgba.len(), 64 * 64 * 4);
    assert!(
        rgba.iter().any(|&b| b != 0),
        "pixel 2D frame should not be all zeros"
    );

    let validation_err = pollster::block_on(device.pop_error_scope());
    assert!(
        validation_err.is_none(),
        "unexpected wgpu validation error on Pixel 2D NonFiltering sampler pipeline: {validation_err:?}"
    );
}

#[test]
fn smoke_4x_msaa_and_npr_slots_pipeline_and_readback() {
    let Some((device, queue, _info)) = request_test_device() else {
        eprintln!(
            "skipping 4x MSAA + NPR slots GPU smoke test: no compatible GPU or software adapter available"
        );
        return;
    };

    device.push_error_scope(wgpu::ErrorFilter::Validation);

    let mut viewport = ViewportRenderer::new(&device, 64, 64);
    assert_eq!(viewport.sample_count(), 1);

    let mut scene = SceneState::new();
    scene.active_mode = AppMode::LowPoly;
    scene.shading_mode = ViewportShadingMode::NprSumie;
    scene.npr_look.msaa_samples = 4;

    let mesh_id = scene.allocate_node_id();
    scene.nodes.insert(
        mesh_id,
        SceneNode {
            id: mesh_id,
            name: "NprHead".into(),
            kind: NodeKind::LowPolyMesh,
            visible: true,
            parent: None,
            transform: Transform::default(),
        },
    );
    let mut mesh = tessel_core::mesh::ops::create_primitive(
        tessel_core::PrimitiveKind::Cube,
        1.0,
        PaletteIndex::new(2).unwrap(),
    );
    mesh.texture = Some(MeshTexture::solid(
        4,
        4,
        tessel_core::Rgba8::rgba(240, 220, 200, 255),
    ));
    mesh.matcap_map = Some(MeshTexture::solid(
        4,
        4,
        tessel_core::Rgba8::rgba(210, 210, 230, 255),
    ));
    mesh.face_sdf_map = Some(MeshTexture::solid(
        4,
        4,
        tessel_core::Rgba8::rgba(180, 180, 180, 255),
    ));
    mesh.hatching_ramp_map = Some(MeshTexture::solid(
        4,
        4,
        tessel_core::Rgba8::rgba(200, 140, 80, 255),
    ));
    mesh.material.matcap_strength = 0.5;
    mesh.material.hatching_scale = 4.0;
    mesh.material.face_sdf_light_override = Some([0.6, 0.3, 0.7]);
    scene.lowpoly_meshes.insert(mesh_id, mesh);
    scene.mark_modified();

    viewport.sync_scene(&device, &queue, &scene, None, None);
    assert_eq!(viewport.sample_count(), 4);

    let mut cam = OrbitCamera::new();
    cam.distance = 3.5;
    let cam_2d = PixelCamera2D::new();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tessel-smoke-4x-msaa-encoder"),
    });
    viewport.render(&queue, &mut encoder, &scene, &cam, &cam_2d, 1.0, false);
    queue.submit(Some(encoder.finish()));

    let (w, h, rgba) = viewport
        .capture_rgba_bytes_with_timeout(&device, &queue, Duration::from_secs(5))
        .expect("4x MSAA resolved RGBA readback must succeed");
    assert_eq!((w, h), (64, 64));
    assert_eq!(rgba.len(), 64 * 64 * 4);
    assert!(
        rgba.iter().any(|&b| b != 0),
        "4x MSAA resolved frame should not be all zeros"
    );

    let validation_err = pollster::block_on(device.pop_error_scope());
    assert!(
        validation_err.is_none(),
        "unexpected wgpu validation error on 4x MSAA + NPR slots pipeline: {validation_err:?}"
    );
}

#[test]
fn smoke_h1_hdr_aces_fog_palette_lut_and_minimized_resize() {
    let Some((device, queue, _info)) = request_test_device() else {
        eprintln!("skipping GPU smoke test: no compatible GPU or software adapter available");
        return;
    };

    device.push_error_scope(wgpu::ErrorFilter::Validation);

    let mut viewport = ViewportRenderer::new(&device, 64, 64);
    let mut scene = SceneState::new();
    scene.active_mode = AppMode::Voxel;
    scene.npr_look.msaa_samples = 4;
    scene.post_process = tessel_core::PostProcessSettings {
        bloom_enabled: false,
        exposure: 1.25,
        aces_enabled: true,
        fog_enabled: true,
        fog_color: [0.12, 0.16, 0.24],
        fog_density: 0.04,
        fog_start: 2.0,
        fog_height_falloff: 0.2,
        ..Default::default()
    };

    // Neon emissive voxel via Palette Material LUT.
    let neon_idx = PaletteIndex::new(9).unwrap();
    let mut neon_entry =
        tessel_core::PaletteEntry::matte("Neon", tessel_core::Rgba8::rgb(255, 180, 40));
    neon_entry.emissive = 8.0;
    scene.palette.set(neon_idx, neon_entry);

    let vox_id = scene.allocate_node_id();
    scene.nodes.insert(
        vox_id,
        SceneNode {
            id: vox_id,
            name: "NeonVoxel".into(),
            kind: NodeKind::VoxelVolume,
            visible: true,
            parent: None,
            transform: Transform::default(),
        },
    );
    let mut vol = VoxelVolume::new();
    vol.set(VoxelCoord::new(0, 0, 0), Some(neon_idx));
    scene.voxel_volumes.insert(vox_id, vol);
    scene.mark_modified();

    // Minimized window safety (0x0 clamped to 1x1) followed by restore to 64x64.
    assert!(viewport.resize(&device, 0, 0));
    assert_eq!(viewport.size(), (1, 1));
    assert!(viewport.resize(&device, 64, 64));
    assert_eq!(viewport.size(), (64, 64));

    viewport.sync_scene(&device, &queue, &scene, None, None);

    let mut cam = OrbitCamera::new();
    cam.distance = 2.0;
    let cam_2d = PixelCamera2D::new();
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("tessel-smoke-h1-hdr-encoder"),
    });
    viewport.render(&queue, &mut encoder, &scene, &cam, &cam_2d, 1.0, false);
    queue.submit(Some(encoder.finish()));

    let (w, h, rgba) = viewport
        .capture_rgba_bytes_with_timeout(&device, &queue, Duration::from_secs(5))
        .expect("H1 HDR+ACES+Fog readback must succeed");
    assert_eq!((w, h), (64, 64));
    assert!(rgba.iter().any(|&b| b != 0));

    let validation_err = pollster::block_on(device.pop_error_scope());
    assert!(
        validation_err.is_none(),
        "unexpected wgpu validation error on H1 HDR pipeline: {validation_err:?}"
    );
}
