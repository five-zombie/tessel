use std::path::Path;
use std::sync::mpsc;

use egui::{Color32, CornerRadius, Frame, Margin, RichText, Stroke};
use tessel_ai::direct::DirectAiResponse;
use tessel_ai::{AiProviderKind, DirectAiConfig};
use tessel_core::{
    AppMode, BoneId, Command, FrameIndex, HistoryStack, Keyframe, NodeId, PaletteEntry,
    PaletteIndex, Rgba8, SceneState,
};

use super::icons::{self, ToolIcon};
use super::theme::tokens;

#[derive(Default)]
pub struct DirectAiUiState {
    pub config: DirectAiConfig,
    pub prompt: String,
    pub pending: Option<mpsc::Receiver<Result<DirectAiResponse, String>>>,
    pub cancel_handle: Option<tokio::task::AbortHandle>,
    pub last_result: Option<Result<String, String>>,
}

impl DirectAiUiState {
    pub fn poll_response(
        &mut self,
        scene: &mut SceneState,
        history: &mut HistoryStack,
        allowed_export_dir: &Path,
    ) {
        let received = self.pending.as_ref().and_then(|rx| rx.try_recv().ok());
        let Some(result) = received else {
            return;
        };
        self.pending = None;
        self.cancel_handle = None;

        match result {
            Ok(resp) => {
                let mut summary_lines = Vec::new();
                for call in &resp.tool_calls {
                    match tessel_ai::translator::execute_tool_call(
                        &call.name,
                        &call.arguments,
                        scene,
                        history,
                        allowed_export_dir,
                    ) {
                        Ok(_) => summary_lines.push(format!("[OK] {}", call.name)),
                        Err(err) => summary_lines.push(format!("[ERR] {}: {err}", call.name)),
                    }
                }
                let mut combined = String::new();
                if !summary_lines.is_empty() {
                    combined.push_str(&summary_lines.join("\n"));
                    if !resp.reply.trim().is_empty() {
                        combined.push_str("\n\n");
                    }
                }
                combined.push_str(resp.reply.trim());
                if combined.is_empty() {
                    combined = "AI completed with no text output.".into();
                }
                self.last_result = Some(Ok(combined));
            }
            Err(err) => {
                self.last_result = Some(Err(err));
            }
        }
    }
}

pub fn show_inspector(
    ctx: &egui::Context,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    ai_state: &mut DirectAiUiState,
    allowed_export_dir: &Path,
) {
    ai_state.poll_response(scene, history, allowed_export_dir);

    let frame = Frame::NONE
        .fill(tokens::SURFACE_LOW)
        .inner_margin(Margin::same(10))
        .stroke(Stroke::new(1.0_f32, tokens::OUTLINE_VARIANT));

    egui::SidePanel::right("tessel_inspector")
        .exact_width(256.0)
        .resizable(false)
        .frame(frame)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                show_palette_section(ui, scene, history);
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);

                if scene.active_mode == AppMode::Pixel {
                    show_pixel_layers_section(ui, scene, history);
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(8.0);
                } else if scene.active_mode == AppMode::Character {
                    show_character_rig_section(ui, scene, history);
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(8.0);
                }

                show_outliner_section(ui, scene, history);
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);
                show_transform_and_lighting_section(ui, scene, history);
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);
                show_direct_ai_section(ui, scene, ai_state);
            });
        });
}

fn show_palette_section(ui: &mut egui::Ui, scene: &mut SceneState, history: &mut HistoryStack) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Palette")
                .strong()
                .size(13.0)
                .color(tokens::ON_SURFACE),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::hud_icon_button(ui, ToolIcon::Add, "+ Color", false) {
                let next_raw = (1u8..=255).find(|&raw| {
                    PaletteIndex::new(raw)
                        .ok()
                        .is_some_and(|idx| scene.palette.get(idx).is_none())
                });
                if let Some(raw) = next_raw {
                    if let Ok(idx) = PaletteIndex::new(raw) {
                        let _ = history.execute(
                            scene,
                            Command::SetPaletteEntry {
                                index: idx,
                                entry: PaletteEntry::matte(
                                    format!("Custom {raw}"),
                                    Rgba8::rgb(180, 180, 190),
                                ),
                            },
                        );
                        scene.active_color = idx;
                    }
                }
            }
        });
    });
    ui.add_space(4.0);

    let entries: Vec<_> = scene
        .palette
        .iter()
        .map(|(idx, entry)| (*idx, entry.name.clone(), entry.color))
        .collect();

    // Fixed column count derived from the inspector panel geometry
    // (exact_width 256 minus 2x10 inner margin = 236 content): 7 columns of
    // 24px cells + 6px gaps = 204px, leaving room for the vertical
    // scrollbar. A static count (instead of measuring available_width)
    // avoids a layout feedback loop where the grid forces the panel wider
    // on the first frame. NOTE: egui Grid does NOT auto-wrap — end_row()
    // must be called explicitly every COLS cells.
    const COLS: usize = 7;
    egui::Grid::new("palette_swatches")
        .num_columns(COLS)
        .min_col_width(24.0)
        .spacing(egui::vec2(6.0, 6.0))
        .show(ui, |ui| {
            for (i, (idx, name, rgba)) in entries.into_iter().enumerate() {
                let selected = scene.active_color == idx;
                let fill = Color32::from_rgba_unmultiplied(rgba.r, rgba.g, rgba.b, rgba.a);
                let stroke = if selected {
                    Stroke::new(2.0_f32, tokens::PRIMARY)
                } else {
                    Stroke::new(1.0_f32, tokens::OUTLINE_VARIANT)
                };
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
                ui.painter().rect(
                    rect,
                    CornerRadius::same(4),
                    fill,
                    stroke,
                    egui::StrokeKind::Inside,
                );
                if resp
                    .on_hover_text(format!("#{}: {}", idx.get(), name))
                    .clicked()
                {
                    scene.active_color = idx;
                }
                if (i + 1) % COLS == 0 {
                    ui.end_row();
                }
            }
        });

    if let Some(active_entry) = scene.palette.get(scene.active_color).cloned() {
        ui.add_space(6.0);
        let mut srgba = Color32::from_rgba_unmultiplied(
            active_entry.color.r,
            active_entry.color.g,
            active_entry.color.b,
            active_entry.color.a,
        );
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "Edit #{}: {}",
                    scene.active_color.get(),
                    active_entry.name
                ))
                .size(11.0)
                .color(tokens::ON_SURFACE_VARIANT),
            );
            if ui.color_edit_button_srgba(&mut srgba).changed() {
                let mut updated = active_entry;
                updated.color = Rgba8::rgba(srgba.r(), srgba.g(), srgba.b(), srgba.a());
                let _ = history.execute(
                    scene,
                    Command::SetPaletteEntry {
                        index: scene.active_color,
                        entry: updated,
                    },
                );
            }
        });
    }
}

fn show_pixel_layers_section(
    ui: &mut egui::Ui,
    scene: &mut SceneState,
    history: &mut HistoryStack,
) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Pixel Layers")
                .strong()
                .size(13.0)
                .color(tokens::ON_SURFACE),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(node_id) = scene.active_pixel_node() {
                if icons::hud_icon_button(ui, ToolIcon::Add, "+ Layer", false) {
                    let frame_idx = scene
                        .pixel_canvases
                        .get(&node_id)
                        .map(|c| c.active_frame)
                        .unwrap_or(0);
                    let layer_num = scene
                        .pixel_canvases
                        .get(&node_id)
                        .and_then(|c| c.frames.get(frame_idx))
                        .map(|f| f.layers.len() + 1)
                        .unwrap_or(1);
                    let _ = history.execute(
                        scene,
                        Command::AddPixelLayer {
                            node_id,
                            frame: FrameIndex(frame_idx),
                            name: format!("Layer {layer_num}"),
                        },
                    );
                }
            }
        });
    });
    ui.add_space(4.0);

    let Some(node_id) = scene.active_pixel_node() else {
        ui.label(
            RichText::new("No pixel canvas yet. Create one below.")
                .size(11.5)
                .color(tokens::ON_SURFACE_VARIANT),
        );
        ui.add_space(4.0);
        let size_id = egui::Id::new("new_canvas_size");
        let (mut w, mut h) = ui
            .ctx()
            .data_mut(|data| data.get_temp::<(u32, u32)>(size_id).unwrap_or((32, 32)));
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("W")
                    .size(11.0)
                    .color(tokens::ON_SURFACE_VARIANT),
            );
            ui.add(
                egui::DragValue::new(&mut w)
                    .range(8..=256)
                    .prefix("")
                    .suffix("px"),
            );
            ui.label(
                RichText::new("H")
                    .size(11.0)
                    .color(tokens::ON_SURFACE_VARIANT),
            );
            ui.add(
                egui::DragValue::new(&mut h)
                    .range(8..=256)
                    .prefix("")
                    .suffix("px"),
            );
        });
        ui.ctx().data_mut(|data| data.insert_temp(size_id, (w, h)));
        ui.add_space(4.0);
        let create_btn = egui::Button::new(
            RichText::new(format!("+ New Canvas ({w}×{h})"))
                .size(12.0)
                .color(tokens::ON_PRIMARY),
        )
        .fill(tokens::PRIMARY_CONTAINER)
        .corner_radius(egui::CornerRadius::same(4))
        .min_size(egui::vec2(ui.available_width(), 28.0));
        if ui.add(create_btn).clicked() {
            let _ = history.execute(
                scene,
                Command::CreatePixelCanvas {
                    name: format!("Sprite {w}x{h}"),
                    width: w,
                    height: h,
                },
            );
        }
        return;
    };

    let snapshot = scene.pixel_canvases.get(&node_id).and_then(|c| {
        c.current_frame().map(|f| {
            (
                c.active_frame,
                f.active_layer,
                f.layers
                    .iter()
                    .enumerate()
                    .map(|(idx, l)| (idx, l.id, l.name.clone(), l.visible))
                    .collect::<Vec<_>>(),
            )
        })
    });

    if let Some((active_frame, active_layer, layers)) = snapshot {
        for (idx, layer_id, name, visible) in layers.into_iter().rev() {
            ui.horizontal(|ui| {
                let (vis_icon, vis_tip) = if visible {
                    (ToolIcon::EyeOpen, "Hide layer")
                } else {
                    (ToolIcon::EyeClosed, "Show layer")
                };
                if icons::hud_icon_button(ui, vis_icon, vis_tip, false) {
                    let _ = history.execute(
                        scene,
                        Command::SetPixelLayerVisible {
                            node_id,
                            frame: FrameIndex(active_frame),
                            layer_id,
                            visible: !visible,
                        },
                    );
                }
                let selected = idx == active_layer;
                if ui.selectable_label(selected, name).clicked() {
                    if let Some(frame) = scene
                        .pixel_canvases
                        .get_mut(&node_id)
                        .and_then(|c| c.current_frame_mut())
                    {
                        frame.active_layer = idx;
                    }
                }
            });
        }
    }
}

fn show_character_rig_section(
    ui: &mut egui::Ui,
    scene: &mut SceneState,
    history: &mut HistoryStack,
) {
    ui.label(
        RichText::new("Character Rig")
            .strong()
            .size(13.0)
            .color(tokens::ON_SURFACE),
    );
    ui.add_space(4.0);

    let Some(rig) = &scene.character_rig else {
        ui.label(
            RichText::new("No rig applied yet.")
                .size(11.5)
                .color(tokens::ON_SURFACE_VARIANT),
        );
        if icons::hud_icon_button(ui, ToolIcon::BipedRig, "Apply Biped Skeleton", false) {
            let _ = history.execute(scene, Command::ApplyBipedRig);
        }
        return;
    };

    let bones: Vec<_> = rig
        .bones
        .iter()
        .map(|b| {
            (
                b.id,
                b.name.clone(),
                b.rest_local.rotation,
                b.pose_local.rotation,
            )
        })
        .collect();
    let active_clip = rig.active_animation;
    let playback_time = rig.playback_time;
    let current_binding = scene.selected_node.and_then(|nid| {
        rig.bindings
            .iter()
            .find(|b| b.node_id == nid)
            .map(|b| b.bone_id)
    });
    let current_mask = scene
        .selected_node
        .and_then(|nid| rig.node_bone_masks.get(&nid).cloned());

    if let Some(selected_id) = scene.selected_node {
        if let Some(node) = scene.nodes.get(&selected_id) {
            let node_name = node.name.clone();
            ui.label(
                RichText::new(format!("Bind '{node_name}' to Bone:"))
                    .size(11.0)
                    .color(tokens::ON_SURFACE_VARIANT),
            );
            ui.horizontal_wrapped(|ui| {
                if ui
                    .selectable_label(current_binding.is_none(), "None")
                    .clicked()
                {
                    let _ = history.execute(
                        scene,
                        Command::BindNodeToBone {
                            node_id: selected_id,
                            bone_id: None,
                        },
                    );
                }
                for (bid, bname, _, _) in &bones {
                    if ui
                        .selectable_label(current_binding == Some(*bid), bname)
                        .clicked()
                    {
                        let _ = history.execute(
                            scene,
                            Command::BindNodeToBone {
                                node_id: selected_id,
                                bone_id: Some(*bid),
                            },
                        );
                    }
                }
            });
            ui.add_space(6.0);

            ui.label(
                RichText::new(format!("Bone Mask ('{node_name}'):"))
                    .size(11.0)
                    .color(tokens::ON_SURFACE_VARIANT),
            );
            ui.horizontal_wrapped(|ui| {
                if ui.selectable_label(current_mask.is_none(), "All").clicked() {
                    let _ = history.execute(
                        scene,
                        Command::SetNodeBoneMask {
                            node_id: selected_id,
                            bone_ids: None,
                        },
                    );
                }
                for (bid, bname, _, _) in &bones {
                    let in_mask = current_mask.as_ref().is_some_and(|list| list.contains(bid));
                    if ui.selectable_label(in_mask, bname).clicked() {
                        let next_mask = toggle_bone_in_mask(current_mask.as_deref(), *bid);
                        let _ = history.execute(
                            scene,
                            Command::SetNodeBoneMask {
                                node_id: selected_id,
                                bone_ids: next_mask,
                            },
                        );
                    }
                }
            });
            ui.add_space(6.0);
        }
    }

    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Pose Editor (Degrees)")
                .size(11.0)
                .color(tokens::ON_SURFACE_VARIANT),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::hud_icon_button(ui, ToolIcon::Reset, "Reset all bones to rest pose", false) {
                let _ = history.execute(scene, Command::ResetCharacterPose);
            }
        });
    });
    ui.horizontal_wrapped(|ui| {
        for (bid, bname, _, _) in &bones {
            let selected = scene.selected_bone == Some(*bid);
            if ui.selectable_label(selected, bname).clicked() {
                scene.selected_bone = Some(*bid);
            }
        }
    });
    ui.add_space(4.0);

    let active_bone = scene
        .selected_bone
        .and_then(|bid| bones.iter().find(|(id, ..)| *id == bid))
        .or_else(|| bones.first());

    if let Some((bone_id, name, rest_rot, pose_rot)) = active_bone.cloned() {
        let delta_rot = rest_rot.inverse() * pose_rot;
        let (rx, ry, rz) = delta_rot.to_euler(glam::EulerRot::XYZ);
        let mut rx_deg = rx.to_degrees();
        let mut ry_deg = ry.to_degrees();
        let mut rz_deg = rz.to_degrees();

        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("Bone: {name}"))
                    .strong()
                    .size(11.5)
                    .color(tokens::PRIMARY),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::hud_icon_button(ui, ToolIcon::Keyframe, "Key", false) {
                    let _ = history.execute(
                        scene,
                        Command::SetAnimationKeyframe {
                            clip_index: active_clip,
                            bone_id,
                            keyframe: Keyframe::rotation_only(playback_time, delta_rot),
                        },
                    );
                }
                if icons::hud_icon_button(ui, ToolIcon::Reset, "Reset bone pose", false) {
                    let _ = history.execute(
                        scene,
                        Command::SetBoneRotation {
                            bone_id,
                            rotation: rest_rot,
                        },
                    );
                }
            });
        });

        let mut changed = false;
        changed |= ui
            .add(egui::Slider::new(&mut rx_deg, -180.0..=180.0).text("Pitch"))
            .changed();
        changed |= ui
            .add(egui::Slider::new(&mut ry_deg, -180.0..=180.0).text("Yaw"))
            .changed();
        changed |= ui
            .add(egui::Slider::new(&mut rz_deg, -180.0..=180.0).text("Roll"))
            .changed();
        if changed {
            let next_delta = glam::Quat::from_euler(
                glam::EulerRot::XYZ,
                rx_deg.to_radians(),
                ry_deg.to_radians(),
                rz_deg.to_radians(),
            );
            let _ = history.execute(
                scene,
                Command::SetBoneRotation {
                    bone_id,
                    rotation: rest_rot * next_delta,
                },
            );
        }
    }
}

fn toggle_bone_in_mask(current: Option<&[BoneId]>, bone_id: BoneId) -> Option<Vec<BoneId>> {
    let mut next = current.map(ToOwned::to_owned).unwrap_or_default();
    if let Some(pos) = next.iter().position(|b| *b == bone_id) {
        next.remove(pos);
    } else {
        next.push(bone_id);
    }
    if next.is_empty() {
        None
    } else {
        Some(next)
    }
}

fn rename_node_if_changed(
    scene: &mut SceneState,
    history: &mut HistoryStack,
    node_id: NodeId,
    draft: &str,
) -> bool {
    let trimmed = draft.trim();
    if trimmed.is_empty() {
        return false;
    }
    if scene.nodes.get(&node_id).is_some_and(|n| n.name == trimmed) {
        return false;
    }
    history
        .execute(
            scene,
            Command::RenameNode {
                node_id,
                name: trimmed.to_string(),
            },
        )
        .is_ok()
}

fn show_outliner_section(ui: &mut egui::Ui, scene: &mut SceneState, history: &mut HistoryStack) {
    ui.label(
        RichText::new("Scene Outliner")
            .strong()
            .size(13.0)
            .color(tokens::ON_SURFACE),
    );
    ui.add_space(4.0);

    if scene.nodes.is_empty() {
        ui.label(
            RichText::new("Scene is empty.")
                .size(11.5)
                .color(tokens::ON_SURFACE_VARIANT),
        );
        return;
    }

    let rename_state_id = egui::Id::new("tessel_outliner_rename");
    let mut renaming: Option<(NodeId, String)> = ui
        .ctx()
        .data_mut(|d| d.get_temp::<(NodeId, String)>(rename_state_id));

    let nodes: Vec<_> = scene
        .nodes
        .values()
        .map(|n| (n.id, n.name.clone(), n.kind, n.visible))
        .collect();
    for (id, name, kind, visible) in nodes {
        let selected = scene.selected_node == Some(id);
        let is_renaming = renaming.as_ref().is_some_and(|(rid, _)| *rid == id);
        ui.horizontal(|ui| {
            if is_renaming {
                if let Some((_, ref mut draft)) = renaming {
                    let resp = ui.add(
                        egui::TextEdit::singleline(draft)
                            .desired_width(140.0)
                            .hint_text("Node name"),
                    );
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    if escape {
                        renaming = None;
                    } else if enter || resp.lost_focus() {
                        let committed = draft.clone();
                        rename_node_if_changed(scene, history, id, &committed);
                        renaming = None;
                    }
                }
            } else {
                let label_text = format!("{name} ({kind:?})");
                let text = if visible {
                    RichText::new(label_text)
                } else {
                    RichText::new(label_text).color(tokens::ON_SURFACE_VARIANT)
                };
                if ui.selectable_label(selected, text).clicked() && !selected {
                    let _ = history.execute(scene, Command::SelectNode(Some(id)));
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::hud_icon_button_full(
                    ui,
                    ToolIcon::Close,
                    "Delete node",
                    false,
                    true,
                    true,
                ) {
                    if renaming.as_ref().is_some_and(|(rid, _)| *rid == id) {
                        renaming = None;
                    }
                    let _ = history.execute(scene, Command::DeleteNode(id));
                }
                if icons::hud_icon_button_full(
                    ui,
                    ToolIcon::Pencil,
                    "Rename node",
                    is_renaming,
                    true,
                    true,
                ) {
                    if is_renaming {
                        if let Some((_, draft)) = renaming.take() {
                            rename_node_if_changed(scene, history, id, &draft);
                        }
                    } else {
                        renaming = Some((id, name.clone()));
                    }
                }
                let (vis_icon, vis_tip) = if visible {
                    (ToolIcon::EyeOpen, "Hide node")
                } else {
                    (ToolIcon::EyeClosed, "Show node")
                };
                if icons::hud_icon_button_full(ui, vis_icon, vis_tip, !visible, true, true) {
                    let _ = history.execute(
                        scene,
                        Command::SetNodeVisibility {
                            node_id: id,
                            visible: !visible,
                        },
                    );
                }
            });
        });
    }

    ui.ctx().data_mut(|d| {
        if let Some(state) = renaming {
            d.insert_temp(rename_state_id, state);
        } else {
            d.remove::<(NodeId, String)>(rename_state_id);
        }
    });
}

fn show_transform_and_lighting_section(
    ui: &mut egui::Ui,
    scene: &mut SceneState,
    history: &mut HistoryStack,
) {
    if let Some(node_id) = scene.selected_node {
        if let Some((node_name, mut tx)) = scene
            .nodes
            .get(&node_id)
            .map(|n| (n.name.clone(), n.transform))
        {
            let (mut rx, mut ry, mut rz) = tx.rotation.to_euler(glam::EulerRot::XYZ);
            let mut rx_deg = rx.to_degrees();
            let mut ry_deg = ry.to_degrees();
            let mut rz_deg = rz.to_degrees();

            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Transform ({node_name})"))
                        .strong()
                        .size(12.5)
                        .color(tokens::ON_SURFACE),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::hud_icon_button(ui, ToolIcon::Reset, "Reset transform", false) {
                        let _ = history.execute(
                            scene,
                            Command::SetNodeTransform {
                                node_id,
                                transform: tessel_core::Transform::default(),
                            },
                        );
                    }
                });
            });

            let mut changed = false;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Pos")
                        .size(11.0)
                        .color(tokens::ON_SURFACE_VARIANT),
                );
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut tx.translation.x)
                            .speed(0.05)
                            .prefix("X "),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut tx.translation.y)
                            .speed(0.05)
                            .prefix("Y "),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut tx.translation.z)
                            .speed(0.05)
                            .prefix("Z "),
                    )
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Rot")
                        .size(11.0)
                        .color(tokens::ON_SURFACE_VARIANT),
                );
                changed |= ui
                    .add(egui::DragValue::new(&mut rx_deg).speed(1.0).prefix("X "))
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut ry_deg).speed(1.0).prefix("Y "))
                    .changed();
                changed |= ui
                    .add(egui::DragValue::new(&mut rz_deg).speed(1.0).prefix("Z "))
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Scale")
                        .size(11.0)
                        .color(tokens::ON_SURFACE_VARIANT),
                );
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut tx.scale.x)
                            .speed(0.02)
                            .range(0.005..=100.0)
                            .prefix("X "),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut tx.scale.y)
                            .speed(0.02)
                            .range(0.005..=100.0)
                            .prefix("Y "),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut tx.scale.z)
                            .speed(0.02)
                            .range(0.005..=100.0)
                            .prefix("Z "),
                    )
                    .changed();
            });

            if changed {
                rx = rx_deg.to_radians();
                ry = ry_deg.to_radians();
                rz = rz_deg.to_radians();
                tx.rotation = glam::Quat::from_euler(glam::EulerRot::XYZ, rx, ry, rz);
                let _ = history.execute(
                    scene,
                    Command::SetNodeTransform {
                        node_id,
                        transform: tx,
                    },
                );
            }
            ui.add_space(6.0);
        }
    }

    if scene.active_mode == AppMode::LowPoly {
        if let Some(node_id) = scene.active_lowpoly_node() {
            if let Some(mut mat) = scene.lowpoly_meshes.get(&node_id).map(|m| m.material) {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("PBR Material")
                            .strong()
                            .size(12.5)
                            .color(tokens::ON_SURFACE),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::hud_icon_button(ui, ToolIcon::Reset, "Reset material", false) {
                            let _ = history.execute(
                                scene,
                                Command::SetMeshMaterial {
                                    node_id,
                                    material: tessel_core::MeshMaterial::default(),
                                },
                            );
                        }
                    });
                });
                let mut changed = false;
                changed |= ui
                    .add(egui::Slider::new(&mut mat.metallic, 0.0..=1.0).text("Metallic"))
                    .changed();
                changed |= ui
                    .add(egui::Slider::new(&mut mat.roughness, 0.05..=1.0).text("Roughness"))
                    .changed();
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Emissive Glow")
                            .size(11.0)
                            .color(tokens::ON_SURFACE_VARIANT),
                    );
                    changed |= ui.color_edit_button_rgb(&mut mat.emissive).changed();
                });
                changed |= ui.checkbox(&mut mat.double_sided, "Double Sided").changed();
                if changed {
                    let _ = history.execute(
                        scene,
                        Command::SetMeshMaterial {
                            node_id,
                            material: mat,
                        },
                    );
                }
                if let Some(face_id) = scene.selected_face {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!("Move Face #{}", face_id.0))
                            .strong()
                            .size(11.5)
                            .color(tokens::ON_SURFACE),
                    );
                    let step = 0.05;
                    ui.horizontal(|ui| {
                        for (lbl, d) in [
                            ("-X", glam::Vec3::new(-step, 0.0, 0.0)),
                            ("+X", glam::Vec3::new(step, 0.0, 0.0)),
                            ("-Y", glam::Vec3::new(0.0, -step, 0.0)),
                            ("+Y", glam::Vec3::new(0.0, step, 0.0)),
                            ("-Z", glam::Vec3::new(0.0, 0.0, -step)),
                            ("+Z", glam::Vec3::new(0.0, 0.0, step)),
                        ] {
                            if icons::text_icon_button(ui, lbl, &format!("Nudge face {lbl}"), false)
                            {
                                let _ = history.execute(
                                    scene,
                                    Command::TranslateMeshFace {
                                        node_id,
                                        face_id,
                                        delta: d,
                                    },
                                );
                            }
                        }
                    });
                }
                ui.add_space(6.0);
            }
        }
    }

    if scene.active_mode != AppMode::Pixel {
        ui.label(
            RichText::new("Studio Lighting")
                .strong()
                .size(12.5)
                .color(tokens::ON_SURFACE),
        );
        let mut az_deg = scene.light_azimuth.to_degrees();
        let mut el_deg = scene.light_elevation.to_degrees();
        if ui
            .add(egui::Slider::new(&mut az_deg, -180.0..=180.0).text("Azimuth"))
            .changed()
        {
            scene.light_azimuth = az_deg.to_radians();
        }
        if ui
            .add(egui::Slider::new(&mut el_deg, 5.0..=89.0).text("Elevation"))
            .changed()
        {
            scene.light_elevation = el_deg.to_radians();
        }
    }
}

fn show_direct_ai_section(ui: &mut egui::Ui, scene: &SceneState, ai_state: &mut DirectAiUiState) {
    ui.label(
        RichText::new("Direct AI")
            .strong()
            .size(13.0)
            .color(tokens::ON_SURFACE),
    );
    ui.add_space(4.0);

    egui::ScrollArea::horizontal()
        .id_salt("direct_ai_providers")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for provider in AiProviderKind::ALL {
                    if ui
                        .selectable_label(ai_state.config.provider == provider, provider.label())
                        .clicked()
                    {
                        ai_state.config.provider = provider;
                        ai_state.config.model = provider.default_model().into();
                        ai_state.config.base_url = provider.default_base_url().into();
                        ai_state.config.api_key =
                            crate::settings::load_api_key_from_keyring(provider);
                    }
                }
            });
        });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Model")
                .size(11.0)
                .color(tokens::ON_SURFACE_VARIANT),
        );
        ui.text_edit_singleline(&mut ai_state.config.model);
    });
    if ai_state.config.provider != AiProviderKind::Ollama {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Key")
                    .size(11.0)
                    .color(tokens::ON_SURFACE_VARIANT),
            );
            ui.add(egui::TextEdit::singleline(&mut ai_state.config.api_key).password(true));
            if icons::hud_icon_button(ui, ToolIcon::Save, "Save API key", false) {
                if let Err(err) = crate::settings::save_api_key_to_keyring(
                    ai_state.config.provider,
                    &ai_state.config.api_key,
                ) {
                    ai_state.last_result = Some(Err(format!("Keyring error: {err}")));
                }
            }
        });
    }

    ui.add_space(4.0);
    ui.add(
        egui::TextEdit::multiline(&mut ai_state.prompt)
            .desired_rows(3)
            .hint_text("Describe an asset or edit..."),
    );

    ui.add_space(4.0);
    let busy = ai_state.pending.is_some();
    let can_send = !busy && !ai_state.prompt.trim().is_empty();
    ui.horizontal(|ui| {
        if icons::hud_icon_button_enabled(ui, ToolIcon::Send, "Send Prompt", false, can_send) {
            let (tx, rx) = mpsc::channel();
            let (abort_tx, abort_rx) = mpsc::channel();
            let config = ai_state.config.clone();
            let scene_clone = scene.clone();
            let prompt = ai_state.prompt.clone();
            ai_state.pending = Some(rx);
            std::thread::spawn(move || {
                let res = match tokio::runtime::Runtime::new() {
                    Ok(rt) => rt.block_on(async move {
                        let task = tokio::spawn(async move {
                            tessel_ai::direct::send_prompt(&config, &scene_clone, &prompt)
                                .await
                                .map_err(|e| e.to_string())
                        });
                        let _ = abort_tx.send(task.abort_handle());
                        match task.await {
                            Ok(inner) => inner,
                            Err(err) if err.is_cancelled() => Err("request cancelled".into()),
                            Err(err) => Err(format!("worker join error: {err}")),
                        }
                    }),
                    Err(e) => Err(format!("async runtime error: {e}")),
                };
                let _ = tx.send(res);
            });
            ai_state.cancel_handle = abort_rx
                .recv_timeout(std::time::Duration::from_millis(200))
                .ok();
        }
        if busy && icons::hud_icon_button(ui, ToolIcon::Close, "Cancel", false) {
            if let Some(handle) = ai_state.cancel_handle.take() {
                handle.abort();
            }
            ai_state.pending = None;
        }
    });

    ui.add_space(4.0);
    if busy {
        ui.label(
            RichText::new("Waiting for provider response...")
                .size(11.0)
                .color(tokens::PRIMARY),
        );
    } else if let Some(result) = &ai_state.last_result {
        match result {
            Ok(reply) => {
                ui.label(RichText::new(reply).size(11.5).color(tokens::ON_SURFACE));
            }
            Err(err) => {
                ui.label(RichText::new(err).size(11.0).color(tokens::ERROR));
            }
        }
    } else {
        ui.label(
            RichText::new("Enter a prompt to collaborate with Direct AI, or run `tessel mcp` for external agents.")
                .size(11.0)
                .color(tokens::ON_SURFACE_VARIANT),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test: with enough swatches to force the vertical scrollbar
    /// (which steals panel width), no painted shape may extend past the panel
    /// edge — i.e. the swatch grid truly fits instead of clipping.
    #[test]
    fn palette_grid_never_overflows_panel() {
        let mut scene = SceneState::default();
        for raw in 9u8..=48 {
            if let Ok(idx) = PaletteIndex::new(raw) {
                scene.palette.set(
                    idx,
                    PaletteEntry::matte(format!("Test {raw}"), Rgba8::rgb(raw, raw, 200)),
                );
            }
        }
        let mut history = HistoryStack::new();

        let ctx = egui::Context::default();
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 600.0),
            )),
            ..Default::default()
        };
        let out = ctx.run(raw_input, |ctx| {
            egui::SidePanel::right("test_inspector")
                .exact_width(256.0)
                .resizable(false)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        show_palette_section(ui, &mut scene, &mut history);
                    });
                });
        });
        let right_edge = 1280.0;
        let mut max_x = 0.0f32;
        for s in &out.shapes {
            max_x = max_x.max(s.shape.visual_bounding_rect().max.x);
        }
        assert!(
            max_x <= right_edge + 0.5,
            "palette paints past the panel edge: max_x={max_x}"
        );
    }

    #[test]
    fn toggle_bone_in_mask_adds_removes_and_clears_to_none() {
        let b1 = BoneId(1);
        let b2 = BoneId(2);
        let m1 = toggle_bone_in_mask(None, b1);
        assert_eq!(m1, Some(vec![b1]));

        let m2 = toggle_bone_in_mask(m1.as_deref(), b2);
        assert_eq!(m2, Some(vec![b1, b2]));

        let m3 = toggle_bone_in_mask(m2.as_deref(), b1);
        assert_eq!(m3, Some(vec![b2]));

        let m4 = toggle_bone_in_mask(m3.as_deref(), b2);
        assert_eq!(m4, None);
    }

    #[test]
    fn rename_node_if_changed_commits_trimmed_and_ignores_blank_or_same() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        history
            .execute(
                &mut scene,
                Command::CreatePrimitiveMesh {
                    name: "Helmet".into(),
                    kind: tessel_core::PrimitiveKind::Cube,
                    size: 1.0,
                    material: PaletteIndex::new(1).unwrap(),
                },
            )
            .unwrap();
        let node_id = scene.selected_node.unwrap();

        assert!(!rename_node_if_changed(
            &mut scene,
            &mut history,
            node_id,
            "   "
        ));
        assert!(!rename_node_if_changed(
            &mut scene,
            &mut history,
            node_id,
            "Helmet"
        ));
        assert!(rename_node_if_changed(
            &mut scene,
            &mut history,
            node_id,
            "  AltynHelmet  "
        ));
        assert_eq!(scene.nodes[&node_id].name, "AltynHelmet");

        history.undo(&mut scene).unwrap();
        assert_eq!(scene.nodes[&node_id].name, "Helmet");
    }

    #[test]
    fn inspector_renders_outliner_and_character_rig_without_panic() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        history
            .execute(
                &mut scene,
                Command::CreatePrimitiveMesh {
                    name: "Torso".into(),
                    kind: tessel_core::PrimitiveKind::Cube,
                    size: 1.0,
                    material: PaletteIndex::new(1).unwrap(),
                },
            )
            .unwrap();
        let node_id = scene.selected_node.unwrap();
        history
            .execute(&mut scene, Command::ApplyHumanoidRig)
            .unwrap();
        history
            .execute(
                &mut scene,
                Command::SetNodeVisibility {
                    node_id,
                    visible: false,
                },
            )
            .unwrap();

        let ctx = egui::Context::default();
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 720.0),
            )),
            ..Default::default()
        };
        let out = ctx.run(raw_input, |ctx| {
            egui::SidePanel::right("test_inspector_v23")
                .exact_width(256.0)
                .resizable(false)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        show_character_rig_section(ui, &mut scene, &mut history);
                        show_outliner_section(ui, &mut scene, &mut history);
                    });
                });
        });
        assert!(!out.shapes.is_empty());
    }
}
