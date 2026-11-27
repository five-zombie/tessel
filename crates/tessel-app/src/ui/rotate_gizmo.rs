use egui::{CornerRadius, Frame, Margin, Sense, Stroke};
use tessel_core::{NodeId, SceneState, Transform};
use tessel_render::{GizmoAxis, OrbitCamera};

use super::theme::tokens;
use super::viewport::{project_world_point, screen_basis_at_point};

/// Active axis mode for the screen-space floating circular rotate gizmo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RotateAxisMode {
    #[default]
    Free,
    X,
    Y,
    Z,
}

impl RotateAxisMode {
    pub const ALL: [Self; 4] = [Self::Free, Self::X, Self::Y, Self::Z];

    pub fn label(self) -> &'static str {
        match self {
            Self::Free => "Free",
            Self::X => "X",
            Self::Y => "Y",
            Self::Z => "Z",
        }
    }

    pub fn accent_color(self) -> egui::Color32 {
        match self {
            Self::Free => egui::Color32::from_rgb(235, 240, 248),
            Self::X => egui::Color32::from_rgb(240, 82, 82),
            Self::Y => egui::Color32::from_rgb(92, 220, 112),
            Self::Z => egui::Color32::from_rgb(86, 152, 245),
        }
    }

    pub fn to_gizmo_axis(self) -> Option<GizmoAxis> {
        match self {
            Self::Free => None,
            Self::X => Some(GizmoAxis::X),
            Self::Y => Some(GizmoAxis::Y),
            Self::Z => Some(GizmoAxis::Z),
        }
    }
}

/// Active drag state for the screen-space floating circular rotate gizmo.
#[derive(Debug, Clone, Copy)]
pub struct RotateDragState {
    pub node_id: NodeId,
    /// Original transform at the start of the drag (used for cancel/Undo commit).
    pub initial_transform: Transform,
    /// Anchor transform for the current axis segment (re-anchored cleanly on mid-drag axis switch).
    pub anchor_transform: Transform,
    pub start_world_pos: glam::Vec3,
    pub parent_world_rot: glam::Quat,
    pub anchor_pointer_pos: egui::Pos2,
    pub current_pointer_pos: egui::Pos2,
    pub rotate_axis: RotateAxisMode,
    pub current_delta_deg: f32,
    pub snap_active: bool,
}

pub const ROTATE_DISC_RADIUS_PX: f32 = 58.0;
const ROTATE_DEG_PER_PX: f32 = 0.75;
const ROTATE_SNAP_STEP_DEG: f32 = 15.0;

pub fn safe_quat_normalize(q: glam::Quat) -> glam::Quat {
    let len_sq = q.length_squared();
    if len_sq.is_finite() && len_sq > 1e-8 {
        q.normalize()
    } else {
        glam::Quat::IDENTITY
    }
}

/// Composes the world-space rotation of `parent_id` by walking the scene hierarchy directly.
/// Unlike matrix decomposition (`to_scale_rotation_translation`), this remains exact under
/// negative/reflected scales (`det < 0`) and non-uniform ancestor scale shear.
pub fn parent_world_rotation(scene: &SceneState, parent_id: Option<NodeId>) -> glam::Quat {
    let mut acc = glam::Quat::IDENTITY;
    let mut cur = parent_id;
    let mut depth = 0usize;
    while let Some(pid) = cur {
        if depth >= 64 {
            break;
        }
        let Some(node) = scene.nodes.get(&pid) else {
            break;
        };
        acc = safe_quat_normalize(safe_quat_normalize(node.transform.rotation) * acc);
        cur = node.parent;
        depth += 1;
    }
    acc
}

/// Composes the full world-space rotation of `node_id` (parent chain + node's own rotation).
pub fn node_world_rotation(scene: &SceneState, node_id: NodeId) -> glam::Quat {
    parent_world_rotation(scene, Some(node_id))
}

/// Returns `true` if `node_id` exists, is visible on the node itself, and is not hidden by a
/// character rig visibility track.
pub fn node_is_effectively_visible(scene: &SceneState, node_id: NodeId) -> bool {
    let rig_visible = scene
        .character_rig
        .as_ref()
        .and_then(|r| r.node_visibility.get(&node_id).copied())
        .unwrap_or(true);
    rig_visible && scene.nodes.get(&node_id).is_some_and(|n| n.visible)
}

fn normalize_zero(v: f32) -> f32 {
    if v.abs() < 1e-4 {
        0.0
    } else {
        v
    }
}

/// Formats the active rotation angle consistently for both the disc badge and bottom-right ruler.
pub fn format_rotate_angle(axis_mode: RotateAxisMode, delta_deg: f32, snap_active: bool) -> String {
    let v = normalize_zero(delta_deg);
    match axis_mode {
        RotateAxisMode::Free => {
            if snap_active {
                format!("{:.0}°", v.abs())
            } else {
                format!("{:.1}°", v.abs())
            }
        }
        RotateAxisMode::X | RotateAxisMode::Y | RotateAxisMode::Z => {
            if snap_active {
                if v == 0.0 {
                    "0°".to_string()
                } else {
                    format!("{v:+.0}°")
                }
            } else if v == 0.0 {
                "0.0°".to_string()
            } else {
                format!("{v:+.1}°")
            }
        }
    }
}

/// Returns `true` when `a` and `b` differ enough to warrant recording an Undo command.
pub fn transform_differs(a: Transform, b: Transform) -> bool {
    (a.translation - b.translation).length_squared() > 1e-10
        || a.rotation.dot(b.rotation).abs() < 0.999_999_5
        || (a.scale - b.scale).length_squared() > 1e-10
}

/// Computes the updated local `Transform` and displayed drag angle (degrees) for the
/// screen-space floating circular rotate gizmo.
/// - `RotateAxisMode::Free`: single-axis view-aligned trackball rotation around
///   `normalize(cam_up * dx + cam_right * dy)` by continuous geodesic angle `|drag| * ROTATE_DEG_PER_PX`
///   (guaranteed continuous with zero quadrant sign jumps and exact 15° Shift snap).
/// - `RotateAxisMode::X | Y | Z`: world-axis locked rotation driven by horizontal drag `dx`.
pub fn compute_floating_rotate_transform(
    start_transform: Transform,
    parent_world_rot: glam::Quat,
    camera: &OrbitCamera,
    world_origin: glam::Vec3,
    drag_delta_px: egui::Vec2,
    axis_mode: RotateAxisMode,
    snap_15_deg: bool,
) -> (Transform, f32) {
    if !drag_delta_px.x.is_finite() || !drag_delta_px.y.is_finite() {
        return (start_transform, 0.0);
    }
    let snap = |deg: f32| -> f32 {
        let out = if snap_15_deg {
            (deg / ROTATE_SNAP_STEP_DEG).round() * ROTATE_SNAP_STEP_DEG
        } else {
            deg
        };
        normalize_zero(out)
    };

    let (world_delta, delta_deg) = match axis_mode {
        RotateAxisMode::Free => {
            let len = drag_delta_px.length();
            if len < 1e-4 {
                (glam::Quat::IDENTITY, 0.0)
            } else {
                let (cam_right, cam_up) = screen_basis_at_point(camera, world_origin);
                let axis =
                    (cam_up * drag_delta_px.x + cam_right * drag_delta_px.y).normalize_or_zero();
                let deg = snap(len * ROTATE_DEG_PER_PX).abs();
                if axis.length_squared() < 1e-8 || deg < 1e-4 {
                    (glam::Quat::IDENTITY, 0.0)
                } else {
                    (
                        safe_quat_normalize(glam::Quat::from_axis_angle(axis, deg.to_radians())),
                        deg,
                    )
                }
            }
        }
        RotateAxisMode::X | RotateAxisMode::Y | RotateAxisMode::Z => {
            let axis = axis_mode.to_gizmo_axis().unwrap_or(GizmoAxis::Y);
            let deg = snap(drag_delta_px.x * ROTATE_DEG_PER_PX);
            let q = if deg.abs() < 1e-4 {
                glam::Quat::IDENTITY
            } else {
                glam::Quat::from_axis_angle(axis.dir(), deg.to_radians())
            };
            (safe_quat_normalize(q), deg)
        }
    };

    let mut out = start_transform;
    let parent_rot = safe_quat_normalize(parent_world_rot);
    let local_delta = parent_rot.conjugate() * world_delta * parent_rot;
    out.rotation = safe_quat_normalize(local_delta * start_transform.rotation);
    (out, delta_deg)
}

/// Checks for an unmodified `X`/`Y`/`Z` key press (excluding Ctrl/Cmd/Alt) and returns the
/// requested axis key if pressed this frame.
pub fn pressed_rotate_axis_key(ctx: &egui::Context) -> Option<RotateAxisMode> {
    if ctx.wants_keyboard_input() {
        return None;
    }
    ctx.input(|i| {
        if i.modifiers.command || i.modifiers.ctrl || i.modifiers.alt {
            return None;
        }
        if i.key_pressed(egui::Key::X) {
            Some(RotateAxisMode::X)
        } else if i.key_pressed(egui::Key::Y) {
            Some(RotateAxisMode::Y)
        } else if i.key_pressed(egui::Key::Z) {
            Some(RotateAxisMode::Z)
        } else {
            None
        }
    })
}

/// Renders the screen-space floating circular rotate trackball disc over the selected node.
/// Displays a crisp ring when idle and fills with semi-transparent white when primary-pressed/dragged.
pub fn draw_floating_rotate_disc(
    ui: &egui::Ui,
    rect: egui::Rect,
    response: &egui::Response,
    scene: &SceneState,
    camera: &OrbitCamera,
    active_drag: Option<RotateDragState>,
    rotate_axis_mode: RotateAxisMode,
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
    let world_origin = scene
        .node_world_matrix(node_id)
        .transform_point3(glam::Vec3::ZERO);
    let Some(center) = project_world_point(world_origin, camera, rect) else {
        return;
    };

    let hover_inside = response
        .hover_pos()
        .is_some_and(|p| p.distance(center) <= ROTATE_DISC_RADIUS_PX);
    let primary_down_clean = ui.input(|i| i.pointer.primary_down() && !i.modifiers.alt);
    let pressed_inside = active_drag.is_some()
        || (hover_inside && response.is_pointer_button_down_on() && primary_down_clean);

    let effective_mode = active_drag
        .map(|d| d.rotate_axis)
        .unwrap_or(rotate_axis_mode);
    let accent = effective_mode.accent_color();
    let painter = ui.painter().with_clip_rect(rect);

    if pressed_inside {
        painter.circle_filled(
            center,
            ROTATE_DISC_RADIUS_PX,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 44),
        );
    } else if hover_inside {
        painter.circle_filled(
            center,
            ROTATE_DISC_RADIUS_PX,
            egui::Color32::from_rgba_unmultiplied(255, 255, 255, 16),
        );
    }

    let stroke_alpha = if pressed_inside {
        235
    } else if hover_inside {
        205
    } else {
        140
    };
    let stroke_width = if pressed_inside { 2.0 } else { 1.4 };
    let ring_color =
        egui::Color32::from_rgba_unmultiplied(accent.r(), accent.g(), accent.b(), stroke_alpha);
    painter.circle_stroke(
        center,
        ROTATE_DISC_RADIUS_PX,
        Stroke::new(stroke_width, ring_color),
    );
    painter.circle_filled(center, 2.5, ring_color);

    if let Some(drag) = active_drag {
        let raw_vec = drag.current_pointer_pos - drag.anchor_pointer_pos;
        let drag_vec = match effective_mode {
            RotateAxisMode::Free => raw_vec,
            RotateAxisMode::X | RotateAxisMode::Y | RotateAxisMode::Z => egui::vec2(raw_vec.x, 0.0),
        };
        let len = drag_vec.length();
        if len > 2.0 {
            let clamped = drag_vec * (len.min(ROTATE_DISC_RADIUS_PX - 4.0) / len);
            let handle_pos = center + clamped;
            painter.line_segment(
                [center, handle_pos],
                Stroke::new(
                    1.5,
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 210),
                ),
            );
            painter.circle_filled(handle_pos, 4.0, ring_color);
        }

        let badge = format!(
            "{} {}",
            effective_mode.label(),
            format_rotate_angle(effective_mode, drag.current_delta_deg, drag.snap_active)
        );
        let half_x = 44.0_f32.min(rect.width() * 0.5);
        let half_y = 14.0_f32.min(rect.height() * 0.5);
        let badge_y = (center.y + ROTATE_DISC_RADIUS_PX + 14.0)
            .clamp(rect.top() + half_y, rect.bottom() - half_y);
        let badge_x = center.x.clamp(rect.left() + half_x, rect.right() - half_x);
        let badge_pos = egui::pos2(badge_x, badge_y);
        let galley =
            painter.layout_no_wrap(badge, egui::FontId::monospace(10.5), egui::Color32::WHITE);
        let pill_rect =
            egui::Rect::from_center_size(badge_pos, galley.size() + egui::vec2(12.0, 6.0));
        painter.rect_filled(
            pill_rect,
            CornerRadius::same(5),
            egui::Color32::from_rgba_unmultiplied(0x14, 0x17, 0x1D, 220),
        );
        painter.rect_stroke(
            pill_rect,
            CornerRadius::same(5),
            Stroke::new(1.0, ring_color),
            egui::StrokeKind::Inside,
        );
        painter.galley(
            pill_rect.center() - galley.size() * 0.5,
            galley,
            egui::Color32::WHITE,
        );
    }
}

/// Renders the bottom-right floating Rotate HUD: 4 child floating pills (`Free`, `X`, `Y`, `Z`)
/// and a horizontal angle drag ruler directly below them.
pub fn show_rotate_axis_and_ruler_hud(
    ui: &egui::Ui,
    rect: egui::Rect,
    rotate_axis_mode: &mut RotateAxisMode,
    active_drag: Option<RotateDragState>,
) {
    // Guard against colliding with the bottom-left camera compass on ultra-narrow viewports.
    if rect.width() < 290.0 || rect.height() < 120.0 {
        return;
    }

    let screen = ui.ctx().screen_rect();
    let target = rect.right_bottom() + egui::vec2(-12.0, -12.0);
    let effective_mode = active_drag
        .map(|d| d.rotate_axis)
        .unwrap_or(*rotate_axis_mode);
    let delta_deg = active_drag.map(|d| d.current_delta_deg).unwrap_or(0.0);
    let snap_active =
        active_drag.is_some_and(|d| d.snap_active) || ui.ctx().input(|i| i.modifiers.shift);

    egui::Area::new(egui::Id::new("tessel_rotate_axis_ruler_hud"))
        .order(egui::Order::Foreground)
        .anchor(
            egui::Align2::RIGHT_BOTTOM,
            egui::vec2(target.x - screen.right(), target.y - screen.bottom()),
        )
        .show(ui.ctx(), |ui| {
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 5.0);

                // Row 1: 4 child floating pills (Free, X, Y, Z), exact 192px total width (4*45 + 3*4).
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for mode in RotateAxisMode::ALL {
                        let selected = effective_mode == mode;
                        let accent = mode.accent_color();
                        let fill = if selected {
                            egui::Color32::from_rgba_unmultiplied(
                                accent.r(),
                                accent.g(),
                                accent.b(),
                                56,
                            )
                        } else {
                            egui::Color32::from_rgba_unmultiplied(0x19, 0x1C, 0x22, 220)
                        };
                        let stroke = if selected {
                            Stroke::new(1.3, accent)
                        } else {
                            Stroke::new(1.0, tokens::OUTLINE_VARIANT)
                        };
                        let text_color = if selected {
                            egui::Color32::WHITE
                        } else {
                            accent
                        };
                        let btn = egui::Button::new(
                            egui::RichText::new(mode.label())
                                .size(11.0)
                                .strong()
                                .color(text_color),
                        )
                        .fill(fill)
                        .stroke(stroke)
                        .corner_radius(CornerRadius::same(10))
                        .min_size(egui::vec2(45.0, 22.0));
                        if ui
                            .add(btn)
                            .on_hover_text(match mode {
                                RotateAxisMode::Free => "Free trackball rotate (camera POV)",
                                RotateAxisMode::X => "Lock rotation to X axis (key X)",
                                RotateAxisMode::Y => "Lock rotation to Y axis (key Y)",
                                RotateAxisMode::Z => "Lock rotation to Z axis (key Z)",
                            })
                            .clicked()
                        {
                            *rotate_axis_mode = mode;
                        }
                    }
                });

                // Row 2: Horizontal angle ruler (174 + 16 margin + 2 stroke = 192px exact alignment).
                Frame::NONE
                    .fill(egui::Color32::from_rgba_unmultiplied(0x19, 0x1C, 0x22, 225))
                    .stroke(Stroke::new(1.0, tokens::OUTLINE_VARIANT))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(8, 5))
                    .show(ui, |ui| {
                        let ruler_size = egui::vec2(174.0, 36.0);
                        let (ruler_rect, _) = ui.allocate_exact_size(ruler_size, Sense::hover());
                        let p = ui.painter();

                        let header_y = ruler_rect.top() + 5.0;
                        let mode_text = if snap_active {
                            format!("{} [15°]", effective_mode.label())
                        } else {
                            effective_mode.label().to_string()
                        };
                        p.text(
                            egui::pos2(ruler_rect.left(), header_y),
                            egui::Align2::LEFT_CENTER,
                            mode_text,
                            egui::FontId::monospace(9.5),
                            effective_mode.accent_color(),
                        );
                        let readout = if active_drag.is_some() {
                            format!(
                                "Δ {}",
                                format_rotate_angle(effective_mode, delta_deg, snap_active)
                            )
                        } else {
                            format_rotate_angle(effective_mode, 0.0, false)
                        };
                        p.text(
                            egui::pos2(ruler_rect.right(), header_y),
                            egui::Align2::RIGHT_CENTER,
                            readout,
                            egui::FontId::monospace(10.0),
                            egui::Color32::WHITE,
                        );

                        let track_top = ruler_rect.top() + 14.0;
                        let track_bottom = ruler_rect.top() + 26.0;
                        let label_y = ruler_rect.bottom() - 3.0;
                        let is_free = effective_mode == RotateAxisMode::Free;
                        let origin_x = if is_free {
                            ruler_rect.left()
                        } else {
                            ruler_rect.center().x
                        };
                        let span_w = if is_free {
                            ruler_rect.width()
                        } else {
                            ruler_rect.width() * 0.5
                        };

                        let clamped_frac = if is_free {
                            (delta_deg.abs() / 180.0).clamp(0.0, 1.0)
                        } else {
                            (delta_deg / 180.0).clamp(-1.0, 1.0)
                        };
                        let needle_x = origin_x + clamped_frac * span_w;
                        let is_clamped = delta_deg.abs() > 180.0;

                        if active_drag.is_some() && clamped_frac.abs() > 1e-4 {
                            let fill_min_x = origin_x.min(needle_x);
                            let fill_max_x = origin_x.max(needle_x);
                            let accent = effective_mode.accent_color();
                            p.rect_filled(
                                egui::Rect::from_min_max(
                                    egui::pos2(fill_min_x, track_top + 2.0),
                                    egui::pos2(fill_max_x, track_bottom - 1.0),
                                ),
                                CornerRadius::same(2),
                                egui::Color32::from_rgba_unmultiplied(
                                    accent.r(),
                                    accent.g(),
                                    accent.b(),
                                    75,
                                ),
                            );
                        }

                        // Tick marks every 15 degrees.
                        let range = if is_free { 0..=12 } else { -12..=12 };
                        for step in range {
                            let frac = step as f32 / 12.0;
                            let x = origin_x + frac * span_w;
                            let is_zero = step == 0;
                            let is_major = step % 3 == 0;
                            let tick_top = if is_zero {
                                track_top
                            } else if is_major {
                                track_top + 3.0
                            } else {
                                track_top + 6.5
                            };
                            let col = if is_zero {
                                egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200)
                            } else if is_major {
                                egui::Color32::from_rgba_unmultiplied(190, 200, 215, 140)
                            } else {
                                egui::Color32::from_rgba_unmultiplied(140, 150, 165, 85)
                            };
                            p.line_segment(
                                [egui::pos2(x, tick_top), egui::pos2(x, track_bottom)],
                                Stroke::new(if is_zero { 1.4 } else { 1.0 }, col),
                            );
                        }

                        // Subtle scale endpoint labels.
                        let label_col = egui::Color32::from_rgba_unmultiplied(150, 160, 175, 170);
                        let (l_txt, m_txt, r_txt) = if is_free {
                            ("0°", "90°", "180°")
                        } else {
                            ("-180°", "0°", "+180°")
                        };
                        p.text(
                            egui::pos2(ruler_rect.left(), label_y),
                            egui::Align2::LEFT_CENTER,
                            l_txt,
                            egui::FontId::monospace(8.0),
                            label_col,
                        );
                        p.text(
                            egui::pos2(ruler_rect.center().x, label_y),
                            egui::Align2::CENTER_CENTER,
                            m_txt,
                            egui::FontId::monospace(8.0),
                            label_col,
                        );
                        p.text(
                            egui::pos2(ruler_rect.right(), label_y),
                            egui::Align2::RIGHT_CENTER,
                            r_txt,
                            egui::FontId::monospace(8.0),
                            label_col,
                        );

                        // Active drag needle (+ amber tip if clamped beyond ±180°).
                        if active_drag.is_some() {
                            let needle_col = if is_clamped {
                                egui::Color32::from_rgb(255, 210, 90)
                            } else {
                                egui::Color32::WHITE
                            };
                            p.line_segment(
                                [
                                    egui::pos2(needle_x, track_top - 1.0),
                                    egui::pos2(needle_x, track_bottom),
                                ],
                                Stroke::new(2.0, needle_col),
                            );
                        }
                    });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tessel_core::Command;

    #[test]
    fn floating_rotate_gizmo_handles_free_axis_lock_snap_and_non_finite() {
        let start = Transform::default();
        let cam = OrbitCamera::new();

        // 1. Axis-locked Y rotation: 120px * 0.75 deg/px = +90 deg around +Y (+Z -> +X).
        let (tx_y, deg_y) = compute_floating_rotate_transform(
            start,
            glam::Quat::IDENTITY,
            &cam,
            glam::Vec3::ZERO,
            egui::vec2(120.0, 0.0),
            RotateAxisMode::Y,
            false,
        );
        assert!((deg_y - 90.0).abs() < 1e-4, "expected 90 deg, got {deg_y}");
        let v_y = tx_y.rotation * glam::Vec3::Z;
        assert!((v_y - glam::Vec3::X).length() < 1e-4, "got {v_y:?}");

        // 2. Shift 15-deg snap in axis mode: 28px * 0.75 = 21 deg -> snaps to 15 deg.
        let (tx_snap, deg_snap) = compute_floating_rotate_transform(
            start,
            glam::Quat::IDENTITY,
            &cam,
            glam::Vec3::ZERO,
            egui::vec2(28.0, 0.0),
            RotateAxisMode::Z,
            true,
        );
        assert!((deg_snap - 15.0).abs() < 1e-4);
        let expected_z = glam::Quat::from_rotation_z(15.0_f32.to_radians());
        assert!(tx_snap.rotation.dot(expected_z).abs() > 0.9999);

        // 3. Free mode is continuous across quadrant diagonals and snaps to exact 15-deg multiples.
        let (_, deg_q4_a) = compute_floating_rotate_transform(
            start,
            glam::Quat::IDENTITY,
            &cam,
            glam::Vec3::ZERO,
            egui::vec2(100.0, -99.0),
            RotateAxisMode::Free,
            false,
        );
        let (_, deg_q4_b) = compute_floating_rotate_transform(
            start,
            glam::Quat::IDENTITY,
            &cam,
            glam::Vec3::ZERO,
            egui::vec2(100.0, -101.0),
            RotateAxisMode::Free,
            false,
        );
        assert!(
            (deg_q4_a - deg_q4_b).abs() < 2.0,
            "unexpected discontinuity: {deg_q4_a} vs {deg_q4_b}"
        );
        let (_, deg_free_snap) = compute_floating_rotate_transform(
            start,
            glam::Quat::IDENTITY,
            &cam,
            glam::Vec3::ZERO,
            egui::vec2(20.0, 20.0),
            RotateAxisMode::Free,
            true,
        );
        assert!(
            (deg_free_snap % 15.0).abs() < 1e-4,
            "free snap must be exact multiple of 15 deg, got {deg_free_snap}"
        );

        // 4. Small negative Shift-snap normalizes -0.0 to 0.0.
        let (_, deg_neg_zero) = compute_floating_rotate_transform(
            start,
            glam::Quat::IDENTITY,
            &cam,
            glam::Vec3::ZERO,
            egui::vec2(-2.0, 0.0),
            RotateAxisMode::X,
            true,
        );
        assert_eq!(deg_neg_zero, 0.0);
        assert_eq!(
            format_rotate_angle(RotateAxisMode::X, deg_neg_zero, true),
            "0°"
        );
        assert_eq!(
            format_rotate_angle(RotateAxisMode::X, deg_neg_zero, false),
            "0.0°"
        );

        // 5. Non-finite drag deltas return start without NaN.
        let (tx_nan, deg_nan) = compute_floating_rotate_transform(
            start,
            glam::Quat::IDENTITY,
            &cam,
            glam::Vec3::ZERO,
            egui::vec2(f32::NAN, 10.0),
            RotateAxisMode::Free,
            false,
        );
        assert_eq!(tx_nan, start);
        assert_eq!(deg_nan, 0.0);
    }

    #[test]
    fn parent_world_rotation_preserves_rotation_under_negative_scale() {
        let mut scene = SceneState::new();
        Command::CreateVoxelVolume {
            name: "Parent".into(),
        }
        .apply(&mut scene)
        .unwrap();
        let parent_id = scene.active_voxel_node().unwrap();
        let parent_rot = glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let mut parent_tx = scene.nodes.get(&parent_id).unwrap().transform;
        parent_tx.rotation = parent_rot;
        parent_tx.scale = glam::Vec3::new(-1.0, 1.0, 1.0); // det < 0
        scene.preview_node_transform(parent_id, parent_tx);

        let extracted = parent_world_rotation(&scene, Some(parent_id));
        assert!(
            extracted.dot(parent_rot).abs() > 0.9999,
            "expected {parent_rot:?}, got {extracted:?}"
        );
    }

    #[test]
    fn transform_differs_rejects_identity_noop() {
        let start = Transform::default();
        assert!(!transform_differs(start, start));
        let mut moved = start;
        moved.rotation = glam::Quat::from_rotation_y(0.1);
        assert!(transform_differs(start, moved));
    }
}
