use egui::{Frame, Margin, RichText, Stroke};
use tessel_core::{AppMode, HistoryStack, SceneState};
use tessel_render::{OrbitCamera, PixelCamera2D};

use super::theme::tokens;

/// Counts triangles across all visible low-poly meshes in the scene.
pub fn compute_scene_triangle_count(scene: &SceneState) -> usize {
    scene
        .lowpoly_meshes
        .iter()
        .filter(|(nid, _)| scene.nodes.get(nid).is_none_or(|n| n.visible))
        .map(|(_, mesh)| {
            mesh.faces
                .iter()
                .map(|f| f.vertices.len().saturating_sub(2))
                .sum::<usize>()
        })
        .sum()
}

/// Returns concise keyboard/mouse shortcut hints tailored to the active mode and Handfree/Select state.
pub fn status_bar_hints(mode: AppMode, free_view: bool) -> &'static str {
    match mode {
        AppMode::Pixel => {
            "LMB: Draw  |  RMB/MMB: Pan  |  Left/Right: Frame  |  B/E/G: Tools  |  X/Y: Sym  |  Ctrl+K: Palette"
        }
        AppMode::Character => {
            if free_view {
                "LMB: Orbit  |  Shift+LMB: Pan  |  Space: Play/Pause  |  Left/Right: Frame  |  Home: Rewind"
            } else {
                "LMB: Pick Bone/Gizmo  |  Space: Play/Pause  |  Left/Right: Frame  |  Home: Rewind  |  Ctrl+K: Palette"
            }
        }
        AppMode::Voxel | AppMode::LowPoly => {
            if free_view {
                "LMB: Orbit  |  Shift+LMB: Pan  |  Scroll: Zoom  |  F: Reset View  |  Ctrl+K: Palette"
            } else {
                "LMB: Select/Gizmo  |  Ctrl+D: Duplicate  |  Del: Delete  |  F: Reset View  |  Ctrl+K: Palette"
            }
        }
    }
}

pub fn show_status_bar(
    ctx: &egui::Context,
    scene: &SceneState,
    history: &HistoryStack,
    camera_3d: &OrbitCamera,
    camera_2d: &PixelCamera2D,
    free_view: bool,
    unsaved: bool,
) {
    let frame = Frame::NONE
        .fill(tokens::SURFACE)
        .inner_margin(Margin::symmetric(12, 4))
        .stroke(Stroke::new(1.0_f32, tokens::OUTLINE_VARIANT));

    egui::TopBottomPanel::bottom("tessel_status_bar")
        .exact_height(24.0)
        .frame(frame)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                if scene.active_mode != AppMode::Pixel {
                    let (badge, color) = if free_view {
                        ("HANDFREE", tokens::ON_SURFACE_VARIANT)
                    } else {
                        ("SELECT", tokens::PRIMARY)
                    };
                    ui.label(RichText::new(badge).strong().monospace().size(10.5).color(color));
                    ui.separator();
                }

                ui.label(
                    RichText::new(status_bar_hints(scene.active_mode, free_view))
                        .size(11.0)
                        .color(tokens::ON_SURFACE_VARIANT),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if unsaved {
                        ui.label(
                            RichText::new("Unsaved*")
                                .strong()
                                .monospace()
                                .size(10.5)
                                .color(tokens::ERROR),
                        );
                        ui.separator();
                    }

                    let right_text = match scene.active_mode {
                        AppMode::Pixel => format!(
                            "Mode: Pixel  |  Zoom: {:.0}x  |  Nodes: {}  |  Undo: {}",
                            camera_2d.zoom,
                            scene.nodes.len(),
                            history.undo_len()
                        ),
                        _ => {
                            let pos = camera_3d.position();
                            let tris = compute_scene_triangle_count(scene);
                            format!(
                                "Mode: {}  |  Nodes: {}  |  Tris: {}  |  Undo: {}  |  Cam: ({:.1}, {:.1}, {:.1})",
                                scene.active_mode.label(),
                                scene.nodes.len(),
                                tris,
                                history.undo_len(),
                                pos.x,
                                pos.y,
                                pos.z
                            )
                        }
                    };
                    ui.label(
                        RichText::new(right_text)
                            .monospace()
                            .size(11.0)
                            .color(tokens::ON_SURFACE_VARIANT),
                    );

                    if let Some(name) = scene
                        .selected_node
                        .and_then(|nid| scene.nodes.get(&nid))
                        .map(|n| n.name.as_str())
                    {
                        ui.separator();
                        ui.label(
                            RichText::new(format!("[ {name} ]"))
                                .monospace()
                                .size(10.5)
                                .color(tokens::PRIMARY),
                        );
                    }
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tessel_core::{Command, PaletteIndex, PrimitiveKind};

    #[test]
    fn triangle_count_counts_visible_lowpoly_meshes_only() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        history
            .execute(
                &mut scene,
                Command::CreatePrimitiveMesh {
                    name: "BoxA".into(),
                    kind: PrimitiveKind::Cube,
                    size: 1.0,
                    material: PaletteIndex::new(1).unwrap(),
                },
            )
            .unwrap();
        let id_a = scene.selected_node.unwrap();
        // A cube has 6 quad faces = 12 triangles.
        assert_eq!(compute_scene_triangle_count(&scene), 12);

        history
            .execute(
                &mut scene,
                Command::SetNodeVisibility {
                    node_id: id_a,
                    visible: false,
                },
            )
            .unwrap();
        assert_eq!(compute_scene_triangle_count(&scene), 0);
    }

    #[test]
    fn status_bar_renders_telemetry_and_hints_cleanly() {
        let mut scene = SceneState::new();
        scene.active_mode = AppMode::Character;
        let history = HistoryStack::new();
        let cam3d = OrbitCamera::new();
        let cam2d = PixelCamera2D::new();

        assert!(status_bar_hints(AppMode::Character, true).contains("Space: Play/Pause"));
        assert!(status_bar_hints(AppMode::LowPoly, false).contains("Select/Gizmo"));

        let ctx = egui::Context::default();
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 720.0),
            )),
            ..Default::default()
        };
        let out = ctx.run(raw_input, |ctx| {
            show_status_bar(ctx, &scene, &history, &cam3d, &cam2d, false, true);
        });
        assert!(!out.shapes.is_empty());
    }
}
