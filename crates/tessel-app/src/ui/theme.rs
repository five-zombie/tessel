use egui::{Color32, CornerRadius, Margin, Shadow, Stroke, TextStyle, Visuals};

/// Tactile Precision Studio color tokens mapped 1:1 from `.knowledges/design/tactile_precision_studio/DESIGN.md`.
pub mod tokens {
    use egui::Color32;

    pub const SURFACE: Color32 = Color32::from_rgb(0x10, 0x13, 0x19);
    pub const SURFACE_LOW: Color32 = Color32::from_rgb(0x19, 0x1C, 0x22);
    pub const SURFACE_CONTAINER: Color32 = Color32::from_rgb(0x1D, 0x20, 0x26);
    pub const SURFACE_HIGH: Color32 = Color32::from_rgb(0x27, 0x2A, 0x30);
    pub const SURFACE_HIGHEST: Color32 = Color32::from_rgb(0x32, 0x35, 0x3B);

    pub const ON_SURFACE: Color32 = Color32::from_rgb(0xE1, 0xE2, 0xEB);
    pub const ON_SURFACE_VARIANT: Color32 = Color32::from_rgb(0xDA, 0xC2, 0xB2);
    pub const OUTLINE_VARIANT: Color32 = Color32::from_rgb(0x54, 0x43, 0x38);

    pub const PRIMARY: Color32 = Color32::from_rgb(0xFF, 0xB7, 0x80);
    pub const PRIMARY_CONTAINER: Color32 = Color32::from_rgb(0xE5, 0x8A, 0x3C);
    pub const ON_PRIMARY: Color32 = Color32::from_rgb(0x4E, 0x26, 0x00);
    pub const ERROR: Color32 = Color32::from_rgb(0xFF, 0xB4, 0xAB);
}

pub fn apply_tactile_precision_theme(ctx: &egui::Context) {
    let mut visuals = Visuals::dark();
    let border_stroke = Stroke::new(1.0, Color32::from_rgba_unmultiplied(0x54, 0x43, 0x38, 128));

    visuals.extreme_bg_color = tokens::SURFACE;
    visuals.panel_fill = tokens::SURFACE_LOW;
    visuals.window_fill = tokens::SURFACE_CONTAINER;
    visuals.faint_bg_color = tokens::SURFACE_CONTAINER;
    visuals.override_text_color = Some(tokens::ON_SURFACE);
    visuals.hyperlink_color = tokens::PRIMARY;
    visuals.error_fg_color = tokens::ERROR;
    visuals.window_shadow = Shadow::NONE;
    visuals.popup_shadow = Shadow::NONE;
    visuals.window_corner_radius = CornerRadius::same(6);
    visuals.window_stroke = border_stroke;

    visuals.selection.bg_fill = tokens::PRIMARY_CONTAINER;
    visuals.selection.stroke = Stroke::new(1.0, tokens::PRIMARY);

    visuals.widgets.noninteractive.bg_fill = tokens::SURFACE_LOW;
    visuals.widgets.noninteractive.bg_stroke = border_stroke;
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(4);

    visuals.widgets.inactive.bg_fill = tokens::SURFACE_HIGH;
    visuals.widgets.inactive.weak_bg_fill = tokens::SURFACE_HIGH;
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.inactive.corner_radius = CornerRadius::same(4);

    visuals.widgets.hovered.bg_fill = tokens::SURFACE_HIGHEST;
    visuals.widgets.hovered.weak_bg_fill = tokens::SURFACE_HIGHEST;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, tokens::OUTLINE_VARIANT);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(4);

    visuals.widgets.active.bg_fill = tokens::PRIMARY_CONTAINER;
    visuals.widgets.active.fg_stroke = Stroke::new(1.5, tokens::ON_PRIMARY);
    visuals.widgets.active.corner_radius = CornerRadius::same(4);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(6.0, 6.0);
    style.spacing.button_padding = egui::vec2(8.0, 4.0);
    style.spacing.window_margin = Margin::same(10);

    if let Some(font) = style.text_styles.get_mut(&TextStyle::Heading) {
        font.size = 15.0;
    }
    if let Some(font) = style.text_styles.get_mut(&TextStyle::Body) {
        font.size = 12.5;
    }
    if let Some(font) = style.text_styles.get_mut(&TextStyle::Small) {
        font.size = 11.0;
    }
    if let Some(font) = style.text_styles.get_mut(&TextStyle::Monospace) {
        font.size = 11.5;
    }

    ctx.set_style(style);
}
