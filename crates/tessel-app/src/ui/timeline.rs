use egui::{CornerRadius, Frame, Margin, RichText, Stroke};
use tessel_core::{AppMode, Command, FrameIndex, HistoryStack, Keyframe, SceneState};

use super::icons::{self, ToolIcon};
use super::theme::tokens;

#[derive(Debug, Clone, Copy)]
struct TimelineViewPrefs {
    timescale: f32,
    fps: u32,
}

impl Default for TimelineViewPrefs {
    fn default() -> Self {
        Self {
            timescale: 1.0,
            fps: 24,
        }
    }
}

const TIMESCALE_PRESETS: [f32; 4] = [0.25, 0.5, 1.0, 2.0];
const FPS_PRESETS: [u32; 3] = [12, 24, 30];

pub fn time_to_frame(time_secs: f32, fps: u32) -> u32 {
    (time_secs.max(0.0) * fps.max(1) as f32).round() as u32
}

pub fn nice_tick_step(duration_secs: f32) -> f32 {
    let d = duration_secs.max(0.1);
    for step in [0.1, 0.25, 0.5, 1.0, 2.0, 5.0] {
        if d / step <= 10.0 {
            return step;
        }
    }
    5.0
}

pub fn scrub_x_to_time(x: f32, rect_left: f32, rect_width: f32, duration_secs: f32) -> f32 {
    if rect_width <= 1e-4 {
        return 0.0;
    }
    let frac = ((x - rect_left) / rect_width).clamp(0.0, 1.0);
    frac * duration_secs.max(0.01)
}

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
    let frame = Frame::NONE
        .fill(tokens::SURFACE_CONTAINER)
        .inner_margin(Margin::symmetric(12, 6))
        .stroke(Stroke::new(1.0, tokens::OUTLINE_VARIANT));

    if scene.character_rig.is_none() {
        let mut pending_cmd: Option<Command> = None;
        egui::TopBottomPanel::bottom("tessel_character_timeline")
            .exact_height(44.0)
            .frame(frame)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(
                        RichText::new("Animation Timeline")
                            .strong()
                            .size(11.5)
                            .color(tokens::ON_SURFACE),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new("No rig applied yet — apply a skeleton to inspect timeframes & keyframes:")
                            .size(11.0)
                            .color(tokens::ON_SURFACE_VARIANT),
                    );
                    if ui.button("Apply Humanoid (19)").clicked() {
                        pending_cmd = Some(Command::ApplyHumanoidRig);
                    }
                    if ui.button("Apply Biped (11)").clicked() {
                        pending_cmd = Some(Command::ApplyBipedRig);
                    }
                });
            });
        if let Some(cmd) = pending_cmd {
            let _ = history.execute(scene, cmd);
        }
        return;
    }

    let prefs_id = egui::Id::new("tessel_timeline_prefs");
    let mut prefs = ctx.data_mut(|d| {
        d.get_temp::<TimelineViewPrefs>(prefs_id)
            .unwrap_or_default()
    });

    if let Some(rig) = scene.character_rig.as_mut() {
        if rig.playing {
            let dt = ctx.input(|i| i.stable_dt).min(0.05) * prefs.timescale;
            rig.playback_time += dt;
            rig.apply_active_animation();
            scene.mark_modified();
        }
    }

    let mut modified = false;
    let mut pending_cmd: Option<Command> = None;

    egui::TopBottomPanel::bottom("tessel_character_timeline")
        .exact_height(104.0)
        .frame(frame)
        .show(ctx, |ui| {
            let Some(rig) = scene.character_rig.as_mut() else {
                return;
            };
            let selected_bone = scene.selected_bone;
            let duration = rig
                .animations
                .get(rig.active_animation)
                .map(|c| c.duration_secs.max(0.1))
                .unwrap_or(1.0);

            // Row 1: Transport, Clips, Timeframe Readout, Timescale (Speed), FPS, Duration, Keyframes
            ui.horizontal(|ui| {
                let (play_icon, play_tip) = if rig.playing {
                    (ToolIcon::Pause, "Pause")
                } else {
                    (ToolIcon::Play, "Play")
                };
                if icons::hud_icon_button(ui, play_icon, play_tip, rig.playing) {
                    rig.playing = !rig.playing;
                }
                if icons::hud_icon_button(ui, ToolIcon::Stop, "Stop & Rewind (0.00s)", false) {
                    rig.playing = false;
                    rig.playback_time = 0.0;
                    rig.apply_active_animation();
                    modified = true;
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

                let cur_time = rig.playback_time.rem_euclid(duration);
                let cur_frame = time_to_frame(cur_time, prefs.fps);
                let max_frame = time_to_frame(duration, prefs.fps);
                ui.label(
                    RichText::new(format!(
                        "{cur_time:.2}s / {duration:.2}s  (F {cur_frame:02}/{max_frame:02})"
                    ))
                    .monospace()
                    .size(11.0)
                    .color(tokens::PRIMARY),
                );
                ui.separator();

                ui.label(
                    RichText::new("Scale:")
                        .size(10.5)
                        .color(tokens::ON_SURFACE_VARIANT),
                );
                for spd in TIMESCALE_PRESETS {
                    let active = (prefs.timescale - spd).abs() < 1e-3;
                    if ui.selectable_label(active, format!("{spd}x")).clicked() {
                        prefs.timescale = spd;
                    }
                }
                ui.separator();

                ui.label(
                    RichText::new("FPS:")
                        .size(10.5)
                        .color(tokens::ON_SURFACE_VARIANT),
                );
                for fps in FPS_PRESETS {
                    if ui
                        .selectable_label(prefs.fps == fps, format!("{fps}"))
                        .clicked()
                    {
                        prefs.fps = fps;
                    }
                }
                ui.separator();

                if let Some(clip) = rig.animations.get_mut(rig.active_animation) {
                    let mut dur = clip.duration_secs;
                    ui.label(
                        RichText::new("Len:")
                            .size(10.5)
                            .color(tokens::ON_SURFACE_VARIANT),
                    );
                    if ui
                        .add(
                            egui::DragValue::new(&mut dur)
                                .speed(0.05)
                                .range(0.1..=30.0)
                                .suffix("s"),
                        )
                        .changed()
                    {
                        clip.duration_secs = dur;
                        rig.playback_time = rig.playback_time.min(dur);
                        rig.apply_active_animation();
                        modified = true;
                    }
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icons::hud_icon_button(
                        ui,
                        ToolIcon::Remove,
                        "Remove keyframe at playhead",
                        false,
                    ) {
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

                    if icons::hud_icon_button(
                        ui,
                        ToolIcon::Keyframe,
                        "Insert keyframe at playhead",
                        false,
                    ) {
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
                });
            });

            ui.add_space(4.0);

            // Row 2: Visual Timeframe Ruler, Keyframe Diamonds & Interactive Playhead Scrubber
            let duration = rig
                .animations
                .get(rig.active_animation)
                .map(|c| c.duration_secs.max(0.1))
                .unwrap_or(1.0);
            let keyframe_times = rig
                .animations
                .get(rig.active_animation)
                .map(|c| c.keyframe_times(selected_bone))
                .unwrap_or_default();

            let desired = egui::vec2(ui.available_width().max(120.0), 54.0);
            let (rect, response) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());
            let pad_x = 10.0;
            let track_left = rect.left() + pad_x;
            let track_right = (rect.right() - pad_x).max(track_left + 1.0);
            let track_w = track_right - track_left;

            if response.clicked() || response.dragged() {
                if let Some(pos) = response.interact_pointer_pos() {
                    rig.playing = false;
                    rig.playback_time = scrub_x_to_time(pos.x, track_left, track_w, duration);
                    rig.apply_active_animation();
                    modified = true;
                }
            }

            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, CornerRadius::same(4), tokens::SURFACE_LOW);
            painter.rect_stroke(
                rect,
                CornerRadius::same(4),
                Stroke::new(1.0, tokens::OUTLINE_VARIANT),
                egui::StrokeKind::Inside,
            );

            let ruler_y = rect.top() + 20.0;
            painter.line_segment(
                [
                    egui::pos2(rect.left(), ruler_y),
                    egui::pos2(rect.right(), ruler_y),
                ],
                Stroke::new(1.0, tokens::OUTLINE_VARIANT),
            );

            // Minor frame ticks
            let total_frames = time_to_frame(duration, prefs.fps).max(1);
            let frame_tick_stride = (total_frames / 60).max(1);
            for f in (0..=total_frames).step_by(frame_tick_stride as usize) {
                let frac = (f as f32 / total_frames as f32).clamp(0.0, 1.0);
                let x = track_left + frac * track_w;
                painter.line_segment(
                    [egui::pos2(x, ruler_y - 4.0), egui::pos2(x, ruler_y)],
                    Stroke::new(1.0, tokens::OUTLINE_VARIANT),
                );
            }

            // Major time ticks + labels
            let step = nice_tick_step(duration);
            let mut t = 0.0f32;
            while t <= duration + 1e-4 {
                let frac = (t / duration).clamp(0.0, 1.0);
                let x = track_left + frac * track_w;
                painter.line_segment(
                    [
                        egui::pos2(x, ruler_y - 9.0),
                        egui::pos2(x, rect.bottom() - 2.0),
                    ],
                    Stroke::new(1.0, tokens::SURFACE_HIGH),
                );
                painter.text(
                    egui::pos2(x + 2.0, rect.top() + 3.0),
                    egui::Align2::LEFT_TOP,
                    format!("{t:.2}s"),
                    egui::FontId::monospace(9.5),
                    tokens::ON_SURFACE_VARIANT,
                );
                t += step;
            }

            // Keyframe diamonds on track
            let track_mid_y = (ruler_y + rect.bottom()) * 0.5;
            let cur_time = rig.playback_time.rem_euclid(duration);
            for kt in keyframe_times {
                let frac = (kt / duration).clamp(0.0, 1.0);
                let x = track_left + frac * track_w;
                let active = (cur_time - kt).abs() <= 0.025;
                let r = if active { 6.0 } else { 4.5 };
                let diamond = vec![
                    egui::pos2(x, track_mid_y - r),
                    egui::pos2(x + r, track_mid_y),
                    egui::pos2(x, track_mid_y + r),
                    egui::pos2(x - r, track_mid_y),
                ];
                let fill = if active {
                    tokens::PRIMARY
                } else {
                    tokens::ON_SURFACE_VARIANT
                };
                painter.add(egui::Shape::convex_polygon(
                    diamond,
                    fill,
                    Stroke::new(1.0, tokens::SURFACE_LOW),
                ));
            }

            // Playhead vertical line + handle
            let play_frac = (cur_time / duration).clamp(0.0, 1.0);
            let play_x = track_left + play_frac * track_w;
            painter.line_segment(
                [
                    egui::pos2(play_x, rect.top()),
                    egui::pos2(play_x, rect.bottom()),
                ],
                Stroke::new(2.0, tokens::PRIMARY),
            );
            let handle = vec![
                egui::pos2(play_x - 5.0, rect.top()),
                egui::pos2(play_x + 5.0, rect.top()),
                egui::pos2(play_x, rect.top() + 7.0),
            ];
            painter.add(egui::Shape::convex_polygon(
                handle,
                tokens::PRIMARY,
                Stroke::NONE,
            ));
        });

    ctx.data_mut(|d| d.insert_temp(prefs_id, prefs));

    if let Some(cmd) = pending_cmd {
        let _ = history.execute(scene, cmd);
    } else if modified {
        scene.mark_modified();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_frame_and_scrub_math() {
        assert_eq!(time_to_frame(0.5, 24), 12);
        assert_eq!(time_to_frame(1.0, 30), 30);
        assert!((nice_tick_step(1.0) - 0.1).abs() < 1e-6);
        assert!((nice_tick_step(2.0) - 0.25).abs() < 1e-6);
        assert!((scrub_x_to_time(150.0, 100.0, 200.0, 2.0) - 0.5).abs() < 1e-6);
        assert!((scrub_x_to_time(50.0, 100.0, 200.0, 2.0) - 0.0).abs() < 1e-6);
        assert!((scrub_x_to_time(500.0, 100.0, 200.0, 2.0) - 2.0).abs() < 1e-6);
    }

    #[test]
    fn character_timeline_renders_without_and_with_rig() {
        let mut scene = SceneState::new();
        scene.active_mode = AppMode::Character;
        let mut history = HistoryStack::new();
        let ctx = egui::Context::default();
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 720.0),
            )),
            ..Default::default()
        };

        // Without rig: shows prompt bar instead of nothing
        let out_empty = ctx.run(raw_input.clone(), |ctx| {
            show_timeline(ctx, &mut scene, &mut history);
        });
        assert!(!out_empty.shapes.is_empty());

        // With Humanoid rig: shows ruler, timescale, keyframe diamonds & playhead
        history
            .execute(&mut scene, Command::ApplyHumanoidRig)
            .unwrap();
        let out_rig = ctx.run(raw_input, |ctx| {
            show_timeline(ctx, &mut scene, &mut history);
        });
        assert!(!out_rig.shapes.is_empty());
    }
}
