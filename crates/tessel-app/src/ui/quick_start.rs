use std::path::PathBuf;

use egui::{
    ColorImage, CornerRadius, RichText, Sense, Stroke, TextureHandle, TextureOptions, Vec2,
};
use tessel_core::{AppMode, Command, HistoryStack, SceneState};

use super::icons::ToolIcon;
use super::theme::tokens;

pub struct QuickStartState {
    pub open: bool,
    pub has_recovery: bool,
    pub error_message: Option<String>,
    pub example_thumbs: Option<[Option<TextureHandle>; 4]>,
}

/// Built-in showcase examples embedded directly in the binary so they always
/// open as an untitled in-memory copy (never overwriting the source `.tessel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinExample {
    PixelRoninFox,
    VoxelRamenTortoise,
    LowPolySkyShrine,
    CharacterRoninIaijutsu,
}

impl BuiltinExample {
    pub const ALL: [Self; 4] = [
        Self::PixelRoninFox,
        Self::VoxelRamenTortoise,
        Self::LowPolySkyShrine,
        Self::CharacterRoninIaijutsu,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::PixelRoninFox => "Ronin Fox",
            Self::VoxelRamenTortoise => "Kame-Yatai",
            Self::LowPolySkyShrine => "Sky-Shrine",
            Self::CharacterRoninIaijutsu => "Ronin Iaijutsu",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::PixelRoninFox => "Pixel • 256x256 4-frame",
            Self::VoxelRamenTortoise => "Voxel • Ramen tortoise",
            Self::LowPolySkyShrine => "Low-Poly • Floating Torii",
            Self::CharacterRoninIaijutsu => "Character • Iaijutsu clip",
        }
    }

    fn bytes(self) -> &'static [u8] {
        match self {
            Self::PixelRoninFox => {
                include_bytes!("../../../../examples/pixel-ronin-fox/ronin_fox_256.tessel")
            }
            Self::VoxelRamenTortoise => {
                include_bytes!("../../../../examples/voxel-ramen-tortoise/kame_yatai_voxel.tessel")
            }
            Self::LowPolySkyShrine => {
                include_bytes!("../../../../examples/lowpoly-sky-shrine/sky_shrine_lowpoly.tessel")
            }
            Self::CharacterRoninIaijutsu => {
                include_bytes!("../../../../examples/lowpoly-ronin-iaijutsu/ronin_iaijutsu.tessel")
            }
        }
    }

    /// Embedded PNG preview thumbnail for the Welcome modal card.
    fn thumbnail_png_bytes(self) -> &'static [u8] {
        match self {
            Self::PixelRoninFox => {
                include_bytes!("../../../../examples/pixel-ronin-fox/ronin_fox_256.png")
            }
            Self::VoxelRamenTortoise => {
                include_bytes!("../../../../examples/voxel-ramen-tortoise/kame_yatai_hero.png")
            }
            Self::LowPolySkyShrine => {
                include_bytes!("../../../../examples/lowpoly-sky-shrine/sky_shrine_pbr.png")
            }
            Self::CharacterRoninIaijutsu => {
                include_bytes!("../../../../examples/lowpoly-ronin-iaijutsu/ronin_iaijutsu_pbr.png")
            }
        }
    }

    fn fallback_icon(self) -> ToolIcon {
        match self {
            Self::PixelRoninFox => ToolIcon::ModePixel,
            Self::VoxelRamenTortoise => ToolIcon::ModeVoxel,
            Self::LowPolySkyShrine => ToolIcon::ModeLowPoly,
            Self::CharacterRoninIaijutsu => ToolIcon::ModeCharacter,
        }
    }

    fn load_thumbnail_texture(self, ctx: &egui::Context) -> Option<TextureHandle> {
        let img = image::load_from_memory(self.thumbnail_png_bytes()).ok()?;
        let filter = if matches!(self, Self::PixelRoninFox) {
            image::imageops::FilterType::Nearest
        } else {
            image::imageops::FilterType::Triangle
        };
        let thumb = img.resize_exact(96, 96, filter).to_rgba8();
        let color_image = ColorImage::from_rgba_unmultiplied([96, 96], thumb.as_raw());
        let opts = if matches!(self, Self::PixelRoninFox) {
            TextureOptions::NEAREST
        } else {
            TextureOptions::LINEAR
        };
        Some(ctx.load_texture(
            format!("welcome_example_{}", self.title()),
            color_image,
            opts,
        ))
    }

    /// Decodes a fresh in-memory copy of the example scene.
    pub fn load_copy(self) -> Result<SceneState, String> {
        tessel_io::decode_tessel_project(self.bytes())
            .map_err(|err| format!("Failed to load example '{}': {err}", self.title()))
    }

    /// Recommended 3D camera framing `(target, yaw_deg, pitch_deg, distance)` for 3D examples.
    pub fn camera_pose(self) -> Option<([f32; 3], f32, f32, f32)> {
        match self {
            Self::PixelRoninFox => None,
            Self::VoxelRamenTortoise => Some(([0.0, 2.2, 0.2], 32.0, 22.0, 8.2)),
            Self::LowPolySkyShrine => Some(([-0.15, 1.15, 0.0], 28.0, 18.0, 10.6)),
            Self::CharacterRoninIaijutsu => Some(([0.05, 0.88, 0.10], -26.0, 15.0, 4.25)),
        }
    }
}

pub enum QuickStartAction {
    None,
    NewPixel,
    NewVoxel,
    NewCharacter,
    OpenAiWorkspace,
    OpenExample(BuiltinExample),
    RecoverSession,
    OpenRecent(PathBuf),
}

pub fn show_quick_start(
    ctx: &egui::Context,
    state: &mut QuickStartState,
    recent_projects: &[PathBuf],
) -> QuickStartAction {
    if !state.open {
        return QuickStartAction::None;
    }

    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.open = false;
        return QuickStartAction::None;
    }

    let mut action = QuickStartAction::None;
    let mut open = state.open;

    let thumbs = state.example_thumbs.get_or_insert_with(|| {
        [
            BuiltinExample::PixelRoninFox.load_thumbnail_texture(ctx),
            BuiltinExample::VoxelRamenTortoise.load_thumbnail_texture(ctx),
            BuiltinExample::LowPolySkyShrine.load_thumbnail_texture(ctx),
            BuiltinExample::CharacterRoninIaijutsu.load_thumbnail_texture(ctx),
        ]
    });

    egui::Window::new("Welcome to Tessel")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .auto_sized()
        .max_width(500.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(484.0);
            if state.has_recovery {
                egui::Frame::NONE
                    .fill(tokens::SURFACE_HIGH)
                    .stroke(Stroke::new(1.0_f32, tokens::PRIMARY))
                    .corner_radius(CornerRadius::same(4))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Unsaved session found from a previous run.")
                                    .size(11.5)
                                    .color(tokens::ON_SURFACE),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("Recover Session")
                                                    .strong()
                                                    .color(tokens::ON_PRIMARY),
                                            )
                                            .fill(tokens::PRIMARY_CONTAINER),
                                        )
                                        .clicked()
                                    {
                                        action = QuickStartAction::RecoverSession;
                                    }
                                },
                            );
                        });
                    });
                ui.add_space(8.0);
            }

            ui.label(
                RichText::new("Start Creating")
                    .strong()
                    .size(13.0)
                    .color(tokens::ON_SURFACE),
            );
            ui.add_space(6.0);

            egui::Grid::new("quick_start_cards")
                .spacing(egui::vec2(8.0, 8.0))
                .show(ui, |ui| {
                    if start_card(
                        ui,
                        ToolIcon::ModeCharacter.glyph(),
                        "New Character",
                        "Biped rig & animation",
                    )
                    .clicked()
                    {
                        action = QuickStartAction::NewCharacter;
                    }
                    if start_card(
                        ui,
                        ToolIcon::ModeVoxel.glyph(),
                        "New Voxel",
                        "3D blocky studio",
                    )
                    .clicked()
                    {
                        action = QuickStartAction::NewVoxel;
                    }
                    ui.end_row();
                    if start_card(
                        ui,
                        ToolIcon::ModePixel.glyph(),
                        "New Pixel",
                        "32x32 layered sprite",
                    )
                    .clicked()
                    {
                        action = QuickStartAction::NewPixel;
                    }
                    if start_card(
                        ui,
                        egui_phosphor::regular::SPARKLE,
                        "Create with AI",
                        "Direct AI & MCP tools",
                    )
                    .clicked()
                    {
                        action = QuickStartAction::OpenAiWorkspace;
                    }
                    ui.end_row();
                });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(
                RichText::new("Examples (Opens as Copy)")
                    .strong()
                    .size(12.5)
                    .color(tokens::ON_SURFACE),
            );
            ui.add_space(6.0);

            egui::Grid::new("quick_start_examples")
                .spacing(egui::vec2(8.0, 8.0))
                .show(ui, |ui| {
                    for (idx, example) in BuiltinExample::ALL.iter().copied().enumerate() {
                        if example_card(
                            ui,
                            thumbs[idx].as_ref(),
                            example.fallback_icon().glyph(),
                            example.title(),
                            example.subtitle(),
                        )
                        .clicked()
                        {
                            action = QuickStartAction::OpenExample(example);
                        }
                        if idx % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(
                RichText::new("Recent Projects")
                    .strong()
                    .size(12.5)
                    .color(tokens::ON_SURFACE),
            );
            ui.add_space(4.0);

            if recent_projects.is_empty() {
                ui.label(
                    RichText::new("No recent .tessel files yet. Save a project with Ctrl+S.")
                        .size(11.5)
                        .color(tokens::ON_SURFACE_VARIANT),
                );
            } else {
                for path in recent_projects {
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("project.tessel");
                    if ui
                        .selectable_label(false, format!("{name}  ({})", path.display()))
                        .clicked()
                    {
                        action = QuickStartAction::OpenRecent(path.clone());
                    }
                }
            }

            if let Some(err) = &state.error_message {
                ui.add_space(6.0);
                ui.label(RichText::new(err).size(11.0).color(tokens::ERROR));
            }
        });

    if !matches!(action, QuickStartAction::None) {
        state.open = false;
    } else {
        state.open = open;
    }
    action
}

fn start_card(ui: &mut egui::Ui, icon_glyph: &str, title: &str, subtitle: &str) -> egui::Response {
    let desired = Vec2::new(236.0, 52.0);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let bg = if response.hovered() {
        tokens::SURFACE_HIGHEST
    } else {
        tokens::SURFACE_HIGH
    };
    let border = if response.hovered() {
        tokens::PRIMARY
    } else {
        tokens::OUTLINE_VARIANT
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(4),
        bg,
        Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );

    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(10.0, 6.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.scope(|ui| {
        egui::Frame::NONE
            .fill(tokens::SURFACE_CONTAINER)
            .corner_radius(CornerRadius::same(4))
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                ui.add_sized(
                    Vec2::new(22.0, 22.0),
                    egui::Label::new(RichText::new(icon_glyph).size(18.0).color(tokens::PRIMARY))
                        .selectable(false),
                );
            });
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add(
                egui::Label::new(
                    RichText::new(title)
                        .strong()
                        .size(12.0)
                        .color(tokens::ON_SURFACE),
                )
                .selectable(false),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(subtitle)
                        .size(10.5)
                        .color(tokens::ON_SURFACE_VARIANT),
                )
                .selectable(false),
            );
        });
    });

    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn example_card(
    ui: &mut egui::Ui,
    thumb: Option<&TextureHandle>,
    fallback_glyph: &str,
    title: &str,
    subtitle: &str,
) -> egui::Response {
    let desired = Vec2::new(236.0, 62.0);
    let (rect, response) = ui.allocate_exact_size(desired, Sense::click());
    let bg = if response.hovered() {
        tokens::SURFACE_HIGHEST
    } else {
        tokens::SURFACE_HIGH
    };
    let border = if response.hovered() {
        tokens::PRIMARY
    } else {
        tokens::OUTLINE_VARIANT
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(4),
        bg,
        Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );

    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(8.0, 7.0)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.scope(|ui| {
        if let Some(tex) = thumb {
            ui.add(
                egui::Image::new((tex.id(), Vec2::new(48.0, 48.0)))
                    .corner_radius(CornerRadius::same(4)),
            );
        } else {
            egui::Frame::NONE
                .fill(tokens::SURFACE_CONTAINER)
                .corner_radius(CornerRadius::same(4))
                .show(ui, |ui| {
                    ui.add_sized(
                        Vec2::new(48.0, 48.0),
                        egui::Label::new(
                            RichText::new(fallback_glyph)
                                .size(22.0)
                                .color(tokens::PRIMARY),
                        )
                        .selectable(false),
                    );
                });
        }
        ui.add_space(6.0);
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add(
                egui::Label::new(
                    RichText::new(title)
                        .strong()
                        .size(12.0)
                        .color(tokens::ON_SURFACE),
                )
                .selectable(false),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(subtitle)
                        .size(10.5)
                        .color(tokens::ON_SURFACE_VARIANT),
                )
                .selectable(false),
            );
        });
    });

    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn apply_quick_start_preset(
    action: &QuickStartAction,
    scene: &mut SceneState,
    history: &mut HistoryStack,
) {
    match action {
        QuickStartAction::NewPixel => {
            let _ = history.execute(scene, Command::SetMode(AppMode::Pixel));
        }
        QuickStartAction::NewVoxel | QuickStartAction::OpenAiWorkspace => {
            let _ = history.execute(scene, Command::SetMode(AppMode::Voxel));
        }
        QuickStartAction::NewCharacter => {
            let _ = history.execute(
                scene,
                Command::Transaction(vec![
                    Command::ApplyBipedRig,
                    Command::SetMode(AppMode::Character),
                ]),
            );
        }
        QuickStartAction::None
        | QuickStartAction::OpenExample(_)
        | QuickStartAction::RecoverSession
        | QuickStartAction::OpenRecent(_) => {}
    }
}

pub fn show_about_dialog(ctx: &egui::Context, open: &mut bool) {
    if !*open {
        return;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        *open = false;
        return;
    }
    let mut is_open = *open;
    egui::Window::new("About Tessel")
        .open(&mut is_open)
        .collapsible(false)
        .resizable(false)
        .auto_sized()
        .max_width(360.0)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .show(ctx, |ui| {
            ui.set_width(340.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("TESSEL")
                        .strong()
                        .size(18.0)
                        .color(tokens::PRIMARY),
                );
                ui.label(
                    RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                        .size(12.0)
                        .color(tokens::ON_SURFACE_VARIANT),
                );
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "AI-first cross-platform desktop studio built in Pure Rust (wgpu, winit, egui) for Pixel, Voxel, Low-Poly, and Character assets with glTF 2.0 (.glb) export and MCP integration.",
                )
                .size(11.5)
                .color(tokens::ON_SURFACE),
            );
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(6.0);
            ui.label(
                RichText::new("Copyright 2026 The Five Zombie Org\nLicensed under Apache-2.0")
                    .size(11.0)
                    .color(tokens::ON_SURFACE_VARIANT),
            );
        });
    *open = is_open;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_examples_decode_as_fresh_copies_in_expected_modes() {
        let expected = [
            (BuiltinExample::PixelRoninFox, AppMode::Pixel),
            (BuiltinExample::VoxelRamenTortoise, AppMode::Voxel),
            (BuiltinExample::LowPolySkyShrine, AppMode::LowPoly),
            (BuiltinExample::CharacterRoninIaijutsu, AppMode::Character),
        ];
        for (example, expected_mode) in expected {
            let scene = example
                .load_copy()
                .expect("embedded example .tessel must decode cleanly");
            assert_eq!(scene.active_mode, expected_mode, "example {:?}", example);
            assert!(
                !scene.nodes.is_empty(),
                "example {:?} should contain scene nodes",
                example
            );
            let img = image::load_from_memory(example.thumbnail_png_bytes())
                .expect("embedded example PNG thumbnail must decode cleanly");
            assert!(img.width() >= 128 && img.height() >= 128);
        }
    }
}
