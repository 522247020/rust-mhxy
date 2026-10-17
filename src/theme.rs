//! 全局配色与通用界面组件。
use egui::{Color32, CornerRadius, FontId, RichText, Stroke, TextStyle};

pub const BACKGROUND: Color32 = Color32::from_rgb(15, 20, 28);
pub const SURFACE: Color32 = Color32::from_rgb(22, 29, 39);
pub const INSET: Color32 = Color32::from_rgb(12, 17, 24);
pub const BORDER: Color32 = Color32::from_rgb(43, 55, 70);
pub const TEXT: Color32 = Color32::from_rgb(229, 236, 243);
pub const MUTED: Color32 = Color32::from_rgb(142, 159, 178);
pub const JADE: Color32 = Color32::from_rgb(99, 218, 183);
pub const GOLD: Color32 = Color32::from_rgb(239, 194, 113);
pub const RED: Color32 = Color32::from_rgb(244, 132, 145);

pub fn install(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut style = egui::Style::default();
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(21.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(14.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(14.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(12.0));
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 30.0;
    style.spacing.combo_width = 160.0;
    style.visuals = egui::Visuals::dark();
    let visuals = &mut style.visuals;
    visuals.override_text_color = Some(TEXT);
    visuals.weak_text_color = Some(MUTED);
    visuals.panel_fill = BACKGROUND;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = INSET;
    visuals.faint_bg_color = SURFACE;
    visuals.selection.bg_fill = Color32::from_rgb(36, 79, 72);
    visuals.selection.stroke = Stroke::new(1.0, JADE);
    visuals.hyperlink_color = JADE;
    visuals.warn_fg_color = GOLD;
    visuals.error_fg_color = RED;
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        widget.corner_radius = CornerRadius::same(6);
        widget.fg_stroke = Stroke::new(1.0, TEXT);
    }
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(33, 43, 56);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(33, 43, 56);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(43, 60, 72);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, JADE.gamma_multiply(0.65));
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(39, 80, 70);
    ctx.set_style_of(egui::Theme::Dark, style);
}

pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(10)
        .inner_margin(14)
}

pub fn section(ui: &mut egui::Ui, title: &str, content: impl FnOnce(&mut egui::Ui)) {
    card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).size(13.0).color(MUTED).strong());
        ui.add_space(2.0);
        content(ui);
    });
}

pub fn primary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(INSET).strong())
        .fill(JADE)
        .min_size(egui::vec2(100.0, 34.0))
}
