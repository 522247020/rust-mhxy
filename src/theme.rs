//! 全局配色与通用界面组件，支持暗色 / 亮色两套主题。
use egui::{Color32, CornerRadius, FontFamily, FontId, RichText, Stroke, TextStyle};
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
    jade_deep: Color32,
    gold: Color32,
    red: Color32,
    selection: Color32,
    control: Color32,
    control_hover: Color32,
    control_active: Color32,
    on_accent: Color32,
}

const DARK: Palette = Palette {
    // Windows 11 深色：Mica 底 #202020、卡片 #2B2B2B，强调色 #4CC2FF。
    background: Color32::from_rgb(32, 32, 32),
    surface: Color32::from_rgb(43, 43, 43),
    inset: Color32::from_rgb(45, 45, 45),
    border: Color32::from_rgb(51, 51, 51),
    text: Color32::from_rgb(255, 255, 255),
    muted: Color32::from_rgb(179, 179, 179),
    jade: Color32::from_rgb(76, 194, 255),
    jade_deep: Color32::from_rgb(15, 108, 189),
    gold: Color32::from_rgb(242, 183, 5),
    red: Color32::from_rgb(255, 153, 164),
    selection: Color32::from_rgb(76, 194, 255),
    control: Color32::from_rgb(51, 51, 51),
    control_hover: Color32::from_rgb(58, 58, 58),
    control_active: Color32::from_rgb(56, 56, 56),
    on_accent: Color32::from_rgb(0, 0, 0),
};

const LIGHT: Palette = Palette {
    // Windows 11 浅色：Mica 底 #F9F9F9、卡片白，强调色 #0067B8。
    background: Color32::from_rgb(249, 249, 249),
    surface: Color32::from_rgb(255, 255, 255),
    inset: Color32::from_rgb(243, 243, 243),
    border: Color32::from_rgb(229, 229, 229),
    text: Color32::from_rgb(27, 27, 27),
    muted: Color32::from_rgb(96, 96, 96),
    jade: Color32::from_rgb(0, 103, 184),
    jade_deep: Color32::from_rgb(0, 90, 158),
    gold: Color32::from_rgb(158, 107, 0),
    red: Color32::from_rgb(196, 43, 28),
    selection: Color32::from_rgb(0, 103, 184),
    control: Color32::from_rgb(243, 243, 243),
    control_hover: Color32::from_rgb(241, 241, 241),
    control_active: Color32::from_rgb(233, 233, 233),
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
/// 深青玉：实心主按钮用它，压住霓虹感。
pub fn jade_deep() -> Color32 {
    palette().jade_deep
}
/// 青玉淡色：选中项的底色，jade 以约 16% 不透明度压在面板上。
pub fn jade_tint() -> Color32 {
    let j = jade();
    Color32::from_rgba_unmultiplied(j.r(), j.g(), j.b(), 40)
}
/// 金色淡色：选中 / 提示的底色，gold 以约 16% 不透明度压在面板上。
pub fn gold_tint() -> Color32 {
    let g = gold();
    Color32::from_rgba_unmultiplied(g.r(), g.g(), g.b(), 40)
}
pub fn gold() -> Color32 {
    palette().gold
}
pub fn red() -> Color32 {
    palette().red
}
#[allow(dead_code)]
/// 强调色（Win11 蓝）上的文字颜色。
pub fn on_accent() -> Color32 {
    palette().on_accent
}
#[allow(dead_code)]
pub fn control() -> Color32 {
    palette().control
}
/// Fluent 图标（Segoe MDL2 Assets），没有图标字体时返回 None。
pub fn icon_font() -> Option<FontId> {
    if crate::has_icon_font() {
        Some(FontId::new(20.0, FontFamily::Name("icons".into())))
    } else {
        None
    }
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
    // 字号阶梯：标题 22 / 正文 14 / 按钮 14.5 / 小字 12 / 等宽 13.5。
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(22.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(14.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(14.5));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(12.0));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(13.5));
    // 8pt 基数：行距收紧，呼吸感靠卡片内边距和卡片间距，不靠堆高度。
    style.spacing.item_spacing = egui::vec2(8.0, 8.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.interact_size.y = 32.0;
    style.spacing.combo_width = 160.0;
    // 滚动条细一点、贴边，不抢内容注意力。
    style.spacing.scroll = egui::style::ScrollStyle::thin();
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
    visuals.window_corner_radius = CornerRadius::same(16);
    visuals.window_stroke = Stroke::new(1.0, palette.border);
    // 窗口阴影：矩形硬阴影没有羽化，只能做短、淡、贴边的一圈，避免发灰发糊。
    visuals.window_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: if is_light() {
            Color32::from_black_alpha(40)
        } else {
            Color32::from_black_alpha(120)
        },
    };
    visuals.popup_shadow = egui::Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: if is_light() {
            Color32::from_black_alpha(32)
        } else {
            Color32::from_black_alpha(90)
        },
    };
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.border);
    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
    ] {
        widget.corner_radius = CornerRadius::same(8);
        widget.fg_stroke = Stroke::new(1.0, palette.text);
    }
    // 控件默认是幽灵样式：不填色块、不描边，悬停才浮出淡底；勾选框的未选态走 weak_bg_fill 保留底色。
    visuals.widgets.inactive.weak_bg_fill = palette.control;
    // inactive.bg_fill 不能透明：滑块轨道（和复选框、输入框的底）都用它，
    // 透明了轨道就只剩滑块，进度条看不见。
    visuals.widgets.inactive.bg_fill = palette.control;
    visuals.widgets.inactive.bg_stroke = Stroke::NONE;
    visuals.widgets.hovered.weak_bg_fill = palette.control_hover;
    visuals.widgets.hovered.bg_fill = palette.control_hover;
    visuals.widgets.hovered.bg_stroke = Stroke::NONE;
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
    card_with(surface())
}

/// 换底色的卡片：状态卡、工具卡这类需要和普通分组卡区分的用这个。
/// Windows 11 风格：8px 圆角、1px 浅边、短而淡的柔阴影。
pub fn card_with(fill: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border()))
        .corner_radius(8)
        .shadow(egui::Shadow {
            offset: [0, 2],
            blur: 6,
            spread: 0,
            color: if is_light() {
                Color32::from_black_alpha(28)
            } else {
                Color32::from_black_alpha(80)
            },
        })
        .inner_margin(16)
}

/// 横向分隔线：内容区内分组用，颜色走主题描边色。
pub fn divider(ui: &mut egui::Ui) {
    let width = ui.available_width().max(1.0);
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 1.0), egui::Sense::hover());
    ui.painter().line_segment(
        [rect.left_center(), rect.right_center()],
        Stroke::new(1.0, border()),
    );
    ui.add_space(4.0);
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
        let area = ui.available_rect_before_wrap();
        let response =
            egui::CollapsingHeader::new(RichText::new(title).size(15.0).color(text()).strong())
                .id_salt(id)
                .default_open(default_open)
                .show(ui, |ui| content(ui));
        open = response.openness > 0.5;
        // 左侧 3px 青玉强调条，落在卡片内边距区：每张分组卡一眼有重心。
        // 高度跟折叠状态走：折叠时只剩标题高，展开时覆盖正文；不要用入口的 area.height()
        // （那是折叠前的全文高度，折叠后条子会跟不下去）。
        let header_rect = response.header_response.rect;
        let bottom = response
            .body_response
            .as_ref()
            .map(|body| body.rect.max.y)
            .unwrap_or(header_rect.max.y);
        let top = header_rect.top() + 4.0;
        let painter = ui.painter();
        let bar = egui::Rect::from_min_size(
            egui::pos2(area.left() - 12.0, top),
            egui::vec2(3.0, (bottom - top).max(12.0)),
        );
        painter.rect_filled(bar, CornerRadius::same(2), jade().gamma_multiply(0.8));
        // 标题下沿一条分隔线，标题和内容分开。
        let header = response.header_response.rect;
        painter.line_segment(
            [header.left_bottom(), header.right_bottom()],
            Stroke::new(1.0, border().gamma_multiply(0.6)),
        );
    });
    open
}

/// 实心主按钮：深青玉底 + 白字，不刺眼但一眼认得出是主操作。
pub fn primary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(Color32::WHITE).strong())
        .fill(jade_deep())
        .min_size(egui::vec2(100.0, 36.0))
}
/// 次级按钮：幽灵描边，不填色块。
pub fn secondary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(text)
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, border().gamma_multiply(0.65)))
}
/// 危险按钮：红色文字、透明底，用于删除等破坏性操作。
pub fn danger_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(RichText::new(text).color(red()))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, red().gamma_multiply(0.45)))
}

/// 状态胶囊：圆点 + 文字，颜色由调用方按状态传入（绿 = 运行中，金 = 警告，灰 = 闲置）。
pub fn status_pill(ui: &mut egui::Ui, dot: Color32, label: &str) {
    // 不要底色胶囊：只留状态色圆点 + 文字，和 Win11 顶栏干净。
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.colored_label(dot, "●");
        ui.label(RichText::new(label).size(13.0).color(text()));
    });
}

/// 分组小标题：弱色 + 粗体，用于卡片内二次分组。
pub fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).size(12.5).color(muted()).strong());
}
