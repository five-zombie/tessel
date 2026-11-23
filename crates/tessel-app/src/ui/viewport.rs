use egui::{CornerRadius, Frame, Margin, RichText, Sense, Stroke};
use tessel_core::pixel::ops as pixel_ops;
use tessel_core::voxel::ops as voxel_ops;
use tessel_core::{
    AppMode, Command, FrameIndex, HistoryStack, NodeId, PixelCoord, PixelEdit, PixelTool,
    SceneState, Transform, VoxelCoord, VoxelEdit, VoxelTool,
};
use tessel_render::overlay_pass::GIZMO_AXIS_LEN;
use tessel_render::{
    closest_point_on_axis, pick_move_gizmo, CameraPreset, GizmoAxis, OrbitCamera, PixelCamera2D,
};

use super::theme::tokens;

#[derive(Debug, Clone, Copy)]
pub struct GizmoDragState {
    pub node_id: NodeId,
    pub axis: GizmoAxis,
    pub start_transform: Transform,
    pub start_axis_t: f32,
}

/// Displays the offscreen wgpu viewport texture, routes 2D/3D viewport input,
/// renders the floating Zoom & Camera Preset HUD, and returns `(desired_physical_size, hover_voxel, hovered_gizmo_axis)`.
#[allow(clippy::too_many_arguments)]
pub fn show_viewport(
    ctx: &egui::Context,
    texture_id: egui::TextureId,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    camera_3d: &mut OrbitCamera,
    camera_2d: &mut PixelCamera2D,
    pixel_drag_start: &mut Option<PixelCoord>,
    gizmo_drag: &mut Option<GizmoDragState>,
    free_view: &mut bool,
) -> ((u32, u32), Option<VoxelCoord>, Option<GizmoAxis>) {
    let mut desired_size = (1, 1);
    let mut hover_voxel = None;
    let mut active_gizmo_axis = gizmo_drag.map(|d| d.axis);
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

        if scene.active_mode == AppMode::Pixel {
            handle_pixel_viewport_input(
                ui,
                rect,
                &response,
                scene,
                history,
                camera_2d,
                pixel_drag_start,
            );
        } else {
            let gizmo_consumed = handle_gizmo_drag_input(
                rect,
                &response,
                scene,
                history,
                camera_3d,
                gizmo_drag,
                &mut active_gizmo_axis,
            );
            handle_3d_viewport_input(ui, &response, camera_3d, *free_view && !gizmo_consumed);

            if !gizmo_consumed {
                match scene.active_mode {
                    AppMode::Voxel => {
                        hover_voxel = compute_hover_voxel(rect, &response, scene, camera_3d);
                        handle_voxel_click_input(ui, rect, &response, scene, history, camera_3d);
                    }
                    AppMode::LowPoly => {
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
                                            tessel_core::mesh::ops::raycast_mesh_face(
                                                mesh, orig, dir,
                                            );
                                        scene.mark_modified();
                                    }
                                }
                            }
                        }
                    }
                    AppMode::Character => {
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
                    AppMode::Pixel => {}
                }
            }
        }

        show_viewport_hud(ui, rect, scene.active_mode, camera_3d, camera_2d, free_view);
        if scene.active_mode != AppMode::Pixel {
            draw_camera_compass(ui, rect, camera_3d);
        }
    });

    (desired_size, hover_voxel, active_gizmo_axis)
}

fn draw_camera_compass(ui: &egui::Ui, rect: egui::Rect, camera: &OrbitCamera) {
    let center = rect.left_bottom() + egui::vec2(44.0, -44.0);
    let radius = 26.0;
    let painter = ui.painter();

    painter.circle_filled(
        center,
        radius + 6.0,
        egui::Color32::from_rgba_unmultiplied(0x19, 0x1C, 0x22, 210),
    );
    painter.circle_stroke(
        center,
        radius + 6.0,
        Stroke::new(1.0, tokens::OUTLINE_VARIANT),
    );

    let view = glam::Mat4::look_at_rh(camera.position(), camera.target, glam::Vec3::Y);
    let axes = [
        ("X", glam::Vec3::X, egui::Color32::from_rgb(240, 82, 82)),
        ("Y", glam::Vec3::Y, egui::Color32::from_rgb(92, 220, 112)),
        ("Z", glam::Vec3::Z, egui::Color32::from_rgb(86, 152, 245)),
    ];

    let mut projected: Vec<(&str, glam::Vec3, egui::Color32)> = axes
        .into_iter()
        .map(|(label, dir, col)| (label, view.transform_vector3(dir), col))
        .collect();
    projected.sort_by(|a, b| a.1.z.total_cmp(&b.1.z));

    for (label, v, col) in projected {
        let end = center + egui::vec2(v.x * radius, -v.y * radius);
        painter.line_segment([center, end], Stroke::new(2.0, col));
        painter.text(
            end + egui::vec2(v.x * 7.0, -v.y * 7.0),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::monospace(10.5),
            col,
        );
    }
}

fn handle_gizmo_drag_input(
    rect: egui::Rect,
    response: &egui::Response,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    camera: &OrbitCamera,
    gizmo_drag: &mut Option<GizmoDragState>,
    active_axis: &mut Option<GizmoAxis>,
) -> bool {
    let Some(node_id) = scene.selected_node else {
        return false;
    };
    let Some(node_transform) = scene.nodes.get(&node_id).map(|n| n.transform) else {
        return false;
    };

    let pointer_ray = |pos: egui::Pos2| -> (glam::Vec3, glam::Vec3) {
        let uv = [
            (pos.x - rect.min.x) / rect.width().max(1.0),
            (pos.y - rect.min.y) / rect.height().max(1.0),
        ];
        let aspect = rect.width() / rect.height().max(1.0);
        camera.screen_ray(uv, aspect)
    };

    if gizmo_drag.is_none() {
        if let Some(hover) = response.hover_pos() {
            let (orig, dir) = pointer_ray(hover);
            *active_axis =
                pick_move_gizmo(orig, dir, node_transform.translation, GIZMO_AXIS_LEN, 0.14);
        }
    }

    if response.drag_started_by(egui::PointerButton::Primary) {
        if let Some(pos) = response.interact_pointer_pos() {
            let (orig, dir) = pointer_ray(pos);
            if let Some(axis) =
                pick_move_gizmo(orig, dir, node_transform.translation, GIZMO_AXIS_LEN, 0.16)
            {
                if let Some((start_t, _)) =
                    closest_point_on_axis(orig, dir, node_transform.translation, axis.dir())
                {
                    *gizmo_drag = Some(GizmoDragState {
                        node_id,
                        axis,
                        start_transform: node_transform,
                        start_axis_t: start_t,
                    });
                    *active_axis = Some(axis);
                    return true;
                }
            }
        }
    }

    if let Some(drag) = *gizmo_drag {
        if response.dragged_by(egui::PointerButton::Primary) {
            if let Some(pos) = response.interact_pointer_pos() {
                let (orig, dir) = pointer_ray(pos);
                if let Some((cur_t, _)) = closest_point_on_axis(
                    orig,
                    dir,
                    drag.start_transform.translation,
                    drag.axis.dir(),
                ) {
                    let delta = cur_t - drag.start_axis_t;
                    if let Some(node) = scene.nodes.get_mut(&drag.node_id) {
                        node.transform.translation =
                            drag.start_transform.translation + drag.axis.dir() * delta;
                        scene.mark_modified();
                    }
                }
            }
            return true;
        }
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            if let Some(final_tx) = scene.nodes.get(&drag.node_id).map(|n| n.transform) {
                if let Some(node) = scene.nodes.get_mut(&drag.node_id) {
                    node.transform = drag.start_transform;
                }
                let _ = history.execute(
                    scene,
                    Command::SetNodeTransform {
                        node_id: drag.node_id,
                        transform: final_tx,
                    },
                );
            }
            *gizmo_drag = None;
            return true;
        }
    }

    false
}

fn show_viewport_hud(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    mode: AppMode,
    camera_3d: &mut OrbitCamera,
    camera_2d: &mut PixelCamera2D,
    free_view: &mut bool,
) {
    let hud_pos = rect.right_top() + egui::vec2(-10.0, 10.0);
    egui::Area::new(egui::Id::new("tessel_viewport_hud"))
        .anchor(
            egui::Align2::RIGHT_TOP,
            egui::vec2(hud_pos.x - ui.ctx().screen_rect().right(), hud_pos.y),
        )
        .show(ui.ctx(), |ui| {
            Frame::NONE
                .fill(tokens::SURFACE_LOW)
                .stroke(Stroke::new(1.0, tokens::OUTLINE_VARIANT))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::symmetric(6, 4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.small_button("-").on_hover_text("Zoom Out").clicked() {
                            match mode {
                                AppMode::Pixel => camera_2d.zoom_step(0.8),
                                _ => camera_3d.zoom_step(1.2),
                            }
                        }
                        if ui
                            .small_button("Reset")
                            .on_hover_text("Reset Camera (F)")
                            .clicked()
                        {
                            match mode {
                                AppMode::Pixel => camera_2d.reset(),
                                _ => camera_3d.reset(),
                            }
                        }
                        if ui.small_button("+").on_hover_text("Zoom In").clicked() {
                            match mode {
                                AppMode::Pixel => camera_2d.zoom_step(1.25),
                                _ => camera_3d.zoom_step(0.82),
                            }
                        }

                        if mode != AppMode::Pixel {
                            ui.separator();
                            let (free_fill, free_text) = if *free_view {
                                (tokens::PRIMARY_CONTAINER, tokens::ON_PRIMARY)
                            } else {
                                (tokens::SURFACE_HIGH, tokens::ON_SURFACE)
                            };
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("Free").size(11.0).color(free_text),
                                    )
                                    .fill(free_fill),
                                )
                                .on_hover_text(
                                    "Free View: Left-drag to orbit 360°, Shift+Left-drag to pan",
                                )
                                .clicked()
                            {
                                *free_view = !*free_view;
                            }
                            let presets = [
                                ("Iso", CameraPreset::Iso),
                                ("Front", CameraPreset::Front),
                                ("Right", CameraPreset::Right),
                                ("Top", CameraPreset::Top),
                            ];
                            for (label, preset) in presets {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(label)
                                                .size(11.0)
                                                .color(tokens::ON_SURFACE),
                                        )
                                        .fill(tokens::SURFACE_HIGH),
                                    )
                                    .clicked()
                                {
                                    camera_3d.set_preset(preset);
                                }
                            }
                        }
                    });
                });
        });
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

fn handle_3d_viewport_input(
    ui: &egui::Ui,
    response: &egui::Response,
    camera: &mut OrbitCamera,
    allow_primary_orbit: bool,
) {
    let primary_drag = response.dragged_by(egui::PointerButton::Primary)
        && (allow_primary_orbit || ui.input(|i| i.modifiers.alt));
    if response.dragged_by(egui::PointerButton::Secondary)
        || response.dragged_by(egui::PointerButton::Middle)
        || primary_drag
    {
        let delta = response.drag_delta();
        let shift = ui.input(|i| i.modifiers.shift);
        if shift {
            camera.pan(delta.x, delta.y);
        } else {
            camera.orbit(delta.x, delta.y);
        }
    }

    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.01 {
            camera.zoom(scroll);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_pixel_viewport_input(
    ui: &egui::Ui,
    rect: egui::Rect,
    response: &egui::Response,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    camera_2d: &mut PixelCamera2D,
    pixel_drag_start: &mut Option<PixelCoord>,
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

    let pointer_to_coord = |pos: egui::Pos2| -> Option<PixelCoord> {
        let local = [pos.x - rect.min.x, pos.y - rect.min.y];
        let viewport_size = [rect.width(), rect.height()];
        camera_2d
            .screen_to_canvas(local, viewport_size, (canvas_w, canvas_h))
            .map(|(px, py)| PixelCoord::new(px, py))
    };

    if matches!(scene.pixel_tool, PixelTool::Line | PixelTool::Rect) {
        if response.drag_started_by(egui::PointerButton::Primary) {
            if let Some(pos) = response.interact_pointer_pos() {
                *pixel_drag_start = pointer_to_coord(pos);
            }
            return;
        }
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            if let (Some(start), Some(pos)) =
                (pixel_drag_start.take(), response.interact_pointer_pos())
            {
                if let Some(end) = pointer_to_coord(pos) {
                    let base_pts = match scene.pixel_tool {
                        PixelTool::Line => pixel_ops::line_points(start, end),
                        PixelTool::Rect => pixel_ops::rect_outline_points(start, end),
                        _ => Vec::new(),
                    };
                    let mut edits = Vec::new();
                    for p in base_pts {
                        for sym in
                            pixel_ops::expand_symmetry(p, canvas_w, canvas_h, scene.pixel_symmetry)
                        {
                            edits.push(PixelEdit {
                                coord: sym,
                                color: Some(scene.active_color),
                            });
                        }
                    }
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
            }
            return;
        }
        if response.dragged_by(egui::PointerButton::Primary) {
            return;
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
    let Some(coord) = pointer_to_coord(pointer) else {
        return;
    };

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
