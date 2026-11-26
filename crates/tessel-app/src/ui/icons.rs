use egui::{Color32, Ui, Vec2};

use super::theme::tokens;

/// Toolbar icon catalog. Every button on the toolbar references one of these;
/// adding a new button is one `IconItem` line in a group list (see
/// `toolbar.rs`), and adding a brand-new glyph is one variant here plus one
/// arm in [`ToolIcon::glyph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolIcon {
    Pencil,
    Eraser,
    Fill,
    Line,
    Rect,
    Eyedropper,
    Place,
    Brush,
    BoxFill,
    Cube,
    Plane,
    Cylinder,
    Cone,
    Sphere,
    Torus,
    Capsule,
    Picture,
    PictureWave,
    PaintBucket,
    Extrude,
    Inset,
    Bevel,
    Smooth,
    Decimate,
    ShadeSmooth,
    ToVoxel,
    ToMesh,
    BipedRig,
    AxisX,
    AxisY,
    AxisZ,
    ZoomIn,
    ZoomOut,
    ResetCamera,
    Preview,
    ViewIso,
    ViewFront,
    ViewRight,
    ViewTop,
    Open,
    Save,
    Import,
    Close,
    Undo,
    Redo,
    ModePixel,
    ModeVoxel,
    ModeLowPoly,
    ModeCharacter,
    Play,
    Pause,
    Add,
    Remove,
    Keyframe,
    Copy,
    RotateButton,
    Settings,
    Send,
    EyeOpen,
    EyeClosed,
    Reset,
    Stop,
    NewProject,
    Hand,
    Select,
    Sun,
    SceneCamera,
}

impl ToolIcon {
    /// Phosphor glyph for this icon (MIT-licensed font, registered at
    /// startup via `egui_phosphor::add_to_fonts`).
    pub const fn glyph(self) -> &'static str {
        use egui_phosphor::regular as r;
        match self {
            Self::Pencil => r::PENCIL_SIMPLE,
            Self::Eraser => r::ERASER,
            Self::Fill => r::PAINT_BUCKET,
            Self::Line => r::LINE_SEGMENT,
            Self::Rect => r::RECTANGLE,
            Self::Eyedropper => r::EYEDROPPER,
            Self::Place => r::CUBE,
            Self::Brush => r::PAINT_BRUSH,
            Self::BoxFill => r::SQUARES_FOUR,
            Self::Cube => r::CUBE,
            Self::Plane => r::RECTANGLE,
            Self::Cylinder => r::CYLINDER,
            Self::Cone => r::TRIANGLE,
            Self::Sphere => r::SPHERE,
            Self::Torus => r::CIRCLE_DASHED,
            Self::Capsule => r::PILL,
            Self::Picture => r::IMAGE,
            Self::PictureWave => r::FILE_IMAGE,
            Self::PaintBucket => r::PAINT_BUCKET,
            Self::Extrude => r::BOX_ARROW_UP,
            Self::Inset => r::CORNERS_IN,
            Self::Bevel => r::SQUARE_HALF,
            Self::Smooth => r::WAVE_SINE,
            Self::Decimate => r::SCISSORS,
            Self::ShadeSmooth => r::CIRCLE_HALF,
            Self::ToVoxel => r::EXPORT,
            Self::ToMesh => r::ARROW_SQUARE_OUT,
            Self::BipedRig => r::PERSON_SIMPLE,
            Self::AxisX => "X",
            Self::AxisY => "Y",
            Self::AxisZ => "Z",
            Self::ZoomIn => r::PLUS,
            Self::ZoomOut => r::MINUS,
            Self::ResetCamera => r::CROSSHAIR,
            Self::Preview => r::EYE,
            Self::ViewIso => r::CUBE,
            Self::ViewFront => r::SQUARE,
            Self::ViewRight => r::ARROW_LEFT,
            Self::ViewTop => r::ARROW_DOWN,
            Self::Open => r::FOLDER_OPEN,
            Self::Save => r::FLOPPY_DISK,
            Self::Import => r::DOWNLOAD,
            Self::Close => r::X,
            Self::Undo => r::ARROW_U_UP_LEFT,
            Self::Redo => r::ARROW_U_UP_RIGHT,
            Self::ModePixel => r::GRID_NINE,
            Self::ModeVoxel => r::CUBE,
            Self::ModeLowPoly => r::POLYGON,
            Self::ModeCharacter => r::PERSON,
            Self::Play => r::PLAY,
            Self::Pause => r::PAUSE,
            Self::Add => r::PLUS,
            Self::Remove => r::MINUS,
            Self::Keyframe => r::DIAMOND,
            Self::Copy => r::COPY,
            Self::RotateButton => r::ARROW_CLOCKWISE,
            Self::Settings => r::GEAR,
            Self::Send => r::PAPER_PLANE_RIGHT,
            Self::EyeOpen => r::EYE,
            Self::EyeClosed => r::EYE_SLASH,
            Self::Reset => r::ARROW_COUNTER_CLOCKWISE,
            Self::Stop => r::STOP,
            Self::NewProject => r::FILE_PLUS,
            Self::Hand => r::HAND,
            Self::Select => r::CURSOR,
            Self::Sun => r::SUN,
            Self::SceneCamera => r::VIDEO_CAMERA,
        }
    }
}

/// Every icon, for the headless paint smoke test below.
#[cfg(test)]
pub const ALL: [ToolIcon; 67] = [
    ToolIcon::Pencil,
    ToolIcon::Eraser,
    ToolIcon::Fill,
    ToolIcon::Line,
    ToolIcon::Rect,
    ToolIcon::Eyedropper,
    ToolIcon::Place,
    ToolIcon::Brush,
    ToolIcon::BoxFill,
    ToolIcon::Cube,
    ToolIcon::Plane,
    ToolIcon::Cylinder,
    ToolIcon::Cone,
    ToolIcon::Sphere,
    ToolIcon::Torus,
    ToolIcon::Capsule,
    ToolIcon::Picture,
    ToolIcon::PictureWave,
    ToolIcon::PaintBucket,
    ToolIcon::Extrude,
    ToolIcon::Inset,
    ToolIcon::Bevel,
    ToolIcon::Smooth,
    ToolIcon::Decimate,
    ToolIcon::ShadeSmooth,
    ToolIcon::ToVoxel,
    ToolIcon::ToMesh,
    ToolIcon::BipedRig,
    ToolIcon::AxisX,
    ToolIcon::AxisY,
    ToolIcon::AxisZ,
    ToolIcon::ZoomIn,
    ToolIcon::ZoomOut,
    ToolIcon::ResetCamera,
    ToolIcon::Preview,
    ToolIcon::ViewIso,
    ToolIcon::ViewFront,
    ToolIcon::ViewRight,
    ToolIcon::ViewTop,
    ToolIcon::Open,
    ToolIcon::Save,
    ToolIcon::Import,
    ToolIcon::Close,
    ToolIcon::Undo,
    ToolIcon::Redo,
    ToolIcon::ModePixel,
    ToolIcon::ModeVoxel,
    ToolIcon::ModeLowPoly,
    ToolIcon::ModeCharacter,
    ToolIcon::Play,
    ToolIcon::Pause,
    ToolIcon::Add,
    ToolIcon::Remove,
    ToolIcon::Keyframe,
    ToolIcon::Copy,
    ToolIcon::RotateButton,
    ToolIcon::Settings,
    ToolIcon::Send,
    ToolIcon::EyeOpen,
    ToolIcon::EyeClosed,
    ToolIcon::Reset,
    ToolIcon::Stop,
    ToolIcon::NewProject,
    ToolIcon::Hand,
    ToolIcon::Select,
    ToolIcon::Sun,
    ToolIcon::SceneCamera,
];

/// One clickable toolbar cell. To add a button, push one of these into a
/// group list in `toolbar.rs`.
pub struct IconItem {
    pub icon: ToolIcon,
    pub tip: &'static str,
    pub active: bool,
    pub action: ToolbarAction,
}

/// Toolbar behavior. Pure `Command` actions execute directly; file dialogs
/// and toggles are handled by [`fire_toolbar_action`] in `toolbar.rs`.
#[derive(Debug, Clone, Copy)]
pub enum ToolbarAction {
    PixelTool(tessel_core::PixelTool),
    VoxelTool(tessel_core::VoxelTool),
    Primitive(tessel_core::PrimitiveKind),
    CreateTerrain,
    ConvertToVoxel,
    ConvertToMesh,
    TogglePaint,
    LoadTexture { normal: bool },
    FillTexture,
    Extrude,
    Inset,
    Bevel,
    Smooth,
    Decimate,
    ToggleShade,
    SubdivideFace,
    MirrorMeshX,
    ApplyTransform,
    ProjectUvs,
    ApplyBipedRig,
    ApplyHumanoidRig,
    ToggleWeightPaint,
    ClearMeshWeights,
    TogglePixelSymX,
    TogglePixelSymY,
    ToggleVoxelSymX,
    ToggleVoxelSymY,
    ToggleVoxelSymZ,
    ToggleMirrorX,
}

fn button_fill(active: bool) -> Color32 {
    if active {
        tokens::PRIMARY_CONTAINER
    } else {
        tokens::SURFACE_HIGH
    }
}

fn button_ink(active: bool) -> Color32 {
    if active {
        tokens::ON_PRIMARY
    } else {
        tokens::ON_SURFACE
    }
}

/// Compact icon button for tight HUD rows (viewport overlay).
pub fn hud_icon_button(ui: &mut Ui, icon: ToolIcon, tooltip: &str, active: bool) -> bool {
    hud_icon_button_enabled(ui, icon, tooltip, active, true)
}

/// Compact icon button with an enabled flag (disabled buttons don't respond).
pub fn hud_icon_button_enabled(
    ui: &mut Ui,
    icon: ToolIcon,
    tooltip: &str,
    active: bool,
    enabled: bool,
) -> bool {
    hud_icon_button_full(ui, icon, tooltip, active, enabled, false)
}

/// Full-control icon button: enabled flag plus a red danger style for
/// destructive actions (delete node).
pub fn hud_icon_button_full(
    ui: &mut Ui,
    icon: ToolIcon,
    tooltip: &str,
    active: bool,
    enabled: bool,
    danger: bool,
) -> bool {
    let ink = if danger && enabled {
        tokens::ERROR
    } else {
        button_ink(active)
    };
    let btn = egui::Button::new(egui::RichText::new(icon.glyph()).size(15.0).color(ink))
        .min_size(Vec2::new(30.0, 26.0))
        .fill(button_fill(active))
        .corner_radius(egui::CornerRadius::same(4));
    ui.add_enabled(enabled, btn)
        .on_hover_text(tooltip)
        .clicked()
}

/// Text button in icon-button chrome, for cases where the text itself is the
/// content (axis nudge labels). Prefer a real [`ToolIcon`] glyph otherwise.
pub fn text_icon_button(ui: &mut Ui, label: &str, tooltip: &str, active: bool) -> bool {
    let btn = egui::Button::new(
        egui::RichText::new(label)
            .size(12.0)
            .monospace()
            .color(button_ink(active)),
    )
    .min_size(Vec2::new(30.0, 26.0))
    .fill(button_fill(active))
    .corner_radius(egui::CornerRadius::same(4));
    ui.add(btn).on_hover_text(tooltip).clicked()
}

/// Single icon button. `active` highlights it (tool toggles).
pub fn icon_button(ui: &mut Ui, icon: ToolIcon, tooltip: &str, active: bool) -> bool {
    let btn = egui::Button::new(
        egui::RichText::new(icon.glyph())
            .size(18.0)
            .color(button_ink(active)),
    )
    .min_size(Vec2::new(46.0, 30.0))
    .fill(button_fill(active))
    .corner_radius(egui::CornerRadius::same(4));
    ui.add(btn).on_hover_text(tooltip).clicked()
}

/// A titled 2-column group of icon buttons. Returns the clicked action, if
/// any. To add a button, push one more [`IconItem`] into the group's list.
pub fn icon_group(ui: &mut Ui, id: &str, title: &str, items: &[IconItem]) -> Option<ToolbarAction> {
    ui.label(
        egui::RichText::new(title)
            .size(10.0)
            .color(tokens::ON_SURFACE_VARIANT),
    );
    ui.add_space(2.0);
    let mut clicked = None;
    egui::Grid::new(id)
        .num_columns(2)
        .spacing(Vec2::new(4.0, 4.0))
        .show(ui, |ui| {
            for (i, item) in items.iter().enumerate() {
                if icon_button(ui, item.icon, item.tip, item.active) {
                    clicked = Some(item.action);
                }
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
            if items.len() % 2 == 1 {
                ui.end_row();
            }
        });
    ui.add_space(4.0);
    clicked
}

/// Icon plus tooltip for a 2D pixel tool. The match is exhaustive so adding
/// a `PixelTool` variant without an icon fails the build.
pub fn pixel_tool_icon(tool: tessel_core::PixelTool) -> (ToolIcon, &'static str) {
    match tool {
        tessel_core::PixelTool::Pencil => (ToolIcon::Pencil, tool.label()),
        tessel_core::PixelTool::Eraser => (ToolIcon::Eraser, tool.label()),
        tessel_core::PixelTool::Fill => (ToolIcon::Fill, tool.label()),
        tessel_core::PixelTool::Line => (ToolIcon::Line, tool.label()),
        tessel_core::PixelTool::Rect => (ToolIcon::Rect, tool.label()),
        tessel_core::PixelTool::Eyedropper => (ToolIcon::Eyedropper, tool.label()),
    }
}

/// Icon plus tooltip for a voxel tool (exhaustive, same as above).
pub fn voxel_tool_icon(tool: tessel_core::VoxelTool) -> (ToolIcon, &'static str) {
    match tool {
        tessel_core::VoxelTool::Place => (ToolIcon::Place, tool.label()),
        tessel_core::VoxelTool::Erase => (ToolIcon::Eraser, tool.label()),
        tessel_core::VoxelTool::Paint => (ToolIcon::Brush, tool.label()),
        tessel_core::VoxelTool::BoxFill => (ToolIcon::BoxFill, tool.label()),
        tessel_core::VoxelTool::Eyedropper => (ToolIcon::Eyedropper, tool.label()),
    }
}

/// Icon plus tooltip for a low-poly primitive (exhaustive, same as above).
pub fn primitive_icon(kind: tessel_core::PrimitiveKind) -> (ToolIcon, &'static str) {
    match kind {
        tessel_core::PrimitiveKind::Cube => (ToolIcon::Cube, kind.label()),
        tessel_core::PrimitiveKind::Plane => (ToolIcon::Plane, kind.label()),
        tessel_core::PrimitiveKind::Cylinder => (ToolIcon::Cylinder, kind.label()),
        tessel_core::PrimitiveKind::Cone => (ToolIcon::Cone, kind.label()),
        tessel_core::PrimitiveKind::Sphere => (ToolIcon::Sphere, kind.label()),
        tessel_core::PrimitiveKind::Torus => (ToolIcon::Torus, kind.label()),
        tessel_core::PrimitiveKind::Capsule => (ToolIcon::Capsule, kind.label()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_tool_icons_paint_without_panic() {
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(200.0, 1400.0),
            )),
            ..Default::default()
        };
        let _ = ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                for icon in ALL {
                    let _ = icon_button(ui, icon, "tip", false);
                    let _ = icon_button(ui, icon, "tip", true);
                }
                for tool in tessel_core::PixelTool::ALL {
                    let (icon, tip) = pixel_tool_icon(tool);
                    let _ = icon_button(ui, icon, tip, false);
                }
                for tool in tessel_core::VoxelTool::ALL {
                    let (icon, tip) = voxel_tool_icon(tool);
                    let _ = icon_button(ui, icon, tip, false);
                }
                for kind in tessel_core::PrimitiveKind::ALL {
                    let (icon, tip) = primitive_icon(kind);
                    let _ = icon_button(ui, icon, tip, false);
                }
            });
        });
    }
}
