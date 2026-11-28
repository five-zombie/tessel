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
    PlacementGhost, TransformGizmoMode,
};

use super::icons::{self, ToolIcon};
pub use super::rotate_gizmo::RotateAxisMode;
use super::rotate_gizmo::{
    compute_floating_rotate_transform, draw_floating_rotate_disc, node_is_effectively_visible,
    node_world_rotation, parent_world_rotation, pressed_rotate_axis_key,
    show_rotate_axis_and_ruler_hud, transform_differs, RotateDragState, ROTATE_DISC_RADIUS_PX,
};
use super::theme::tokens;

/// Explicit 3D viewport interaction mode separating object selection/transform
/// from mode-specific authoring and interactive primitive placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewportInteractionTool {
    /// Click objects to select & drag gizmo without triggering voxel/sculpt/paint edits.
    #[default]
    Select,
    /// Mode-specific authoring tool (voxel place/erase/paint, low-poly paint/sculpt).
    Edit,
    /// Interactive primitive placement with wireframe ghost preview raycast to surface/grid.
    PlacePrimitive(tessel_core::PrimitiveKind),
}

/// Active 3D transform gizmo drag state (separated cleanly by mode).
#[derive(Debug, Clone, Copy)]
pub enum GizmoDragState {
    Move {
        node_id: NodeId,
        axis: GizmoAxis,
        start_transform: Transform,
        start_world_pos: glam::Vec3,
        parent_world_inv: glam::Mat4,
        start_axis_t: f32,
    },
    Rotate(RotateDragState),
    Scale {
        node_id: NodeId,
        axis: GizmoAxis,
        start_transform: Transform,
        start_pointer_pos: egui::Pos2,
        drag_dir_2d: egui::Vec2,
        drag_norm_px: f32,
    },
}

impl GizmoDragState {
    pub fn node_id(&self) -> NodeId {
        match *self {
            Self::Move { node_id, .. } | Self::Scale { node_id, .. } => node_id,
            Self::Rotate(r) => r.node_id,
        }
    }

    pub fn mode(&self) -> TransformGizmoMode {
        match self {
            Self::Move { .. } => TransformGizmoMode::Move,
            Self::Rotate(_) => TransformGizmoMode::Rotate,
            Self::Scale { .. } => TransformGizmoMode::Scale,
        }
    }

    pub fn active_3d_axis(&self) -> Option<GizmoAxis> {
        match *self {
            Self::Move { axis, .. } | Self::Scale { axis, .. } => Some(axis),
            Self::Rotate(_) => None,
        }
    }

    pub fn initial_transform(&self) -> Transform {
        match *self {
            Self::Move {
                start_transform, ..
            }
            | Self::Scale {
                start_transform, ..
            } => start_transform,
            Self::Rotate(r) => r.initial_transform,
        }
    }

    pub fn as_rotate(&self) -> Option<RotateDragState> {
        match *self {
            Self::Rotate(r) => Some(r),
            Self::Move { .. } | Self::Scale { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SceneGadgetDrag {
    Sun {
        start_azimuth: f32,
        start_elevation: f32,
    },
    Camera,
}

/// Per-gadget visibility toggles for the 3D viewport overlay (`camera` proxy & `sun` light).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportGadgetVisibility {
    pub camera: bool,
    pub sun: bool,
}

impl Default for ViewportGadgetVisibility {
    fn default() -> Self {
        Self {
            camera: true,
            sun: true,
        }
    }
}

/// Cancels an in-progress gizmo drag and restores the node's pre-drag transform so no
/// unrecorded mutation leaks outside the `Command` bus and `scene.revision` stays clean.
pub fn cancel_gizmo_drag(scene: &mut SceneState, gizmo_drag: &mut Option<GizmoDragState>) {
    if let Some(drag) = gizmo_drag.take() {
        scene.preview_node_transform(drag.node_id(), drag.initial_transform());
    }
}

/// Single authoritative gate deciding whether the 3D transform gizmo (Move / Floating Rotate / Scale)
/// is active for the current viewport mode, tool state, and sidebar `Viewport Gizmo` checkbox.
pub fn is_transform_gizmo_enabled(
    scene: &SceneState,
    interaction_tool: ViewportInteractionTool,
    free_view: bool,
    render_preview: bool,
    show_viewport_gizmo: bool,
) -> bool {
    show_viewport_gizmo
        && scene.active_mode != AppMode::Pixel
        && !free_view
        && !render_preview
        && interaction_tool == ViewportInteractionTool::Select
        && !(scene.active_mode == AppMode::Character && scene.weight_paint_mode)
}

/// Single authoritative helper for activating 3D transform gizmos from either the Inspector
/// sidebar or the top-right Viewport HUD. Clears `free_view`, `render_preview`, and any
/// conflicting paint/sculpt/weight-paint authoring modes.
pub fn activate_transform_gizmo(
    scene: &mut SceneState,
    show_viewport_gizmo: &mut bool,
    free_view: &mut bool,
    render_preview: &mut bool,
    interaction_tool: &mut ViewportInteractionTool,
) {
    *show_viewport_gizmo = true;
    *free_view = false;
    *render_preview = false;
    *interaction_tool = ViewportInteractionTool::Select;
    scene.lowpoly_paint_mode = false;
    scene.sculpt_mode = false;
    scene.weight_paint_mode = false;
}

fn has_visible_selected_node(scene: &SceneState) -> bool {
    scene
        .selected_node
        .is_some_and(|id| node_is_effectively_visible(scene, id))
}

/// Cancels an in-progress scene gadget drag (e.g. Sun light) and restores its pre-drag state.
pub fn cancel_scene_gadget_drag(
    scene: &mut SceneState,
    scene_gadget_drag: &mut Option<SceneGadgetDrag>,
) {
    if let Some(SceneGadgetDrag::Sun {
        start_azimuth,
        start_elevation,
    }) = scene_gadget_drag.take()
    {
        scene.preview_lighting(start_azimuth, start_elevation);
    }
}

/// Displays the offscreen wgpu viewport texture, routes 2D/3D viewport input,
/// renders the floating Zoom & Camera Preset HUD, and returns
/// `(desired_physical_size, hover_voxel, hovered_gizmo_axis, placement_ghost)`.
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
    gizmo_mode: &mut TransformGizmoMode,
    rotate_axis_mode: &mut RotateAxisMode,
    show_viewport_gizmo: &mut bool,
    scene_gadget_drag: &mut Option<SceneGadgetDrag>,
    gadget_visibility: &mut ViewportGadgetVisibility,
    interaction_tool: &mut ViewportInteractionTool,
    free_view: &mut bool,
    render_preview: &mut bool,
) -> (
    (u32, u32),
    Option<VoxelCoord>,
    Option<GizmoAxis>,
    Option<PlacementGhost>,
) {
    let mut desired_size = (1, 1);
    let mut hover_voxel = None;
    let mut active_gizmo_axis = gizmo_drag.and_then(|d| d.active_3d_axis());
    let mut placement_ghost: Option<PlacementGhost> = None;
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

        if scene.active_mode != AppMode::LowPoly
            && matches!(
                *interaction_tool,
                ViewportInteractionTool::PlacePrimitive(_)
            )
        {
            *interaction_tool = ViewportInteractionTool::Select;
        }

        if scene.active_mode == AppMode::Pixel {
            cancel_scene_gadget_drag(scene, scene_gadget_drag);
            cancel_gizmo_drag(scene, gizmo_drag);
            active_gizmo_axis = None;
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
                cancel_scene_gadget_drag(scene, scene_gadget_drag);
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
                    *gadget_visibility,
                )
            };
            let gizmo_enabled = is_transform_gizmo_enabled(
                scene,
                *interaction_tool,
                *free_view,
                *render_preview,
                *show_viewport_gizmo,
            );
            let gizmo_consumed = if !gizmo_enabled || gadget_consumed {
                cancel_gizmo_drag(scene, gizmo_drag);
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
                    *gizmo_mode,
                    rotate_axis_mode,
                    &mut active_gizmo_axis,
                )
            };
            let any_gadget_consumed = gadget_consumed || gizmo_consumed;
            let allow_primary_orbit = *free_view && !any_gadget_consumed;
            if !any_gadget_consumed {
                handle_3d_viewport_input(
                    ui,
                    &response,
                    camera_3d,
                    camera_3d_target,
                    allow_primary_orbit,
                );
            }

            if !any_gadget_consumed && !*free_view {
                if let ViewportInteractionTool::PlacePrimitive(kind) = *interaction_tool {
                    if ui.ctx().input(|i| i.key_pressed(egui::Key::Escape))
                        || response.clicked_by(egui::PointerButton::Secondary)
                    {
                        *interaction_tool = ViewportInteractionTool::Select;
                    } else if let Some(pointer) = response
                        .hover_pos()
                        .or_else(|| response.interact_pointer_pos())
                    {
                        let uv = [
                            (pointer.x - rect.min.x) / rect.width().max(1.0),
                            (pointer.y - rect.min.y) / rect.height().max(1.0),
                        ];
                        let aspect = rect.width() / rect.height().max(1.0);
                        let (orig, dir) = camera_3d.screen_ray(uv, aspect);
                        if let Some(hit) = raycast_placement_point(scene, orig, dir) {
                            let half = 0.4_f32;
                            placement_ghost = Some(PlacementGhost {
                                position: hit.to_array(),
                                half_extents: [half, half, half],
                            });
                            if response.clicked_by(egui::PointerButton::Primary) {
                                place_primitive_at(ui.ctx(), scene, history, kind, hit);
                                let keep_placing = ui.ctx().input(|i| i.modifiers.shift);
                                if !keep_placing {
                                    *interaction_tool = ViewportInteractionTool::Select;
                                }
                            }
                        }
                    }
                } else {
                    match scene.active_mode {
                        AppMode::Voxel => {
                            if *interaction_tool == ViewportInteractionTool::Edit {
                                hover_voxel =
                                    compute_hover_voxel(rect, &response, scene, camera_3d);
                                handle_voxel_click_input(
                                    ui, rect, &response, scene, history, camera_3d,
                                );
                            } else if response.clicked_by(egui::PointerButton::Primary) {
                                if let Some(pointer) = response.interact_pointer_pos() {
                                    let uv = [
                                        (pointer.x - rect.min.x) / rect.width().max(1.0),
                                        (pointer.y - rect.min.y) / rect.height().max(1.0),
                                    ];
                                    let aspect = rect.width() / rect.height().max(1.0);
                                    let (orig, dir) = camera_3d.screen_ray(uv, aspect);
                                    if let Some((hit_nid, face_id)) =
                                        pick_scene_node(scene, orig, dir)
                                    {
                                        if scene.selected_node != Some(hit_nid) {
                                            super::exec_history(
                                                ui.ctx(),
                                                history,
                                                scene,
                                                Command::SelectNode(Some(hit_nid)),
                                            );
                                        }
                                        scene.select_face_ui(face_id);
                                    }
                                }
                            }
                        }
                        AppMode::LowPoly => {
                            let is_select = *interaction_tool == ViewportInteractionTool::Select;
                            let sculpt_active = !is_select
                                && scene.sculpt_mode
                                && !scene.lowpoly_paint_mode
                                && (response.clicked_by(egui::PointerButton::Primary)
                                    || (response.dragged_by(egui::PointerButton::Primary)
                                        && response.drag_delta().length_sq() > 0.0));
                            let paint_active = !is_select
                                && scene.lowpoly_paint_mode
                                && (response.clicked_by(egui::PointerButton::Primary)
                                    || response.dragged_by(egui::PointerButton::Primary));
                            let pick_active = (is_select
                                || (!scene.lowpoly_paint_mode && !scene.sculpt_mode))
                                && response.clicked_by(egui::PointerButton::Primary);
                            if sculpt_active || paint_active || pick_active {
                                if let Some(pointer) = response.interact_pointer_pos() {
                                    let uv = [
                                        (pointer.x - rect.min.x) / rect.width().max(1.0),
                                        (pointer.y - rect.min.y) / rect.height().max(1.0),
                                    ];
                                    let aspect = rect.width() / rect.height().max(1.0);
                                    let (orig, dir) = camera_3d.screen_ray(uv, aspect);
                                    if sculpt_active {
                                        if let Some(node_id) = scene.active_lowpoly_node() {
                                            let node_mat = scene.node_world_matrix(node_id);
                                            if node_mat.determinant().abs() > 1e-8 {
                                                let inv = node_mat.inverse();
                                                let local_orig = inv.transform_point3(orig);
                                                let local_dir =
                                                    inv.transform_vector3(dir).normalize_or_zero();
                                                let hit = scene
                                                    .lowpoly_meshes
                                                    .get(&node_id)
                                                    .and_then(|mesh| {
                                                        let bvh =
                                                            tessel_core::MeshBvh::from_mesh(mesh);
                                                        bvh.raycast(local_orig, local_dir)
                                                    });
                                                if let Some(h) = hit {
                                                    let d = response.drag_delta();
                                                    let scale =
                                                        (camera_3d.distance * 0.004).max(0.002);
                                                    let cam_fwd = dir.normalize_or_zero();
                                                    let cam_right = cam_fwd
                                                        .cross(glam::Vec3::Y)
                                                        .normalize_or_zero();
                                                    let cam_up = cam_right
                                                        .cross(cam_fwd)
                                                        .normalize_or_zero();
                                                    let world_drag = cam_right * (d.x * scale)
                                                        - cam_up * (d.y * scale);
                                                    let local_drag =
                                                        inv.transform_vector3(world_drag);
                                                    super::exec_history(
                                                        ui.ctx(),
                                                        history,
                                                        scene,
                                                        Command::SculptMeshStroke {
                                                            node_id,
                                                            params:
                                                                tessel_core::SculptStrokeParams {
                                                                    brush: scene.sculpt_brush,
                                                                    center: h.point.to_array(),
                                                                    normal: h.normal.to_array(),
                                                                    drag_delta: local_drag
                                                                        .to_array(),
                                                                    radius: scene.sculpt_radius,
                                                                    strength: scene.sculpt_strength,
                                                                    mirror_x: scene
                                                                        .lowpoly_mirror_x,
                                                                },
                                                        },
                                                    );
                                                }
                                            }
                                        }
                                    } else if paint_active {
                                        if let Some(node_id) = scene.active_lowpoly_node() {
                                            let node_mat = scene.node_world_matrix(node_id);
                                            let inv = node_mat.inverse();
                                            let local_orig = inv.transform_point3(orig);
                                            let local_dir =
                                                inv.transform_vector3(dir).normalize_or_zero();
                                            let hit = scene.lowpoly_meshes.get(&node_id).and_then(
                                                |mesh| {
                                                    tessel_core::mesh::ops::raycast_mesh_hit(
                                                        mesh, local_orig, local_dir,
                                                    )
                                                },
                                            );
                                            if let Some(h) = hit {
                                                let color = scene
                                                    .palette
                                                    .get(scene.active_color)
                                                    .map(|e| e.color)
                                                    .unwrap_or(tessel_core::Rgba8::rgb(
                                                        225, 226, 235,
                                                    ));
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
                                                    let mut cmds =
                                                        vec![Command::PaintMeshTexture {
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
                                                    super::exec_history(
                                                        ui.ctx(),
                                                        history,
                                                        scene,
                                                        Command::Transaction(cmds),
                                                    );
                                                } else {
                                                    super::exec_history(
                                                        ui.ctx(),
                                                        history,
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
                                    } else if let Some((hit_nid, face_id)) =
                                        pick_scene_node(scene, orig, dir)
                                    {
                                        if scene.selected_node != Some(hit_nid) {
                                            super::exec_history(
                                                ui.ctx(),
                                                history,
                                                scene,
                                                Command::SelectNode(Some(hit_nid)),
                                            );
                                        }
                                        scene.select_face_ui(face_id);
                                    } else {
                                        scene.select_face_ui(None);
                                    }
                                }
                            }
                        }
                        AppMode::Character => {
                            let weight_paint_active = scene.weight_paint_mode
                                && (response.clicked_by(egui::PointerButton::Primary)
                                    || (response.dragged_by(egui::PointerButton::Primary)
                                        && response.drag_delta().length_sq() > 0.0));
                            let pick_bone_active = !scene.weight_paint_mode
                                && response.clicked_by(egui::PointerButton::Primary);
                            if weight_paint_active {
                                if let Some(pointer) = response.interact_pointer_pos() {
                                    let uv = [
                                        (pointer.x - rect.min.x) / rect.width().max(1.0),
                                        (pointer.y - rect.min.y) / rect.height().max(1.0),
                                    ];
                                    let aspect = rect.width() / rect.height().max(1.0);
                                    let (orig, dir) = camera_3d.screen_ray(uv, aspect);
                                    let target_bone = scene.selected_bone.or_else(|| {
                                        scene
                                            .character_rig
                                            .as_ref()
                                            .and_then(|r| r.bones.first().map(|b| b.id))
                                    });
                                    if let (Some(node_id), Some(bone_id)) =
                                        (scene.active_lowpoly_node(), target_bone)
                                    {
                                        let node_mat = scene.node_world_matrix(node_id);
                                        if node_mat.determinant().abs() > 1e-8 {
                                            let inv = node_mat.inverse();
                                            let local_orig = inv.transform_point3(orig);
                                            let local_dir =
                                                inv.transform_vector3(dir).normalize_or_zero();
                                            let hit = scene.lowpoly_meshes.get(&node_id).and_then(
                                                |mesh| {
                                                    tessel_core::mesh::ops::raycast_mesh_hit(
                                                        mesh, local_orig, local_dir,
                                                    )
                                                },
                                            );
                                            if let Some(h) = hit {
                                                if let Err(err) = history.execute(
                                                    scene,
                                                    Command::PaintMeshWeights {
                                                        node_id,
                                                        hit_point: h.point.to_array(),
                                                        radius: scene.weight_brush_radius,
                                                        bone_id,
                                                        strength: scene.weight_brush_strength,
                                                    },
                                                ) {
                                                    if err != tessel_core::CoreError::OutOfBounds {
                                                        tracing::warn!(
                                                            "weight paint stroke failed: {err}"
                                                        );
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            } else if pick_bone_active {
                                if let Some(pointer) = response.interact_pointer_pos() {
                                    let uv = [
                                        (pointer.x - rect.min.x) / rect.width().max(1.0),
                                        (pointer.y - rect.min.y) / rect.height().max(1.0),
                                    ];
                                    let aspect = rect.width() / rect.height().max(1.0);
                                    let (orig, dir) = camera_3d.screen_ray(uv, aspect);
                                    if let Some(rig) = &scene.character_rig {
                                        let picked = rig.pick_bone(orig, dir, 0.16);
                                        scene.select_bone_ui(picked);
                                    }
                                }
                            }
                        }
                        AppMode::Pixel => {}
                    }
                }
            }
        }

        let gizmo_visible = is_transform_gizmo_enabled(
            scene,
            *interaction_tool,
            *free_view,
            *render_preview,
            *show_viewport_gizmo,
        ) && has_visible_selected_node(scene);
        let show_rotate_ui = gizmo_visible && *gizmo_mode == TransformGizmoMode::Rotate;
        let show_scale_ui = gizmo_visible && *gizmo_mode == TransformGizmoMode::Scale;

        if show_rotate_ui {
            show_rotate_axis_and_ruler_hud(
                ui,
                rect,
                rotate_axis_mode,
                gizmo_drag.and_then(|d| d.as_rotate()),
            );
        }

        if scene.active_mode != AppMode::Pixel && !*render_preview {
            let selected_bounds = scene.selected_node.and_then(|id| {
                let playback_bits = scene
                    .character_rig
                    .as_ref()
                    .map(|r| r.playback_time.to_bits())
                    .unwrap_or(0);
                let cache_key = (id, scene.revision, playback_bits);
                let cache_id = egui::Id::new("tessel_selected_bounds_cache");
                if let Some((cached_key, cached_bounds)) = ui.ctx().data(|d| {
                    d.get_temp::<((NodeId, u64, u32), Option<(glam::Vec3, glam::Vec3)>)>(cache_id)
                }) {
                    if cached_key == cache_key {
                        return cached_bounds;
                    }
                }
                let computed = selected_node_world_bounds(scene, id);
                ui.ctx()
                    .data_mut(|d| d.insert_temp(cache_id, (cache_key, computed)));
                computed
            });
            let selected_name = scene
                .selected_node
                .and_then(|id| scene.nodes.get(&id).map(|n| n.name.as_str()));
            draw_selection_brackets(ui, rect, selected_bounds, selected_name, camera_3d);
            if show_rotate_ui {
                draw_floating_rotate_disc(
                    ui,
                    rect,
                    &response,
                    scene,
                    camera_3d,
                    gizmo_drag.and_then(|d| d.as_rotate()),
                    *rotate_axis_mode,
                );
            } else if show_scale_ui {
                draw_scale_gizmo_overlay(
                    ui,
                    rect,
                    scene,
                    camera_3d,
                    active_gizmo_axis,
                    *gizmo_drag,
                    selected_bounds,
                );
            }
            draw_scene_gadgets(
                ui,
                rect,
                &response,
                display_light,
                camera_3d,
                showcase_camera,
                *scene_gadget_drag,
                *gadget_visibility,
            );
        }

        show_viewport_hud(
            ui,
            rect,
            scene,
            camera_3d,
            camera_3d_target,
            showcase_camera,
            camera_2d,
            interaction_tool,
            gizmo_mode,
            show_viewport_gizmo,
            free_view,
            render_preview,
            gadget_visibility,
        );
        if scene.active_mode != AppMode::Pixel && !*render_preview {
            draw_camera_compass(ui, rect, camera_3d);
        }
    });

    (
        desired_size,
        hover_voxel,
        active_gizmo_axis,
        placement_ghost,
    )
}

/// Raycasts a world-space ray against visible scene meshes/voxels and falls back to the `y = 0` ground plane.
pub fn raycast_placement_point(
    scene: &SceneState,
    orig: glam::Vec3,
    dir: glam::Vec3,
) -> Option<glam::Vec3> {
    let dir = dir.normalize_or_zero();
    if dir.length_squared() < 1e-8 {
        return None;
    }
    let mut best: Option<(f32, glam::Vec3)> = None;

    for (&nid, mesh) in &scene.lowpoly_meshes {
        if !scene.nodes.get(&nid).is_some_and(|n| n.visible) || mesh.faces.is_empty() {
            continue;
        }
        let mat = scene.node_world_matrix(nid);
        if mat.determinant().abs() <= 1e-8 {
            continue;
        }
        let inv = mat.inverse();
        let l_orig = inv.transform_point3(orig);
        let l_dir = inv.transform_vector3(dir).normalize_or_zero();
        if let Some(h) = tessel_core::mesh::ops::raycast_mesh_hit(mesh, l_orig, l_dir) {
            let world_hit = mat.transform_point3(h.point);
            let dist_sq = (world_hit - orig).length_squared();
            if best.is_none_or(|(d, _)| dist_sq < d) {
                best = Some((dist_sq, world_hit));
            }
        }
    }

    for (&nid, vol) in &scene.voxel_volumes {
        if !scene.nodes.get(&nid).is_some_and(|n| n.visible) || vol.is_empty() {
            continue;
        }
        let mat = scene.node_world_matrix(nid);
        if mat.determinant().abs() <= 1e-8 {
            continue;
        }
        let inv = mat.inverse();
        let l_orig = inv.transform_point3(orig);
        let l_dir = inv.transform_vector3(dir).normalize_or_zero();
        if let Some(hit) = voxel_ops::raycast_voxels(vol, l_orig, l_dir, scene.unit_scale, 256) {
            let s = scene.unit_scale;
            let place_c = hit.adjacent();
            let local_pt = glam::Vec3::new(
                (place_c.x as f32 + 0.5) * s,
                place_c.y as f32 * s,
                (place_c.z as f32 + 0.5) * s,
            );
            let world_hit = mat.transform_point3(local_pt);
            let dist_sq = (world_hit - orig).length_squared();
            if best.is_none_or(|(d, _)| dist_sq < d) {
                best = Some((dist_sq, world_hit));
            }
        }
    }

    if let Some((_, pt)) = best {
        return Some(pt);
    }

    // Fallback: ground plane y = 0.
    if dir.y.abs() > 1e-5 {
        let t = -orig.y / dir.y;
        if t > 0.0 && t.is_finite() {
            let p = orig + dir * t;
            if p.x.abs() <= 64.0 && p.z.abs() <= 64.0 {
                return Some(glam::Vec3::new(p.x, 0.0, p.z));
            }
        }
    }
    None
}

/// Picks the closest visible `LowPolyMesh` or `VoxelVolume` node intersected by `(orig, dir)`.
pub fn pick_scene_node(
    scene: &SceneState,
    orig: glam::Vec3,
    dir: glam::Vec3,
) -> Option<(NodeId, Option<tessel_core::FaceId>)> {
    let dir = dir.normalize_or_zero();
    if dir.length_squared() < 1e-8 {
        return None;
    }
    let mut best: Option<(f32, NodeId, Option<tessel_core::FaceId>)> = None;

    for (&nid, mesh) in &scene.lowpoly_meshes {
        if !scene.nodes.get(&nid).is_some_and(|n| n.visible) || mesh.faces.is_empty() {
            continue;
        }
        let mat = scene.node_world_matrix(nid);
        if mat.determinant().abs() <= 1e-8 {
            continue;
        }
        let inv = mat.inverse();
        let l_orig = inv.transform_point3(orig);
        let l_dir = inv.transform_vector3(dir).normalize_or_zero();
        if let Some(h) = tessel_core::mesh::ops::raycast_mesh_hit(mesh, l_orig, l_dir) {
            let world_hit = mat.transform_point3(h.point);
            let dist_sq = (world_hit - orig).length_squared();
            if best.is_none_or(|(d, _, _)| dist_sq < d) {
                best = Some((dist_sq, nid, Some(h.face_id)));
            }
        }
    }

    for (&nid, vol) in &scene.voxel_volumes {
        if !scene.nodes.get(&nid).is_some_and(|n| n.visible) || vol.is_empty() {
            continue;
        }
        let mat = scene.node_world_matrix(nid);
        if mat.determinant().abs() <= 1e-8 {
            continue;
        }
        let inv = mat.inverse();
        let l_orig = inv.transform_point3(orig);
        let l_dir = inv.transform_vector3(dir).normalize_or_zero();
        if let Some(hit) = voxel_ops::raycast_voxels(vol, l_orig, l_dir, scene.unit_scale, 256) {
            let s = scene.unit_scale;
            let local_pt = glam::Vec3::new(
                (hit.voxel.x as f32 + 0.5) * s,
                (hit.voxel.y as f32 + 0.5) * s,
                (hit.voxel.z as f32 + 0.5) * s,
            );
            let world_hit = mat.transform_point3(local_pt);
            let dist_sq = (world_hit - orig).length_squared();
            if best.is_none_or(|(d, _, _)| dist_sq < d) {
                best = Some((dist_sq, nid, None));
            }
        }
    }

    best.map(|(_, nid, fid)| (nid, fid))
}

fn place_primitive_at(
    ctx: &egui::Context,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    kind: tessel_core::PrimitiveKind,
    world_pos: glam::Vec3,
) {
    super::exec_history(
        ctx,
        history,
        scene,
        Command::CreatePrimitiveMesh {
            name: kind.label().into(),
            kind,
            size: 0.8,
            material: scene.active_color,
        },
    );
    if let Some(node_id) = scene.selected_node {
        if world_pos.length_squared() > 1e-8 {
            let mut tx = scene
                .nodes
                .get(&node_id)
                .map(|n| n.transform)
                .unwrap_or_default();
            tx.translation = world_pos;
            super::exec_history(
                ctx,
                history,
                scene,
                Command::SetNodeTransform {
                    node_id,
                    transform: tx,
                },
            );
        }
    }
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

const SCALE_HANDLE_HIT_RADIUS_PX: f32 = 12.0;

fn pick_scale_handle_screen(
    pos: egui::Pos2,
    world_origin: glam::Vec3,
    world_rot: glam::Quat,
    camera: &OrbitCamera,
    rect: egui::Rect,
) -> Option<GizmoAxis> {
    let mut best: Option<(GizmoAxis, f32)> = None;
    for axis in GizmoAxis::ALL {
        let tip_world = world_origin + (world_rot * axis.dir()) * GIZMO_AXIS_LEN;
        if let Some(tip_px) = project_world_point(tip_world, camera, rect) {
            let d = pos.distance(tip_px);
            if d <= SCALE_HANDLE_HIT_RADIUS_PX && best.is_none_or(|(_, bd)| d < bd) {
                best = Some((axis, d));
            }
        }
    }
    best.map(|(a, _)| a)
}

fn pick_oriented_scale_shaft(
    ray_origin: glam::Vec3,
    ray_dir: glam::Vec3,
    world_origin: glam::Vec3,
    world_rot: glam::Quat,
    hit_tolerance: f32,
) -> Option<GizmoAxis> {
    let mut best: Option<(GizmoAxis, f32)> = None;
    for axis in GizmoAxis::ALL {
        let dir = (world_rot * axis.dir()).normalize_or_zero();
        if let Some((t, dist)) = closest_point_on_axis(ray_origin, ray_dir, world_origin, dir) {
            if (0.08..=GIZMO_AXIS_LEN + 0.14).contains(&t)
                && dist <= hit_tolerance
                && best.is_none_or(|(_, bd)| dist < bd)
            {
                best = Some((axis, dist));
            }
        }
    }
    best.map(|(a, _)| a)
}

/// Computes a stable 2D screen-space drag direction and normalization length (in logical pixels)
/// for scaling along `world_axis_dir`, with a clean horizontal fallback when viewing axially.
fn scale_screen_drag_basis(
    world_origin: glam::Vec3,
    world_axis_dir: glam::Vec3,
    camera: &OrbitCamera,
    rect: egui::Rect,
) -> (egui::Vec2, f32) {
    if let (Some(o_px), Some(t_px)) = (
        project_world_point(world_origin, camera, rect),
        project_world_point(world_origin + world_axis_dir * GIZMO_AXIS_LEN, camera, rect),
    ) {
        let diff = t_px - o_px;
        let len = diff.length();
        if len > 6.0 && len.is_finite() {
            return (diff / len, len.max(48.0));
        }
    }
    (egui::vec2(1.0, 0.0), 96.0)
}

/// Renders crisp screen-space X/Y/Z square handles at the tips of the node's local scale axes
/// plus a live `W × H × D` and `Scale` badge below the selected object.
#[allow(clippy::too_many_arguments)]
fn draw_scale_gizmo_overlay(
    ui: &egui::Ui,
    rect: egui::Rect,
    scene: &SceneState,
    camera: &OrbitCamera,
    active_axis: Option<GizmoAxis>,
    gizmo_drag: Option<GizmoDragState>,
    selected_bounds: Option<(glam::Vec3, glam::Vec3)>,
) {
    if rect.width() <= 1.0 || rect.height() <= 1.0 {
        return;
    }
    let Some(node_id) = scene.selected_node else {
        return;
    };
    if !node_is_effectively_visible(scene, node_id) {
        return;
    }
    let Some(node) = scene.nodes.get(&node_id) else {
        return;
    };
    let world_origin = scene
        .node_world_matrix(node_id)
        .transform_point3(glam::Vec3::ZERO);
    let world_rot = node_world_rotation(scene, node_id);
    let painter = ui.painter().with_clip_rect(rect);
    let highlight = egui::Color32::from_rgb(255, 235, 102);

    let axes = [
        (GizmoAxis::X, "X", egui::Color32::from_rgb(240, 82, 82)),
        (GizmoAxis::Y, "Y", egui::Color32::from_rgb(92, 220, 112)),
        (GizmoAxis::Z, "Z", egui::Color32::from_rgb(86, 152, 245)),
    ];
    for (axis, label, base_col) in axes {
        let tip_world = world_origin + (world_rot * axis.dir()) * GIZMO_AXIS_LEN;
        if let Some(tip_px) = project_world_point(tip_world, camera, rect) {
            let is_active = active_axis == Some(axis);
            let fill = if is_active { highlight } else { base_col };
            let box_size = if is_active { 12.0 } else { 10.0 };
            let handle_rect = egui::Rect::from_center_size(tip_px, egui::vec2(box_size, box_size));
            painter.rect_filled(handle_rect, CornerRadius::same(2), fill);
            painter.rect_stroke(
                handle_rect,
                CornerRadius::same(2),
                Stroke::new(1.2_f32, egui::Color32::from_rgb(20, 23, 29)),
                egui::StrokeKind::Outside,
            );
            painter.text(
                tip_px + egui::vec2(0.0, -11.0),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::monospace(9.5),
                fill,
            );
        }
    }

    // Live sizing badge below the selected node's projected bounds (or origin).
    let anchor = selected_bounds
        .and_then(|(wmin, wmax)| {
            let size = (wmax - wmin).abs();
            project_bounds_to_screen_rect(wmin, wmax, camera, rect)
                .map(|sel| (egui::pos2(sel.center().x, sel.bottom() + 14.0), Some(size)))
        })
        .or_else(|| {
            project_world_point(world_origin, camera, rect)
                .map(|c| (egui::pos2(c.x, c.y + 28.0), None))
        });

    if let Some((raw_pos, world_size)) = anchor {
        let uniform = matches!(gizmo_drag, Some(GizmoDragState::Scale { .. }))
            && ui.ctx().input(|i| i.modifiers.shift);
        let s = node.transform.scale;
        let suffix = if uniform { " [Uniform]" } else { "" };
        let text = match world_size {
            Some(sz) => format!(
                "Size {:.2}×{:.2}×{:.2}m  Scale {:.2}, {:.2}, {:.2}{suffix}",
                sz.x, sz.y, sz.z, s.x, s.y, s.z
            ),
            None => format!("Scale {:.2}, {:.2}, {:.2}{suffix}", s.x, s.y, s.z),
        };
        let galley =
            painter.layout_no_wrap(text, egui::FontId::monospace(10.0), egui::Color32::WHITE);
        let pill_w = galley.size().x + 12.0;
        let pill_h = galley.size().y + 6.0;
        let half_x = (pill_w * 0.5).min(rect.width() * 0.5);
        let half_y = (pill_h * 0.5).min(rect.height() * 0.5);
        let badge_pos = egui::pos2(
            raw_pos.x.clamp(rect.left() + half_x, rect.right() - half_x),
            raw_pos.y.clamp(rect.top() + half_y, rect.bottom() - half_y),
        );
        let pill_rect = egui::Rect::from_center_size(badge_pos, egui::vec2(pill_w, pill_h));
        painter.rect_filled(
            pill_rect,
            CornerRadius::same(4),
            egui::Color32::from_rgba_unmultiplied(0x14, 0x17, 0x1D, 220),
        );
        painter.rect_stroke(
            pill_rect,
            CornerRadius::same(4),
            Stroke::new(1.0_f32, tokens::OUTLINE_VARIANT),
            egui::StrokeKind::Inside,
        );
        painter.galley(
            pill_rect.center() - galley.size() * 0.5,
            galley,
            egui::Color32::WHITE,
        );
    }
}

/// Computes the updated local `Transform` after dragging a Scale gizmo handle by normalized
/// screen distance `delta_t`. Preserves mirrored/negative scale signs and preserves exact aspect
/// ratio under `uniform` (`Shift`) clamping within `0.005..=100.0`.
pub fn compute_scale_gizmo_transform(
    start_transform: Transform,
    axis: GizmoAxis,
    delta_t: f32,
    uniform: bool,
) -> Transform {
    if !delta_t.is_finite() {
        return start_transform;
    }
    let scale_axis = |v: f32, factor: f32| -> f32 {
        let sign = if v < 0.0 { -1.0 } else { 1.0 };
        sign * (v.abs() * factor).clamp(0.005, 100.0)
    };
    let raw_factor = (1.0 + delta_t).clamp(0.05, 20.0);
    let mut out = start_transform;
    if uniform {
        let mags = [
            start_transform.scale.x.abs().max(1e-6),
            start_transform.scale.y.abs().max(1e-6),
            start_transform.scale.z.abs().max(1e-6),
        ];
        let min_f = mags.iter().map(|&m| 0.005 / m).fold(0.05_f32, f32::max);
        let max_f = mags.iter().map(|&m| 100.0 / m).fold(20.0_f32, f32::min);
        let eff_factor = if min_f <= max_f {
            raw_factor.clamp(min_f, max_f)
        } else {
            1.0
        };
        out.scale = glam::Vec3::new(
            scale_axis(start_transform.scale.x, eff_factor),
            scale_axis(start_transform.scale.y, eff_factor),
            scale_axis(start_transform.scale.z, eff_factor),
        );
    } else {
        match axis {
            GizmoAxis::X => out.scale.x = scale_axis(start_transform.scale.x, raw_factor),
            GizmoAxis::Y => out.scale.y = scale_axis(start_transform.scale.y, raw_factor),
            GizmoAxis::Z => out.scale.z = scale_axis(start_transform.scale.z, raw_factor),
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn handle_gizmo_drag_input(
    rect: egui::Rect,
    response: &egui::Response,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    camera: &OrbitCamera,
    gizmo_drag: &mut Option<GizmoDragState>,
    gizmo_mode: TransformGizmoMode,
    rotate_axis_mode: &mut RotateAxisMode,
    active_axis: &mut Option<GizmoAxis>,
) -> bool {
    let Some(node_id) = scene.selected_node else {
        cancel_gizmo_drag(scene, gizmo_drag);
        *active_axis = None;
        return false;
    };
    if gizmo_drag.is_some_and(|d| d.node_id() != node_id || d.mode() != gizmo_mode) {
        cancel_gizmo_drag(scene, gizmo_drag);
        *active_axis = None;
    }
    let Some(node) = scene.nodes.get(&node_id) else {
        cancel_gizmo_drag(scene, gizmo_drag);
        *active_axis = None;
        return false;
    };
    if !node_is_effectively_visible(scene, node_id) {
        cancel_gizmo_drag(scene, gizmo_drag);
        *active_axis = None;
        return false;
    }
    let node_transform = node.transform;
    let parent_id = node.parent;
    let world_origin = scene
        .node_world_matrix(node_id)
        .transform_point3(glam::Vec3::ZERO);
    let node_world_rot = node_world_rotation(scene, node_id);

    // Allow unmodified X/Y/Z key press to toggle axis lock cleanly in Rotate mode.
    let axis_key_pressed = if gizmo_mode == TransformGizmoMode::Rotate {
        pressed_rotate_axis_key(&response.ctx)
    } else {
        None
    };
    if gizmo_drag.is_none() {
        if let Some(pressed_axis) = axis_key_pressed {
            *rotate_axis_mode = if *rotate_axis_mode == pressed_axis {
                RotateAxisMode::Free
            } else {
                pressed_axis
            };
        }
    }

    let pointer_ray = |pos: egui::Pos2| -> (glam::Vec3, glam::Vec3) {
        let uv = [
            (pos.x - rect.min.x) / rect.width().max(1.0),
            (pos.y - rect.min.y) / rect.height().max(1.0),
        ];
        let aspect = rect.width() / rect.height().max(1.0);
        camera.screen_ray(uv, aspect)
    };

    if gizmo_drag.is_none() {
        *active_axis = match gizmo_mode {
            TransformGizmoMode::Move => response.hover_pos().and_then(|hover| {
                let (orig, dir) = pointer_ray(hover);
                pick_move_gizmo(orig, dir, world_origin, GIZMO_AXIS_LEN, 0.14)
            }),
            TransformGizmoMode::Scale => response.hover_pos().and_then(|hover| {
                pick_scale_handle_screen(hover, world_origin, node_world_rot, camera, rect).or_else(
                    || {
                        let (orig, dir) = pointer_ray(hover);
                        pick_oriented_scale_shaft(orig, dir, world_origin, node_world_rot, 0.14)
                    },
                )
            }),
            TransformGizmoMode::Rotate => None,
        };
    }

    let alt_down = response.ctx.input(|i| i.modifiers.alt);

    // Plain click on any active gizmo handle without dragging is consumed by the gizmo
    // so it does not leak through to object / face / bone picking behind it.
    if gizmo_drag.is_none() && !alt_down && response.clicked_by(egui::PointerButton::Primary) {
        if let Some(pos) = response.interact_pointer_pos() {
            let hit = match gizmo_mode {
                TransformGizmoMode::Rotate => project_world_point(world_origin, camera, rect)
                    .is_some_and(|c| pos.distance(c) <= ROTATE_DISC_RADIUS_PX),
                TransformGizmoMode::Scale => {
                    pick_scale_handle_screen(pos, world_origin, node_world_rot, camera, rect)
                        .is_some()
                        || {
                            let (orig, dir) = pointer_ray(pos);
                            pick_oriented_scale_shaft(orig, dir, world_origin, node_world_rot, 0.16)
                                .is_some()
                        }
                }
                TransformGizmoMode::Move => {
                    let (orig, dir) = pointer_ray(pos);
                    pick_move_gizmo(orig, dir, world_origin, GIZMO_AXIS_LEN, 0.16).is_some()
                }
            };
            if hit {
                return true;
            }
        }
    }

    if !alt_down && response.drag_started_by(egui::PointerButton::Primary) {
        let start_pos = response
            .ctx
            .input(|i| i.pointer.press_origin())
            .or_else(|| response.interact_pointer_pos());
        if let Some(pos) = start_pos {
            match gizmo_mode {
                TransformGizmoMode::Move => {
                    let (orig, dir) = pointer_ray(pos);
                    if let Some(axis) =
                        pick_move_gizmo(orig, dir, world_origin, GIZMO_AXIS_LEN, 0.16)
                    {
                        let parent_mat = parent_id
                            .map(|pid| scene.node_world_matrix(pid))
                            .unwrap_or(glam::Mat4::IDENTITY);
                        let parent_det = parent_mat.determinant();
                        let parent_world_inv = if parent_det.is_finite() && parent_det.abs() > 1e-8
                        {
                            parent_mat.inverse()
                        } else {
                            glam::Mat4::IDENTITY
                        };
                        if let Some((start_t, _)) =
                            closest_point_on_axis(orig, dir, world_origin, axis.dir())
                        {
                            *gizmo_drag = Some(GizmoDragState::Move {
                                node_id,
                                axis,
                                start_transform: node_transform,
                                start_world_pos: world_origin,
                                parent_world_inv,
                                start_axis_t: start_t,
                            });
                            *active_axis = Some(axis);
                            return true;
                        }
                    }
                }
                TransformGizmoMode::Rotate => {
                    if let Some(disc_center) = project_world_point(world_origin, camera, rect) {
                        if pos.distance(disc_center) <= ROTATE_DISC_RADIUS_PX {
                            let cur_pos = response.interact_pointer_pos().unwrap_or(pos);
                            let snap = response.ctx.input(|i| i.modifiers.shift);
                            let parent_world_rot = parent_world_rotation(scene, parent_id);
                            *gizmo_drag = Some(GizmoDragState::Rotate(RotateDragState {
                                node_id,
                                initial_transform: node_transform,
                                anchor_transform: node_transform,
                                start_world_pos: world_origin,
                                parent_world_rot,
                                anchor_pointer_pos: pos,
                                current_pointer_pos: cur_pos,
                                rotate_axis: *rotate_axis_mode,
                                current_delta_deg: 0.0,
                                snap_active: snap,
                            }));
                            *active_axis = None;
                            return true;
                        }
                    }
                }
                TransformGizmoMode::Scale => {
                    let (orig, dir) = pointer_ray(pos);
                    if let Some(axis) =
                        pick_scale_handle_screen(pos, world_origin, node_world_rot, camera, rect)
                            .or_else(|| {
                                pick_oriented_scale_shaft(
                                    orig,
                                    dir,
                                    world_origin,
                                    node_world_rot,
                                    0.16,
                                )
                            })
                    {
                        let axis_dir = (node_world_rot * axis.dir()).normalize_or_zero();
                        let (drag_dir_2d, drag_norm_px) =
                            scale_screen_drag_basis(world_origin, axis_dir, camera, rect);
                        *gizmo_drag = Some(GizmoDragState::Scale {
                            node_id,
                            axis,
                            start_transform: node_transform,
                            start_pointer_pos: pos,
                            drag_dir_2d,
                            drag_norm_px,
                        });
                        *active_axis = Some(axis);
                        return true;
                    }
                }
            }
        }
    }

    if let Some(mut drag) = *gizmo_drag {
        if response.ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            cancel_gizmo_drag(scene, gizmo_drag);
            *active_axis = None;
            return true;
        }
        if response.dragged_by(egui::PointerButton::Primary) {
            if let Some(pos) = response.interact_pointer_pos() {
                match &mut drag {
                    GizmoDragState::Move {
                        node_id,
                        axis,
                        start_transform,
                        start_world_pos,
                        parent_world_inv,
                        start_axis_t,
                    } => {
                        let (orig, dir) = pointer_ray(pos);
                        if let Some((cur_t, _)) =
                            closest_point_on_axis(orig, dir, *start_world_pos, axis.dir())
                        {
                            let world_delta = axis.dir() * (cur_t - *start_axis_t);
                            let local_delta = parent_world_inv.transform_vector3(world_delta);
                            let mut preview_tx = *start_transform;
                            preview_tx.translation = start_transform.translation + local_delta;
                            scene.preview_node_transform(*node_id, preview_tx);
                        }
                    }
                    GizmoDragState::Rotate(r) => {
                        // Re-anchor cleanly on mid-drag X/Y/Z key press so there is zero jump
                        // and releasing the key never reverts the axis.
                        if let Some(pressed_axis) = axis_key_pressed {
                            let next_axis = if r.rotate_axis == pressed_axis {
                                RotateAxisMode::Free
                            } else {
                                pressed_axis
                            };
                            if let Some(cur_tx) = scene.nodes.get(&r.node_id).map(|n| n.transform) {
                                r.anchor_transform = cur_tx;
                            }
                            r.anchor_pointer_pos = pos;
                            r.rotate_axis = next_axis;
                            *rotate_axis_mode = next_axis;
                        }
                        let snap = response.ctx.input(|i| i.modifiers.shift);
                        let delta_px = pos - r.anchor_pointer_pos;
                        let (preview_tx, delta_deg) = compute_floating_rotate_transform(
                            r.anchor_transform,
                            r.parent_world_rot,
                            camera,
                            r.start_world_pos,
                            delta_px,
                            r.rotate_axis,
                            snap,
                        );
                        r.current_pointer_pos = pos;
                        r.current_delta_deg = delta_deg;
                        r.snap_active = snap;
                        scene.preview_node_transform(r.node_id, preview_tx);
                    }
                    GizmoDragState::Scale {
                        node_id,
                        axis,
                        start_transform,
                        start_pointer_pos,
                        drag_dir_2d,
                        drag_norm_px,
                    } => {
                        let delta_t = (pos - *start_pointer_pos).dot(*drag_dir_2d) / *drag_norm_px;
                        let uniform = response.ctx.input(|i| i.modifiers.shift);
                        let preview_tx = compute_scale_gizmo_transform(
                            *start_transform,
                            *axis,
                            delta_t,
                            uniform,
                        );
                        scene.preview_node_transform(*node_id, preview_tx);
                    }
                }
                *gizmo_drag = Some(drag);
            }
            return true;
        }
        if response.drag_stopped_by(egui::PointerButton::Primary) {
            let drag_node_id = drag.node_id();
            let initial_tx = drag.initial_transform();
            if let Some(final_tx) = scene.nodes.get(&drag_node_id).map(|n| n.transform) {
                scene.preview_node_transform(drag_node_id, initial_tx);
                if transform_differs(final_tx, initial_tx) {
                    super::exec_history(
                        &response.ctx,
                        history,
                        scene,
                        Command::SetNodeTransform {
                            node_id: drag_node_id,
                            transform: final_tx,
                        },
                    );
                }
            }
            *gizmo_drag = None;
            *active_axis = None;
            return true;
        }
        // Drag was interrupted without a normal stop (e.g. focus loss or modal); restore state.
        cancel_gizmo_drag(scene, gizmo_drag);
        *active_axis = None;
    }

    false
}

#[allow(clippy::too_many_arguments)]
fn show_viewport_hud(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    scene: &mut SceneState,
    camera_3d: &mut OrbitCamera,
    camera_3d_target: &mut Option<OrbitCamera>,
    showcase_camera: &OrbitCamera,
    camera_2d: &mut PixelCamera2D,
    interaction_tool: &mut ViewportInteractionTool,
    gizmo_mode: &mut TransformGizmoMode,
    show_viewport_gizmo: &mut bool,
    free_view: &mut bool,
    render_preview: &mut bool,
    gadget_visibility: &mut ViewportGadgetVisibility,
) {
    let mode = scene.active_mode;
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
                                AppMode::Pixel => camera_2d
                                    .fit_active_canvas([rect.width(), rect.height()], scene),
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
                                "Move Gizmo: select objects & drag X/Y/Z arrows to translate",
                                is_transform_gizmo_enabled(
                                    scene,
                                    *interaction_tool,
                                    *free_view,
                                    *render_preview,
                                    *show_viewport_gizmo,
                                ) && *gizmo_mode == TransformGizmoMode::Move,
                            ) {
                                activate_transform_gizmo(
                                    scene,
                                    show_viewport_gizmo,
                                    free_view,
                                    render_preview,
                                    interaction_tool,
                                );
                                *gizmo_mode = TransformGizmoMode::Move;
                            }
                            if icons::hud_icon_button(
                                ui,
                                ToolIcon::RotateButton,
                                "Rotate Gizmo: drag floating disc (Free / X / Y / Z, Shift snap 15°)",
                                is_transform_gizmo_enabled(
                                    scene,
                                    *interaction_tool,
                                    *free_view,
                                    *render_preview,
                                    *show_viewport_gizmo,
                                ) && *gizmo_mode == TransformGizmoMode::Rotate,
                            ) {
                                activate_transform_gizmo(
                                    scene,
                                    show_viewport_gizmo,
                                    free_view,
                                    render_preview,
                                    interaction_tool,
                                );
                                *gizmo_mode = TransformGizmoMode::Rotate;
                            }
                            if icons::hud_icon_button(
                                ui,
                                ToolIcon::ScaleButton,
                                "Scale Gizmo: drag X/Y/Z handles to resize object (Shift for uniform)",
                                is_transform_gizmo_enabled(
                                    scene,
                                    *interaction_tool,
                                    *free_view,
                                    *render_preview,
                                    *show_viewport_gizmo,
                                ) && *gizmo_mode == TransformGizmoMode::Scale,
                            ) {
                                activate_transform_gizmo(
                                    scene,
                                    show_viewport_gizmo,
                                    free_view,
                                    render_preview,
                                    interaction_tool,
                                );
                                *gizmo_mode = TransformGizmoMode::Scale;
                            }
                            if let ViewportInteractionTool::PlacePrimitive(kind) =
                                *interaction_tool
                            {
                                if ui
                                    .button(
                                        egui::RichText::new(format!(
                                            "Placing {} (Esc)",
                                            kind.label()
                                        ))
                                        .size(10.5)
                                        .color(tokens::PRIMARY),
                                    )
                                    .on_hover_text(
                                        "Click viewport to place (Shift+click to repeat, Esc to cancel)",
                                    )
                                    .clicked()
                                {
                                    *interaction_tool = ViewportInteractionTool::Select;
                                }
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
                            ui.separator();
                            let cam_tip = if gadget_visibility.camera {
                                "Hide Camera Gizmo"
                            } else {
                                "Show Camera Gizmo"
                            };
                            if icons::hud_icon_button(
                                ui,
                                ToolIcon::CameraGizmo,
                                cam_tip,
                                gadget_visibility.camera,
                            ) {
                                gadget_visibility.camera = !gadget_visibility.camera;
                            }
                            let sun_tip = if gadget_visibility.sun {
                                "Hide Sun Gizmo"
                            } else {
                                "Show Sun Gizmo"
                            };
                            if icons::hud_icon_button(
                                ui,
                                ToolIcon::Sun,
                                sun_tip,
                                gadget_visibility.sun,
                            ) {
                                gadget_visibility.sun = !gadget_visibility.sun;
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
                        super::exec_history(
                            ui.ctx(),
                            history,
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
            scene.set_active_color(sampled);
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
            let already_same = scene
                .pixel_canvases
                .get(&node_id)
                .and_then(|c| c.frame(FrameIndex(active_frame)))
                .and_then(|f| f.layer(layer_id))
                .is_some_and(|layer| layer.get(coord) == Some(scene.active_color));
            if !already_same {
                super::exec_history(
                    ui.ctx(),
                    history,
                    scene,
                    Command::FloodFillPixels {
                        node_id,
                        frame: FrameIndex(active_frame),
                        layer_id,
                        start: coord,
                        color: Some(scene.active_color),
                    },
                );
            }
            return;
        }
        PixelTool::Eyedropper => return,
    };

    if !edits.is_empty() {
        super::exec_history(
            ui.ctx(),
            history,
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
                        let max_t = if camera.orthographic {
                            camera.far * 1.5
                        } else {
                            80.0
                        };
                        if t > 0.0 && t < max_t {
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
        super::exec_history(
            ui.ctx(),
            history,
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
        super::exec_history(
            ui.ctx(),
            history,
            scene,
            Command::SetVoxels { node_id, edits },
        );
    }
}

/// Computes the world-space AABB `(min, max)` of `node_id` for selection brackets.
pub fn selected_node_world_bounds(
    scene: &SceneState,
    node_id: NodeId,
) -> Option<(glam::Vec3, glam::Vec3)> {
    let node = scene.nodes.get(&node_id)?;
    let visible = scene
        .character_rig
        .as_ref()
        .and_then(|r| r.node_visibility.get(&node_id).copied())
        .unwrap_or(node.visible);
    if !visible {
        return None;
    }
    let base_mat = scene.node_world_matrix(node_id);
    let parent_override = scene
        .character_rig
        .as_ref()
        .and_then(|r| r.node_parent_overrides.get(&node_id).copied());
    let mat = match (parent_override, scene.character_rig.as_ref()) {
        (Some(ps_key), Some(rig)) => tessel_core::CharacterRig::compute_node_override_matrix(
            ps_key,
            base_mat,
            &rig.world_pose_matrices(),
            &rig.world_rest_matrices(),
        ),
        _ => base_mat,
    };
    if let Some(mesh) = scene.lowpoly_meshes.get(&node_id) {
        if !mesh.vertices.is_empty() {
            let rest_mat = scene.node_rest_world_matrix(node_id);
            let skin_ctx = if parent_override.is_none() {
                scene
                    .character_rig
                    .as_ref()
                    .filter(|r| r.should_skin_node(node_id))
                    .and_then(|rig| {
                        let solver = tessel_core::AutoSkinSolver::from_rig(rig, 1.0)?;
                        let mats = solver.skin_matrices(rig);
                        let is_identity = mats
                            .iter()
                            .all(|m| m.abs_diff_eq(glam::Mat4::IDENTITY, 1e-4));
                        (!is_identity).then_some((solver, mats))
                    })
            } else {
                None
            };
            let active_morphs: Vec<(f32, &std::collections::BTreeMap<usize, glam::Vec3>)> = mesh
                .morphs
                .iter()
                .filter_map(|m| {
                    let w = scene
                        .character_rig
                        .as_ref()
                        .and_then(|rig| rig.morph_weights.get(&(node_id, m.name.clone())).copied())
                        .unwrap_or(m.default_weight);
                    (w.abs() > 1e-5).then_some((w, &m.deltas))
                })
                .collect();
            if skin_ctx.is_some() || !active_morphs.is_empty() {
                let mut wmin = glam::Vec3::splat(f32::INFINITY);
                let mut wmax = glam::Vec3::splat(f32::NEG_INFINITY);
                for (vi, &v) in mesh.vertices.iter().enumerate() {
                    let mut lv = v;
                    for &(w, deltas) in &active_morphs {
                        if let Some(&delta) = deltas.get(&vi) {
                            lv += delta * w;
                        }
                    }
                    let mut wp = mat.transform_point3(lv);
                    if let Some((solver, skin_mats)) = &skin_ctx {
                        let w = mesh.skin_weights.get(&vi).copied().unwrap_or_else(|| {
                            let rest_wp = rest_mat.transform_point3(v);
                            solver.weights_for_vertex(Some(node_id), rest_wp)
                        });
                        let (sp, _) = solver.deform_point_normal(skin_mats, w, wp, glam::Vec3::Y);
                        wp = sp;
                    }
                    wmin = wmin.min(wp);
                    wmax = wmax.max(wp);
                }
                if wmin.is_finite() && wmax.is_finite() {
                    return Some((wmin, wmax));
                }
            }
            let (lmin, lmax) = mesh.bounds();
            return Some(transform_aabb(mat, lmin, lmax));
        }
    }
    if let Some(vol) = scene.voxel_volumes.get(&node_id) {
        if !vol.is_empty() {
            let s = scene.unit_scale;
            if parent_override.is_none() {
                if let Some(rig) = scene
                    .character_rig
                    .as_ref()
                    .filter(|r| r.should_skin_node(node_id))
                {
                    if let Some(solver) = tessel_core::AutoSkinSolver::from_rig(rig, 1.0) {
                        let skin_mats = solver.skin_matrices(rig);
                        let is_identity = skin_mats
                            .iter()
                            .all(|m| m.abs_diff_eq(glam::Mat4::IDENTITY, 1e-4));
                        if !is_identity {
                            let rest_mat = scene.node_rest_world_matrix(node_id);
                            let mut wmin = glam::Vec3::splat(f32::INFINITY);
                            let mut wmax = glam::Vec3::splat(f32::NEG_INFINITY);
                            let half = glam::Vec3::splat(s * 0.5);
                            for (coord, _) in vol.iter_voxels() {
                                let base = glam::Vec3::new(
                                    coord.x as f32 * s,
                                    coord.y as f32 * s,
                                    coord.z as f32 * s,
                                );
                                let center_local = base + half;
                                let rest_wp = rest_mat.transform_point3(center_local);
                                let w = solver.weights_for_vertex(Some(node_id), rest_wp);
                                for corner in [base, base + glam::Vec3::splat(s)] {
                                    let wp = mat.transform_point3(corner);
                                    let (sp, _) = solver.deform_point_normal(
                                        &skin_mats,
                                        w,
                                        wp,
                                        glam::Vec3::Y,
                                    );
                                    wmin = wmin.min(sp);
                                    wmax = wmax.max(sp);
                                }
                            }
                            if wmin.is_finite() && wmax.is_finite() {
                                return Some((wmin, wmax));
                            }
                        }
                    }
                }
            }
            let mut min_c = glam::IVec3::splat(i32::MAX);
            let mut max_c = glam::IVec3::splat(i32::MIN);
            for (coord, _) in vol.iter_voxels() {
                min_c = min_c.min(glam::IVec3::new(coord.x, coord.y, coord.z));
                max_c = max_c.max(glam::IVec3::new(coord.x, coord.y, coord.z));
            }
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
    let vp = camera.view_proj(aspect);

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
    selected_bounds: Option<(glam::Vec3, glam::Vec3)>,
    selected_name: Option<&str>,
    camera: &OrbitCamera,
) {
    let Some((wmin, wmax)) = selected_bounds else {
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

    if let Some(name) = selected_name {
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
    let clip = camera.view_proj(aspect) * glam::Vec4::new(pt.x, pt.y, pt.z, 1.0);
    if clip.w <= 1e-4 {
        return None;
    }
    let ndc = clip.truncate() / clip.w;
    let sx = viewport_rect.left() + (ndc.x * 0.5 + 0.5) * w;
    let sy = viewport_rect.top() + (0.5 - ndc.y * 0.5) * h;
    Some(egui::pos2(sx, sy))
}

pub(super) fn screen_basis_at_point(
    camera: &OrbitCamera,
    world_pos: glam::Vec3,
) -> (glam::Vec3, glam::Vec3) {
    let forward = if camera.orthographic {
        (camera.target - camera.position()).normalize_or_zero()
    } else {
        let to_pt = world_pos - camera.position();
        if to_pt.length_squared() > 1e-6 {
            to_pt.normalize()
        } else {
            (camera.target - camera.position()).normalize_or_zero()
        }
    };
    let mut right = forward.cross(glam::Vec3::Y);
    if right.length_squared() < 1e-6 {
        right = glam::Vec3::X;
    } else {
        right = right.normalize();
    }
    let up = right.cross(forward).normalize_or_zero();
    (right, up)
}

/// Solves a damped 2x2 Jacobian step `(d_az, d_el)` so dragging along `delta_px`
/// moves the spherical gadget along the active `view_camera` screen basis at `world_pos`.
fn solve_spherical_screen_drag(
    az: f32,
    el: f32,
    world_pos: glam::Vec3,
    view_camera: &OrbitCamera,
    delta_px: egui::Vec2,
    sensitivity: f32,
) -> (f32, f32) {
    let (right, up) = screen_basis_at_point(view_camera, world_pos);
    let (sin_az, cos_az) = az.sin_cos();
    let (sin_el, cos_el) = el.sin_cos();

    // Unit tangent directions for +azimuth and +elevation on the orbit sphere.
    let t_az = glam::Vec3::new(cos_az, 0.0, -sin_az);
    let t_el = glam::Vec3::new(-sin_el * sin_az, cos_el, -sin_el * cos_az);

    // Screen-space columns (x = right, y = down = -up).
    let j00 = t_az.dot(right);
    let j10 = -t_az.dot(up);
    let j01 = t_el.dot(right);
    let j11 = -t_el.dot(up);

    // Damped least-squares: (J^T J + lambda I)^-1 J^T * b
    let lambda = 0.08_f32;
    let a = j00 * j00 + j10 * j10 + lambda;
    let b = j00 * j01 + j10 * j11;
    let c = j01 * j01 + j11 * j11 + lambda;
    let det = a * c - b * b;

    let rhs_0 = j00 * delta_px.x + j10 * delta_px.y;
    let rhs_1 = j01 * delta_px.x + j11 * delta_px.y;

    let (d_az, d_el) = if det.abs() > 1e-6 {
        (
            (c * rhs_0 - b * rhs_1) / det * sensitivity,
            (-b * rhs_0 + a * rhs_1) / det * sensitivity,
        )
    } else {
        (rhs_0 * sensitivity, rhs_1 * sensitivity)
    };
    (az + d_az, el + d_el)
}

/// Applies pointer drag delta in screen pixels to `(light_azimuth, light_elevation)`
/// relative to the active viewport `camera` screen basis (`right` and `up`).
pub fn apply_sun_drag_delta(
    azimuth: f32,
    elevation: f32,
    delta_px: egui::Vec2,
    camera: &OrbitCamera,
) -> (f32, f32) {
    let sun_world =
        light_world_position(camera.target, azimuth, elevation, sun_gizmo_radius(camera));
    let (next_az, next_el) =
        solve_spherical_screen_drag(azimuth, elevation, sun_world, camera, delta_px, 0.011);
    (
        next_az.rem_euclid(std::f32::consts::TAU),
        next_el.clamp(MIN_SUN_ELEVATION, MAX_SUN_ELEVATION),
    )
}

/// Applies pointer drag delta in screen pixels to `showcase_camera` relative to the
/// active viewport `view_camera` screen basis (`right` and `up`).
pub fn apply_camera_gizmo_drag_delta(
    showcase_camera: &mut OrbitCamera,
    view_camera: &OrbitCamera,
    delta_px: egui::Vec2,
    pan_mode: bool,
) {
    let pos = showcase_camera.position();
    if pan_mode {
        let (right, up) = screen_basis_at_point(view_camera, pos);
        let scale = view_camera.distance * 0.0018;
        showcase_camera.target += (right * delta_px.x - up * delta_px.y) * scale;
    } else {
        let (next_yaw, next_pitch) = solve_spherical_screen_drag(
            showcase_camera.yaw,
            showcase_camera.pitch,
            pos,
            view_camera,
            delta_px,
            0.008,
        );
        showcase_camera.yaw = next_yaw;
        showcase_camera.pitch = next_pitch.clamp(-1.52, 1.52);
    }
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
    visibility: ViewportGadgetVisibility,
) -> bool {
    cancel_hidden_scene_gadget_drag(scene, gadget_drag, visibility);

    let sun_screen = if visibility.sun {
        let sun_world = light_world_position(
            camera_3d.target,
            display_light.0,
            display_light.1,
            sun_gizmo_radius(camera_3d),
        );
        project_world_point(sun_world, camera_3d, rect)
    } else {
        None
    };
    let cam_screen = if visibility.camera {
        project_world_point(showcase_camera.position(), camera_3d, rect)
    } else {
        None
    };

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
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            cancel_scene_gadget_drag(scene, gadget_drag);
            return true;
        }
        match drag {
            SceneGadgetDrag::Sun {
                start_azimuth,
                start_elevation,
            } => {
                if response.dragged_by(egui::PointerButton::Primary) {
                    let delta = response.drag_delta();
                    let (az, el) = apply_sun_drag_delta(
                        scene.light_azimuth,
                        scene.light_elevation,
                        delta,
                        camera_3d,
                    );
                    scene.preview_lighting(az, el);
                    return true;
                }
                if response.drag_stopped_by(egui::PointerButton::Primary) {
                    let final_az = scene.light_azimuth;
                    let final_el = scene.light_elevation;
                    scene.light_azimuth = start_azimuth;
                    scene.light_elevation = start_elevation;
                    super::exec_history(
                        ui.ctx(),
                        history,
                        scene,
                        Command::SetLightDirection {
                            azimuth: final_az,
                            elevation: final_el,
                        },
                    );
                    *gadget_drag = None;
                    return true;
                }
                cancel_scene_gadget_drag(scene, gadget_drag);
            }
            SceneGadgetDrag::Camera => {
                if response.dragged_by(egui::PointerButton::Primary) {
                    *showcase_camera_target = None;
                    let delta = response.drag_delta();
                    let pan_mode = ui.input(|i| i.modifiers.shift);
                    apply_camera_gizmo_drag_delta(showcase_camera, camera_3d, delta, pan_mode);
                    return true;
                }
                if response.drag_stopped_by(egui::PointerButton::Primary) {
                    *gadget_drag = None;
                    return true;
                }
                *gadget_drag = None;
            }
        }
    }

    false
}

fn cancel_hidden_scene_gadget_drag(
    scene: &mut SceneState,
    gadget_drag: &mut Option<SceneGadgetDrag>,
    visibility: ViewportGadgetVisibility,
) {
    match *gadget_drag {
        Some(SceneGadgetDrag::Sun { .. }) if !visibility.sun => {
            cancel_scene_gadget_drag(scene, gadget_drag);
        }
        Some(SceneGadgetDrag::Camera) if !visibility.camera => {
            *gadget_drag = None;
        }
        _ => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_scene_gadgets(
    ui: &egui::Ui,
    viewport_rect: egui::Rect,
    response: &egui::Response,
    display_light: (f32, f32),
    camera_3d: &OrbitCamera,
    showcase_camera: &OrbitCamera,
    active_drag: Option<SceneGadgetDrag>,
    visibility: ViewportGadgetVisibility,
) {
    if !visibility.sun && !visibility.camera {
        return;
    }
    let painter = ui.painter_at(viewport_rect);
    let hover = response.hover_pos();

    // 1. Draggable Sun / Key Light gizmo
    if visibility.sun {
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
            let sun_ray_col = egui::Color32::from_rgba_unmultiplied(
                255,
                206,
                84,
                if sun_active { 170 } else { 95 },
            );
            if let Some(target_pos) =
                project_world_point(camera_3d.target, camera_3d, viewport_rect)
            {
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
    }

    // 2. Draggable Camera / Projector proxy gizmo + frustum pyramid
    if visibility.camera {
        let cam_origin = showcase_camera.position();
        if let Some(cam_pos) = project_world_point(cam_origin, camera_3d, viewport_rect) {
            let cam_active = matches!(active_drag, Some(SceneGadgetDrag::Camera))
                || hover.is_some_and(|h| h.distance(cam_pos) <= GADGET_HIT_RADIUS_PX);
            let cam_cyan = egui::Color32::from_rgb(92, 196, 255);
            let frustum_col = egui::Color32::from_rgba_unmultiplied(
                92,
                196,
                255,
                if cam_active { 200 } else { 120 },
            );

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
        let cam = OrbitCamera::new();
        let (az, el) = apply_sun_drag_delta(0.5, 0.8, egui::vec2(10.0, -500.0), &cam);
        assert!((0.0..std::f32::consts::TAU).contains(&az));
        assert!((el - MAX_SUN_ELEVATION).abs() < 1e-5);

        let (_, el_low) = apply_sun_drag_delta(0.5, 0.4, egui::vec2(0.0, 500.0), &cam);
        assert!((el_low - MIN_SUN_ELEVATION).abs() < 1e-5);
    }

    #[test]
    fn sun_and_camera_gizmo_drag_follow_screen_view_across_camera_angles() {
        let vp = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut cam_front = OrbitCamera::new();
        cam_front.yaw = 0.0;
        cam_front.pitch = 0.25;
        let mut cam_back = OrbitCamera::new();
        cam_back.yaw = std::f32::consts::PI;
        cam_back.pitch = 0.25;
        let mut cam_side = OrbitCamera::new();
        cam_side.yaw = std::f32::consts::FRAC_PI_2;
        cam_side.pitch = 0.25;

        for view_cam in [cam_front, cam_back, cam_side] {
            // Sun drag right (+x) moves projected sun right (+x) on screen; drag down (+y) moves down (+y).
            let az0 = 0.6;
            let el0 = 0.7;
            let r = sun_gizmo_radius(&view_cam);
            let s0 = project_world_point(
                light_world_position(view_cam.target, az0, el0, r),
                &view_cam,
                vp,
            )
            .unwrap();
            let (az_r, el_r) = apply_sun_drag_delta(az0, el0, egui::vec2(12.0, 0.0), &view_cam);
            let sr = project_world_point(
                light_world_position(view_cam.target, az_r, el_r, r),
                &view_cam,
                vp,
            )
            .unwrap();
            assert!(
                sr.x > s0.x,
                "sun drag right should move screen x right for yaw={}",
                view_cam.yaw
            );

            let (az_d, el_d) = apply_sun_drag_delta(az0, el0, egui::vec2(0.0, 12.0), &view_cam);
            let sd = project_world_point(
                light_world_position(view_cam.target, az_d, el_d, r),
                &view_cam,
                vp,
            )
            .unwrap();
            assert!(
                sd.y > s0.y,
                "sun drag down should move screen y down for yaw={}",
                view_cam.yaw
            );

            // Showcase camera drag right/down (both orbit and shift-pan) follows screen cursor.
            let mut sc_orbit = OrbitCamera::new();
            sc_orbit.yaw = -0.5;
            sc_orbit.pitch = 0.4;
            let c0 = project_world_point(sc_orbit.position(), &view_cam, vp).unwrap();
            let mut sc_right = sc_orbit;
            apply_camera_gizmo_drag_delta(&mut sc_right, &view_cam, egui::vec2(14.0, 0.0), false);
            let cr = project_world_point(sc_right.position(), &view_cam, vp).unwrap();
            assert!(
                cr.x > c0.x,
                "camera orbit drag right should move screen x right for yaw={}",
                view_cam.yaw
            );

            let mut sc_down = sc_orbit;
            apply_camera_gizmo_drag_delta(&mut sc_down, &view_cam, egui::vec2(0.0, 14.0), false);
            let cd = project_world_point(sc_down.position(), &view_cam, vp).unwrap();
            assert!(
                cd.y > c0.y,
                "camera orbit drag down should move screen y down for yaw={}",
                view_cam.yaw
            );

            let mut sc_pan = sc_orbit;
            apply_camera_gizmo_drag_delta(&mut sc_pan, &view_cam, egui::vec2(20.0, 10.0), true);
            let cp = project_world_point(sc_pan.position(), &view_cam, vp).unwrap();
            assert!(cp.x > c0.x && cp.y > c0.y);
        }
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

    #[test]
    fn project_world_point_and_bounds_match_orthographic_camera() {
        let mut cam = OrbitCamera::new();
        cam.orthographic = true;
        cam.distance = 8.0;
        let vp = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));

        let center = project_world_point(cam.target, &cam, vp).unwrap();
        assert!((center.x - 400.0).abs() < 2.0);
        assert!((center.y - 300.0).abs() < 2.0);

        // In orthographic projection, a point offset perpendicular to view does not scale with depth.
        let p_near = cam.target + glam::Vec3::new(0.0, 1.0, 0.0);
        let s_ortho = project_world_point(p_near, &cam, vp).unwrap();
        let rect = project_bounds_to_screen_rect(
            cam.target - glam::Vec3::splat(0.5),
            cam.target + glam::Vec3::splat(0.5),
            &cam,
            vp,
        )
        .unwrap();
        assert!(rect.contains(center));
        assert!(s_ortho.y < center.y);
    }

    #[test]
    fn cancel_gizmo_and_sun_drag_restores_initial_values() {
        let mut scene = SceneState::new();
        Command::CreateVoxelVolume { name: "Box".into() }
            .apply(&mut scene)
            .unwrap();
        let node_id = scene.active_voxel_node().unwrap();
        let start_tx = scene.nodes.get(&node_id).unwrap().transform;
        let start_rev = scene.revision;

        // Simulate mid-drag node mutation followed by cancel.
        scene.nodes.get_mut(&node_id).unwrap().transform.translation =
            glam::Vec3::new(3.0, 2.0, -1.0);
        let mut gizmo_drag = Some(GizmoDragState::Move {
            node_id,
            axis: GizmoAxis::X,
            start_transform: start_tx,
            start_world_pos: glam::Vec3::ZERO,
            parent_world_inv: glam::Mat4::IDENTITY,
            start_axis_t: 0.0,
        });
        cancel_gizmo_drag(&mut scene, &mut gizmo_drag);
        assert!(gizmo_drag.is_none());
        assert_eq!(scene.nodes.get(&node_id).unwrap().transform, start_tx);
        assert_eq!(scene.revision, start_rev);
        assert!(scene.dirty_viewport);

        // Simulate mid-drag Sun mutation followed by cancel.
        let orig_az = scene.light_azimuth;
        let orig_el = scene.light_elevation;
        scene.light_azimuth = 2.5;
        scene.light_elevation = 0.3;
        scene.dirty_viewport = false;
        let mut gadget_drag = Some(SceneGadgetDrag::Sun {
            start_azimuth: orig_az,
            start_elevation: orig_el,
        });
        cancel_scene_gadget_drag(&mut scene, &mut gadget_drag);
        assert!(gadget_drag.is_none());
        assert!((scene.light_azimuth - orig_az).abs() < 1e-6);
        assert!((scene.light_elevation - orig_el).abs() < 1e-6);
        assert_eq!(scene.revision, start_rev);
        assert!(scene.dirty_viewport);
    }

    #[test]
    fn hiding_gadget_cancels_only_hidden_drag() {
        let mut scene = SceneState::new();
        let orig_az = scene.light_azimuth;
        let orig_el = scene.light_elevation;

        scene.light_azimuth = 2.0;
        let mut drag = Some(SceneGadgetDrag::Sun {
            start_azimuth: orig_az,
            start_elevation: orig_el,
        });
        cancel_hidden_scene_gadget_drag(
            &mut scene,
            &mut drag,
            ViewportGadgetVisibility {
                camera: false,
                sun: true,
            },
        );
        assert!(drag.is_some());

        cancel_hidden_scene_gadget_drag(
            &mut scene,
            &mut drag,
            ViewportGadgetVisibility {
                camera: true,
                sun: false,
            },
        );
        assert!(drag.is_none());
        assert!((scene.light_azimuth - orig_az).abs() < 1e-6);

        let mut cam_drag = Some(SceneGadgetDrag::Camera);
        cancel_hidden_scene_gadget_drag(
            &mut scene,
            &mut cam_drag,
            ViewportGadgetVisibility {
                camera: false,
                sun: true,
            },
        );
        assert!(cam_drag.is_none());
    }

    #[test]
    fn raycast_placement_hits_ground_and_mesh_surface() {
        let mut scene = SceneState::new();
        // Downward ray hits y = 0 ground plane at (2, 0, -1).
        let pt = raycast_placement_point(
            &scene,
            glam::Vec3::new(2.0, 5.0, -1.0),
            glam::Vec3::new(0.0, -1.0, 0.0),
        )
        .unwrap();
        assert!((pt - glam::Vec3::new(2.0, 0.0, -1.0)).length() < 1e-4);

        // Upward ray misses ground plane.
        assert!(raycast_placement_point(
            &scene,
            glam::Vec3::new(0.0, 2.0, 0.0),
            glam::Vec3::new(0.0, 1.0, 0.0),
        )
        .is_none());

        // Adding a cube at origin causes downward ray at (0, 5, 0) to land on top of cube (y > 0.5).
        Command::CreatePrimitiveMesh {
            name: "Cube".into(),
            kind: tessel_core::PrimitiveKind::Cube,
            size: 1.0,
            material: scene.active_color,
        }
        .apply(&mut scene)
        .unwrap();
        let top_hit = raycast_placement_point(
            &scene,
            glam::Vec3::new(0.0, 5.0, 0.0),
            glam::Vec3::new(0.0, -1.0, 0.0),
        )
        .unwrap();
        assert!(top_hit.y > 0.5, "expected hit on cube top, got {top_hit:?}");
    }

    #[test]
    fn pick_scene_node_selects_voxel_and_mesh_objects() {
        let mut scene = SceneState::new();
        Command::CreateVoxelVolume { name: "Vox".into() }
            .apply(&mut scene)
            .unwrap();
        let vox_id = scene.active_voxel_node().unwrap();
        Command::SetVoxels {
            node_id: vox_id,
            edits: vec![VoxelEdit {
                coord: VoxelCoord::new(0, 0, 0),
                color: Some(scene.active_color),
            }],
        }
        .apply(&mut scene)
        .unwrap();

        let (picked, face) = pick_scene_node(
            &scene,
            glam::Vec3::new(0.05, 2.0, 0.05),
            glam::Vec3::new(0.0, -1.0, 0.0),
        )
        .unwrap();
        assert_eq!(picked, vox_id);
        assert!(face.is_none());
    }

    #[test]
    fn scale_gizmo_transform_handles_axis_uniform_clamp_and_nan() {
        let start = Transform::default();

        // Per-axis X scale by +0.5 -> scale.x = 1.5, Y/Z unchanged.
        let tx_x = compute_scale_gizmo_transform(start, GizmoAxis::X, 0.5, false);
        assert!((tx_x.scale.x - 1.5).abs() < 1e-4);
        assert!((tx_x.scale.y - 1.0).abs() < 1e-4);
        assert!((tx_x.scale.z - 1.0).abs() < 1e-4);

        // Uniform Shift scale by +1.0 -> all axes doubled.
        let tx_u = compute_scale_gizmo_transform(start, GizmoAxis::Y, 1.0, true);
        assert!((tx_u.scale - glam::Vec3::splat(2.0)).length() < 1e-4);

        // Extreme negative drag clamps safely above zero (never flips negative).
        let tx_min = compute_scale_gizmo_transform(start, GizmoAxis::Z, -50.0, false);
        assert!(tx_min.scale.z >= 0.005);

        // Mirrored (negative) scale preserves its sign.
        let mut mirrored = start;
        mirrored.scale = glam::Vec3::new(-2.0, 1.0, 1.0);
        let tx_mir = compute_scale_gizmo_transform(mirrored, GizmoAxis::X, 0.5, false);
        assert!(
            (tx_mir.scale.x + 3.0).abs() < 1e-4,
            "got {:?}",
            tx_mir.scale
        );

        // Uniform clamping preserves exact aspect ratio on non-uniform scales.
        let mut non_uniform = start;
        non_uniform.scale = glam::Vec3::new(10.0, 20.0, 50.0);
        let tx_clamped = compute_scale_gizmo_transform(non_uniform, GizmoAxis::X, 5.0, true);
        assert!((tx_clamped.scale.z - 100.0).abs() < 1e-3);
        assert!((tx_clamped.scale.x / tx_clamped.scale.z - 0.2).abs() < 1e-4);
        assert!((tx_clamped.scale.y / tx_clamped.scale.z - 0.4).abs() < 1e-4);

        // Non-finite delta returns start transform untouched.
        let tx_nan = compute_scale_gizmo_transform(start, GizmoAxis::X, f32::NAN, false);
        assert_eq!(tx_nan, start);
    }

    #[test]
    fn g1_selected_node_world_bounds_skips_skinning_for_unrigged_node() {
        let mut scene = SceneState::new();
        Command::CreatePrimitiveMesh {
            name: "StaticProp".into(),
            kind: tessel_core::PrimitiveKind::Cube,
            size: 1.0,
            material: scene.active_color,
        }
        .apply(&mut scene)
        .unwrap();
        let node_id = scene.selected_node.unwrap();
        let rest_bounds = selected_node_world_bounds(&scene, node_id).unwrap();

        Command::ApplyBipedRig.apply(&mut scene).unwrap();
        let root_id = scene.character_rig.as_ref().unwrap().bones[0].id;
        Command::SetBoneTranslation {
            bone_id: root_id,
            translation: glam::Vec3::new(0.0, 10.0, 0.0),
        }
        .apply(&mut scene)
        .unwrap();

        let skinned_bounds = selected_node_world_bounds(&scene, node_id).unwrap();
        assert!(
            skinned_bounds.0.y > rest_bounds.0.y + 5.0,
            "skinned node bounds should follow root bone translation"
        );

        Command::SetNodeRigSkinned {
            node_id,
            skinned: false,
        }
        .apply(&mut scene)
        .unwrap();
        let unrigged_bounds = selected_node_world_bounds(&scene, node_id).unwrap();
        assert!(
            (unrigged_bounds.0 - rest_bounds.0).length() < 1e-4
                && (unrigged_bounds.1 - rest_bounds.1).length() < 1e-4,
            "unrigged node bounds must stay at rest transform"
        );

        // VoxelVolume bounds must also follow skinning when rigged and stay at rest when unrigged.
        Command::CreateVoxelVolume {
            name: "VoxHero".into(),
        }
        .apply(&mut scene)
        .unwrap();
        let vox_id = scene.active_voxel_node().unwrap();
        Command::SetVoxels {
            node_id: vox_id,
            edits: vec![VoxelEdit {
                coord: VoxelCoord::new(0, 8, 0),
                color: Some(scene.active_color),
            }],
        }
        .apply(&mut scene)
        .unwrap();
        let vox_skinned = selected_node_world_bounds(&scene, vox_id).unwrap();
        assert!(
            vox_skinned.0.y > 5.0,
            "skinned voxel bounds should follow root bone translation"
        );
        Command::SetNodeRigSkinned {
            node_id: vox_id,
            skinned: false,
        }
        .apply(&mut scene)
        .unwrap();
        let vox_unrigged = selected_node_world_bounds(&scene, vox_id).unwrap();
        assert!(
            vox_unrigged.0.y < 1.5,
            "unrigged voxel bounds must stay at rest transform"
        );
    }
}
