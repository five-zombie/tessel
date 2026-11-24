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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SceneGadgetDrag {
    Sun {
        start_azimuth: f32,
        start_elevation: f32,
    },
    Camera,
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
    camera_3d_target: &mut Option<OrbitCamera>,
    showcase_camera: &mut OrbitCamera,
    showcase_camera_target: &mut Option<OrbitCamera>,
    display_light: (f32, f32),
    camera_2d: &mut PixelCamera2D,
    pixel_drag_start: &mut Option<PixelCoord>,
    gizmo_drag: &mut Option<GizmoDragState>,
    scene_gadget_drag: &mut Option<SceneGadgetDrag>,
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
            let gadget_consumed = if *render_preview {
                *scene_gadget_drag = None;
                false
            } else {
                handle_scene_gadget_input(
                    ui,
                    rect,
                    &response,
                    scene,
                    history,
                    camera_3d,
                    camera_3d_target,
                    showcase_camera,
                    showcase_camera_target,
                    display_light,
                    scene_gadget_drag,
                )
            };
            let gizmo_consumed = if *free_view || gadget_consumed {
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
            let any_gadget_consumed = gadget_consumed || gizmo_consumed;
            let allow_primary_orbit = *free_view
                && !any_gadget_consumed
                && !(scene.active_mode == AppMode::LowPoly && scene.lowpoly_paint_mode);
            if !gadget_consumed {
                handle_3d_viewport_input(
                    ui,
                    &response,
                    camera_3d,
                    camera_3d_target,
                    allow_primary_orbit,
                );
            }

            if !any_gadget_consumed && !*free_view {
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
            draw_scene_gadgets(
                ui,
                rect,
                &response,
                display_light,
                camera_3d,
                showcase_camera,
                *scene_gadget_drag,
            );
        }

        show_viewport_hud(
            ui,
            rect,
            scene.active_mode,
            camera_3d,
            camera_3d_target,
            showcase_camera,
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
        Stroke::new(1.0_f32, tokens::OUTLINE_VARIANT),
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
        painter.line_segment([center, end], Stroke::new(2.0_f32, col));
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

#[allow(clippy::too_many_arguments)]
fn show_viewport_hud(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    mode: AppMode,
    camera_3d: &mut OrbitCamera,
    camera_3d_target: &mut Option<OrbitCamera>,
    showcase_camera: &OrbitCamera,
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
                .stroke(Stroke::new(1.0_f32, tokens::OUTLINE_VARIANT))
                .corner_radius(CornerRadius::same(4))
                .inner_margin(Margin::symmetric(6, 4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if icons::hud_icon_button(ui, ToolIcon::ZoomOut, "Zoom Out", false) {
                            match mode {
                                AppMode::Pixel => camera_2d.zoom_step(0.8),
                                _ => {
                                    *camera_3d_target = None;
                                    camera_3d.zoom_step(1.2);
                                }
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
                                _ => *camera_3d_target = Some(OrbitCamera::new()),
                            }
                        }
                        if icons::hud_icon_button(ui, ToolIcon::ZoomIn, "Zoom In", false) {
                            match mode {
                                AppMode::Pixel => camera_2d.zoom_step(1.25),
                                _ => {
                                    *camera_3d_target = None;
                                    camera_3d.zoom_step(0.82);
                                }
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
                                    let mut goal = camera_3d_target.unwrap_or(*camera_3d);
                                    goal.set_preset(preset);
                                    *camera_3d_target = Some(goal);
                                }
                            }
                            if icons::hud_icon_button(
                                ui,
                                ToolIcon::SceneCamera,
                                "Snap view to Scene Camera / Projector",
                                false,
                            ) {
                                *camera_3d_target = Some(*showcase_camera);
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
    camera_target: &mut Option<OrbitCamera>,
    allow_primary_orbit: bool,
) {
    let primary_drag = response.dragged_by(egui::PointerButton::Primary)
        && (allow_primary_orbit || ui.input(|i| i.modifiers.alt));
    if response.dragged_by(egui::PointerButton::Secondary)
        || response.dragged_by(egui::PointerButton::Middle)
        || primary_drag
    {
        *camera_target = None;
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
            *camera_target = None;
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
    let stroke = Stroke::new(2.2_f32, tokens::PRIMARY);
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

const GADGET_HIT_RADIUS_PX: f32 = 18.0;
const MIN_SUN_ELEVATION: f32 = 0.12;
const MAX_SUN_ELEVATION: f32 = 1.50;

/// Computes the 3D world position of the interactive Sun gizmo orbiting `target`.
pub fn light_world_position(
    target: glam::Vec3,
    azimuth: f32,
    elevation: f32,
    radius: f32,
) -> glam::Vec3 {
    let cos_e = elevation.cos();
    let dir = glam::Vec3::new(
        cos_e * azimuth.sin(),
        elevation.sin(),
        cos_e * azimuth.cos(),
    )
    .normalize_or_zero();
    target + dir * radius
}

/// Projects a 3D world point into 2D screen coordinates inside `viewport_rect`.
/// Returns `None` if the point lies behind the camera near plane.
pub fn project_world_point(
    pt: glam::Vec3,
    camera: &OrbitCamera,
    viewport_rect: egui::Rect,
) -> Option<egui::Pos2> {
    let w = viewport_rect.width().max(1.0);
    let h = viewport_rect.height().max(1.0);
    let aspect = w / h;
    let view = glam::Mat4::look_at_rh(camera.position(), camera.target, glam::Vec3::Y);
    let proj = glam::Mat4::perspective_rh(camera.fov_y_radians, aspect, camera.near, camera.far);
    let clip = (proj * view) * glam::Vec4::new(pt.x, pt.y, pt.z, 1.0);
    if clip.w <= 1e-4 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    let sx = viewport_rect.left() + (ndc.x * 0.5 + 0.5) * w;
    let sy = viewport_rect.top() + (0.5 - ndc.y * 0.5) * h;
    Some(egui::pos2(sx, sy))
}

/// Applies pointer drag delta in screen pixels to `(light_azimuth, light_elevation)`.
pub fn apply_sun_drag_delta(azimuth: f32, elevation: f32, delta_px: egui::Vec2) -> (f32, f32) {
    let next_az = (azimuth - delta_px.x * 0.012).rem_euclid(std::f32::consts::TAU);
    let next_el = (elevation - delta_px.y * 0.010).clamp(MIN_SUN_ELEVATION, MAX_SUN_ELEVATION);
    (next_az, next_el)
}

fn sun_gizmo_radius(camera: &OrbitCamera) -> f32 {
    (camera.distance * 0.36).clamp(2.2, 5.5)
}

#[allow(clippy::too_many_arguments)]
fn handle_scene_gadget_input(
    ui: &egui::Ui,
    rect: egui::Rect,
    response: &egui::Response,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    camera_3d: &mut OrbitCamera,
    camera_3d_target: &mut Option<OrbitCamera>,
    showcase_camera: &mut OrbitCamera,
    showcase_camera_target: &mut Option<OrbitCamera>,
    display_light: (f32, f32),
    gadget_drag: &mut Option<SceneGadgetDrag>,
) -> bool {
    let sun_world = light_world_position(
        camera_3d.target,
        display_light.0,
        display_light.1,
        sun_gizmo_radius(camera_3d),
    );
    let sun_screen = project_world_point(sun_world, camera_3d, rect);
    let cam_screen = project_world_point(showcase_camera.position(), camera_3d, rect);

    if response.double_clicked_by(egui::PointerButton::Primary) {
        if let (Some(pos), Some(cs)) = (response.interact_pointer_pos(), cam_screen) {
            if pos.distance(cs) <= GADGET_HIT_RADIUS_PX {
                *camera_3d_target = Some(*showcase_camera);
                return true;
            }
        }
    }

    if gadget_drag.is_none() && response.drag_started_by(egui::PointerButton::Primary) {
        if let Some(pos) = response.interact_pointer_pos() {
            if let Some(ss) = sun_screen {
                if pos.distance(ss) <= GADGET_HIT_RADIUS_PX {
                    *gadget_drag = Some(SceneGadgetDrag::Sun {
                        start_azimuth: scene.light_azimuth,
                        start_elevation: scene.light_elevation,
                    });
                    return true;
                }
            }
            if let Some(cs) = cam_screen {
                if pos.distance(cs) <= GADGET_HIT_RADIUS_PX {
                    *showcase_camera_target = None;
                    *gadget_drag = Some(SceneGadgetDrag::Camera);
                    return true;
                }
            }
        }
    }

    if let Some(drag) = *gadget_drag {
        match drag {
            SceneGadgetDrag::Sun {
                start_azimuth,
                start_elevation,
            } => {
                if response.dragged_by(egui::PointerButton::Primary) {
                    let delta = response.drag_delta();
                    let (az, el) =
                        apply_sun_drag_delta(scene.light_azimuth, scene.light_elevation, delta);
                    scene.light_azimuth = az;
                    scene.light_elevation = el;
                    scene.mark_modified();
                    return true;
                }
                if response.drag_stopped_by(egui::PointerButton::Primary) {
                    let final_az = scene.light_azimuth;
                    let final_el = scene.light_elevation;
                    scene.light_azimuth = start_azimuth;
                    scene.light_elevation = start_elevation;
                    let _ = history.execute(
                        scene,
                        Command::SetLightDirection {
                            azimuth: final_az,
                            elevation: final_el,
                        },
                    );
                    *gadget_drag = None;
                    return true;
                }
            }
            SceneGadgetDrag::Camera => {
                if response.dragged_by(egui::PointerButton::Primary) {
                    *showcase_camera_target = None;
                    let delta = response.drag_delta();
                    if ui.input(|i| i.modifiers.shift) {
                        showcase_camera.pan(delta.x, delta.y);
                    } else {
                        showcase_camera.orbit(delta.x, delta.y);
                    }
                    return true;
                }
                if response.drag_stopped_by(egui::PointerButton::Primary) {
                    *gadget_drag = None;
                    return true;
                }
            }
        }
    }

    false
}

fn draw_scene_gadgets(
    ui: &egui::Ui,
    viewport_rect: egui::Rect,
    response: &egui::Response,
    display_light: (f32, f32),
    camera_3d: &OrbitCamera,
    showcase_camera: &OrbitCamera,
    active_drag: Option<SceneGadgetDrag>,
) {
    let painter = ui.painter_at(viewport_rect);
    let hover = response.hover_pos();

    // 1. Draggable Sun / Key Light gizmo
    let sun_world = light_world_position(
        camera_3d.target,
        display_light.0,
        display_light.1,
        sun_gizmo_radius(camera_3d),
    );
    if let Some(sun_pos) = project_world_point(sun_world, camera_3d, viewport_rect) {
        let sun_active = matches!(active_drag, Some(SceneGadgetDrag::Sun { .. }))
            || hover.is_some_and(|h| h.distance(sun_pos) <= GADGET_HIT_RADIUS_PX);
        let sun_gold = egui::Color32::from_rgb(255, 206, 84);
        let sun_ray_col =
            egui::Color32::from_rgba_unmultiplied(255, 206, 84, if sun_active { 170 } else { 95 });
        if let Some(target_pos) = project_world_point(camera_3d.target, camera_3d, viewport_rect) {
            painter.line_segment([sun_pos, target_pos], Stroke::new(1.4_f32, sun_ray_col));
        }
        let fill = if sun_active {
            egui::Color32::from_rgba_unmultiplied(68, 52, 18, 235)
        } else {
            egui::Color32::from_rgba_unmultiplied(28, 30, 38, 215)
        };
        painter.circle_filled(sun_pos, 14.5, fill);
        painter.circle_stroke(
            sun_pos,
            14.5,
            Stroke::new(if sun_active { 2.2_f32 } else { 1.5_f32 }, sun_gold),
        );
        painter.text(
            sun_pos,
            egui::Align2::CENTER_CENTER,
            ToolIcon::Sun.glyph(),
            egui::FontId::proportional(16.0),
            sun_gold,
        );
        if sun_active {
            painter.text(
                sun_pos + egui::vec2(0.0, 21.0),
                egui::Align2::CENTER_TOP,
                format!(
                    "SUN {:.0}° / {:.0}°",
                    display_light.0.to_degrees(),
                    display_light.1.to_degrees()
                ),
                egui::FontId::monospace(10.0),
                sun_gold,
            );
        }
    }

    // 2. Draggable Camera / Projector proxy gizmo + frustum pyramid
    let cam_origin = showcase_camera.position();
    if let Some(cam_pos) = project_world_point(cam_origin, camera_3d, viewport_rect) {
        let cam_active = matches!(active_drag, Some(SceneGadgetDrag::Camera))
            || hover.is_some_and(|h| h.distance(cam_pos) <= GADGET_HIT_RADIUS_PX);
        let cam_cyan = egui::Color32::from_rgb(92, 196, 255);
        let frustum_col =
            egui::Color32::from_rgba_unmultiplied(92, 196, 255, if cam_active { 200 } else { 120 });

        let fwd = (showcase_camera.target - cam_origin).normalize_or_zero();
        let right = fwd.cross(glam::Vec3::Y).normalize_or_zero();
        let up = right.cross(fwd).normalize_or_zero();
        let frustum_center = cam_origin + fwd * 0.65;
        let hw = 0.24;
        let hh = 0.15;
        let corners_world = [
            frustum_center - right * hw + up * hh,
            frustum_center + right * hw + up * hh,
            frustum_center + right * hw - up * hh,
            frustum_center - right * hw - up * hh,
        ];
        let corners_2d: [Option<egui::Pos2>; 4] = [
            project_world_point(corners_world[0], camera_3d, viewport_rect),
            project_world_point(corners_world[1], camera_3d, viewport_rect),
            project_world_point(corners_world[2], camera_3d, viewport_rect),
            project_world_point(corners_world[3], camera_3d, viewport_rect),
        ];
        for c in corners_2d.into_iter().flatten() {
            painter.line_segment([cam_pos, c], Stroke::new(1.3_f32, frustum_col));
        }
        for i in 0..4 {
            if let (Some(a), Some(b)) = (corners_2d[i], corners_2d[(i + 1) % 4]) {
                painter.line_segment([a, b], Stroke::new(1.3_f32, frustum_col));
            }
        }
        if let Some(target_pos) =
            project_world_point(showcase_camera.target, camera_3d, viewport_rect)
        {
            painter.line_segment(
                [cam_pos, target_pos],
                Stroke::new(
                    1.0_f32,
                    egui::Color32::from_rgba_unmultiplied(
                        92,
                        196,
                        255,
                        if cam_active { 125 } else { 55 },
                    ),
                ),
            );
        }

        let fill = if cam_active {
            egui::Color32::from_rgba_unmultiplied(20, 48, 68, 235)
        } else {
            egui::Color32::from_rgba_unmultiplied(28, 30, 38, 215)
        };
        painter.circle_filled(cam_pos, 14.5, fill);
        painter.circle_stroke(
            cam_pos,
            14.5,
            Stroke::new(if cam_active { 2.2_f32 } else { 1.5_f32 }, cam_cyan),
        );
        painter.text(
            cam_pos,
            egui::Align2::CENTER_CENTER,
            ToolIcon::SceneCamera.glyph(),
            egui::FontId::proportional(15.5),
            cam_cyan,
        );
        if cam_active {
            painter.text(
                cam_pos + egui::vec2(0.0, 21.0),
                egui::Align2::CENTER_TOP,
                "CAM (drag / 2x click)",
                egui::FontId::monospace(10.0),
                cam_cyan,
            );
        }
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

    #[test]
    fn light_world_position_matches_spherical_light_direction() {
        let target = glam::Vec3::new(0.0, 1.0, 0.0);
        let pos = light_world_position(target, 0.0, std::f32::consts::FRAC_PI_2, 4.0);
        assert!((pos - glam::Vec3::new(0.0, 5.0, 0.0)).length() < 1e-4);

        let side = light_world_position(glam::Vec3::ZERO, std::f32::consts::FRAC_PI_2, 0.0, 3.0);
        assert!((side - glam::Vec3::new(3.0, 0.0, 0.0)).length() < 1e-4);
    }

    #[test]
    fn apply_sun_drag_delta_updates_azimuth_and_clamps_elevation() {
        let (az, el) = apply_sun_drag_delta(0.5, 0.8, egui::vec2(100.0, -500.0));
        assert!((0.0..std::f32::consts::TAU).contains(&az));
        assert!((el - MAX_SUN_ELEVATION).abs() < 1e-5);

        let (_, el_low) = apply_sun_drag_delta(0.5, 0.2, egui::vec2(0.0, 500.0));
        assert!((el_low - MIN_SUN_ELEVATION).abs() < 1e-5);
    }

    #[test]
    fn project_world_point_projects_in_front_and_rejects_behind() {
        let cam = OrbitCamera::new();
        let vp = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let center = project_world_point(cam.target, &cam, vp).unwrap();
        assert!((center.x - 400.0).abs() < 2.0);
        assert!((center.y - 300.0).abs() < 2.0);

        let behind = cam.position() + (cam.position() - cam.target).normalize() * 2.0;
        assert!(project_world_point(behind, &cam, vp).is_none());
    }
}
