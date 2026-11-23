use egui::{CornerRadius, Frame, Margin, RichText, Stroke};
use tessel_core::{AppMode, Command, FrameIndex, HistoryStack, Keyframe, SceneState};

use super::icons::{self, ToolIcon};
use super::theme::tokens;

pub fn show_timeline(ctx: &egui::Context, scene: &mut SceneState, history: &mut HistoryStack) {
    match scene.active_mode {
        AppMode::Pixel => show_pixel_timeline(ctx, scene, history),
        AppMode::Character => show_character_timeline(ctx, scene, history),
        AppMode::Voxel | AppMode::LowPoly => {}
    }
}

fn show_pixel_timeline(ctx: &egui::Context, scene: &mut SceneState, history: &mut HistoryStack) {
    let Some(node_id) = scene.active_pixel_node() else {
        return;
    };

    let (frame_count, active_frame, onion_skin) = match scene.pixel_canvases.get(&node_id) {
        Some(c) => (c.frames.len(), c.active_frame, c.onion_skin),
        None => return,
    };

    let frame = Frame::NONE
        .fill(tokens::SURFACE_CONTAINER)
        .inner_margin(Margin::symmetric(12, 6))
        .stroke(Stroke::new(1.0, tokens::OUTLINE_VARIANT));

    egui::TopBottomPanel::bottom("tessel_timeline")
        .exact_height(38.0)
        .frame(frame)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.label(
                    RichText::new("Frames")
                        .strong()
                        .size(11.5)
                        .color(tokens::ON_SURFACE_VARIANT),
                );

                for idx in 0..frame_count {
                    let selected = idx == active_frame;
                    let fill = if selected {
                        tokens::PRIMARY_CONTAINER
                    } else {
                        tokens::SURFACE_HIGH
                    };
                    let text_color = if selected {
                        tokens::ON_PRIMARY
                    } else {
                        tokens::ON_SURFACE
                    };
                    let btn = egui::Button::new(
                        RichText::new(format!("{}", idx + 1))
                            .monospace()
                            .size(11.0)
                            .color(text_color),
                    )
                    .fill(fill)
                    .corner_radius(CornerRadius::same(4));
                    if ui.add(btn).clicked() && !selected {
                        let _ = history.execute(
                            scene,
                            Command::SetActivePixelFrame {
                                node_id,
                                frame: FrameIndex(idx),
                            },
                        );
                    }
                }

                if icons::hud_icon_button(ui, ToolIcon::Add, "+ Frame", false) {
                    let _ = history.execute(scene, Command::AddPixelFrame { node_id });
                }

                ui.separator();

                let mut onion = onion_skin;
                if ui.checkbox(&mut onion, "Onion Skin").changed() {
                    if let Some(canvas) = scene.pixel_canvases.get_mut(&node_id) {
                        canvas.onion_skin = onion;
                        scene.mark_modified();
                    }
                }
            });
        });
}

fn show_character_timeline(
    ctx: &egui::Context,
    scene: &mut SceneState,
    history: &mut HistoryStack,
) {
    let Some(rig) = scene.character_rig.as_mut() else {
        return;
    };

    if rig.playing {
        let dt = ctx.input(|i| i.stable_dt).min(0.05);
        rig.playback_time += dt;
        rig.apply_active_animation();
        scene.mark_modified();
    }

    let frame = Frame::NONE
        .fill(tokens::SURFACE_CONTAINER)
        .inner_margin(Margin::symmetric(12, 6))
        .stroke(Stroke::new(1.0, tokens::OUTLINE_VARIANT));

    let mut modified = false;
    let mut pending_cmd: Option<Command> = None;

    egui::TopBottomPanel::bottom("tessel_character_timeline")
        .exact_height(42.0)
        .frame(frame)
        .show(ctx, |ui| {
            let Some(rig) = scene.character_rig.as_mut() else {
                return;
            };
            let selected_bone = scene.selected_bone;
            ui.horizontal_centered(|ui| {
                let (play_icon, play_tip) = if rig.playing {
                    (ToolIcon::Pause, "Pause")
                } else {
                    (ToolIcon::Play, "Play")
                };
                if icons::hud_icon_button(ui, play_icon, play_tip, false) {
                    rig.playing = !rig.playing;
                }
                ui.separator();

                let clip_names: Vec<String> =
                    rig.animations.iter().map(|c| c.name.clone()).collect();
                for (i, name) in clip_names.into_iter().enumerate() {
                    if ui
                        .selectable_label(rig.active_animation == i, name)
                        .clicked()
                    {
                        rig.active_animation = i;
                        rig.playback_time = 0.0;
                        rig.apply_active_animation();
                        modified = true;
                    }
                }
                if icons::hud_icon_button(ui, ToolIcon::Add, "+ Clip", false) {
                    pending_cmd = Some(Command::AddAnimationClip {
                        name: format!("Clip {}", rig.animations.len() + 1),
                        duration_secs: 2.0,
                    });
                }
                ui.separator();

                let duration = rig
                    .animations
                    .get(rig.active_animation)
                    .map(|c| c.duration_secs)
                    .unwrap_or(1.0);
                if ui
                    .add(egui::Slider::new(&mut rig.playback_time, 0.0..=duration).text("Time"))
                    .changed()
                {
                    rig.apply_active_animation();
                    modified = true;
                }
                ui.separator();

                if icons::hud_icon_button(ui, ToolIcon::Keyframe, "+ Key", false) {
                    let clip_index = rig.active_animation;
                    let t = rig.playback_time;
                    let targets: Vec<_> = match selected_bone {
                        Some(bid) => rig
                            .bone(bid)
                            .map(|b| {
                                vec![(
                                    b.id,
                                    b.rest_local.rotation.inverse() * b.pose_local.rotation,
                                )]
                            })
                            .unwrap_or_default(),
                        None => rig
                            .bones
                            .iter()
                            .map(|b| {
                                (
                                    b.id,
                                    b.rest_local.rotation.inverse() * b.pose_local.rotation,
                                )
                            })
                            .collect(),
                    };
                    if targets.len() == 1 {
                        let (bone_id, delta_rot) = targets[0];
                        pending_cmd = Some(Command::SetAnimationKeyframe {
                            clip_index,
                            bone_id,
                            keyframe: Keyframe::rotation_only(t, delta_rot),
                        });
                    } else if targets.len() > 1 {
                        pending_cmd = Some(Command::Transaction(
                            targets
                                .into_iter()
                                .map(|(bone_id, delta_rot)| Command::SetAnimationKeyframe {
                                    clip_index,
                                    bone_id,
                                    keyframe: Keyframe::rotation_only(t, delta_rot),
                                })
                                .collect(),
                        ));
                    }
                }

                if icons::hud_icon_button(ui, ToolIcon::Remove, "- Key", false) {
                    let clip_index = rig.active_animation;
                    let t = rig.playback_time;
                    if let Some(bone_id) = selected_bone {
                        pending_cmd = Some(Command::RemoveAnimationKeyframe {
                            clip_index,
                            bone_id,
                            time_secs: t,
                        });
                    } else if let Some(clip) = rig.animations.get(clip_index) {
                        let cmds: Vec<_> = clip
                            .channels
                            .iter()
                            .map(|ch| Command::RemoveAnimationKeyframe {
                                clip_index,
                                bone_id: ch.bone_id,
                                time_secs: t,
                            })
                            .collect();
                        if !cmds.is_empty() {
                            pending_cmd = Some(Command::Transaction(cmds));
                        }
                    }
                }

                if let Some(clip) = rig.animations.get(rig.active_animation) {
                    let times = clip.keyframe_times(selected_bone);
                    if !times.is_empty() {
                        ui.separator();
                        for kt in times {
                            let active = (rig.playback_time - kt).abs() <= 0.02;
                            if ui.selectable_label(active, format!("{kt:.2}s")).clicked() {
                                rig.playback_time = kt;
                                rig.apply_active_animation();
                                modified = true;
                            }
                        }
                    }
                }
            });
        });

    if let Some(cmd) = pending_cmd {
        let _ = history.execute(scene, cmd);
    } else if modified {
        scene.mark_modified();
    }
}
