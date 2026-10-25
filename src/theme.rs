//! 全局配色与通用界面组件，支持暗色 / 亮色两套主题。
use egui::{Color32, CornerRadius, FontId, RichText, Stroke, TextStyle};
use std::sync::atomic::{AtomicBool, Ordering};

static LIGHT_MODE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy)]
struct Palette {
    background: Color32,
    surface: Color32,
    inset: Color32,
    border: Color32,
    text: Color32,
    muted: Color32,
    jade: Color32,
    gold: Color32,
    red: Color32,
    selection: Color32,
    control: Color32,
    control_hover: Color32,
    control_active: Color32,
    on_accent: Color32,
}

const DARK: Palette = Palette {
    background: Color32::from_rgb(15, 20, 28),
    surface: Color32::from_rgb(22, 29, 39),
    inset: Color32::from_rgb(12, 17, 24),
    border: Color32::from_rgb(43, 55, 70),
    text: Color32::from_rgb(229, 236, 243),
    muted: Color32::from_rgb(142, 159, 178),
    jade: Color32::from_rgb(99, 218, 183),
    gold: Color32::from_rgb(239, 194, 113),
    red: Color32::from_rgb(244, 132, 145),
    selection: Color32::from_rgb(36, 79, 72),
    control: Color32::from_rgb(33, 43, 56),
    control_hover: Color32::from_rgb(43, 60, 72),
    control_active: Color32::from_rgb(39, 80, 70),
    on_accent: Color32::from_rgb(12, 17, 24),
};

const LIGHT: Palette = Palette {
    background: Color32::from_rgb(243, 245, 248),
    surface: Color32::from_rgb(255, 255, 255),
    inset: Color32::from_rgb(237, 240, 244),
    border: Color32::from_rgb(211, 218, 227),
    text: Color32::from_rgb(29, 39, 51),
    muted: Color32::from_rgb(95, 111, 128),
    jade: Color32::from_rgb(15, 143, 111),
    gold: Color32::from_rgb(176, 116, 26),
    red: Color32::from_rgb(192, 57, 43),
    selection: Color32::from_rgb(205, 238, 225),
    control: Color32::from_rgb(232, 237, 243),
    control_hover: Color32::from_rgb(219, 228, 238),
    control_active: Color32::from_rgb(201, 233, 221),
    on_accent: Color32::from_rgb(255, 255, 255),
};

fn palette() -> Palette {
    if LIGHT_MODE.load(Ordering::Relaxed) {
        LIGHT
    } else {
        DARK
    }
}

pub fn is_light() -> bool {
    LIGHT_MODE.load(Ordering::Relaxed)
}

pub fn background() -> Color32 {
    palette().background
}
pub fn surface() -> Color32 {
    palette().surface
}
pub fn inset() -> Color32 {
    palette().inset
}
pub fn border() -> Color32 {
    palette().border
}
pub fn text() -> Color32 {
    palette().text
}
pub fn muted() -> Color32 {
    palette().muted
}
pub fn jade() -> Color32 {
    palette().jade
}
pub fn gold() -> Color32 {
    palette().gold
}
pub fn red() -> Color32 {
    palette().red
}
/// 强调色（青绿 / 金色）上的文字颜色。
pub fn on_accent() -> Color32 {
    palette().on_accent
}

pub fn install(ctx: &egui::Context, light: bool) {
    LIGHT_MODE.store(light, Ordering::Relaxed);
    apply_style(ctx);
}

/// 按配置值解析实际主题：`system` 跟随操作系统，其余强制亮 / 暗。
pub fn resolve_light(mode: &str, ctx: &egui::Context) -> bool {
    match mode {
        "light" => true,
        "dark" => false,
        _ => ctx.system_theme() == Some(egui::Theme::Light),
    }
}

/// 运行中切换主题，立即生效。
pub fn set_light(ctx: &egui::Context, light: bool) {
    LIGHT_MODE.store(light, Ordering::Relaxed);
    apply_style(ctx);
    ctx.request_repaint();
}

fn apply_style(ctx: &egui::Context) {
    let palette = palette();
    let theme = if is_light() {
        egui::Theme::Light
    } else {
        egui::Theme::Dark
    };
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
    style.visuals = if is_light() {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    let visuals = &mut style.visuals;
    visuals.override_text_color = Some(palette.text);
    visuals.weak_text_color = Some(palette.muted);
    visuals.panel_fill = palette.background;
    visuals.window_fill = palette.surface;
    visuals.extreme_bg_color = palette.inset;
    visuals.faint_bg_color = palette.surface;
    visuals.selection.bg_fill = palette.selection;
    visuals.selection.stroke = Stroke::new(1.0, palette.jade);
    visuals.hyperlink_color = palette.jade;
    visuals.warn_fg_color = palette.gold;
    visuals.error_fg_color = palette.red;
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.window_stroke = Stroke::new(1.0, palette.border);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        widget.corner_radius = CornerRadius::same(6);
        widget.fg_stroke = Stroke::new(1.0, palette.text);
    }
    visuals.widgets.inactive.weak_bg_fill = palette.control;
    visuals.widgets.inactive.bg_fill = palette.control;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, palette.border);
    visuals.widgets.hovered.weak_bg_fill = palette.control_hover;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, palette.jade.gamma_multiply(0.65));
    visuals.widgets.active.weak_bg_fill = palette.control_active;
    visuals.widgets.active.bg_fill = palette.control_active;

    ctx.set_theme(if is_light() {
        egui::ThemePreference::Light
    } else {
        egui::ThemePreference::Dark
    });
    ctx.set_style_of(theme, style);
}

pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(surface())
        .stroke(Stroke::new(1.0, border()))
        .corner_radius(10)
        .inner_margin(14)
}

/// 可折叠的分组卡片，返回当前是否展开（用于持久化折叠状态）。
pub fn collapsible_section(
    ui: &mut egui::Ui,
    id: &str,
    title: &str,
    default_open: bool,
    content: impl FnOnce(&mut egui::Ui),
) -> bool {
    let mut open = default_open;
    card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        let response =
            egui::CollapsingHeader::new(RichText::new(title).size(13.0).color(muted()).strong())
                .id_salt(id)
                .default_open(default_open)
                .show(ui, |ui| content(ui));
        open = response.openness > 0.5;
    });
    open
}

pub fn primary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(on_accent()).strong())
        .fill(jade())
        .min_size(egui::vec2(100.0, 34.0))
}
