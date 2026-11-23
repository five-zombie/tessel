use std::path::Path;
use std::sync::mpsc;

use egui::{Color32, CornerRadius, Frame, Margin, RichText, Stroke};
use tessel_ai::direct::DirectAiResponse;
use tessel_ai::{AiProviderKind, DirectAiConfig};
use tessel_core::{
    AppMode, Command, FrameIndex, HistoryStack, PaletteEntry, PaletteIndex, Rgba8, SceneState,
};

use super::theme::tokens;

#[derive(Default)]
pub struct DirectAiUiState {
    pub config: DirectAiConfig,
    pub prompt: String,
    pub pending: Option<mpsc::Receiver<Result<DirectAiResponse, String>>>,
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
        .stroke(Stroke::new(1.0, tokens::OUTLINE_VARIANT));

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
            if ui.small_button("+ Color").clicked() {
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

    egui::Grid::new("palette_swatches")
        .spacing(egui::vec2(6.0, 6.0))
        .show(ui, |ui| {
            for (i, (idx, name, rgba)) in entries.into_iter().enumerate() {
                let selected = scene.active_color == idx;
                let fill = Color32::from_rgba_unmultiplied(rgba.r, rgba.g, rgba.b, rgba.a);
                let stroke = if selected {
                    Stroke::new(2.0, tokens::PRIMARY)
                } else {
                    Stroke::new(1.0, tokens::OUTLINE_VARIANT)
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
                if (i + 1) % 7 == 0 {
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
                if ui.small_button("+ Layer").clicked() {
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
        if ui.button("New 32x32 Pixel Canvas").clicked() {
            let _ = history.execute(
                scene,
                Command::CreatePixelCanvas {
                    name: "Sprite 32x32".into(),
                    width: 32,
                    height: 32,
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
                let vis_label = if visible { "On" } else { "Off" };
                if ui.small_button(vis_label).clicked() {
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
        if ui.button("Apply Biped Skeleton").clicked() {
            let _ = history.execute(scene, Command::ApplyBipedRig);
        }
        return;
    };

    let bones: Vec<_> = rig
        .bones
        .iter()
        .map(|b| (b.id, b.name.clone(), b.pose_local.rotation))
        .collect();
    let current_binding = scene.selected_node.and_then(|nid| {
        rig.bindings
            .iter()
            .find(|b| b.node_id == nid)
            .map(|b| b.bone_id)
    });

    if let Some(selected_id) = scene.selected_node {
        if let Some(node) = scene.nodes.get(&selected_id) {
            ui.label(
                RichText::new(format!("Bind '{}' to Bone:", node.name))
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
                for (bid, bname, _) in &bones {
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
        }
    }

    for (bone_id, name, rot) in bones {
        let (mut rx, mut ry, mut rz) = rot.to_euler(glam::EulerRot::XYZ);
        ui.collapsing(name, |ui| {
            let mut changed = false;
            changed |= ui
                .add(egui::Slider::new(&mut rx, -1.5..=1.5).text("Pitch"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut ry, -1.5..=1.5).text("Yaw"))
                .changed();
            changed |= ui
                .add(egui::Slider::new(&mut rz, -1.5..=1.5).text("Roll"))
                .changed();
            if changed {
                let next_rot = glam::Quat::from_euler(glam::EulerRot::XYZ, rx, ry, rz);
                let _ = history.execute(
                    scene,
                    Command::SetBoneRotation {
                        bone_id,
                        rotation: next_rot,
                    },
                );
            }
        });
    }
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

    let nodes: Vec<_> = scene
        .nodes
        .values()
        .map(|n| (n.id, n.name.clone(), n.kind))
        .collect();
    for (id, name, kind) in nodes {
        let selected = scene.selected_node == Some(id);
        let label = format!("{name} ({kind:?})");
        if ui.selectable_label(selected, label).clicked() && !selected {
            let _ = history.execute(scene, Command::SelectNode(Some(id)));
        }
    }
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
                    if ui.small_button("Reset").clicked() {
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
                            .range(0.05..=20.0)
                            .prefix("X "),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut tx.scale.y)
                            .speed(0.02)
                            .range(0.05..=20.0)
                            .prefix("Y "),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut tx.scale.z)
                            .speed(0.02)
                            .range(0.05..=20.0)
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

    ui.horizontal_wrapped(|ui| {
        for provider in AiProviderKind::ALL {
            if ui
                .selectable_label(ai_state.config.provider == provider, provider.label())
                .clicked()
            {
                ai_state.config.provider = provider;
                ai_state.config.model = provider.default_model().into();
                ai_state.config.api_key = crate::settings::load_api_key_from_keyring(provider);
            }
        }
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
            if ui.small_button("Save").clicked() {
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
        if ui
            .add_enabled(can_send, egui::Button::new("Send Prompt"))
            .clicked()
        {
            let (tx, rx) = mpsc::channel();
            let config = ai_state.config.clone();
            let scene_clone = scene.clone();
            let prompt = ai_state.prompt.clone();
            ai_state.pending = Some(rx);
            std::thread::spawn(move || {
                let res = match tokio::runtime::Runtime::new() {
                    Ok(rt) => rt
                        .block_on(tessel_ai::direct::send_prompt(
                            &config,
                            &scene_clone,
                            &prompt,
                        ))
                        .map_err(|e| e.to_string()),
                    Err(e) => Err(format!("async runtime error: {e}")),
                };
                let _ = tx.send(res);
            });
        }
        if busy && ui.button("Cancel").clicked() {
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
