use egui::{Frame, Margin, RichText, Stroke};
use tessel_core::{AppMode, Command, FaceId, HistoryStack, SceneState};

use super::icons::{self, IconItem, ToolbarAction};
use super::theme::tokens;

pub fn show_toolbar(ctx: &egui::Context, scene: &mut SceneState, history: &mut HistoryStack) {
    let frame = Frame::NONE
        .fill(tokens::SURFACE_LOW)
        .inner_margin(Margin::symmetric(6, 10))
        .stroke(Stroke::new(1.0_f32, tokens::OUTLINE_VARIANT));

    egui::SidePanel::left("tessel_toolbar")
        .exact_width(112.0)
        .resizable(false)
        .frame(frame)
        .show(ctx, |ui| {
            ui.vertical_centered(|ui| match scene.active_mode {
                AppMode::Pixel => {
                    let items: Vec<IconItem> = tessel_core::PixelTool::ALL
                        .iter()
                        .map(|tool| {
                            let (icon, tip) = icons::pixel_tool_icon(*tool);
                            IconItem {
                                icon,
                                tip,
                                active: scene.pixel_tool == *tool,
                                action: ToolbarAction::PixelTool(*tool),
                            }
                        })
                        .collect();
                    if let Some(action) = icons::icon_group(ui, "pixel_tools", "TOOLS", &items) {
                        fire_toolbar_action(action, scene, history);
                    }
                    ui.separator();
                    let sym = [
                        IconItem {
                            icon: icons::ToolIcon::AxisX,
                            tip: "Mirror X",
                            active: scene.pixel_symmetry.x,
                            action: ToolbarAction::TogglePixelSymX,
                        },
                        IconItem {
                            icon: icons::ToolIcon::AxisY,
                            tip: "Mirror Y",
                            active: scene.pixel_symmetry.y,
                            action: ToolbarAction::TogglePixelSymY,
                        },
                    ];
                    if let Some(action) = icons::icon_group(ui, "pixel_sym", "SYM", &sym) {
                        fire_toolbar_action(action, scene, history);
                    }
                    ui.separator();
                    if scene.active_pixel_node().is_some() {
                        let convert = [IconItem {
                            icon: icons::ToolIcon::ToVoxel,
                            tip: "To Voxel",
                            active: false,
                            action: ToolbarAction::ConvertToVoxel,
                        }];
                        if let Some(action) =
                            icons::icon_group(ui, "pixel_convert", "CONVERT", &convert)
                        {
                            fire_toolbar_action(action, scene, history);
                        }
                    }
                }
                AppMode::Voxel => {
                    let items: Vec<IconItem> = tessel_core::VoxelTool::ALL
                        .iter()
                        .map(|tool| {
                            let (icon, tip) = icons::voxel_tool_icon(*tool);
                            IconItem {
                                icon,
                                tip,
                                active: scene.voxel_tool == *tool,
                                action: ToolbarAction::VoxelTool(*tool),
                            }
                        })
                        .collect();
                    if let Some(action) = icons::icon_group(ui, "voxel_tools", "TOOLS", &items) {
                        fire_toolbar_action(action, scene, history);
                    }
                    ui.separator();
                    let mirror = [
                        IconItem {
                            icon: icons::ToolIcon::AxisX,
                            tip: "Mirror X",
                            active: scene.voxel_symmetry.x,
                            action: ToolbarAction::ToggleVoxelSymX,
                        },
                        IconItem {
                            icon: icons::ToolIcon::AxisY,
                            tip: "Mirror Y",
                            active: scene.voxel_symmetry.y,
                            action: ToolbarAction::ToggleVoxelSymY,
                        },
                        IconItem {
                            icon: icons::ToolIcon::AxisZ,
                            tip: "Mirror Z",
                            active: scene.voxel_symmetry.z,
                            action: ToolbarAction::ToggleVoxelSymZ,
                        },
                    ];
                    if let Some(action) = icons::icon_group(ui, "voxel_mirror", "MIRROR", &mirror) {
                        fire_toolbar_action(action, scene, history);
                    }
                    ui.separator();
                    if scene.active_voxel_node().is_some() {
                        let convert = [IconItem {
                            icon: icons::ToolIcon::ToMesh,
                            tip: "To Mesh",
                            active: false,
                            action: ToolbarAction::ConvertToMesh,
                        }];
                        if let Some(action) =
                            icons::icon_group(ui, "voxel_convert", "CONVERT", &convert)
                        {
                            fire_toolbar_action(action, scene, history);
                        }
                    }
                }
                AppMode::LowPoly => {
                    let shapes: Vec<IconItem> = tessel_core::PrimitiveKind::ALL
                        .iter()
                        .map(|kind| {
                            let (icon, tip) = icons::primitive_icon(*kind);
                            IconItem {
                                icon,
                                tip,
                                active: false,
                                action: ToolbarAction::Primitive(*kind),
                            }
                        })
                        .collect();
                    if let Some(action) = icons::icon_group(ui, "prim_shapes", "SHAPE", &shapes) {
                        fire_toolbar_action(action, scene, history);
                    }
                    if icons::text_icon_button(
                        ui,
                        "Terrain",
                        "Create Low-Poly Terrain (v4.1)",
                        false,
                    ) {
                        fire_toolbar_action(ToolbarAction::CreateTerrain, scene, history);
                    }
                    ui.separator();
                    if scene.active_lowpoly_node().is_some() {
                        let mut paint: Vec<IconItem> = vec![
                            IconItem {
                                icon: icons::ToolIcon::Brush,
                                tip: "Paint",
                                active: scene.lowpoly_paint_mode,
                                action: ToolbarAction::TogglePaint,
                            },
                            IconItem {
                                icon: icons::ToolIcon::Picture,
                                tip: "Load Tex",
                                active: false,
                                action: ToolbarAction::LoadTexture { normal: false },
                            },
                            IconItem {
                                icon: icons::ToolIcon::PictureWave,
                                tip: "Load Norm",
                                active: false,
                                action: ToolbarAction::LoadTexture { normal: true },
                            },
                        ];
                        if scene.lowpoly_paint_mode {
                            paint.push(IconItem {
                                icon: icons::ToolIcon::PaintBucket,
                                tip: "Fill Tex",
                                active: false,
                                action: ToolbarAction::FillTexture,
                            });
                            paint.push(IconItem {
                                icon: icons::ToolIcon::AxisX,
                                tip: "Mirror X",
                                active: scene.lowpoly_mirror_x,
                                action: ToolbarAction::ToggleMirrorX,
                            });
                        }
                        if let Some(action) = icons::icon_group(ui, "mesh_paint", "PAINT", &paint) {
                            fire_toolbar_action(action, scene, history);
                        }
                        ui.separator();
                        let is_smooth = scene
                            .active_lowpoly_node()
                            .and_then(|id| scene.lowpoly_meshes.get(&id))
                            .is_some_and(|m| m.smooth_shading);
                        let edit = [
                            IconItem {
                                icon: icons::ToolIcon::Extrude,
                                tip: "Extrude",
                                active: false,
                                action: ToolbarAction::Extrude,
                            },
                            IconItem {
                                icon: icons::ToolIcon::Inset,
                                tip: "Inset",
                                active: false,
                                action: ToolbarAction::Inset,
                            },
                            IconItem {
                                icon: icons::ToolIcon::Bevel,
                                tip: "Bevel",
                                active: false,
                                action: ToolbarAction::Bevel,
                            },
                            IconItem {
                                icon: icons::ToolIcon::Smooth,
                                tip: "Smooth",
                                active: false,
                                action: ToolbarAction::Smooth,
                            },
                            IconItem {
                                icon: icons::ToolIcon::Decimate,
                                tip: "Decimate",
                                active: false,
                                action: ToolbarAction::Decimate,
                            },
                            IconItem {
                                icon: icons::ToolIcon::ShadeSmooth,
                                tip: "Shade Smooth",
                                active: is_smooth,
                                action: ToolbarAction::ToggleShade,
                            },
                            IconItem {
                                icon: icons::ToolIcon::BoxFill,
                                tip: "Subdiv Face",
                                active: false,
                                action: ToolbarAction::SubdivideFace,
                            },
                            IconItem {
                                icon: icons::ToolIcon::AxisX,
                                tip: "Mirror Mesh X",
                                active: false,
                                action: ToolbarAction::MirrorMeshX,
                            },
                            IconItem {
                                icon: icons::ToolIcon::Reset,
                                tip: "Apply Xform",
                                active: false,
                                action: ToolbarAction::ApplyTransform,
                            },
                            IconItem {
                                icon: icons::ToolIcon::Picture,
                                tip: "Project UVs",
                                active: false,
                                action: ToolbarAction::ProjectUvs,
                            },
                        ];
                        if let Some(action) = icons::icon_group(ui, "mesh_edit", "EDIT", &edit) {
                            fire_toolbar_action(action, scene, history);
                        }
                        if scene.lowpoly_paint_mode {
                            ui.label(
                                RichText::new(format!("R: {:.0}", scene.lowpoly_brush_radius))
                                    .monospace()
                                    .size(10.0)
                                    .color(tokens::ON_SURFACE_VARIANT),
                            );
                            ui.add(
                                egui::Slider::new(&mut scene.lowpoly_brush_radius, 2.0..=40.0)
                                    .show_value(false),
                            );
                        }
                    }
                }
                AppMode::Character => {
                    let rig = [
                        IconItem {
                            icon: icons::ToolIcon::BipedRig,
                            tip: "Biped 7",
                            active: false,
                            action: ToolbarAction::ApplyBipedRig,
                        },
                        IconItem {
                            icon: icons::ToolIcon::ModeCharacter,
                            tip: "Humanoid 19",
                            active: false,
                            action: ToolbarAction::ApplyHumanoidRig,
                        },
                    ];
                    if let Some(action) = icons::icon_group(ui, "char_rig", "RIG", &rig) {
                        fire_toolbar_action(action, scene, history);
                    }
                    if scene.character_rig.is_some() && scene.active_lowpoly_node().is_some() {
                        ui.separator();
                        let weight_tools = [
                            IconItem {
                                icon: icons::ToolIcon::Brush,
                                tip: "Weight Paint",
                                active: scene.weight_paint_mode,
                                action: ToolbarAction::ToggleWeightPaint,
                            },
                            IconItem {
                                icon: icons::ToolIcon::Eraser,
                                tip: "Clear Weights",
                                active: false,
                                action: ToolbarAction::ClearMeshWeights,
                            },
                        ];
                        if let Some(action) =
                            icons::icon_group(ui, "char_weights", "WEIGHT", &weight_tools)
                        {
                            fire_toolbar_action(action, scene, history);
                        }
                        if scene.weight_paint_mode {
                            ui.label(
                                RichText::new(format!("R: {:.2}m", scene.weight_brush_radius))
                                    .monospace()
                                    .size(10.0)
                                    .color(tokens::ON_SURFACE_VARIANT),
                            );
                            ui.add(
                                egui::Slider::new(&mut scene.weight_brush_radius, 0.05..=2.0)
                                    .show_value(false),
                            );
                            ui.label(
                                RichText::new(format!(
                                    "S: {:.0}%",
                                    scene.weight_brush_strength * 100.0
                                ))
                                .monospace()
                                .size(10.0)
                                .color(tokens::ON_SURFACE_VARIANT),
                            );
                            ui.add(
                                egui::Slider::new(&mut scene.weight_brush_strength, 0.05..=1.0)
                                    .show_value(false),
                            );
                        }
                    }
                }
            });
        });
}

/// Executes a toolbar action. To add a button, add one [`IconItem`] line to a
/// group above with either an existing action or a new [`ToolbarAction`]
/// variant handled here.
fn fire_toolbar_action(action: ToolbarAction, scene: &mut SceneState, history: &mut HistoryStack) {
    match action {
        ToolbarAction::PixelTool(tool) => scene.pixel_tool = tool,
        ToolbarAction::VoxelTool(tool) => scene.voxel_tool = tool,
        ToolbarAction::Primitive(kind) => {
            let _ = history.execute(
                scene,
                Command::CreatePrimitiveMesh {
                    name: kind.label().into(),
                    kind,
                    size: 0.8,
                    material: scene.active_color,
                },
            );
        }
        ToolbarAction::CreateTerrain => {
            let _ = history.execute(
                scene,
                Command::CreateTerrainMesh {
                    name: "Terrain".into(),
                    params: tessel_core::TerrainParams {
                        width: 8.0,
                        depth: 8.0,
                        subdiv_x: 16,
                        subdiv_z: 16,
                        height_scale: 1.5,
                        seed: 42,
                        octaves: 3,
                        frequency: 0.35,
                        island_falloff: 0.3,
                        heights: None,
                        palette_bands: Vec::new(),
                        default_material: scene.active_color,
                        smooth_shading: false,
                    },
                    transform: tessel_core::Transform::default(),
                },
            );
        }
        ToolbarAction::ConvertToVoxel => {
            if let Some(pixel_node_id) = scene.active_pixel_node() {
                let _ = history.execute(
                    scene,
                    Command::Transaction(vec![
                        Command::ConvertPixelToVoxel {
                            pixel_node_id,
                            depth: 3,
                            name: "Voxelized Sprite".into(),
                        },
                        Command::SetMode(AppMode::Voxel),
                    ]),
                );
            }
        }
        ToolbarAction::ConvertToMesh => {
            if let Some(voxel_node_id) = scene.active_voxel_node() {
                let _ = history.execute(
                    scene,
                    Command::Transaction(vec![
                        Command::ConvertVoxelToLowPoly {
                            voxel_node_id,
                            name: "LowPoly Mesh".into(),
                        },
                        Command::SetMode(AppMode::LowPoly),
                    ]),
                );
            }
        }
        ToolbarAction::TogglePaint => {
            scene.lowpoly_paint_mode = !scene.lowpoly_paint_mode;
        }
        ToolbarAction::LoadTexture { normal } => {
            let Some(node_id) = scene.active_lowpoly_node() else {
                return;
            };
            let (kind, exts) = if normal {
                ("Normal Map (.png, .jpg)", &["png", "jpg", "jpeg"][..])
            } else {
                ("Texture Image (.png, .jpg)", &["png", "jpg", "jpeg"][..])
            };
            if let Some(path) = rfd::FileDialog::new().add_filter(kind, exts).pick_file() {
                match tessel_io::import_mesh_texture_from_path(&path) {
                    Ok(tex) => {
                        let _ = history.execute(
                            scene,
                            if normal {
                                Command::SetMeshNormalMap {
                                    node_id,
                                    normal_map: Some(tex),
                                }
                            } else {
                                Command::SetMeshTexture {
                                    node_id,
                                    texture: Some(tex),
                                }
                            },
                        );
                    }
                    Err(err) => {
                        tracing::error!("Load custom texture failed: {err}");
                    }
                }
            }
        }
        ToolbarAction::FillTexture => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                let color = scene
                    .palette
                    .get(scene.active_color)
                    .map(|e| e.color)
                    .unwrap_or(tessel_core::Rgba8::rgb(225, 226, 235));
                let _ = history.execute(scene, Command::FillMeshTexture { node_id, color });
            }
        }
        ToolbarAction::Extrude => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                let face_id = scene.selected_face.unwrap_or(FaceId(0));
                let _ = history.execute(
                    scene,
                    Command::ExtrudeMeshFace {
                        node_id,
                        face_id,
                        distance: 0.25,
                    },
                );
            }
        }
        ToolbarAction::Inset => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                let face_id = scene.selected_face.unwrap_or(FaceId(0));
                let _ = history.execute(
                    scene,
                    Command::InsetMeshFace {
                        node_id,
                        face_id,
                        factor: 0.3,
                    },
                );
            }
        }
        ToolbarAction::Bevel => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                let _ = history.execute(
                    scene,
                    Command::BevelMesh {
                        node_id,
                        amount: 0.18,
                    },
                );
            }
        }
        ToolbarAction::Smooth => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                let _ = history.execute(scene, Command::SmoothSubdivideMesh { node_id });
            }
        }
        ToolbarAction::Decimate => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                let _ = history.execute(
                    scene,
                    Command::DecimateMesh {
                        node_id,
                        ratio: 0.5,
                    },
                );
            }
        }
        ToolbarAction::ToggleShade => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                let smooth = scene
                    .lowpoly_meshes
                    .get(&node_id)
                    .is_some_and(|m| m.smooth_shading);
                let _ = history.execute(
                    scene,
                    Command::SetMeshSmoothShading {
                        node_id,
                        smooth: !smooth,
                    },
                );
            }
        }
        ToolbarAction::SubdivideFace => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                let face_id = scene.selected_face.unwrap_or(FaceId(0));
                let _ = history.execute(scene, Command::SubdivideMeshFace { node_id, face_id });
            }
        }
        ToolbarAction::MirrorMeshX => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                let _ = history.execute(
                    scene,
                    Command::MirrorLowPolyMesh {
                        node_id,
                        axis: 0,
                        merge_original: true,
                    },
                );
            }
        }
        ToolbarAction::ApplyTransform => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                let _ = history.execute(scene, Command::ApplyMeshTransform { node_id });
            }
        }
        ToolbarAction::ProjectUvs => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                let _ = history.execute(
                    scene,
                    Command::ProjectMeshUvs {
                        node_id,
                        mode: tessel_core::mesh::UvProjection::Box,
                    },
                );
            }
        }
        ToolbarAction::ApplyBipedRig => {
            let _ = history.execute(scene, Command::ApplyBipedRig);
        }
        ToolbarAction::ApplyHumanoidRig => {
            let _ = history.execute(scene, Command::ApplyHumanoidRig);
        }
        ToolbarAction::ToggleWeightPaint => {
            scene.weight_paint_mode = !scene.weight_paint_mode;
        }
        ToolbarAction::ClearMeshWeights => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                if let Err(err) = history.execute(
                    scene,
                    Command::ClearMeshVertexWeights {
                        node_id,
                        vertex_ids: None,
                    },
                ) {
                    tracing::warn!("clear mesh weights failed: {err}");
                }
            }
        }
        ToolbarAction::TogglePixelSymX => {
            scene.pixel_symmetry.x = !scene.pixel_symmetry.x;
        }
        ToolbarAction::TogglePixelSymY => {
            scene.pixel_symmetry.y = !scene.pixel_symmetry.y;
        }
        ToolbarAction::ToggleVoxelSymX => {
            scene.voxel_symmetry.x = !scene.voxel_symmetry.x;
        }
        ToolbarAction::ToggleVoxelSymY => {
            scene.voxel_symmetry.y = !scene.voxel_symmetry.y;
        }
        ToolbarAction::ToggleVoxelSymZ => {
            scene.voxel_symmetry.z = !scene.voxel_symmetry.z;
        }
        ToolbarAction::ToggleMirrorX => {
            scene.lowpoly_mirror_x = !scene.lowpoly_mirror_x;
        }
    }
}
