//! 独立地图浮窗（新「地图浮窗」模式）。
//!
//! 把主界面里的地图移到单独的无边框窗口：可以拖动、拖边缘缩放，
//! 透明度、置顶和鼠标穿透都在主窗口里设置。浮窗本身不画标题栏，
//! 悬停时右上角出现关闭按钮；位置和大小会自动写回配置。
use super::shell::centered_window_position;
use super::*;

/// 浮窗默认尺寸：大约放得下一张地图，上下两个任务时用户自行拉高。
pub(super) const MAP_WINDOW_SIZE: Vec2 = Vec2::new(560.0, 460.0);
pub(super) const MAP_WINDOW_MIN_SIZE: Vec2 = Vec2::new(240.0, 180.0);
/// 浮窗不透明度下限：0 会让窗口完全看不见，也找不回来。
pub(super) const MAP_WINDOW_MIN_OPACITY: f32 = 0.1;
/// 贴边多少像素算「拖动缩放」而不是「拖动窗口」。
const RESIZE_BORDER: f32 = 6.0;
/// 右上角关闭按钮的尺寸与边距。
const CLOSE_BUTTON_SIZE: Vec2 = Vec2::new(20.0, 20.0);
const CLOSE_BUTTON_MARGIN: f32 = 6.0;

fn map_window_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("map_window")
}

impl MhxyApp {
    pub(super) fn map_window_size(&self) -> Vec2 {
        self.config
            .map_window_size
            .map(|(width, height)| vec2(width, height).max(MAP_WINDOW_MIN_SIZE))
            .unwrap_or(MAP_WINDOW_SIZE)
    }

    pub(super) fn map_window_alpha(&self) -> u8 {
        let opacity = self
            .config
            .map_window_opacity
            .clamp(MAP_WINDOW_MIN_OPACITY, 1.0);
        (opacity * 255.0).round() as u8
    }

    /// 打开 / 关闭地图浮窗。打开时记下默认位置，关闭后不会丢掉用户调好的位置。
    pub(super) fn set_map_window(&mut self, enabled: bool, ctx: &egui::Context) {
        if self.config.map_window_enabled == enabled {
            return;
        }
        self.config.map_window_enabled = enabled;
        // 新窗口没有继承上一次的置顶 / 穿透状态，缓存要清掉重新下发。
        self.map_window_shadow_attempts = 0;
        self.map_window_last_outer = None;
        self.map_window_last_size = None;
        self.applied_map_window_on_top = None;
        self.applied_map_window_passthrough = None;
        self.map_window_minimized = false;
        if enabled {
            if self.config.map_window_position.is_none() {
                let position = centered_window_position(ctx, self.map_window_size());
                self.config.map_window_position = Some((position.x, position.y));
            }
            self.log_detail(format!(
                "打开独立地图浮窗（{}×{}）",
                self.map_window_size().x.round(),
                self.map_window_size().y.round()
            ));
        } else {
            self.log_detail("关闭独立地图浮窗");
        }
        self.persist();
    }

    /// 把浮窗恢复到默认大小并放回主窗口中间。
    pub(super) fn reset_map_window_geometry(&mut self, ctx: &egui::Context) {
        let size = MAP_WINDOW_SIZE;
        let position = centered_window_position(ctx, size);
        self.config.map_window_size = Some((size.x, size.y));
        self.config.map_window_position = Some((position.x, position.y));
        self.map_window_last_size = Some(size);
        self.map_window_last_outer = Some(position);
        if self.config.map_window_enabled {
            let id = map_window_id();
            ctx.send_viewport_cmd_to(id, egui::ViewportCommand::InnerSize(size));
            ctx.send_viewport_cmd_to(id, egui::ViewportCommand::OuterPosition(position));
        }
        self.persist();
    }

    /// 每帧调度浮窗；只有模式开启时才创建原生窗口。
    pub(super) fn map_window(&mut self, ctx: &egui::Context) {
        if !self.config.map_window_enabled {
            return;
        }
        let size = self.map_window_size();
        let mut builder = egui::ViewportBuilder::default()
            .with_title("地图浮窗")
            .with_decorations(false)
            .with_transparent(true)
            .with_resizable(true)
            .with_inner_size(size)
            .with_min_inner_size(MAP_WINDOW_MIN_SIZE);
        if self.config.map_window_on_top {
            builder = builder.with_always_on_top();
        }
        // 只在建立窗口时定位，之后用户拖动不会被每帧抢回。
        if let Some((x, y)) = self.config.map_window_position {
            builder = builder.with_position(pos2(x, y));
        }
        ctx.show_viewport_immediate(map_window_id(), builder, |ui, _class| {
            if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
                // 独立视口还没建好（测试 / 首帧），不画东西，避免闪一帧带边框的窗口。
                return;
            }
            self.map_window_ui(ui);
        });
    }

    fn map_window_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if ctx.input(|input| input.viewport().close_requested()) {
            self.set_map_window(false, &ctx);
            return;
        }
        // Windows 会给无边框窗口加 DWM 投影和圆角，透明背景下看得到；
        // 窗口刚创建时可能还找不到，下一帧再试（其他平台是空操作）。
        if self.map_window_shadow_attempts < 30 {
            self.map_window_shadow_attempts += 1;
            if borderless::prepare("地图浮窗", true) {
                self.map_window_shadow_attempts = 30;
            }
        }
        self.sync_map_window_state(&ctx);
        // Win+D（显示桌面）会把无边框工具窗口一起最小化。它没有任务栏按钮，
        // 不主动还原就会彻底消失，所以只在「可见→最小化」边沿还原一次。
        let minimized = ctx.input(|input| input.viewport().minimized == Some(true));
        if minimized && !self.map_window_minimized {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        }
        self.map_window_minimized = minimized;

        // 整窗按不透明度绘制：地图、标记和底板一起变透明。
        let opacity = self
            .config
            .map_window_opacity
            .clamp(MAP_WINDOW_MIN_OPACITY, 1.0);
        let alpha = self.map_window_alpha();
        let rect = ui.max_rect();
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            CornerRadius::same(8),
            Color32::from_rgb(15, 20, 28).gamma_multiply(opacity),
        );
        painter.rect_stroke(
            rect.shrink(0.5),
            CornerRadius::same(8),
            egui::Stroke::new(1.0, theme::border().gamma_multiply(opacity)),
            egui::StrokeKind::Inside,
        );

        let content = rect.shrink(2.0);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content));
        let has_task = TASK_ORDER.iter().any(|task| self.task_active(task));
        if has_task {
            self.task_area(&mut child, alpha, false);
            if !self.has_task_location() {
                child.painter().text(
                    content.center_bottom() - vec2(0.0, 12.0),
                    Align2::CENTER_CENTER,
                    "等待识别到任务…",
                    FontId::proportional(13.0),
                    theme::muted().gamma_multiply(opacity),
                );
            }
        } else {
            child.painter().text(
                content.center(),
                Align2::CENTER_CENTER,
                "未勾选任务类型",
                FontId::proportional(14.0),
                theme::muted().gamma_multiply(opacity),
            );
        }

        // 锁定（鼠标穿透）时窗口不接收任何输入，只能回主界面取消。
        if self.config.map_window_locked {
            return;
        }
        let surface_id = ui.id().with("map_window_surface");
        let response = ui.interact(rect, surface_id, Sense::click_and_drag());
        let close_rect = Rect::from_min_size(
            pos2(
                rect.right() - CLOSE_BUTTON_MARGIN - CLOSE_BUTTON_SIZE.x,
                rect.top() + CLOSE_BUTTON_MARGIN,
            ),
            CLOSE_BUTTON_SIZE,
        );
        let pointer = response.hover_pos();
        if response.hovered() {
            if let Some(pos) = pointer {
                if close_rect.contains(pos) {
                    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                } else if let Some(direction) = resize_direction(rect, pos, RESIZE_BORDER) {
                    ctx.set_cursor_icon(cursor_for(direction));
                }
            }
        }
        // 关闭按钮后画，保证盖在拖动热区上面能正常点到；只在悬停时出现。
        if response.hovered() {
            let button = egui::Button::new(
                egui::RichText::new("✕")
                    .size(12.0)
                    .color(Color32::WHITE.gamma_multiply(opacity)),
            )
            .fill(Color32::from_black_alpha(150).gamma_multiply(opacity))
            .corner_radius(4.0)
            .min_size(CLOSE_BUTTON_SIZE);
            if ui.put(close_rect, button).clicked() {
                self.set_map_window(false, &ctx);
                return;
            }
        }
        if response.drag_started() {
            if let Some(pos) = response.interact_pointer_pos() {
                if !close_rect.contains(pos) {
                    match resize_direction(rect, pos, RESIZE_BORDER) {
                        Some(direction) => {
                            ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(direction))
                        }
                        None => ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag),
                    }
                }
            }
        }
    }

    /// 同步置顶 / 鼠标穿透，并把拖动、缩放后的位置和大小写回配置。
    fn sync_map_window_state(&mut self, ctx: &egui::Context) {
        let on_top = self.config.map_window_on_top;
        if self.applied_map_window_on_top != Some(on_top) {
            let level = if on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
            self.applied_map_window_on_top = Some(on_top);
        }
        let locked = self.config.map_window_locked;
        if self.applied_map_window_passthrough != Some(locked) {
            ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(locked));
            self.applied_map_window_passthrough = Some(locked);
        }
        let (outer, inner) = ctx.input(|input| {
            let viewport = input.viewport();
            (viewport.outer_rect, viewport.inner_rect)
        });
        if let Some(inner) = inner {
            let size = inner.size();
            let stored = self
                .config
                .map_window_size
                .map(|(width, height)| vec2(width, height));
            if self
                .map_window_last_size
                .is_none_or(|last| (last - size).length() > 0.5)
            {
                self.map_window_last_size = Some(size);
                if stored.is_none_or(|stored| (stored - size).length() > 0.5) {
                    self.config.map_window_size = Some((size.x, size.y));
                    self.persist();
                }
            }
        }
        if let Some(outer) = outer {
            let position = outer.min;
            let stored = self.config.map_window_position.map(|(x, y)| pos2(x, y));
            if self
                .map_window_last_outer
                .is_none_or(|last| last.distance(position) > 0.5)
            {
                self.map_window_last_outer = Some(position);
                if stored.is_none_or(|stored| stored.distance(position) > 0.5) {
                    self.config.map_window_position = Some((position.x, position.y));
                    self.persist();
                }
            }
        }
    }

    /// 主窗口里的占位：地图已经弹到浮窗，这里只留说明和关闭入口。
    pub(super) fn map_window_placeholder(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_rect_before_wrap();
        let (rect, _) = ui.allocate_exact_size(available.size(), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(8), theme::inset());
        painter.text(
            pos2(rect.center().x, rect.center().y - 48.0),
            Align2::CENTER_CENTER,
            "地图已在独立浮窗中显示",
            FontId::proportional(18.0),
            theme::text(),
        );
        // 提示文字按可用宽度换行，主窗口变窄时不会冲出面板。
        let hint = painter.layout(
            "拖动浮窗移动位置，拖动边缘调整大小；透明度、置顶和锁定在「显示与布局」里调整"
                .to_owned(),
            FontId::proportional(13.0),
            theme::muted(),
            (rect.width() - 48.0).max(80.0),
        );
        let hint_pos = pos2(
            rect.center().x - hint.size().x / 2.0,
            rect.center().y - 18.0,
        );
        painter.galley(hint_pos, hint, theme::muted());
        let button_rect =
            Rect::from_center_size(rect.center() + vec2(0.0, 60.0), vec2(150.0, 32.0));
        if ui
            .put(button_rect, egui::Button::new("关闭地图浮窗"))
            .clicked()
        {
            self.set_map_window(false, ui.ctx());
        }
    }

    fn has_task_location(&self) -> bool {
        TASK_ORDER.iter().any(|task| {
            self.task_active(task)
                && self
                    .tasks
                    .get(task)
                    .is_some_and(|state| state.location.is_some())
        })
    }
}

/// 指针落在窗口哪条边 / 哪个角：角落优先于单边。
pub(super) fn resize_direction(
    rect: Rect,
    pos: egui::Pos2,
    border: f32,
) -> Option<egui::ResizeDirection> {
    let left = pos.x - rect.left() <= border;
    let right = rect.right() - pos.x <= border;
    let top = pos.y - rect.top() <= border;
    let bottom = rect.bottom() - pos.y <= border;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(egui::ResizeDirection::NorthWest),
        (_, true, true, _) => Some(egui::ResizeDirection::NorthEast),
        (true, _, _, true) => Some(egui::ResizeDirection::SouthWest),
        (_, true, _, true) => Some(egui::ResizeDirection::SouthEast),
        (true, _, _, _) => Some(egui::ResizeDirection::West),
        (_, true, _, _) => Some(egui::ResizeDirection::East),
        (_, _, true, _) => Some(egui::ResizeDirection::North),
        (_, _, _, true) => Some(egui::ResizeDirection::South),
        _ => None,
    }
}

fn cursor_for(direction: egui::ResizeDirection) -> egui::CursorIcon {
    match direction {
        egui::ResizeDirection::North | egui::ResizeDirection::South => {
            egui::CursorIcon::ResizeVertical
        }
        egui::ResizeDirection::East | egui::ResizeDirection::West => {
            egui::CursorIcon::ResizeHorizontal
        }
        egui::ResizeDirection::NorthWest | egui::ResizeDirection::SouthEast => {
            egui::CursorIcon::ResizeNwSe
        }
        egui::ResizeDirection::NorthEast | egui::ResizeDirection::SouthWest => {
            egui::CursorIcon::ResizeNeSw
        }
    }
}
