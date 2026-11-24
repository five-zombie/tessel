use egui::{CornerRadius, Frame, Margin, Sense, Stroke};
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

use super::icons::{self, ToolIcon};
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
    render_preview: &mut bool,
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
            let gizmo_consumed = if *free_view {
                *gizmo_drag = None;
                active_gizmo_axis = None;
                false
            } else {
                handle_gizmo_drag_input(
                    rect,
                    &response,
                    scene,
                    history,
                    camera_3d,
                    gizmo_drag,
                    &mut active_gizmo_axis,
                )
            };
            let allow_primary_orbit = *free_view
                && !gizmo_consumed
                && !(scene.active_mode == AppMode::LowPoly && scene.lowpoly_paint_mode);
            handle_3d_viewport_input(ui, &response, camera_3d, allow_primary_orbit);

            if !gizmo_consumed && !*free_view {
                match scene.active_mode {
                    AppMode::Voxel => {
                        hover_voxel = compute_hover_voxel(rect, &response, scene, camera_3d);
                        handle_voxel_click_input(ui, rect, &response, scene, history, camera_3d);
                    }
                    AppMode::LowPoly => {
                        let paint_active = scene.lowpoly_paint_mode
                            && (response.clicked_by(egui::PointerButton::Primary)
                                || response.dragged_by(egui::PointerButton::Primary));
                        let pick_active = !scene.lowpoly_paint_mode
                            && response.clicked_by(egui::PointerButton::Primary);
                        if paint_active || pick_active {
                            if let Some(pointer) = response.interact_pointer_pos() {
                                let uv = [
                                    (pointer.x - rect.min.x) / rect.width().max(1.0),
                                    (pointer.y - rect.min.y) / rect.height().max(1.0),
                                ];
                                let aspect = rect.width() / rect.height().max(1.0);
                                let (orig, dir) = camera_3d.screen_ray(uv, aspect);
                                if paint_active {
                                    if let Some(node_id) = scene.active_lowpoly_node() {
                                        let node_mat = scene.node_world_matrix(node_id);
                                        let inv = node_mat.inverse();
                                        let local_orig = inv.transform_point3(orig);
                                        let local_dir =
                                            inv.transform_vector3(dir).normalize_or_zero();
                                        let hit =
                                            scene.lowpoly_meshes.get(&node_id).and_then(|mesh| {
                                                tessel_core::mesh::ops::raycast_mesh_hit(
                                                    mesh, local_orig, local_dir,
                                                )
                                            });
                                        if let Some(h) = hit {
                                            let color = scene
                                                .palette
                                                .get(scene.active_color)
                                                .map(|e| e.color)
                                                .unwrap_or(tessel_core::Rgba8::rgb(225, 226, 235));
                                            if scene.lowpoly_mirror_x {
                                                let mirror_uv = scene
                                                    .lowpoly_meshes
                                                    .get(&node_id)
                                                    .map(|mesh| {
                                                        let (min_b, max_b) = mesh.bounds();
                                                        let mp = glam::Vec3::new(
                                                            -h.point.x, h.point.y, h.point.z,
                                                        );
                                                        let mn = glam::Vec3::new(
                                                            -h.normal.x,
                                                            h.normal.y,
                                                            h.normal.z,
                                                        );
                                                        tessel_core::box_atlas_uv(
                                                            mp, mn, min_b, max_b,
                                                        )
                                                    });
                                                let mut cmds = vec![Command::PaintMeshTexture {
                                                    node_id,
                                                    uv: h.uv,
                                                    radius_px: scene.lowpoly_brush_radius,
                                                    color,
                                                }];
                                                if let Some(muv) = mirror_uv {
                                                    cmds.push(Command::PaintMeshTexture {
                                                        node_id,
                                                        uv: muv,
                                                        radius_px: scene.lowpoly_brush_radius,
                                                        color,
                                                    });
                                                }
                                                let _ = history
                                                    .execute(scene, Command::Transaction(cmds));
                                            } else {
                                                let _ = history.execute(
                                                    scene,
                                                    Command::PaintMeshTexture {
                                                        node_id,
                                                        uv: h.uv,
                                                        radius_px: scene.lowpoly_brush_radius,
                                                        color,
                                                    },
                                                );
                                            }
                                        }
                                    }
                                } else {
                                    // Select mode: pick closest visible low-poly mesh & face
                                    let mut best: Option<(f32, NodeId, tessel_core::FaceId)> = None;
                                    for (&nid, mesh) in &scene.lowpoly_meshes {
                                        let visible =
                                            scene.nodes.get(&nid).is_some_and(|n| n.visible);
                                        if !visible {
                                            continue;
                                        }
                                        let node_mat = scene.node_world_matrix(nid);
                                        let inv = node_mat.inverse();
                                        let local_orig = inv.transform_point3(orig);
                                        let local_dir =
                                            inv.transform_vector3(dir).normalize_or_zero();
                                        if let Some(h) = tessel_core::mesh::ops::raycast_mesh_hit(
                                            mesh, local_orig, local_dir,
                                        ) {
                                            let world_hit = node_mat.transform_point3(h.point);
                                            let dist_sq = (world_hit - orig).length_squared();
                                            if best.is_none_or(|(d, _, _)| dist_sq < d) {
                                                best = Some((dist_sq, nid, h.face_id));
                                            }
                                        }
                                    }
                                    if let Some((_, hit_nid, face_id)) = best {
                                        if scene.selected_node != Some(hit_nid) {
                                            let _ = history
                                                .execute(scene, Command::SelectNode(Some(hit_nid)));
                                        }
                                        scene.selected_face = Some(face_id);
                                        scene.mark_modified();
                                    } else {
                                        scene.selected_face = None;
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

        if scene.active_mode != AppMode::Pixel && !*render_preview {
            draw_selection_brackets(ui, rect, scene, camera_3d);
        }

        show_viewport_hud(
            ui,
            rect,
            scene.active_mode,
            camera_3d,
            camera_2d,
            free_view,
            render_preview,
        );
        if scene.active_mode != AppMode::Pixel && !*render_preview {
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
    render_preview: &mut bool,
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
                        if icons::hud_icon_button(ui, ToolIcon::ZoomOut, "Zoom Out", false) {
                            match mode {
                                AppMode::Pixel => camera_2d.zoom_step(0.8),
                                _ => camera_3d.zoom_step(1.2),
                            }
                        }
                        if icons::hud_icon_button(
                            ui,
                            ToolIcon::ResetCamera,
                            "Reset Camera (F)",
                            false,
                        ) {
                            match mode {
                                AppMode::Pixel => camera_2d.reset(),
                                _ => camera_3d.reset(),
                            }
                        }
                        if icons::hud_icon_button(ui, ToolIcon::ZoomIn, "Zoom In", false) {
                            match mode {
                                AppMode::Pixel => camera_2d.zoom_step(1.25),
                                _ => camera_3d.zoom_step(0.82),
                            }
                        }

                        if mode != AppMode::Pixel {
                            ui.separator();
                            if icons::hud_icon_button(
                                ui,
                                ToolIcon::Select,
                                "Select Mode: click objects to select & drag gizmo to move",
                                !*free_view,
                            ) {
                                *free_view = false;
                            }
                            if icons::hud_icon_button(
                                ui,
                                ToolIcon::Hand,
                                "Handfree Mode: orbit & pan camera only (objects locked)",
                                *free_view,
                            ) {
                                *free_view = true;
                            }
                            if icons::hud_icon_button(
                                ui,
                                ToolIcon::Preview,
                                "Render Preview: hide grid, wireframe, gizmo, and compass",
                                *render_preview,
                            ) {
                                *render_preview = !*render_preview;
                            }
                            let presets = [
                                (ToolIcon::ViewIso, "Iso view", CameraPreset::Iso),
                                (ToolIcon::ViewFront, "Front view", CameraPreset::Front),
                                (ToolIcon::ViewRight, "Right view", CameraPreset::Right),
                                (ToolIcon::ViewTop, "Top view", CameraPreset::Top),
                            ];
                            for (icon, tip, preset) in presets {
                                if icons::hud_icon_button(ui, icon, tip, false) {
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

/// Computes the world-space AABB `(min, max)` of `node_id` for selection brackets.
pub fn selected_node_world_bounds(
    scene: &SceneState,
    node_id: NodeId,
) -> Option<(glam::Vec3, glam::Vec3)> {
    let node = scene.nodes.get(&node_id)?;
    if !node.visible {
        return None;
    }
    let mat = scene.node_world_matrix(node_id);
    if let Some(mesh) = scene.lowpoly_meshes.get(&node_id) {
        if !mesh.vertices.is_empty() {
            let (lmin, lmax) = mesh.bounds();
            return Some(transform_aabb(mat, lmin, lmax));
        }
    }
    if let Some(vol) = scene.voxel_volumes.get(&node_id) {
        if !vol.is_empty() {
            let mut min_c = glam::IVec3::splat(i32::MAX);
            let mut max_c = glam::IVec3::splat(i32::MIN);
            for (coord, _) in vol.iter_voxels() {
                min_c = min_c.min(glam::IVec3::new(coord.x, coord.y, coord.z));
                max_c = max_c.max(glam::IVec3::new(coord.x, coord.y, coord.z));
            }
            let s = scene.unit_scale;
            let lmin = min_c.as_vec3() * s;
            let lmax = (max_c + glam::IVec3::ONE).as_vec3() * s;
            return Some(transform_aabb(mat, lmin, lmax));
        }
    }
    let center = mat.transform_point3(glam::Vec3::ZERO);
    Some((
        center - glam::Vec3::splat(0.25),
        center + glam::Vec3::splat(0.25),
    ))
}

fn transform_aabb(mat: glam::Mat4, min: glam::Vec3, max: glam::Vec3) -> (glam::Vec3, glam::Vec3) {
    let mut wmin = glam::Vec3::splat(f32::INFINITY);
    let mut wmax = glam::Vec3::splat(f32::NEG_INFINITY);
    for dx in [min.x, max.x] {
        for dy in [min.y, max.y] {
            for dz in [min.z, max.z] {
                let p = mat.transform_point3(glam::Vec3::new(dx, dy, dz));
                wmin = wmin.min(p);
                wmax = wmax.max(p);
            }
        }
    }
    (wmin, wmax)
}

/// Projects a world-space AABB into a padded 2D screen rectangle inside `viewport_rect`.
pub fn project_bounds_to_screen_rect(
    min: glam::Vec3,
    max: glam::Vec3,
    camera: &OrbitCamera,
    viewport_rect: egui::Rect,
) -> Option<egui::Rect> {
    let w = viewport_rect.width().max(1.0);
    let h = viewport_rect.height().max(1.0);
    let aspect = w / h;
    let view = glam::Mat4::look_at_rh(camera.position(), camera.target, glam::Vec3::Y);
    let proj = glam::Mat4::perspective_rh(camera.fov_y_radians, aspect, camera.near, camera.far);
    let vp = proj * view;

    let mut smin = egui::pos2(f32::INFINITY, f32::INFINITY);
    let mut smax = egui::pos2(f32::NEG_INFINITY, f32::NEG_INFINITY);
    let mut any_visible = false;

    for dx in [min.x, max.x] {
        for dy in [min.y, max.y] {
            for dz in [min.z, max.z] {
                let clip = vp * glam::Vec4::new(dx, dy, dz, 1.0);
                if clip.w <= 1e-4 {
                    continue;
                }
                let ndc = clip.truncate() / clip.w;
                let sx = viewport_rect.left() + (ndc.x * 0.5 + 0.5) * w;
                let sy = viewport_rect.top() + (0.5 - ndc.y * 0.5) * h;
                smin.x = smin.x.min(sx);
                smin.y = smin.y.min(sy);
                smax.x = smax.x.max(sx);
                smax.y = smax.y.max(sy);
                any_visible = true;
            }
        }
    }
    if !any_visible {
        return None;
    }
    let rect = egui::Rect::from_min_max(smin, smax).expand(8.0);
    if !rect.intersects(viewport_rect) {
        return None;
    }
    Some(rect.intersect(viewport_rect))
}

fn draw_selection_brackets(
    ui: &egui::Ui,
    viewport_rect: egui::Rect,
    scene: &SceneState,
    camera: &OrbitCamera,
) {
    let Some(node_id) = scene.selected_node else {
        return;
    };
    let Some((wmin, wmax)) = selected_node_world_bounds(scene, node_id) else {
        return;
    };
    let Some(sel) = project_bounds_to_screen_rect(wmin, wmax, camera, viewport_rect) else {
        return;
    };

    let painter = ui.painter_at(viewport_rect);
    let stroke = Stroke::new(2.2, tokens::PRIMARY);
    let arm_x = (sel.width() * 0.22).clamp(8.0, 22.0);
    let arm_y = (sel.height() * 0.22).clamp(8.0, 22.0);

    // Left `[` bracket (top-left + bottom-left corners)
    let tl = sel.left_top();
    let bl = sel.left_bottom();
    painter.line_segment([tl, egui::pos2(tl.x + arm_x, tl.y)], stroke);
    painter.line_segment([tl, egui::pos2(tl.x, tl.y + arm_y)], stroke);
    painter.line_segment([bl, egui::pos2(bl.x + arm_x, bl.y)], stroke);
    painter.line_segment([bl, egui::pos2(bl.x, bl.y - arm_y)], stroke);

    // Right `]` bracket (top-right + bottom-right corners)
    let tr = sel.right_top();
    let br = sel.right_bottom();
    painter.line_segment([tr, egui::pos2(tr.x - arm_x, tr.y)], stroke);
    painter.line_segment([tr, egui::pos2(tr.x, tr.y + arm_y)], stroke);
    painter.line_segment([br, egui::pos2(br.x - arm_x, br.y)], stroke);
    painter.line_segment([br, egui::pos2(br.x, br.y - arm_y)], stroke);

    if let Some(name) = scene.nodes.get(&node_id).map(|n| n.name.as_str()) {
        let tag_pos = egui::pos2(tl.x + 2.0, (tl.y - 16.0).max(viewport_rect.top() + 4.0));
        painter.text(
            tag_pos,
            egui::Align2::LEFT_TOP,
            format!("[ {name} ]"),
            egui::FontId::monospace(10.5),
            tokens::PRIMARY,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tessel_core::{PaletteIndex, PrimitiveKind};

    #[test]
    fn selection_brackets_compute_and_project_mesh_bounds() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        history
            .execute(
                &mut scene,
                Command::CreatePrimitiveMesh {
                    name: "HeroCube".into(),
                    kind: PrimitiveKind::Cube,
                    size: 1.0,
                    material: PaletteIndex::new(1).unwrap(),
                },
            )
            .unwrap();
        let node_id = scene.selected_node.unwrap();
        let (wmin, wmax) = selected_node_world_bounds(&scene, node_id).unwrap();
        assert!(wmax.y > wmin.y);

        let cam = OrbitCamera::new();
        let vp = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let screen = project_bounds_to_screen_rect(wmin, wmax, &cam, vp).unwrap();
        assert!(screen.width() > 10.0 && screen.height() > 10.0);
        assert!(vp.contains_rect(screen));
    }
}
