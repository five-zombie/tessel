use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{mpsc, OnceLock};

use egui::{Color32, CornerRadius, Frame, Margin, RichText, Stroke};
use tessel_ai::direct::{tool_call_requires_confirmation, DirectAiResponse, PlannedToolCall};
use tessel_ai::{AiProviderKind, DirectAiConfig};
use tessel_core::{
    AppMode, BoneId, Command, FrameIndex, HistoryStack, Keyframe, NodeId, PaletteEntry,
    PaletteIndex, Rgba8, SceneState,
};

use super::icons::{self, ToolIcon};
use super::theme::tokens;

static DIRECT_AI_RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();

fn direct_ai_runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    match DIRECT_AI_RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("tessel-direct-ai")
            .enable_all()
            .build()
            .map_err(|e| format!("async runtime error: {e}"))
    }) {
        Ok(rt) => Ok(rt),
        Err(e) => Err(e.clone()),
    }
}

#[derive(Default)]
pub struct DirectAiUiState {
    pub config: DirectAiConfig,
    pub prompt: String,
    pub pending: Option<mpsc::Receiver<Result<DirectAiResponse, String>>>,
    pub cancel_handle: Option<tokio::task::AbortHandle>,
    pub pending_confirm_calls: Vec<PlannedToolCall>,
    pub last_result: Option<Result<String, String>>,
}

impl DirectAiUiState {
    pub fn poll_response(
        &mut self,
        scene: &mut SceneState,
        history: &mut HistoryStack,
        allowed_export_dir: &Path,
    ) {
        let received = match self.pending.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(res)) => res,
            Some(Err(mpsc::TryRecvError::Disconnected)) => {
                Err("Direct AI worker disconnected unexpectedly".into())
            }
            _ => return,
        };
        let result = received;
        self.pending = None;
        self.cancel_handle = None;

        match result {
            Ok(resp) => {
                let mut summary_lines = Vec::new();
                self.pending_confirm_calls.clear();
                for (idx, call) in resp.tool_calls.iter().enumerate() {
                    if tool_call_requires_confirmation(call) {
                        for remaining in &resp.tool_calls[idx..] {
                            self.pending_confirm_calls.push(remaining.clone());
                            summary_lines.push(format!("[CONFIRM REQUIRED] {}", remaining.name));
                        }
                        break;
                    }
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

    pub fn approve_pending_confirm_calls(
        &mut self,
        scene: &mut SceneState,
        history: &mut HistoryStack,
        allowed_export_dir: &Path,
    ) {
        let calls = std::mem::take(&mut self.pending_confirm_calls);
        let mut lines = Vec::new();
        for call in &calls {
            match tessel_ai::translator::execute_tool_call(
                &call.name,
                &call.arguments,
                scene,
                history,
                allowed_export_dir,
            ) {
                Ok(_) => lines.push(format!("[APPROVED OK] {}", call.name)),
                Err(err) => lines.push(format!("[APPROVED ERR] {}: {err}", call.name)),
            }
        }
        if !lines.is_empty() {
            self.last_result = Some(Ok(lines.join("\n")));
        }
    }
}

/// Per-project inline rename and tree collapse state for the Scene Outliner (owned by `StudioState`
/// so it clears cleanly on project switch/load and requests keyboard focus automatically on open).
#[derive(Debug, Default)]
pub struct OutlinerRenameState {
    pub active: Option<(NodeId, String)>,
    pub request_focus: bool,
    pub collapsed_nodes: BTreeSet<NodeId>,
}

impl OutlinerRenameState {
    pub fn clear(&mut self) {
        self.clear_rename();
        self.collapsed_nodes.clear();
    }

    fn clear_rename(&mut self) {
        self.active = None;
        self.request_focus = false;
    }
}

#[allow(clippy::too_many_arguments)]
pub fn show_inspector(
    ctx: &egui::Context,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    ai_state: &mut DirectAiUiState,
    allowed_export_dir: &Path,
    settings: &crate::settings::AppSettings,
    show_viewport_gizmo: &mut bool,
    gizmo_mode: &mut tessel_render::TransformGizmoMode,
    free_view: &mut bool,
    render_preview: &mut bool,
    interaction_tool: &mut super::viewport::ViewportInteractionTool,
    outliner_rename: &mut OutlinerRenameState,
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

                show_outliner_section(ui, scene, history, outliner_rename);
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);
                show_transform_and_lighting_section(
                    ui,
                    scene,
                    history,
                    show_viewport_gizmo,
                    gizmo_mode,
                    free_view,
                    render_preview,
                    interaction_tool,
                );
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);
                show_direct_ai_section(ui, scene, history, ai_state, allowed_export_dir, settings);
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
                        super::exec_history(
                            ui.ctx(),
                            history,
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
                super::exec_history(
                    ui.ctx(),
                    history,
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
                    super::exec_history(
                        ui.ctx(),
                        history,
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
            super::exec_history(
                ui.ctx(),
                history,
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
                    super::exec_history(
                        ui.ctx(),
                        history,
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
            super::exec_history(ui.ctx(), history, scene, Command::ApplyBipedRig);
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
    let rig_targets: Vec<(NodeId, String, bool)> = scene
        .nodes
        .values()
        .filter(|n| {
            matches!(
                n.kind,
                tessel_core::NodeKind::VoxelVolume | tessel_core::NodeKind::LowPolyMesh
            )
        })
        .map(|n| (n.id, n.name.clone(), rig.should_skin_node(n.id)))
        .collect();

    ui.label(
        RichText::new("Rig Target Objects")
            .strong()
            .size(11.5)
            .color(tokens::ON_SURFACE),
    );
    if rig_targets.is_empty() {
        ui.label(
            RichText::new("No 3D objects in scene.")
                .size(11.0)
                .color(tokens::ON_SURFACE_VARIANT),
        );
    } else {
        for (target_id, target_name, mut is_skinned) in rig_targets {
            ui.push_id(target_id, |ui| {
                if ui
                    .checkbox(&mut is_skinned, &target_name)
                    .on_hover_text("Include this object in character rig skinning deformation")
                    .changed()
                {
                    super::exec_history(
                        ui.ctx(),
                        history,
                        scene,
                        Command::SetNodeRigSkinned {
                            node_id: target_id,
                            skinned: is_skinned,
                        },
                    );
                }
            });
        }
    }
    ui.add_space(6.0);

    let selected_skinned = scene.selected_node.is_some_and(|nid| {
        scene
            .character_rig
            .as_ref()
            .is_some_and(|r| r.should_skin_node(nid))
    });

    if let Some(selected_id) = scene.selected_node {
        if let Some((node_name, is_3d_geo)) = scene.nodes.get(&selected_id).map(|node| {
            (
                node.name.clone(),
                matches!(
                    node.kind,
                    tessel_core::NodeKind::VoxelVolume | tessel_core::NodeKind::LowPolyMesh
                ),
            )
        }) {
            if is_3d_geo {
                let mut skinned_toggle = selected_skinned;
                if ui
                    .checkbox(&mut skinned_toggle, "Skinned by Rig")
                    .on_hover_text("Toggle whether the selected object is deformed by this rig")
                    .changed()
                {
                    super::exec_history(
                        ui.ctx(),
                        history,
                        scene,
                        Command::SetNodeRigSkinned {
                            node_id: selected_id,
                            skinned: skinned_toggle,
                        },
                    );
                }
                if !selected_skinned {
                    ui.label(
                        RichText::new("Excluded from rig deformation.")
                            .size(10.5)
                            .color(tokens::ON_SURFACE_VARIANT),
                    );
                    ui.add_space(6.0);
                }
            }
            if is_3d_geo && selected_skinned {
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
                        super::exec_history(
                            ui.ctx(),
                            history,
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
                            super::exec_history(
                                ui.ctx(),
                                history,
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
                        super::exec_history(
                            ui.ctx(),
                            history,
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
                            super::exec_history(
                                ui.ctx(),
                                history,
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
    }

    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Pose Editor (Degrees)")
                .size(11.0)
                .color(tokens::ON_SURFACE_VARIANT),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if icons::hud_icon_button(ui, ToolIcon::Reset, "Reset all bones to rest pose", false) {
                super::exec_history(ui.ctx(), history, scene, Command::ResetCharacterPose);
            }
            if icons::text_icon_button(ui, "Floor", "Clamp rig to floor (y=0)", false) {
                if let Some(rig_ref) = &scene.character_rig {
                    let mut clamped = rig_ref.clone();
                    let bounds = tessel_core::SkinnedBoundsCache::from_scene(scene, rig_ref);
                    if clamped
                        .clamp_sub_skeleton_to_floor_with_bounds("", 0.0, bounds.as_ref())
                        .is_none()
                    {
                        super::push_ui_error(ui.ctx(), "Floor clamp found no bones to adjust.");
                    } else if let Some(root) = clamped.bones.first() {
                        super::exec_history(
                            ui.ctx(),
                            history,
                            scene,
                            Command::SetBoneTranslation {
                                bone_id: root.id,
                                translation: root.pose_local.translation,
                            },
                        );
                    }
                }
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
                    super::exec_history(
                        ui.ctx(),
                        history,
                        scene,
                        Command::SetAnimationKeyframe {
                            clip_index: active_clip,
                            bone_id,
                            keyframe: Keyframe::rotation_only(playback_time, delta_rot),
                        },
                    );
                }
                if icons::hud_icon_button(ui, ToolIcon::Reset, "Reset bone pose", false) {
                    super::exec_history(
                        ui.ctx(),
                        history,
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
        let mut dragging = false;
        let mut drag_stopped = false;
        for resp in [
            ui.add(egui::Slider::new(&mut rx_deg, -180.0..=180.0).text("Pitch")),
            ui.add(egui::Slider::new(&mut ry_deg, -180.0..=180.0).text("Yaw")),
            ui.add(egui::Slider::new(&mut rz_deg, -180.0..=180.0).text("Roll")),
        ] {
            changed |= resp.changed();
            dragging |= resp.dragged();
            drag_stopped |= resp.drag_stopped();
        }
        if changed || drag_stopped {
            let next_delta = glam::Quat::from_euler(
                glam::EulerRot::XYZ,
                rx_deg.to_radians(),
                ry_deg.to_radians(),
                rz_deg.to_radians(),
            );
            super::commit_debounced_command(
                ui.ctx(),
                egui::Id::new(("bone_pose_debounce", bone_id)),
                history,
                scene,
                changed,
                dragging,
                drag_stopped,
                |s| {
                    s.character_rig
                        .as_ref()
                        .and_then(|r| r.bone(bone_id))
                        .map(|b| Command::SetBoneRotation {
                            bone_id,
                            rotation: b.pose_local.rotation,
                        })
                },
                Command::SetBoneRotation {
                    bone_id,
                    rotation: rest_rot * next_delta,
                },
            );
        }

        ui.add_space(6.0);
        ui.label(
            RichText::new("Stick Gizmo (Octahedral)")
                .strong()
                .size(12.0)
                .color(tokens::ON_SURFACE),
        );
        ui.horizontal(|ui| {
            ui.checkbox(&mut scene.stick_gizmo.enabled, "Octahedral");
            ui.checkbox(&mut scene.stick_gizmo.mirror_lr, "Mirror L/R");
        });
        ui.horizontal_wrapped(|ui| {
            for (mode, lbl) in [
                (tessel_core::StickGizmoMode::Roll, "Roll"),
                (tessel_core::StickGizmoMode::Pitch, "Pitch"),
                (tessel_core::StickGizmoMode::Yaw, "Yaw"),
                (tessel_core::StickGizmoMode::Twist, "Twist"),
                (tessel_core::StickGizmoMode::RollTwist, "R+T"),
            ] {
                if ui
                    .selectable_label(scene.stick_gizmo.mode == mode, lbl)
                    .clicked()
                {
                    scene.stick_gizmo.mode = mode;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Snap:")
                    .size(11.0)
                    .color(tokens::ON_SURFACE_VARIANT),
            );
            for (snap, lbl) in [(0.0f32, "Off"), (5.0, "5°"), (15.0, "15°"), (45.0, "45°")] {
                if ui
                    .selectable_label((scene.stick_gizmo.snap_deg - snap).abs() < 1e-3, lbl)
                    .clicked()
                {
                    scene.stick_gizmo.snap_deg = snap;
                }
            }
        });
        ui.horizontal(|ui| {
            let step = if scene.stick_gizmo.snap_deg > 0.0 {
                scene.stick_gizmo.snap_deg
            } else {
                15.0
            };
            for (lbl, delta_deg) in [
                (&format!("-{step:.0}°"), -step),
                (&format!("+{step:.0}°"), step),
            ] {
                if ui.small_button(lbl).clicked() {
                    let params = tessel_core::StickGizmoParams {
                        bone_id,
                        mode: scene.stick_gizmo.mode,
                        angle_deg: delta_deg,
                        chain_len: scene.stick_gizmo.twist_chain_len,
                        snap_deg: scene.stick_gizmo.snap_deg,
                        mirror_lr: scene.stick_gizmo.mirror_lr,
                        axis_lock: scene.stick_gizmo.axis_lock,
                    };
                    super::exec_history(
                        ui.ctx(),
                        history,
                        scene,
                        Command::ApplyStickGizmoPose { params },
                    );
                }
            }
        });
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

const MAX_OUTLINER_DEPTH: usize = 16;
const OUTLINER_INDENT_PX: f32 = 12.0;
const MAX_OUTLINER_INDENT_PX: f32 = 72.0;

#[derive(Debug, Clone, PartialEq, Eq)]
struct OutlinerRow {
    id: NodeId,
    name: String,
    kind: tessel_core::NodeKind,
    visible: bool,
    depth: usize,
    has_children: bool,
    collapsed: bool,
}

struct OutlinerTreeBuilder<'a> {
    scene: &'a SceneState,
    children_by_parent: BTreeMap<NodeId, Vec<NodeId>>,
    collapsed: &'a BTreeSet<NodeId>,
    visited: BTreeSet<NodeId>,
    rows: Vec<OutlinerRow>,
}

impl OutlinerTreeBuilder<'_> {
    fn visit(&mut self, node_id: NodeId, depth: usize, emit: bool) {
        if !self.visited.insert(node_id) {
            return;
        }
        let Some(node) = self.scene.nodes.get(&node_id) else {
            return;
        };
        let children = self
            .children_by_parent
            .get(&node_id)
            .cloned()
            .unwrap_or_default();
        let has_children = children.iter().any(|cid| !self.visited.contains(cid));
        let collapsed = has_children && self.collapsed.contains(&node_id);

        if emit {
            self.rows.push(OutlinerRow {
                id: node.id,
                name: node.name.clone(),
                kind: node.kind,
                visible: node.visible,
                depth,
                has_children,
                collapsed,
            });
        }

        if depth >= MAX_OUTLINER_DEPTH {
            return;
        }

        let emit_children = emit && !collapsed;
        for child_id in children {
            self.visit(child_id, depth + 1, emit_children);
        }
    }
}

fn build_outliner_tree_rows(scene: &SceneState, collapsed: &BTreeSet<NodeId>) -> Vec<OutlinerRow> {
    let mut children_by_parent: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    let mut roots: Vec<NodeId> = Vec::new();

    for (&id, node) in &scene.nodes {
        match node.parent {
            Some(pid) if pid != id && scene.nodes.contains_key(&pid) => {
                children_by_parent.entry(pid).or_default().push(id);
            }
            _ => roots.push(id),
        }
    }

    let mut builder = OutlinerTreeBuilder {
        scene,
        children_by_parent,
        collapsed,
        visited: BTreeSet::new(),
        rows: Vec::with_capacity(scene.nodes.len()),
    };

    for root_id in roots {
        builder.visit(root_id, 0, true);
    }

    // Fallback pass: surface any remaining unvisited nodes (closed parent cycles or chains beyond
    // MAX_OUTLINER_DEPTH) safely at root depth without dropping any node.
    for &id in scene.nodes.keys() {
        if !builder.visited.contains(&id) {
            builder.visit(id, 0, true);
        }
    }

    builder.rows
}

fn is_node_or_descendant(scene: &SceneState, candidate_id: NodeId, node_id: NodeId) -> bool {
    let mut curr = Some(candidate_id);
    let mut visited = BTreeSet::new();
    for _ in 0..MAX_OUTLINER_DEPTH {
        let Some(id) = curr else {
            return false;
        };
        if id == node_id || !visited.insert(id) {
            return true;
        }
        curr = scene.nodes.get(&id).and_then(|n| n.parent);
    }
    curr.is_some()
}

fn valid_parent_candidates(scene: &SceneState, node_id: NodeId) -> Vec<(NodeId, String)> {
    scene
        .nodes
        .iter()
        .filter(|(&cand_id, _)| !is_node_or_descendant(scene, cand_id, node_id))
        .map(|(&cand_id, node)| (cand_id, node.name.clone()))
        .collect()
}

fn outliner_text_button(ui: &mut egui::Ui, label: &str, tooltip: &str) -> bool {
    let btn = egui::Button::new(
        RichText::new(label)
            .size(11.0)
            .color(tokens::ON_SURFACE_VARIANT),
    )
    .min_size(egui::vec2(48.0, 20.0))
    .fill(egui::Color32::TRANSPARENT)
    .corner_radius(CornerRadius::same(3));
    ui.add(btn).on_hover_text(tooltip).clicked()
}

fn create_outliner_group_command(scene: &SceneState) -> Command {
    let parent = scene.selected_node.filter(|id| {
        scene
            .nodes
            .get(id)
            .is_some_and(|n| n.kind == tessel_core::NodeKind::Group)
    });
    Command::CreateGroupNode {
        name: next_outliner_node_name(scene, "Group"),
        transform: tessel_core::Transform::default(),
        parent,
    }
}

fn outliner_icon_button(
    ui: &mut egui::Ui,
    icon: ToolIcon,
    tooltip: &str,
    active: bool,
    danger: bool,
) -> bool {
    let ink = if danger {
        tokens::ERROR
    } else if active {
        tokens::PRIMARY
    } else {
        tokens::ON_SURFACE_VARIANT
    };
    let fill = if active {
        tokens::SURFACE_HIGHEST
    } else {
        egui::Color32::TRANSPARENT
    };
    let btn = egui::Button::new(RichText::new(icon.glyph()).size(13.0).color(ink))
        .min_size(egui::vec2(22.0, 20.0))
        .fill(fill)
        .corner_radius(CornerRadius::same(3));
    ui.add(btn).on_hover_text(tooltip).clicked()
}

fn next_outliner_node_name(scene: &SceneState, base: &str) -> String {
    if !scene.nodes.values().any(|n| n.name == base) {
        return base.to_string();
    }
    let mut idx = 2usize;
    loop {
        let candidate = format!("{base} {idx}");
        if !scene.nodes.values().any(|n| n.name == candidate) {
            return candidate;
        }
        idx = idx.saturating_add(1);
    }
}

fn outliner_add_tooltip(scene: &SceneState) -> &'static str {
    match scene.active_mode {
        AppMode::Pixel => "Add Pixel Canvas",
        AppMode::Voxel => "Add Voxel Volume",
        AppMode::LowPoly => "Add Low-Poly Mesh",
        AppMode::Character => {
            if !scene.voxel_volumes.is_empty() && scene.lowpoly_meshes.is_empty() {
                "Add Voxel Volume"
            } else {
                "Add Low-Poly Mesh"
            }
        }
    }
}

fn create_outliner_node_command(scene: &SceneState) -> Command {
    match scene.active_mode {
        AppMode::Pixel => Command::CreatePixelCanvas {
            name: next_outliner_node_name(scene, "Sprite 32x32"),
            width: 32,
            height: 32,
        },
        AppMode::Voxel => Command::CreateVoxelVolume {
            name: next_outliner_node_name(scene, "Voxel Volume"),
        },
        AppMode::LowPoly => Command::CreatePrimitiveMesh {
            name: next_outliner_node_name(scene, "Mesh"),
            kind: tessel_core::PrimitiveKind::Cube,
            size: 0.8,
            material: scene.active_color,
        },
        AppMode::Character => {
            if !scene.voxel_volumes.is_empty() && scene.lowpoly_meshes.is_empty() {
                Command::CreateVoxelVolume {
                    name: next_outliner_node_name(scene, "Voxel Volume"),
                }
            } else {
                Command::CreatePrimitiveMesh {
                    name: next_outliner_node_name(scene, "Mesh"),
                    kind: tessel_core::PrimitiveKind::Cube,
                    size: 0.8,
                    material: scene.active_color,
                }
            }
        }
    }
}

fn show_outliner_section(
    ui: &mut egui::Ui,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    outliner_rename: &mut OutlinerRenameState,
) {
    outliner_rename
        .collapsed_nodes
        .retain(|id| scene.nodes.contains_key(id));

    // Drop stale rename state immediately if the target node was deleted externally.
    if outliner_rename
        .active
        .as_ref()
        .is_some_and(|(rid, _)| !scene.nodes.contains_key(rid))
    {
        outliner_rename.clear_rename();
    }

    let header_width = ui.available_width().max(96.0);
    let add_tooltip = outliner_add_tooltip(scene);
    ui.allocate_ui_with_layout(
        egui::vec2(header_width, 22.0),
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            if outliner_icon_button(ui, ToolIcon::Add, add_tooltip, false, false) {
                let add_cmd = create_outliner_node_command(scene);
                super::exec_history(ui.ctx(), history, scene, add_cmd);
            }
            if outliner_text_button(ui, "+ Group", "Add Group") {
                let group_cmd = create_outliner_group_command(scene);
                super::exec_history(ui.ctx(), history, scene, group_cmd);
            }
            let title_width = ui.available_width().max(24.0);
            ui.allocate_ui_with_layout(
                egui::vec2(title_width, 22.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.label(
                        RichText::new("Scene Outliner")
                            .strong()
                            .size(13.0)
                            .color(tokens::ON_SURFACE),
                    );
                },
            );
        },
    );
    ui.add_space(4.0);

    if scene.nodes.is_empty() {
        outliner_rename.clear();
        ui.label(
            RichText::new("Scene is empty.")
                .size(11.5)
                .color(tokens::ON_SURFACE_VARIANT),
        );
        return;
    }

    let rows = build_outliner_tree_rows(scene, &outliner_rename.collapsed_nodes);
    for row in rows {
        let id = row.id;
        let selected = scene.selected_node == Some(id);
        let is_renaming = outliner_rename
            .active
            .as_ref()
            .is_some_and(|(rid, _)| *rid == id);
        let row_width = ui.available_width().max(96.0);
        ui.allocate_ui_with_layout(
            egui::vec2(row_width, 24.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                // Allocate compact right-aligned action buttons first inside a fixed 24px row
                // so long node names never overlap icons and vertical padding stays tight.
                if outliner_icon_button(ui, ToolIcon::Close, "Delete node", false, true) {
                    if outliner_rename
                        .active
                        .as_ref()
                        .is_some_and(|(rid, _)| *rid == id)
                    {
                        outliner_rename.clear_rename();
                    }
                    super::exec_history(ui.ctx(), history, scene, Command::DeleteNode(id));
                }
                if outliner_icon_button(ui, ToolIcon::Pencil, "Rename node", is_renaming, false) {
                    if is_renaming {
                        if let Some((prev_id, draft)) = outliner_rename.active.take() {
                            rename_node_if_changed(scene, history, prev_id, &draft);
                        }
                        outliner_rename.request_focus = false;
                    } else {
                        // Commit any in-progress rename on another row before switching.
                        if let Some((prev_id, prev_draft)) = outliner_rename.active.take() {
                            rename_node_if_changed(scene, history, prev_id, &prev_draft);
                        }
                        outliner_rename.active = Some((id, row.name.clone()));
                        outliner_rename.request_focus = true;
                    }
                }
                let (vis_icon, vis_tip) = if row.visible {
                    (ToolIcon::EyeOpen, "Hide node")
                } else {
                    (ToolIcon::EyeClosed, "Show node")
                };
                if outliner_icon_button(ui, vis_icon, vis_tip, !row.visible, false) {
                    super::exec_history(
                        ui.ctx(),
                        history,
                        scene,
                        Command::SetNodeVisibility {
                            node_id: id,
                            visible: !row.visible,
                        },
                    );
                }

                ui.add_space(4.0);
                let text_width = ui.available_width().max(24.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(text_width, 22.0),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_max_width(text_width);
                        let indent =
                            (row.depth as f32 * OUTLINER_INDENT_PX).min(MAX_OUTLINER_INDENT_PX);
                        if indent > 0.0 {
                            ui.add_space(indent);
                        }
                        if row.has_children {
                            let (glyph, tip) = if row.collapsed {
                                ("▸", "Expand children")
                            } else {
                                ("▾", "Collapse children")
                            };
                            let toggle_btn = egui::Button::new(
                                RichText::new(glyph)
                                    .size(11.0)
                                    .color(tokens::ON_SURFACE_VARIANT),
                            )
                            .min_size(egui::vec2(14.0, 18.0))
                            .fill(Color32::TRANSPARENT)
                            .corner_radius(CornerRadius::same(2));
                            if ui.add(toggle_btn).on_hover_text(tip).clicked() {
                                if row.collapsed {
                                    outliner_rename.collapsed_nodes.remove(&id);
                                } else {
                                    outliner_rename.collapsed_nodes.insert(id);
                                }
                            }
                        }
                        if is_renaming {
                            if let Some((_, ref mut draft)) = outliner_rename.active {
                                let edit_width = ui.available_width().max(24.0);
                                let resp = ui.add(
                                    egui::TextEdit::singleline(draft)
                                        .desired_width(edit_width)
                                        .hint_text("Node name"),
                                );
                                if outliner_rename.request_focus {
                                    resp.request_focus();
                                    outliner_rename.request_focus = false;
                                }
                                let enter = resp.lost_focus()
                                    && ui.input(|i| i.key_pressed(egui::Key::Enter));
                                let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                                if escape {
                                    outliner_rename.clear_rename();
                                } else if enter || resp.lost_focus() {
                                    let committed = draft.clone();
                                    rename_node_if_changed(scene, history, id, &committed);
                                    outliner_rename.clear_rename();
                                }
                            }
                        } else {
                            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                            let label_text = format!("{} ({:?})", row.name, row.kind);
                            let text = if row.visible {
                                RichText::new(&label_text)
                            } else {
                                RichText::new(&label_text).color(tokens::ON_SURFACE_VARIANT)
                            };
                            let resp = ui
                                .selectable_label(selected, text)
                                .on_hover_text(&label_text);
                            if resp.clicked() && !selected {
                                super::exec_history(
                                    ui.ctx(),
                                    history,
                                    scene,
                                    Command::SelectNode(Some(id)),
                                );
                            }
                        }
                    },
                );
            },
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn show_transform_and_lighting_section(
    ui: &mut egui::Ui,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    show_viewport_gizmo: &mut bool,
    gizmo_mode: &mut tessel_render::TransformGizmoMode,
    free_view: &mut bool,
    render_preview: &mut bool,
    interaction_tool: &mut super::viewport::ViewportInteractionTool,
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
                        super::exec_history(
                            ui.ctx(),
                            history,
                            scene,
                            Command::SetNodeTransform {
                                node_id,
                                transform: tessel_core::Transform::default(),
                            },
                        );
                    }
                });
            });
            let current_parent = scene.nodes.get(&node_id).and_then(|n| n.parent);
            let current_parent_label = current_parent
                .and_then(|pid| scene.nodes.get(&pid).map(|n| n.name.as_str()))
                .unwrap_or("None (Root)")
                .to_string();
            let parent_candidates = valid_parent_candidates(scene, node_id);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Parent")
                        .size(11.0)
                        .color(tokens::ON_SURFACE_VARIANT),
                );
                egui::ComboBox::from_id_salt(("node_parent_selector", node_id))
                    .width(ui.available_width().max(96.0))
                    .selected_text(current_parent_label)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(current_parent.is_none(), "None (Root)")
                            .clicked()
                            && current_parent.is_some()
                        {
                            super::exec_history(
                                ui.ctx(),
                                history,
                                scene,
                                Command::SetNodeParent {
                                    node_id,
                                    parent: None,
                                },
                            );
                        }
                        for (cand_id, cand_name) in &parent_candidates {
                            ui.push_id(*cand_id, |ui| {
                                if ui
                                    .selectable_label(current_parent == Some(*cand_id), cand_name)
                                    .clicked()
                                    && current_parent != Some(*cand_id)
                                {
                                    super::exec_history(
                                        ui.ctx(),
                                        history,
                                        scene,
                                        Command::SetNodeParent {
                                            node_id,
                                            parent: Some(*cand_id),
                                        },
                                    );
                                }
                            });
                        }
                    });
            });
            if scene.active_mode != AppMode::Pixel {
                let mut gizmo_active = super::viewport::is_transform_gizmo_enabled(
                    scene,
                    *interaction_tool,
                    *free_view,
                    *render_preview,
                    *show_viewport_gizmo,
                );
                if ui
                    .checkbox(&mut gizmo_active, "Viewport Gizmo")
                    .on_hover_text(
                        "Show interactive Move / Rotate / Scale gizmo in the 3D viewport",
                    )
                    .changed()
                {
                    if gizmo_active {
                        super::viewport::activate_transform_gizmo(
                            scene,
                            show_viewport_gizmo,
                            free_view,
                            render_preview,
                            interaction_tool,
                        );
                    } else {
                        *show_viewport_gizmo = false;
                    }
                }
                if gizmo_active {
                    ui.horizontal(|ui| {
                        for (mode, label) in [
                            (tessel_render::TransformGizmoMode::Move, "Move"),
                            (tessel_render::TransformGizmoMode::Rotate, "Rotate"),
                            (tessel_render::TransformGizmoMode::Scale, "Scale"),
                        ] {
                            if ui.radio_value(gizmo_mode, mode, label).clicked() {
                                super::viewport::activate_transform_gizmo(
                                    scene,
                                    show_viewport_gizmo,
                                    free_view,
                                    render_preview,
                                    interaction_tool,
                                );
                            }
                        }
                    });
                }
                if scene.active_mode != AppMode::Character
                    && scene.nodes.get(&node_id).is_some_and(|n| {
                        matches!(
                            n.kind,
                            tessel_core::NodeKind::VoxelVolume | tessel_core::NodeKind::LowPolyMesh
                        )
                    })
                {
                    if let Some(mut skinned) = scene
                        .character_rig
                        .as_ref()
                        .map(|r| r.should_skin_node(node_id))
                    {
                        if ui
                            .checkbox(&mut skinned, "Skinned by Rig")
                            .on_hover_text(
                                "Toggle whether this object is deformed by the active character rig",
                            )
                            .changed()
                        {
                            super::exec_history(
                                ui.ctx(),
                                history,
                                scene,
                                Command::SetNodeRigSkinned { node_id, skinned },
                            );
                        }
                    }
                }
            }

            let mut changed = false;
            let mut dragging = false;
            let mut drag_stopped = false;
            let mut track = |r: egui::Response| {
                changed |= r.changed();
                dragging |= r.dragged();
                drag_stopped |= r.drag_stopped();
            };
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Pos")
                        .size(11.0)
                        .color(tokens::ON_SURFACE_VARIANT),
                );
                track(
                    ui.add(
                        egui::DragValue::new(&mut tx.translation.x)
                            .speed(0.05)
                            .prefix("X "),
                    ),
                );
                track(
                    ui.add(
                        egui::DragValue::new(&mut tx.translation.y)
                            .speed(0.05)
                            .prefix("Y "),
                    ),
                );
                track(
                    ui.add(
                        egui::DragValue::new(&mut tx.translation.z)
                            .speed(0.05)
                            .prefix("Z "),
                    ),
                );
            });
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Rot")
                        .size(11.0)
                        .color(tokens::ON_SURFACE_VARIANT),
                );
                track(ui.add(egui::DragValue::new(&mut rx_deg).speed(1.0).prefix("X ")));
                track(ui.add(egui::DragValue::new(&mut ry_deg).speed(1.0).prefix("Y ")));
                track(ui.add(egui::DragValue::new(&mut rz_deg).speed(1.0).prefix("Z ")));
            });
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("Scale")
                        .size(11.0)
                        .color(tokens::ON_SURFACE_VARIANT),
                );
                track(
                    ui.add(
                        egui::DragValue::new(&mut tx.scale.x)
                            .speed(0.02)
                            .range(0.005..=100.0)
                            .prefix("X "),
                    ),
                );
                track(
                    ui.add(
                        egui::DragValue::new(&mut tx.scale.y)
                            .speed(0.02)
                            .range(0.005..=100.0)
                            .prefix("Y "),
                    ),
                );
                track(
                    ui.add(
                        egui::DragValue::new(&mut tx.scale.z)
                            .speed(0.02)
                            .range(0.005..=100.0)
                            .prefix("Z "),
                    ),
                );
            });

            if changed || drag_stopped {
                rx = rx_deg.to_radians();
                ry = ry_deg.to_radians();
                rz = rz_deg.to_radians();
                tx.rotation = glam::Quat::from_euler(glam::EulerRot::XYZ, rx, ry, rz);
                super::commit_debounced_command(
                    ui.ctx(),
                    egui::Id::new(("node_transform_debounce", node_id)),
                    history,
                    scene,
                    changed,
                    dragging,
                    drag_stopped,
                    |s| {
                        s.nodes.get(&node_id).map(|n| Command::SetNodeTransform {
                            node_id,
                            transform: n.transform,
                        })
                    },
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
                            super::exec_history(
                                ui.ctx(),
                                history,
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
                let mut dragging = false;
                let mut drag_stopped = false;
                for r in [
                    ui.add(egui::Slider::new(&mut mat.metallic, 0.0..=1.0).text("Metallic")),
                    ui.add(egui::Slider::new(&mut mat.roughness, 0.05..=1.0).text("Roughness")),
                ] {
                    changed |= r.changed();
                    dragging |= r.dragged();
                    drag_stopped |= r.drag_stopped();
                }
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Emissive Glow")
                            .size(11.0)
                            .color(tokens::ON_SURFACE_VARIANT),
                    );
                    changed |= ui.color_edit_button_rgb(&mut mat.emissive).changed();
                });
                changed |= ui.checkbox(&mut mat.double_sided, "Double Sided").changed();
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut mat.cast_shadow, "Cast Shadow").changed();
                    changed |= ui.checkbox(&mut mat.receive_shadow, "Receive").changed();
                });
                changed |= ui
                    .checkbox(&mut mat.unlit, "Unlit (KHR_materials_unlit)")
                    .changed();
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Alpha Mode")
                            .size(11.0)
                            .color(tokens::ON_SURFACE_VARIANT),
                    );
                    let is_opaque = matches!(mat.alpha_mode, tessel_core::AlphaMode::Opaque);
                    if ui.selectable_label(is_opaque, "Opaque").clicked() && !is_opaque {
                        mat.alpha_mode = tessel_core::AlphaMode::Opaque;
                        changed = true;
                    }
                    let is_mask = matches!(mat.alpha_mode, tessel_core::AlphaMode::Mask { .. });
                    if ui.selectable_label(is_mask, "Mask").clicked() && !is_mask {
                        mat.alpha_mode = tessel_core::AlphaMode::Mask { cutoff: 0.5 };
                        changed = true;
                    }
                    let is_blend = matches!(mat.alpha_mode, tessel_core::AlphaMode::Blend);
                    if ui.selectable_label(is_blend, "Blend").clicked() && !is_blend {
                        mat.alpha_mode = tessel_core::AlphaMode::Blend;
                        changed = true;
                    }
                });
                if let tessel_core::AlphaMode::Mask { ref mut cutoff } = mat.alpha_mode {
                    let r = ui.add(egui::Slider::new(cutoff, 0.0..=1.0).text("Cutoff"));
                    changed |= r.changed();
                    dragging |= r.dragged();
                    drag_stopped |= r.drag_stopped();
                }
                if changed || drag_stopped {
                    super::commit_debounced_command(
                        ui.ctx(),
                        egui::Id::new(("mesh_material_debounce", node_id)),
                        history,
                        scene,
                        changed,
                        dragging,
                        drag_stopped,
                        |s| {
                            s.lowpoly_meshes
                                .get(&node_id)
                                .map(|m| Command::SetMeshMaterial {
                                    node_id,
                                    material: m.material,
                                })
                        },
                        Command::SetMeshMaterial {
                            node_id,
                            material: mat,
                        },
                    );
                }
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    if icons::text_icon_button(ui, "Bake AO", "Bake CPU AO map (v4.2)", false) {
                        if let Some(mesh) = scene.lowpoly_meshes.get(&node_id) {
                            if let Ok(baked) = tessel_core::bake_mesh_maps(
                                mesh,
                                &tessel_core::BakeParams::default(),
                            ) {
                                super::exec_history(
                                    ui.ctx(),
                                    history,
                                    scene,
                                    Command::SetMeshAoMap {
                                        node_id,
                                        ao_map: baked.ao,
                                    },
                                );
                            }
                        }
                    }
                    if icons::text_icon_button(
                        ui,
                        "Smart Mat",
                        "Generate procedural edge-wear/dirt texture (v4.4)",
                        false,
                    ) {
                        if let Some(mesh) = scene.lowpoly_meshes.get(&node_id) {
                            let base_rgba = scene
                                .palette
                                .get(scene.active_color)
                                .map_or(tessel_core::Rgba8::rgba(180, 180, 180, 255), |e| e.color);
                            let params = tessel_core::SmartMaterialParams::from_base(base_rgba);
                            if let Ok(out) =
                                tessel_core::generate_smart_material_texture(mesh, &params)
                            {
                                super::exec_history(
                                    ui.ctx(),
                                    history,
                                    scene,
                                    Command::SetMeshTexture {
                                        node_id,
                                        texture: Some(out.albedo),
                                    },
                                );
                            }
                        }
                    }
                    if icons::text_icon_button(
                        ui,
                        "Validate",
                        "Check mesh topology (sliver/non-manifold, v5.2)",
                        false,
                    ) {
                        if let Some(mesh) = scene.lowpoly_meshes.get(&node_id) {
                            let report = tessel_core::validate_mesh_topology(mesh);
                            let msg = if report.is_clean {
                                format!("Topology OK ({} tris)", report.triangle_count)
                            } else {
                                format!(
                                    "Issues: {} sliver, {} non-manifold, {} deg-norm",
                                    report.sliver_triangles,
                                    report.non_manifold_edges,
                                    report.degenerate_normals
                                )
                            };
                            ui.ctx().data_mut(|d| {
                                d.insert_temp(egui::Id::new(("topo_report", node_id)), msg);
                            });
                        }
                    }
                });
                if let Some(msg) = ui
                    .ctx()
                    .data(|d| d.get_temp::<String>(egui::Id::new(("topo_report", node_id))))
                {
                    ui.label(
                        RichText::new(msg)
                            .size(10.5)
                            .color(tokens::ON_SURFACE_VARIANT),
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
                                super::exec_history(
                                    ui.ctx(),
                                    history,
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
            let morph_entries: Vec<(String, f32)> = scene
                .lowpoly_meshes
                .get(&node_id)
                .map(|m| {
                    m.morphs
                        .iter()
                        .map(|t| (t.name.clone(), t.default_weight))
                        .collect()
                })
                .unwrap_or_default();
            if !morph_entries.is_empty() {
                ui.label(
                    RichText::new("Morph Targets / Blend Shapes")
                        .strong()
                        .size(12.0)
                        .color(tokens::ON_SURFACE),
                );
                for (m_name, cur_w) in morph_entries {
                    let mut w = cur_w;
                    ui.horizontal(|ui| {
                        let resp = ui.add(egui::Slider::new(&mut w, 0.0..=1.0).text(&m_name));
                        if resp.changed() || resp.drag_stopped() {
                            let target_name = m_name.clone();
                            super::commit_debounced_command(
                                ui.ctx(),
                                egui::Id::new(("morph_weight_debounce", node_id, &m_name)),
                                history,
                                scene,
                                resp.changed(),
                                resp.dragged(),
                                resp.drag_stopped(),
                                |_| {
                                    Some(Command::SetMorphDefaultWeight {
                                        node_id,
                                        name: target_name.clone(),
                                        weight: cur_w,
                                    })
                                },
                                Command::SetMorphDefaultWeight {
                                    node_id,
                                    name: m_name.clone(),
                                    weight: w,
                                },
                            );
                        }
                        if ui
                            .small_button("Key")
                            .on_hover_text("Keyframe morph weight")
                            .clicked()
                        {
                            if let Some(rig) = &scene.character_rig {
                                let clip_index = rig.active_animation;
                                let time_secs = rig.playback_time;
                                super::exec_history(
                                    ui.ctx(),
                                    history,
                                    scene,
                                    Command::SetMorphWeightKeyframe {
                                        clip_index,
                                        node_id,
                                        target: m_name.clone(),
                                        time_secs,
                                        weight: w,
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
        let r_az = ui.add(egui::Slider::new(&mut az_deg, -180.0..=180.0).text("Azimuth"));
        let r_el = ui.add(egui::Slider::new(&mut el_deg, 5.0..=89.0).text("Elevation"));
        let changed = r_az.changed() || r_el.changed();
        let dragging = r_az.dragged() || r_el.dragged();
        let drag_stopped = r_az.drag_stopped() || r_el.drag_stopped();
        if changed || drag_stopped {
            super::commit_debounced_command(
                ui.ctx(),
                egui::Id::new("studio_lighting_debounce"),
                history,
                scene,
                changed,
                dragging,
                drag_stopped,
                |s| {
                    Some(Command::SetLightDirection {
                        azimuth: s.light_azimuth,
                        elevation: s.light_elevation,
                    })
                },
                Command::SetLightDirection {
                    azimuth: az_deg.to_radians(),
                    elevation: el_deg.to_radians(),
                },
            );
        }
    }
}

fn apply_provider_selection(
    ai_state: &mut DirectAiUiState,
    settings: &crate::settings::AppSettings,
    provider: AiProviderKind,
) {
    let profile = settings.profile_for(provider);
    ai_state.config.provider = provider;
    ai_state.config.model = profile.model;
    ai_state.config.base_url = profile.base_url;
    ai_state.config.api_key = crate::settings::load_api_key_from_keyring(provider);
}

fn show_direct_ai_section(
    ui: &mut egui::Ui,
    scene: &mut SceneState,
    history: &mut HistoryStack,
    ai_state: &mut DirectAiUiState,
    allowed_export_dir: &Path,
    settings: &crate::settings::AppSettings,
) {
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
                        apply_provider_selection(ai_state, settings, provider);
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
            ai_state.pending_confirm_calls.clear();
            match direct_ai_runtime() {
                Ok(rt) => {
                    let (tx, rx) = mpsc::channel();
                    let config = ai_state.config.clone();
                    let scene_clone = scene.clone();
                    let prompt = ai_state.prompt.clone();
                    let handle = rt.spawn(async move {
                        let res = tessel_ai::direct::send_prompt(&config, &scene_clone, &prompt)
                            .await
                            .map_err(|e| e.to_string());
                        let _ = tx.send(res);
                    });
                    ai_state.cancel_handle = Some(handle.abort_handle());
                    ai_state.pending = Some(rx);
                }
                Err(err) => {
                    ai_state.last_result = Some(Err(err));
                }
            }
        }
        if busy && icons::hud_icon_button(ui, ToolIcon::Close, "Cancel", false) {
            if let Some(handle) = ai_state.cancel_handle.take() {
                handle.abort();
            }
            ai_state.pending = None;
            ai_state.last_result = Some(Err("Direct AI request cancelled".into()));
        }
    });

    if !ai_state.pending_confirm_calls.is_empty() {
        ui.add_space(4.0);
        let names: Vec<String> = ai_state
            .pending_confirm_calls
            .iter()
            .map(|c| {
                let arg_str = c.arguments.to_string();
                let preview: String = arg_str.chars().take(48).collect();
                format!("{}({preview})", c.name)
            })
            .collect();
        ui.label(
            RichText::new(format!(
                "Confirm destructive/file tool(s): {}",
                names.join(", ")
            ))
            .size(11.0)
            .color(tokens::ERROR),
        );
        ui.horizontal(|ui| {
            if ui.small_button("Approve").clicked() {
                ai_state.approve_pending_confirm_calls(scene, history, allowed_export_dir);
            }
            if ui.small_button("Reject").clicked() {
                ai_state.pending_confirm_calls.clear();
                ai_state.last_result =
                    Some(Ok("Rejected pending destructive/file tool calls.".into()));
            }
        });
    }

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
                        let mut rename_state = OutlinerRenameState::default();
                        show_outliner_section(ui, &mut scene, &mut history, &mut rename_state);
                    });
                });
        });
        assert!(!out.shapes.is_empty());
    }

    #[test]
    fn c1_gui_parity_controls_render_and_dispatch_commands() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let default_material = scene.active_color;
        history
            .execute(
                &mut scene,
                Command::CreateTerrainMesh {
                    name: "ParityTerrain".into(),
                    params: tessel_core::TerrainParams {
                        width: 8.0,
                        depth: 8.0,
                        subdiv_x: 8,
                        subdiv_z: 8,
                        height_scale: 1.5,
                        seed: 42,
                        octaves: 3,
                        frequency: 0.35,
                        island_falloff: 0.3,
                        heights: None,
                        palette_bands: Vec::new(),
                        default_material,
                        smooth_shading: false,
                    },
                    transform: tessel_core::Transform::default(),
                },
            )
            .unwrap();
        history
            .execute(&mut scene, Command::SetMode(AppMode::LowPoly))
            .unwrap();
        history
            .execute(
                &mut scene,
                Command::SetShadingMode(tessel_core::ViewportShadingMode::NprSumie),
            )
            .unwrap();
        assert_eq!(
            scene.shading_mode,
            tessel_core::ViewportShadingMode::NprSumie
        );

        let ctx = egui::Context::default();
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 720.0),
            )),
            ..Default::default()
        };
        let out = ctx.run(raw_input, |ctx| {
            egui::SidePanel::right("test_inspector_c1")
                .exact_width(256.0)
                .resizable(false)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let mut show_gizmo = true;
                        let mut gizmo_mode = tessel_render::TransformGizmoMode::Move;
                        let mut free_view = false;
                        let mut render_preview = false;
                        let mut tool = crate::ui::viewport::ViewportInteractionTool::Select;
                        show_transform_and_lighting_section(
                            ui,
                            &mut scene,
                            &mut history,
                            &mut show_gizmo,
                            &mut gizmo_mode,
                            &mut free_view,
                            &mut render_preview,
                            &mut tool,
                        );
                    });
                });
        });
        assert!(!out.shapes.is_empty());
    }

    fn find_text_center_in_shape(shape: &egui::epaint::Shape, target: &str) -> Option<egui::Pos2> {
        match shape {
            egui::epaint::Shape::Text(t) if t.galley.job.text == target => {
                Some(t.pos + t.galley.size() * 0.5)
            }
            egui::epaint::Shape::Vec(v) => {
                v.iter().find_map(|s| find_text_center_in_shape(s, target))
            }
            _ => None,
        }
    }

    fn find_label_center(
        shapes: &[egui::epaint::ClippedShape],
        target: &str,
    ) -> Option<egui::Pos2> {
        shapes
            .iter()
            .find_map(|cs| find_text_center_in_shape(&cs.shape, target))
    }

    fn click_at(ctx: &egui::Context, pos: egui::Pos2, mut draw: impl FnMut(&egui::Context)) {
        let press = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 720.0),
            )),
            events: vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            ..Default::default()
        };
        let _ = ctx.run(press, &mut draw);
        let release = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 720.0),
            )),
            events: vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let _ = ctx.run(release, &mut draw);
    }

    #[test]
    fn c1_click_simulation_bake_smart_mat_validate_and_floor() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        history
            .execute(
                &mut scene,
                Command::CreatePrimitiveMesh {
                    name: "ClickCube".into(),
                    kind: tessel_core::PrimitiveKind::Cube,
                    size: 1.0,
                    material: PaletteIndex::new(1).unwrap(),
                },
            )
            .unwrap();
        let node_id = scene.selected_node.unwrap();
        history
            .execute(&mut scene, Command::SetMode(AppMode::LowPoly))
            .unwrap();

        let ctx = egui::Context::default();
        let base_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 720.0),
            )),
            ..Default::default()
        };
        let draw_inspector =
            |ctx: &egui::Context, scene: &mut SceneState, history: &mut HistoryStack| {
                egui::SidePanel::right("test_click_c1")
                    .exact_width(256.0)
                    .resizable(false)
                    .show(ctx, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            let mut show_gizmo = true;
                            let mut gizmo_mode = tessel_render::TransformGizmoMode::Move;
                            let mut free_view = false;
                            let mut render_preview = false;
                            let mut tool = crate::ui::viewport::ViewportInteractionTool::Select;
                            show_transform_and_lighting_section(
                                ui,
                                scene,
                                history,
                                &mut show_gizmo,
                                &mut gizmo_mode,
                                &mut free_view,
                                &mut render_preview,
                                &mut tool,
                            );
                        });
                    });
            };

        let out = ctx.run(base_input.clone(), |c| {
            draw_inspector(c, &mut scene, &mut history);
        });
        let bake_pos = find_label_center(&out.shapes, "Bake AO").unwrap();
        click_at(&ctx, bake_pos, |c| {
            draw_inspector(c, &mut scene, &mut history)
        });
        assert!(scene.lowpoly_meshes[&node_id].ao_map.is_some());

        let smart_pos = find_label_center(&out.shapes, "Smart Mat").unwrap();
        click_at(&ctx, smart_pos, |c| {
            draw_inspector(c, &mut scene, &mut history);
        });
        assert!(scene.lowpoly_meshes[&node_id].texture.is_some());

        let val_pos = find_label_center(&out.shapes, "Validate").unwrap();
        click_at(&ctx, val_pos, |c| {
            draw_inspector(c, &mut scene, &mut history)
        });
        let msg = ctx.data(|d| d.get_temp::<String>(egui::Id::new(("topo_report", node_id))));
        assert!(msg.unwrap().contains("Topology OK"));

        // Character Floor clamp click simulation
        history
            .execute(&mut scene, Command::ApplyHumanoidRig)
            .unwrap();
        let root_id = scene.character_rig.as_ref().unwrap().bones[0].id;
        history
            .execute(
                &mut scene,
                Command::SetBoneTranslation {
                    bone_id: root_id,
                    translation: glam::Vec3::new(0.0, 2.0, 0.0),
                },
            )
            .unwrap();
        let draw_rig = |ctx: &egui::Context, scene: &mut SceneState, history: &mut HistoryStack| {
            egui::SidePanel::right("test_click_floor")
                .exact_width(256.0)
                .resizable(false)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        show_character_rig_section(ui, scene, history);
                    });
                });
        };
        let rig_out = ctx.run(base_input, |c| draw_rig(c, &mut scene, &mut history));
        let floor_pos = find_label_center(&rig_out.shapes, "Floor").unwrap();
        click_at(&ctx, floor_pos, |c| draw_rig(c, &mut scene, &mut history));
        let root_y = scene.character_rig.as_ref().unwrap().bones[0]
            .pose_local
            .translation
            .y;
        assert!(root_y < 1.0, "Floor click must lower root y, got {root_y}");
    }

    #[test]
    fn direct_ai_ui_state_poll_response_executes_tools_and_surfaces_errors() {
        let dir = std::env::temp_dir().join("tessel_c2_direct_ai_ui_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let mut ai_state = DirectAiUiState::default();

        let (tx, rx) = mpsc::channel();
        ai_state.pending = Some(rx);
        tx.send(Ok(DirectAiResponse {
            reply: "Built a centered cube.".into(),
            tool_calls: vec![tessel_ai::direct::PlannedToolCall {
                name: "mesh_create_primitive".into(),
                arguments: serde_json::json!({
                    "name": "AiCube",
                    "kind": "cube",
                    "size": 1.0,
                    "anchor": "center"
                }),
            }],
        }))
        .unwrap();

        ai_state.poll_response(&mut scene, &mut history, &dir);
        assert!(ai_state.pending.is_none());
        assert_eq!(scene.lowpoly_meshes.len(), 1);
        let ok_msg = ai_state
            .last_result
            .as_ref()
            .unwrap()
            .as_ref()
            .unwrap()
            .as_str();
        assert!(ok_msg.contains("[OK] mesh_create_primitive"));
        assert!(ok_msg.contains("Built a centered cube."));

        // Error path surfaces directly to last_result without panicking or losing scene state.
        let (tx2, rx2) = mpsc::channel();
        ai_state.pending = Some(rx2);
        tx2.send(Err("provider status 401: <redacted>".into()))
            .unwrap();
        ai_state.poll_response(&mut scene, &mut history, &dir);
        assert!(matches!(
            &ai_state.last_result,
            Some(Err(msg)) if msg.contains("401")
        ));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_provider_selection_uses_saved_settings_profile() {
        let target_provider = AiProviderKind::ALL
            .into_iter()
            .find(|p| *p != AiProviderKind::Ollama)
            .unwrap();
        let mut settings = crate::settings::AppSettings::default();
        settings.set_profile(
            target_provider,
            crate::settings::ProviderProfile {
                model: "custom-model-v2".into(),
                base_url: "http://127.0.0.1:9999/v1".into(),
            },
        );
        let mut ai_state = DirectAiUiState::default();
        apply_provider_selection(&mut ai_state, &settings, target_provider);
        assert_eq!(ai_state.config.provider, target_provider);
        assert_eq!(ai_state.config.model, "custom-model-v2");
        assert_eq!(ai_state.config.base_url, "http://127.0.0.1:9999/v1");
    }

    #[test]
    fn outliner_add_button_creates_unique_mode_appropriate_nodes() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();

        // Voxel mode: adds unique VoxelVolume nodes.
        scene.active_mode = AppMode::Voxel;
        let cmd1 = create_outliner_node_command(&scene);
        history.execute(&mut scene, cmd1).unwrap();
        let cmd2 = create_outliner_node_command(&scene);
        history.execute(&mut scene, cmd2).unwrap();
        assert_eq!(scene.voxel_volumes.len(), 2);
        let names: Vec<&str> = scene.nodes.values().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"Voxel Volume"));
        assert!(names.contains(&"Voxel Volume 2"));

        // Character mode on a Voxel project adds a VoxelVolume.
        scene.active_mode = AppMode::Character;
        let cmd_ch_vox = create_outliner_node_command(&scene);
        history.execute(&mut scene, cmd_ch_vox).unwrap();
        assert_eq!(scene.voxel_volumes.len(), 3);

        // Pixel mode: adds PixelCanvas.
        scene.active_mode = AppMode::Pixel;
        let cmd_px = create_outliner_node_command(&scene);
        history.execute(&mut scene, cmd_px).unwrap();
        assert_eq!(scene.pixel_canvases.len(), 1);

        // LowPoly & Character modes (with LowPoly meshes): add LowPolyMesh.
        scene.active_mode = AppMode::LowPoly;
        let cmd_lp = create_outliner_node_command(&scene);
        history.execute(&mut scene, cmd_lp).unwrap();
        scene.active_mode = AppMode::Character;
        let cmd_ch = create_outliner_node_command(&scene);
        history.execute(&mut scene, cmd_ch).unwrap();
        assert_eq!(scene.lowpoly_meshes.len(), 2);
    }

    #[test]
    fn outliner_tree_orders_hierarchy_and_falls_back_for_orphans() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();

        history
            .execute(
                &mut scene,
                Command::CreateGroupNode {
                    name: "RootGroup".into(),
                    transform: tessel_core::Transform::default(),
                    parent: None,
                },
            )
            .unwrap();
        let root_group_id = scene.selected_node.unwrap();

        history
            .execute(
                &mut scene,
                Command::CreateGroupNode {
                    name: "SubGroup".into(),
                    transform: tessel_core::Transform::default(),
                    parent: Some(root_group_id),
                },
            )
            .unwrap();
        let sub_group_id = scene.selected_node.unwrap();

        history
            .execute(
                &mut scene,
                Command::CreatePrimitiveMesh {
                    name: "LeafCube".into(),
                    kind: tessel_core::PrimitiveKind::Cube,
                    size: 1.0,
                    material: PaletteIndex::new(1).unwrap(),
                },
            )
            .unwrap();
        let leaf_id = scene.selected_node.unwrap();
        history
            .execute(
                &mut scene,
                Command::SetNodeParent {
                    node_id: leaf_id,
                    parent: Some(sub_group_id),
                },
            )
            .unwrap();

        // Inject an orphan node pointing to a non-existent parent ID.
        let orphan_id = scene.allocate_node_id();
        scene.nodes.insert(
            orphan_id,
            tessel_core::SceneNode {
                id: orphan_id,
                name: "Orphan".into(),
                kind: tessel_core::NodeKind::Group,
                visible: true,
                parent: Some(NodeId(999_999)),
                transform: tessel_core::Transform::default(),
            },
        );

        let rows = build_outliner_tree_rows(&scene, &BTreeSet::new());
        let summary: Vec<(&str, usize, bool)> = rows
            .iter()
            .map(|r| (r.name.as_str(), r.depth, r.has_children))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("RootGroup", 0, true),
                ("SubGroup", 1, true),
                ("LeafCube", 2, false),
                ("Orphan", 0, false),
            ]
        );
    }

    #[test]
    fn outliner_tree_collapse_hides_descendants_but_keeps_siblings() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();

        history
            .execute(
                &mut scene,
                Command::CreateGroupNode {
                    name: "GroupA".into(),
                    transform: tessel_core::Transform::default(),
                    parent: None,
                },
            )
            .unwrap();
        let group_a = scene.selected_node.unwrap();

        history
            .execute(
                &mut scene,
                Command::CreateGroupNode {
                    name: "ChildA1".into(),
                    transform: tessel_core::Transform::default(),
                    parent: Some(group_a),
                },
            )
            .unwrap();
        let child_a1 = scene.selected_node.unwrap();

        history
            .execute(
                &mut scene,
                Command::CreateGroupNode {
                    name: "GrandChildA1".into(),
                    transform: tessel_core::Transform::default(),
                    parent: Some(child_a1),
                },
            )
            .unwrap();

        history
            .execute(
                &mut scene,
                Command::CreateGroupNode {
                    name: "SiblingB".into(),
                    transform: tessel_core::Transform::default(),
                    parent: None,
                },
            )
            .unwrap();

        let mut collapsed = BTreeSet::new();
        collapsed.insert(group_a);
        let rows = build_outliner_tree_rows(&scene, &collapsed);
        let names: Vec<(&str, usize, bool, bool)> = rows
            .iter()
            .map(|r| (r.name.as_str(), r.depth, r.has_children, r.collapsed))
            .collect();
        assert_eq!(
            names,
            vec![("GroupA", 0, true, true), ("SiblingB", 0, false, false),]
        );
    }

    #[test]
    fn outliner_tree_handles_cycles_with_16_hop_guard_without_dropping_nodes() {
        let mut scene = SceneState::new();
        let id1 = scene.allocate_node_id();
        let id2 = scene.allocate_node_id();
        let id3 = scene.allocate_node_id();
        scene.nodes.insert(
            id1,
            tessel_core::SceneNode {
                id: id1,
                name: "CycleA".into(),
                kind: tessel_core::NodeKind::Group,
                visible: true,
                parent: Some(id2),
                transform: tessel_core::Transform::default(),
            },
        );
        scene.nodes.insert(
            id2,
            tessel_core::SceneNode {
                id: id2,
                name: "CycleB".into(),
                kind: tessel_core::NodeKind::Group,
                visible: true,
                parent: Some(id1),
                transform: tessel_core::Transform::default(),
            },
        );
        scene.nodes.insert(
            id3,
            tessel_core::SceneNode {
                id: id3,
                name: "CycleChild".into(),
                kind: tessel_core::NodeKind::Group,
                visible: true,
                parent: Some(id2),
                transform: tessel_core::Transform::default(),
            },
        );

        let rows = build_outliner_tree_rows(&scene, &BTreeSet::new());
        assert_eq!(rows.len(), 3);
        let ids: BTreeSet<NodeId> = rows.iter().map(|r| r.id).collect();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
        assert!(ids.contains(&id3));
        assert_eq!(rows[0].depth, 0);

        // Also verify a chain deeper than MAX_OUTLINER_DEPTH terminates and renders every node once.
        let mut deep_scene = SceneState::new();
        let mut prev: Option<NodeId> = None;
        for idx in 0..=(MAX_OUTLINER_DEPTH + 4) {
            let nid = deep_scene.allocate_node_id();
            deep_scene.nodes.insert(
                nid,
                tessel_core::SceneNode {
                    id: nid,
                    name: format!("Deep{idx}"),
                    kind: tessel_core::NodeKind::Group,
                    visible: true,
                    parent: prev,
                    transform: tessel_core::Transform::default(),
                },
            );
            prev = Some(nid);
        }
        let deep_rows = build_outliner_tree_rows(&deep_scene, &BTreeSet::new());
        assert_eq!(deep_rows.len(), MAX_OUTLINER_DEPTH + 5);
        assert!(deep_rows.iter().all(|r| r.depth <= MAX_OUTLINER_DEPTH));
        let unique: BTreeSet<NodeId> = deep_rows.iter().map(|r| r.id).collect();
        assert_eq!(unique.len(), deep_rows.len());
    }

    #[test]
    fn valid_parent_candidates_excludes_self_and_all_descendants() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();

        for name in ["Root", "Child", "GrandChild", "Other"] {
            history
                .execute(
                    &mut scene,
                    Command::CreateGroupNode {
                        name: name.into(),
                        transform: tessel_core::Transform::default(),
                        parent: None,
                    },
                )
                .unwrap();
        }
        let ids: BTreeMap<String, NodeId> = scene
            .nodes
            .values()
            .map(|n| (n.name.clone(), n.id))
            .collect();
        let root_id = ids["Root"];
        let child_id = ids["Child"];
        let grandchild_id = ids["GrandChild"];
        let other_id = ids["Other"];

        history
            .execute(
                &mut scene,
                Command::SetNodeParent {
                    node_id: child_id,
                    parent: Some(root_id),
                },
            )
            .unwrap();
        history
            .execute(
                &mut scene,
                Command::SetNodeParent {
                    node_id: grandchild_id,
                    parent: Some(child_id),
                },
            )
            .unwrap();

        let root_candidates: Vec<NodeId> = valid_parent_candidates(&scene, root_id)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(root_candidates, vec![other_id]);

        let child_candidates: Vec<NodeId> = valid_parent_candidates(&scene, child_id)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(child_candidates, vec![root_id, other_id]);
    }

    #[test]
    fn outliner_create_group_and_reparent_commands_work_with_undo() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();

        // No selection -> creates root Group.
        let cmd1 = create_outliner_group_command(&scene);
        history.execute(&mut scene, cmd1).unwrap();
        let group1 = scene.selected_node.unwrap();
        assert_eq!(scene.nodes[&group1].kind, tessel_core::NodeKind::Group);
        assert_eq!(scene.nodes[&group1].parent, None);
        assert_eq!(scene.nodes[&group1].name, "Group");

        // Group selected -> nests new Group inside selected Group.
        let cmd2 = create_outliner_group_command(&scene);
        history.execute(&mut scene, cmd2).unwrap();
        let group2 = scene.selected_node.unwrap();
        assert_eq!(scene.nodes[&group2].parent, Some(group1));
        assert_eq!(scene.nodes[&group2].name, "Group 2");

        // Non-group selected -> creates root Group.
        history
            .execute(
                &mut scene,
                Command::CreatePrimitiveMesh {
                    name: "Prop".into(),
                    kind: tessel_core::PrimitiveKind::Cube,
                    size: 1.0,
                    material: PaletteIndex::new(1).unwrap(),
                },
            )
            .unwrap();
        let prop_id = scene.selected_node.unwrap();
        let cmd3 = create_outliner_group_command(&scene);
        history.execute(&mut scene, cmd3).unwrap();
        let group3 = scene.selected_node.unwrap();
        assert_eq!(scene.nodes[&group3].parent, None);

        // Reparent Prop inside Group 2 and undo back to root.
        history
            .execute(
                &mut scene,
                Command::SetNodeParent {
                    node_id: prop_id,
                    parent: Some(group2),
                },
            )
            .unwrap();
        assert_eq!(scene.nodes[&prop_id].parent, Some(group2));
        history.undo(&mut scene).unwrap();
        assert_eq!(scene.nodes[&prop_id].parent, None);
    }

    #[test]
    fn outliner_click_group_button_and_parent_selector_render_cleanly() {
        let mut scene = SceneState::new();
        let mut history = HistoryStack::new();
        let mut rename_state = OutlinerRenameState::default();

        let ctx = egui::Context::default();
        let base_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 720.0),
            )),
            ..Default::default()
        };
        let draw_outliner = |ctx: &egui::Context,
                             scene: &mut SceneState,
                             history: &mut HistoryStack,
                             rename_state: &mut OutlinerRenameState| {
            egui::SidePanel::right("test_outliner_g2")
                .exact_width(256.0)
                .resizable(false)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        show_outliner_section(ui, scene, history, rename_state);
                        let mut show_gizmo = false;
                        let mut gizmo_mode = tessel_render::TransformGizmoMode::Move;
                        let mut free_view = false;
                        let mut render_preview = false;
                        let mut tool = crate::ui::viewport::ViewportInteractionTool::Select;
                        show_transform_and_lighting_section(
                            ui,
                            scene,
                            history,
                            &mut show_gizmo,
                            &mut gizmo_mode,
                            &mut free_view,
                            &mut render_preview,
                            &mut tool,
                        );
                    });
                });
        };

        let out = ctx.run(base_input, |c| {
            draw_outliner(c, &mut scene, &mut history, &mut rename_state);
        });
        let add_group_pos = find_label_center(&out.shapes, "+ Group").unwrap();
        click_at(&ctx, add_group_pos, |c| {
            draw_outliner(c, &mut scene, &mut history, &mut rename_state);
        });
        assert_eq!(scene.nodes.len(), 1);
        let group_id = scene.selected_node.unwrap();
        assert_eq!(scene.nodes[&group_id].kind, tessel_core::NodeKind::Group);

        // Deleted node IDs are pruned from collapsed_nodes automatically.
        rename_state.collapsed_nodes.insert(NodeId(987_654));
        let _ = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 720.0),
                )),
                ..Default::default()
            },
            |c| {
                draw_outliner(c, &mut scene, &mut history, &mut rename_state);
            },
        );
        assert!(rename_state.collapsed_nodes.is_empty());
    }
}
