use std::sync::{Arc, Mutex};

use egui::{Frame, Margin, RichText, Stroke};
use tessel_core::{AppMode, Command, FaceId, HistoryStack, NodeId, SceneState};

use super::icons::{self, IconItem, ToolbarAction};
use super::theme::tokens;
use super::viewport::ViewportInteractionTool;
use super::AsyncJobSlot;

type TextureLoadResult = Option<(
    NodeId,
    String,
    bool,
    Result<tessel_core::MeshTexture, String>,
)>;

fn poll_texture_load_job(ctx: &egui::Context, scene: &mut SceneState, history: &mut HistoryStack) {
    let slot_id = egui::Id::new("toolbar_texture_load_slot");
    let Some(slot) =
        ctx.data(|d| d.get_temp::<Arc<Mutex<AsyncJobSlot<TextureLoadResult>>>>(slot_id))
    else {
        return;
    };
    let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
    let polled = guard.poll();
    let busy = guard.is_busy();
    drop(guard);
    if busy {
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
    }
    match polled {
        Ok(Some(Some((node_id, node_name, normal, res)))) => {
            let same_node = scene
                .nodes
                .get(&node_id)
                .is_some_and(|n| n.name == node_name)
                && scene.lowpoly_meshes.contains_key(&node_id);
            if !same_node {
                super::push_ui_error(
                    ctx,
                    "Target mesh no longer exists in the current scene; texture load skipped.",
                );
                return;
            }
            match res {
                Ok(tex) => {
                    super::exec_history(
                        ctx,
                        history,
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
                    super::push_ui_error(ctx, format!("Load custom texture failed: {err}"));
                }
            }
        }
        Ok(Some(None)) | Ok(None) => {}
        Err(err) => super::push_ui_error(ctx, err),
    }
}

pub fn show_toolbar(
    ctx: &egui::Context,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    interaction_tool: &mut ViewportInteractionTool,
    free_view: &mut bool,
) -> super::RigIoRequests {
    poll_texture_load_job(ctx, scene, history);
    let mut rig_io = super::RigIoRequests::default();
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
                        fire_toolbar_action(
                            ui.ctx(),
                            action,
                            scene,
                            history,
                            interaction_tool,
                            free_view,
                        );
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
                        fire_toolbar_action(
                            ui.ctx(),
                            action,
                            scene,
                            history,
                            interaction_tool,
                            free_view,
                        );
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
                            fire_toolbar_action(
                                ui.ctx(),
                                action,
                                scene,
                                history,
                                interaction_tool,
                                free_view,
                            );
                        }
                    }
                }
                AppMode::Voxel => {
                    let select_group = [IconItem {
                        icon: icons::ToolIcon::Select,
                        tip: "Select / Transform Object (no voxel edit)",
                        active: *interaction_tool == ViewportInteractionTool::Select,
                        action: ToolbarAction::SelectObject,
                    }];
                    if let Some(action) =
                        icons::icon_group(ui, "voxel_select", "OBJECT", &select_group)
                    {
                        fire_toolbar_action(
                            ui.ctx(),
                            action,
                            scene,
                            history,
                            interaction_tool,
                            free_view,
                        );
                    }
                    ui.separator();
                    let items: Vec<IconItem> = tessel_core::VoxelTool::ALL
                        .iter()
                        .map(|tool| {
                            let (icon, tip) = icons::voxel_tool_icon(*tool);
                            IconItem {
                                icon,
                                tip,
                                active: *interaction_tool == ViewportInteractionTool::Edit
                                    && scene.voxel_tool == *tool,
                                action: ToolbarAction::VoxelTool(*tool),
                            }
                        })
                        .collect();
                    if let Some(action) = icons::icon_group(ui, "voxel_tools", "VOXEL", &items) {
                        fire_toolbar_action(
                            ui.ctx(),
                            action,
                            scene,
                            history,
                            interaction_tool,
                            free_view,
                        );
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
                        fire_toolbar_action(
                            ui.ctx(),
                            action,
                            scene,
                            history,
                            interaction_tool,
                            free_view,
                        );
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
                            fire_toolbar_action(
                                ui.ctx(),
                                action,
                                scene,
                                history,
                                interaction_tool,
                                free_view,
                            );
                        }
                    }
                }
                AppMode::LowPoly => {
                    let select_group = [IconItem {
                        icon: icons::ToolIcon::Select,
                        tip: "Select / Transform Object",
                        active: *interaction_tool == ViewportInteractionTool::Select
                            && !scene.lowpoly_paint_mode
                            && !scene.sculpt_mode,
                        action: ToolbarAction::SelectObject,
                    }];
                    if let Some(action) =
                        icons::icon_group(ui, "lowpoly_select", "OBJECT", &select_group)
                    {
                        fire_toolbar_action(
                            ui.ctx(),
                            action,
                            scene,
                            history,
                            interaction_tool,
                            free_view,
                        );
                    }
                    ui.separator();
                    let shapes: Vec<IconItem> = tessel_core::PrimitiveKind::ALL
                        .iter()
                        .map(|kind| {
                            let (icon, tip) = icons::primitive_icon(*kind);
                            IconItem {
                                icon,
                                tip,
                                active: *interaction_tool
                                    == ViewportInteractionTool::PlacePrimitive(*kind),
                                action: ToolbarAction::Primitive(*kind),
                            }
                        })
                        .collect();
                    if let Some(action) = icons::icon_group(ui, "prim_shapes", "PLACE", &shapes) {
                        fire_toolbar_action(
                            ui.ctx(),
                            action,
                            scene,
                            history,
                            interaction_tool,
                            free_view,
                        );
                    }
                    if icons::text_icon_button(
                        ui,
                        "Terrain",
                        "Create Low-Poly Terrain (v4.1)",
                        false,
                    ) {
                        fire_toolbar_action(
                            ui.ctx(),
                            ToolbarAction::CreateTerrain,
                            scene,
                            history,
                            interaction_tool,
                            free_view,
                        );
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
                            fire_toolbar_action(
                                ui.ctx(),
                                action,
                                scene,
                                history,
                                interaction_tool,
                                free_view,
                            );
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
                            fire_toolbar_action(
                                ui.ctx(),
                                action,
                                scene,
                                history,
                                interaction_tool,
                                free_view,
                            );
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
                        ui.separator();
                        if ui
                            .selectable_label(scene.sculpt_mode, "Sculpt")
                            .on_hover_text(
                                "Toggle organic sculpting brush (Smooth/Inflate/Grab/Crease)",
                            )
                            .clicked()
                        {
                            scene.sculpt_mode = !scene.sculpt_mode;
                            if scene.sculpt_mode {
                                scene.lowpoly_paint_mode = false;
                                *interaction_tool = ViewportInteractionTool::Edit;
                                *free_view = false;
                            } else {
                                *interaction_tool = ViewportInteractionTool::Select;
                            }
                        }
                        if scene.sculpt_mode {
                            for brush in tessel_core::SculptBrushKind::ALL {
                                if ui
                                    .selectable_label(scene.sculpt_brush == brush, brush.label())
                                    .clicked()
                                {
                                    scene.sculpt_brush = brush;
                                }
                            }
                            ui.label(
                                RichText::new(format!("R:{:.2}", scene.sculpt_radius))
                                    .monospace()
                                    .size(10.0)
                                    .color(tokens::ON_SURFACE_VARIANT),
                            );
                            ui.add(
                                egui::Slider::new(&mut scene.sculpt_radius, 0.05..=2.0)
                                    .show_value(false),
                            );
                            ui.label(
                                RichText::new(format!("S:{:.2}", scene.sculpt_strength))
                                    .monospace()
                                    .size(10.0)
                                    .color(tokens::ON_SURFACE_VARIANT),
                            );
                            ui.add(
                                egui::Slider::new(&mut scene.sculpt_strength, 0.05..=1.0)
                                    .show_value(false),
                            );
                        }
                        if ui
                            .button("TD")
                            .on_hover_text("Normalize UV Texel Density")
                            .clicked()
                        {
                            if let Some(node_id) = scene.active_lowpoly_node() {
                                super::exec_history(
                                    ui.ctx(),
                                    history,
                                    scene,
                                    Command::NormalizeMeshTexelDensity {
                                        node_id,
                                        texture_size: tessel_core::DEFAULT_MESH_TEX_SIZE,
                                        target_px_per_unit: None,
                                        padding_uv: 0.01,
                                    },
                                );
                            }
                        }
                    }
                }
                AppMode::Character => {
                    let mut rig = vec![
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
                        IconItem {
                            icon: icons::ToolIcon::NewProject,
                            tip: "New Empty Rig",
                            active: false,
                            action: ToolbarAction::NewEmptyRig,
                        },
                        IconItem {
                            icon: icons::ToolIcon::Import,
                            tip: "Import Rig (.tesselrig.json / .glb)",
                            active: false,
                            action: ToolbarAction::ImportRig,
                        },
                    ];
                    if scene.character_rig.is_some() {
                        rig.push(IconItem {
                            icon: icons::ToolIcon::Export,
                            tip: "Export Rig (.tesselrig.json)",
                            active: false,
                            action: ToolbarAction::ExportRig,
                        });
                    }
                    if let Some(action) = icons::icon_group(ui, "char_rig", "RIG", &rig) {
                        rig_io.merge(fire_toolbar_action(
                            ui.ctx(),
                            action,
                            scene,
                            history,
                            interaction_tool,
                            free_view,
                        ));
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
                            fire_toolbar_action(
                                ui.ctx(),
                                action,
                                scene,
                                history,
                                interaction_tool,
                                free_view,
                            );
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
    rig_io
}

/// Executes a toolbar action. To add a button, add one [`IconItem`] line to a
/// group above with either an existing action or a new [`ToolbarAction`]
/// variant handled here.
fn fire_toolbar_action(
    ctx: &egui::Context,
    action: ToolbarAction,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    interaction_tool: &mut ViewportInteractionTool,
    free_view: &mut bool,
) -> super::RigIoRequests {
    let mut rig_io = super::RigIoRequests::default();
    match action {
        ToolbarAction::SelectObject => {
            *interaction_tool = ViewportInteractionTool::Select;
            scene.lowpoly_paint_mode = false;
            scene.sculpt_mode = false;
            *free_view = false;
        }
        ToolbarAction::PixelTool(tool) => scene.pixel_tool = tool,
        ToolbarAction::VoxelTool(tool) => {
            scene.voxel_tool = tool;
            *interaction_tool = ViewportInteractionTool::Edit;
            *free_view = false;
        }
        ToolbarAction::Primitive(kind) => {
            if *interaction_tool == ViewportInteractionTool::PlacePrimitive(kind) {
                *interaction_tool = ViewportInteractionTool::Select;
            } else {
                *interaction_tool = ViewportInteractionTool::PlacePrimitive(kind);
                scene.lowpoly_paint_mode = false;
                scene.sculpt_mode = false;
                *free_view = false;
            }
        }
        ToolbarAction::CreateTerrain => {
            super::exec_history(
                ctx,
                history,
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
                super::exec_history(
                    ctx,
                    history,
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
                super::exec_history(
                    ctx,
                    history,
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
            if scene.lowpoly_paint_mode {
                scene.sculpt_mode = false;
                *interaction_tool = ViewportInteractionTool::Edit;
                *free_view = false;
            } else {
                *interaction_tool = ViewportInteractionTool::Select;
            }
        }
        ToolbarAction::LoadTexture { normal } => {
            let Some(node_id) = scene.active_lowpoly_node() else {
                return rig_io;
            };
            let Some(node_name) = scene.nodes.get(&node_id).map(|n| n.name.clone()) else {
                return rig_io;
            };
            let (kind, exts) = if normal {
                ("Normal Map (.png, .jpg)", &["png", "jpg", "jpeg"][..])
            } else {
                ("Texture Image (.png, .jpg)", &["png", "jpg", "jpeg"][..])
            };
            let slot_id = egui::Id::new("toolbar_texture_load_slot");
            let slot = ctx.data_mut(|d| {
                d.get_temp_mut_or_insert_with(slot_id, || {
                    Arc::new(Mutex::new(AsyncJobSlot::<TextureLoadResult>::default()))
                })
                .clone()
            });
            let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
            if guard.is_busy() {
                super::push_ui_error(ctx, "A texture file dialog is already open.");
                return rig_io;
            }
            guard.spawn(move || {
                let path = rfd::FileDialog::new().add_filter(kind, exts).pick_file()?;
                let res =
                    tessel_io::import_mesh_texture_from_path(&path).map_err(|e| e.to_string());
                Some((node_id, node_name, normal, res))
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
        ToolbarAction::FillTexture => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                let color = scene
                    .palette
                    .get(scene.active_color)
                    .map(|e| e.color)
                    .unwrap_or(tessel_core::Rgba8::rgb(225, 226, 235));
                super::exec_history(
                    ctx,
                    history,
                    scene,
                    Command::FillMeshTexture { node_id, color },
                );
            }
        }
        ToolbarAction::Extrude => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                let face_id = scene.selected_face.unwrap_or(FaceId(0));
                super::exec_history(
                    ctx,
                    history,
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
                super::exec_history(
                    ctx,
                    history,
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
                super::exec_history(
                    ctx,
                    history,
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
                super::exec_history(
                    ctx,
                    history,
                    scene,
                    Command::SmoothSubdivideMesh { node_id },
                );
            }
        }
        ToolbarAction::Decimate => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                super::exec_history(
                    ctx,
                    history,
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
                super::exec_history(
                    ctx,
                    history,
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
                super::exec_history(
                    ctx,
                    history,
                    scene,
                    Command::SubdivideMeshFace { node_id, face_id },
                );
            }
        }
        ToolbarAction::MirrorMeshX => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                super::exec_history(
                    ctx,
                    history,
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
                super::exec_history(ctx, history, scene, Command::ApplyMeshTransform { node_id });
            }
        }
        ToolbarAction::ProjectUvs => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                super::exec_history(
                    ctx,
                    history,
                    scene,
                    Command::ProjectMeshUvs {
                        node_id,
                        mode: tessel_core::mesh::UvProjection::Box,
                    },
                );
            }
        }
        ToolbarAction::ApplyBipedRig => {
            super::clear_rig_status(ctx);
            super::exec_history(ctx, history, scene, Command::ApplyBipedRig);
        }
        ToolbarAction::ApplyHumanoidRig => {
            super::clear_rig_status(ctx);
            super::exec_history(ctx, history, scene, Command::ApplyHumanoidRig);
        }
        ToolbarAction::NewEmptyRig => {
            super::clear_rig_status(ctx);
            super::exec_history(
                ctx,
                history,
                scene,
                Command::SetCharacterRig {
                    rig: Some(tessel_core::CharacterRig::default()),
                },
            );
        }
        ToolbarAction::ImportRig => {
            rig_io.import_rig = true;
        }
        ToolbarAction::ExportRig => {
            rig_io.export_rig = true;
        }
        ToolbarAction::ToggleWeightPaint => {
            scene.weight_paint_mode = !scene.weight_paint_mode;
        }
        ToolbarAction::ClearMeshWeights => {
            if let Some(node_id) = scene.active_lowpoly_node() {
                super::exec_history(
                    ctx,
                    history,
                    scene,
                    Command::ClearMeshVertexWeights {
                        node_id,
                        vertex_ids: None,
                    },
                );
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
    rig_io
}
