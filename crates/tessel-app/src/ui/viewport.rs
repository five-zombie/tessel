use egui::{Frame, Margin, Sense};
use tessel_core::pixel::ops as pixel_ops;
use tessel_core::voxel::ops as voxel_ops;
use tessel_core::{
    AppMode, Command, FrameIndex, HistoryStack, PixelCoord, PixelEdit, PixelTool, SceneState,
    VoxelCoord, VoxelEdit, VoxelTool,
};
use tessel_render::{OrbitCamera, PixelCamera2D};

use super::theme::tokens;

/// Displays the offscreen wgpu viewport texture, routes 2D/3D viewport input,
/// and returns `(desired_physical_size, hover_voxel)`.
pub fn show_viewport(
    ctx: &egui::Context,
    texture_id: egui::TextureId,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    camera_3d: &mut OrbitCamera,
    camera_2d: &mut PixelCamera2D,
) -> ((u32, u32), Option<VoxelCoord>) {
    let mut desired_size = (1, 1);
    let mut hover_voxel = None;
    let frame = Frame::NONE.fill(tokens::SURFACE).inner_margin(Margin::ZERO);

    egui::CentralPanel::default().frame(frame).show(ctx, |ui| {
        let available = ui.available_size();
        let ppp = ctx.pixels_per_point();
        desired_size = (
            (available.x * ppp).round().max(1.0) as u32,
            (available.y * ppp).round().max(1.0) as u32,
        );

        let (rect, response) = ui.allocate_exact_size(available, Sense::click_and_drag());

        ui.painter().image(
            texture_id,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );

        match scene.active_mode {
            AppMode::Pixel => {
                handle_pixel_viewport_input(ui, rect, &response, scene, history, camera_2d);
            }
            AppMode::Voxel => {
                handle_3d_viewport_input(ui, &response, camera_3d);
                hover_voxel = compute_hover_voxel(rect, &response, scene, camera_3d);
                handle_voxel_click_input(ui, rect, &response, scene, history, camera_3d);
            }
            AppMode::LowPoly => {
                handle_3d_viewport_input(ui, &response, camera_3d);
                if response.clicked_by(egui::PointerButton::Primary) {
                    if let Some(pointer) = response.interact_pointer_pos() {
                        let uv = [
                            (pointer.x - rect.min.x) / rect.width().max(1.0),
                            (pointer.y - rect.min.y) / rect.height().max(1.0),
                        ];
                        let aspect = rect.width() / rect.height().max(1.0);
                        let (orig, dir) = camera_3d.screen_ray(uv, aspect);
                        if let Some(node_id) = scene.active_lowpoly_node() {
                            if let Some(mesh) = scene.lowpoly_meshes.get(&node_id) {
                                scene.selected_face =
                                    tessel_core::mesh::ops::raycast_mesh_face(mesh, orig, dir);
                                scene.mark_modified();
                            }
                        }
                    }
                }
            }
            AppMode::Character => {
                handle_3d_viewport_input(ui, &response, camera_3d);
                if response.clicked_by(egui::PointerButton::Primary) {
                    if let Some(pointer) = response.interact_pointer_pos() {
                        let uv = [
                            (pointer.x - rect.min.x) / rect.width().max(1.0),
                            (pointer.y - rect.min.y) / rect.height().max(1.0),
                        ];
                        let aspect = rect.width() / rect.height().max(1.0);
                        let (orig, dir) = camera_3d.screen_ray(uv, aspect);
                        if let Some(rig) = &scene.character_rig {
                            scene.selected_bone = rig.pick_bone(orig, dir, 0.16);
                            scene.mark_modified();
                        }
                    }
                }
            }
        }
    });

    (desired_size, hover_voxel)
}

fn compute_hover_voxel(
    rect: egui::Rect,
    response: &egui::Response,
    scene: &SceneState,
    camera: &OrbitCamera,
) -> Option<VoxelCoord> {
    let pointer = response.hover_pos()?;
    let node_id = scene.active_voxel_node()?;
    let volume = scene.voxel_volumes.get(&node_id)?;
    let uv = [
        (pointer.x - rect.min.x) / rect.width().max(1.0),
        (pointer.y - rect.min.y) / rect.height().max(1.0),
    ];
    let aspect = rect.width() / rect.height().max(1.0);
    let (ray_origin, ray_dir) = camera.screen_ray(uv, aspect);
    let hit = voxel_ops::raycast_voxels(volume, ray_origin, ray_dir, scene.unit_scale, 256);
    match scene.voxel_tool {
        VoxelTool::Place | VoxelTool::BoxFill => hit.map(|h| h.adjacent()),
        VoxelTool::Erase | VoxelTool::Paint | VoxelTool::Eyedropper => hit.map(|h| h.voxel),
    }
}

fn handle_3d_viewport_input(ui: &egui::Ui, response: &egui::Response, camera: &mut OrbitCamera) {
    if response.dragged_by(egui::PointerButton::Secondary)
        || response.dragged_by(egui::PointerButton::Middle)
    {
        let delta = response.drag_delta();
        let shift = ui.input(|i| i.modifiers.shift);
        if shift {
            camera.pan(delta.x, delta.y);
        } else {
            camera.orbit(delta.x, delta.y);
        }
    } else if response.dragged_by(egui::PointerButton::Primary) && ui.input(|i| i.modifiers.alt) {
        let delta = response.drag_delta();
        camera.orbit(delta.x, delta.y);
    }

    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.01 {
            camera.zoom(scroll);
        }
    }
}

fn handle_pixel_viewport_input(
    ui: &egui::Ui,
    rect: egui::Rect,
    response: &egui::Response,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    camera_2d: &mut PixelCamera2D,
) {
    if response.dragged_by(egui::PointerButton::Middle)
        || response.dragged_by(egui::PointerButton::Secondary)
    {
        let delta = response.drag_delta();
        camera_2d.pan[0] += delta.x;
        camera_2d.pan[1] += delta.y;
        return;
    }

    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.01 {
            camera_2d.zoom_by(scroll);
        }
    }

    let primary_active = response.clicked_by(egui::PointerButton::Primary)
        || response.dragged_by(egui::PointerButton::Primary);
    if !primary_active {
        return;
    }

    let Some(pointer) = response.interact_pointer_pos() else {
        return;
    };
    let Some(node_id) = scene.active_pixel_node() else {
        return;
    };
    let Some((canvas_w, canvas_h, active_frame, layer_id)) =
        scene.pixel_canvases.get(&node_id).and_then(|c| {
            c.current_frame()
                .and_then(|f| f.active_layer_id())
                .map(|lid| (c.width, c.height, c.active_frame, lid))
        })
    else {
        return;
    };

    let local = [pointer.x - rect.min.x, pointer.y - rect.min.y];
    let viewport_size = [rect.width(), rect.height()];
    let Some((px, py)) = camera_2d.screen_to_canvas(local, viewport_size, (canvas_w, canvas_h))
    else {
        return;
    };
    let coord = PixelCoord::new(px, py);

    let alt = ui.input(|i| i.modifiers.alt);
    if alt || scene.pixel_tool == PixelTool::Eyedropper {
        if let Some(sampled) = scene
            .pixel_canvases
            .get(&node_id)
            .and_then(|c| c.sample_active_pixel(coord))
        {
            scene.active_color = sampled;
        }
        return;
    }

    let edits = match scene.pixel_tool {
        PixelTool::Pencil | PixelTool::Line | PixelTool::Rect => {
            pixel_ops::expand_symmetry(coord, canvas_w, canvas_h, scene.pixel_symmetry)
                .into_iter()
                .map(|c| PixelEdit {
                    coord: c,
                    color: Some(scene.active_color),
                })
                .collect::<Vec<_>>()
        }
        PixelTool::Eraser => {
            pixel_ops::expand_symmetry(coord, canvas_w, canvas_h, scene.pixel_symmetry)
                .into_iter()
                .map(|c| PixelEdit {
                    coord: c,
                    color: None,
                })
                .collect::<Vec<_>>()
        }
        PixelTool::Fill => {
            if !response.clicked_by(egui::PointerButton::Primary) {
                return;
            }
            scene
                .pixel_canvases
                .get(&node_id)
                .and_then(|c| c.frame(FrameIndex(active_frame)))
                .and_then(|f| f.layer(layer_id))
                .map(|layer| pixel_ops::flood_fill_edits(layer, coord, Some(scene.active_color)))
                .unwrap_or_default()
        }
        PixelTool::Eyedropper => return,
    };

    if !edits.is_empty() {
        let _ = history.execute(
            scene,
            Command::DrawPixels {
                node_id,
                frame: FrameIndex(active_frame),
                layer_id,
                edits,
            },
        );
    }
}

fn handle_voxel_click_input(
    ui: &egui::Ui,
    rect: egui::Rect,
    response: &egui::Response,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    camera: &OrbitCamera,
) {
    if !response.clicked_by(egui::PointerButton::Primary) {
        return;
    }
    let alt = ui.input(|i| i.modifiers.alt);
    let Some(pointer) = response.interact_pointer_pos() else {
        return;
    };
    let Some(node_id) = scene.active_voxel_node() else {
        return;
    };
    let Some(volume) = scene.voxel_volumes.get(&node_id) else {
        return;
    };

    let uv = [
        (pointer.x - rect.min.x) / rect.width().max(1.0),
        (pointer.y - rect.min.y) / rect.height().max(1.0),
    ];
    let aspect = rect.width() / rect.height().max(1.0);
    let (ray_origin, ray_dir) = camera.screen_ray(uv, aspect);

    let hit = voxel_ops::raycast_voxels(volume, ray_origin, ray_dir, scene.unit_scale, 256);

    if alt || scene.voxel_tool == VoxelTool::Eyedropper {
        if let Some(h) = hit {
            if let Some(color) = volume.get(h.voxel) {
                scene.active_color = color;
            }
        }
        return;
    }

    let target_edit: Option<(VoxelCoord, Option<tessel_core::PaletteIndex>)> =
        match scene.voxel_tool {
            VoxelTool::Place => hit
                .map(|h| (h.adjacent(), Some(scene.active_color)))
                .or_else(|| {
                    if ray_dir.y.abs() > 1e-5 {
                        let t = -ray_origin.y / ray_dir.y;
                        if t > 0.0 && t < 80.0 {
                            let p = (ray_origin + ray_dir * t) / scene.unit_scale;
                            return Some((
                                VoxelCoord::new(p.x.floor() as i32, 0, p.z.floor() as i32),
                                Some(scene.active_color),
                            ));
                        }
                    }
                    None
                }),
            VoxelTool::Erase => hit.map(|h| (h.voxel, None)),
            VoxelTool::Paint => hit.map(|h| (h.voxel, Some(scene.active_color))),
            VoxelTool::BoxFill => hit.map(|h| (h.adjacent(), Some(scene.active_color))),
            VoxelTool::Eyedropper => None,
        };

    let Some((base_coord, color)) = target_edit else {
        return;
    };

    if scene.voxel_tool == VoxelTool::BoxFill {
        let min = VoxelCoord::new(base_coord.x - 1, base_coord.y, base_coord.z - 1);
        let max = VoxelCoord::new(base_coord.x + 1, base_coord.y + 2, base_coord.z + 1);
        let _ = history.execute(
            scene,
            Command::FillVoxelBox {
                node_id,
                min,
                max,
                color,
                hollow: false,
            },
        );
        return;
    }

    let edits: Vec<VoxelEdit> = voxel_ops::expand_symmetry_3d(base_coord, scene.voxel_symmetry)
        .into_iter()
        .map(|coord| VoxelEdit { coord, color })
        .collect();

    if !edits.is_empty() {
        let _ = history.execute(scene, Command::SetVoxels { node_id, edits });
    }
}
