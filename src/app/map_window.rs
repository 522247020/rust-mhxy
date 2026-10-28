//! 独立地图浮窗（新「地图浮窗」模式）。
//!
//! 每个任务类型（抓鬼 / 鬼王 / 通用）都有自己的无边框窗口，可以分别拖动、
//! 缩放和摆放。透明度、置顶和鼠标穿透在主窗口里统一设置；每个窗口的位置
//! 和大小在拖动结束后自动写回配置。
use super::shell::centered_window_position;
use super::*;
use crate::config::MapWindowLayout;

/// 单个浮窗的默认尺寸：大约放得下一张地图。
pub(super) const MAP_WINDOW_SIZE: Vec2 = Vec2::new(560.0, 460.0);
pub(super) const MAP_WINDOW_MIN_SIZE: Vec2 = Vec2::new(240.0, 180.0);
/// 浮窗不透明度下限：0 会让窗口完全看不见，也找不回来。
pub(super) const MAP_WINDOW_MIN_OPACITY: f32 = 0.1;
/// 贴边多少像素算「拖动缩放」而不是「拖动窗口」。
const RESIZE_BORDER: f32 = 6.0;
/// 右上角关闭按钮的尺寸与边距。
const CLOSE_BUTTON_SIZE: Vec2 = Vec2::new(20.0, 20.0);
const CLOSE_BUTTON_MARGIN: f32 = 6.0;
/// 多个浮窗首次出现时逐层错开的距离，避免完全叠在一起。
const CASCADE_OFFSET: f32 = 28.0;

/// 每个任务浮窗的运行时缓存，不进入配置文件。
#[derive(Default)]
pub(super) struct MapWindowRuntime {
    last_outer: Option<egui::Pos2>,
    last_size: Option<Vec2>,
    applied_on_top: Option<bool>,
    applied_passthrough: Option<bool>,
    shadow_attempts: u8,
    /// 上一帧是否最小化：无边框工具窗口没有任务栏按钮，Win+D 之后必须自己还原。
    minimized: bool,
}

pub(super) fn map_window_id(task: &str) -> egui::ViewportId {
    egui::ViewportId::from_hash_of(("map_window", task))
}

/// 浮窗标题同时用于 Windows 无边框修饰时的窗口查找，必须一窗一标题。
fn map_window_title(task: &str) -> String {
    format!("地图浮窗 · {task}")
}

impl MhxyApp {
    pub(super) fn map_window_alpha(&self) -> u8 {
        let opacity = self
            .config
            .map_window_opacity
            .clamp(MAP_WINDOW_MIN_OPACITY, 1.0);
        (opacity * 255.0).round() as u8
    }

    /// 当前启用的任务类型（顺序与 TASK_ORDER 一致）。
    pub(super) fn active_tasks(&self) -> Vec<&'static str> {
        TASK_ORDER
            .iter()
            .copied()
            .filter(|task| self.task_active(task))
            .collect()
    }

    /// 某个任务的浮窗当前是否应该显示。
    pub(super) fn map_window_open(&self, task: &str) -> bool {
        self.config.map_window_enabled
            && self.task_active(task)
            && !self.closed_map_windows.contains(task)
    }

    /// 打开 / 关闭整个浮窗模式。重新打开时所有任务浮窗都会恢复。
    pub(super) fn set_map_window(&mut self, enabled: bool) {
        if self.config.map_window_enabled == enabled {
            return;
        }
        self.config.map_window_enabled = enabled;
        // 新窗口没有继承上一次的置顶 / 穿透状态，缓存和手动关闭记录都清空。
        self.map_windows.clear();
        self.closed_map_windows.clear();
        if enabled {
            self.log_detail("打开独立地图浮窗（每个任务一个窗口）");
        } else {
            self.log_detail("关闭独立地图浮窗");
        }
        self.persist();
    }

    /// 单独显示 / 隐藏某个任务的浮窗。
    pub(super) fn set_map_window_visible(&mut self, task: &'static str, visible: bool) {
        if visible {
            self.closed_map_windows.remove(task);
        } else {
            self.closed_map_windows.insert(task);
        }
        // 重新打开时按配置里的位置 / 大小重建，需要重置缓存。
        self.map_windows.remove(task);
        self.persist();
    }

    /// 把所有任务浮窗恢复到默认大小，并逐个错开摆回主窗口中间。
    pub(super) fn reset_map_window_geometry(&mut self, ctx: &egui::Context) {
        self.config.map_window_layouts.clear();
        self.map_windows.clear();
        self.closed_map_windows.clear();
        if self.config.map_window_enabled {
            for (index, task) in self.active_tasks().into_iter().enumerate() {
                let layout = self.ensure_map_window_layout(task, index, ctx);
                let id = map_window_id(task);
                ctx.send_viewport_cmd_to(
                    id,
                    egui::ViewportCommand::InnerSize(vec2(layout.width, layout.height)),
                );
                ctx.send_viewport_cmd_to(
                    id,
                    egui::ViewportCommand::OuterPosition(pos2(layout.x, layout.y)),
                );
            }
        }
        self.persist();
    }

    /// 读取任务浮窗的保存位置；第一次出现时给出错开的默认位置。
    fn ensure_map_window_layout(
        &mut self,
        task: &'static str,
        index: usize,
        ctx: &egui::Context,
    ) -> MapWindowLayout {
        if let Some(layout) = self.config.map_window_layouts.get(task) {
            return *layout;
        }
        let size = MAP_WINDOW_SIZE;
        let base = centered_window_position(ctx, size);
        let monitor = ctx.input(|input| {
            input
                .viewport()
                .monitor_size
                .unwrap_or_else(|| vec2(1920.0, 1080.0))
        });
        // 默认把各任务浮窗横向排开，围绕主窗口居中；屏幕放不下时再错开。
        const GAP: f32 = 12.0;
        let count = self.active_tasks().len().max(1) as f32;
        let total_width = count * size.x + (count - 1.0) * GAP;
        let x = if total_width <= monitor.x {
            base.x - (total_width - size.x) / 2.0 + index as f32 * (size.x + GAP)
        } else {
            base.x + index as f32 * CASCADE_OFFSET
        };
        let y = if total_width <= monitor.x {
            base.y
        } else {
            base.y + index as f32 * CASCADE_OFFSET
        };
        let layout = MapWindowLayout {
            x: x.clamp(0.0, (monitor.x - size.x).max(0.0)),
            y: y.clamp(0.0, (monitor.y - size.y).max(0.0)),
            width: size.x,
            height: size.y,
        };
        self.config
            .map_window_layouts
            .insert(task.to_string(), layout);
        self.persist();
        layout
    }

    /// 每帧调度所有任务浮窗；任务被取消勾选后对应的窗口消失。
    pub(super) fn map_windows(&mut self, ctx: &egui::Context) {
        if !self.config.map_window_enabled {
            return;
        }
        let tasks = self.active_tasks();
        self.map_windows.retain(|task, _| tasks.contains(task));
        for (index, task) in tasks.iter().copied().enumerate() {
            if self.closed_map_windows.contains(task) {
                continue;
            }
            let layout = self.ensure_map_window_layout(task, index, ctx);
            let title = map_window_title(task);
            let mut builder = egui::ViewportBuilder::default()
                .with_title(title.as_str())
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(true)
                .with_inner_size(vec2(layout.width, layout.height))
                .with_min_inner_size(MAP_WINDOW_MIN_SIZE)
                // 只在建立窗口时定位，之后每个窗口各自记住用户拖动的位置。
                .with_position(pos2(layout.x, layout.y));
            if self.config.map_window_on_top {
                builder = builder.with_always_on_top();
            }
            ctx.show_viewport_immediate(map_window_id(task), builder, |ui, _class| {
                if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
                    // 独立视口还没建好（测试 / 首帧），不画东西。
                    return;
                }
                self.map_window_ui(ui, task);
            });
        }
    }

    fn map_window_ui(&mut self, ui: &mut egui::Ui, task: &'static str) {
        let ctx = ui.ctx().clone();
        if ctx.input(|input| input.viewport().close_requested()) {
            self.set_map_window_visible(task, false);
            return;
        }
        let title = map_window_title(task);
        let attempts = self.map_windows.entry(task).or_default().shadow_attempts;
        if attempts < 30 {
            // Windows 会给无边框窗口加 DWM 投影和圆角，透明背景下看得到；
            // 窗口刚创建时可能还找不到，下一帧再试（其他平台是空操作）。
            let found = borderless::prepare(&title, true);
            self.map_windows.entry(task).or_default().shadow_attempts =
                if found { 30 } else { attempts + 1 };
        }
        self.sync_map_window_state(&ctx, task);
        // Win+D（显示桌面）会把无边框工具窗口一起最小化。它没有任务栏按钮，
        // 不主动还原就会彻底消失，所以只在「可见→最小化」边沿还原一次。
        let minimized = ctx.input(|input| input.viewport().minimized == Some(true));
        {
            let runtime = self.map_windows.entry(task).or_default();
            if minimized && !runtime.minimized {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            }
            runtime.minimized = minimized;
        }

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
        self.task_panel_with(&mut child, task, alpha, false);
        if !self
            .tasks
            .get(task)
            .is_some_and(|state| state.location.is_some())
        {
            child.painter().text(
                content.center_bottom() - vec2(0.0, 12.0),
                Align2::CENTER_CENTER,
                "等待识别到任务…",
                FontId::proportional(13.0),
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
                // 只关掉这个任务的浮窗，其他任务的窗口保持不动。
                self.set_map_window_visible(task, false);
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

    /// 同步置顶 / 鼠标穿透，并把当前任务浮窗的位置和大小写回配置。
    fn sync_map_window_state(&mut self, ctx: &egui::Context, task: &'static str) {
        let on_top = self.config.map_window_on_top;
        let locked = self.config.map_window_locked;
        {
            let runtime = self.map_windows.entry(task).or_default();
            if runtime.applied_on_top != Some(on_top) {
                let level = if on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                };
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
                runtime.applied_on_top = Some(on_top);
            }
            if runtime.applied_passthrough != Some(locked) {
                ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(locked));
                runtime.applied_passthrough = Some(locked);
            }
        }
        let (outer, inner) = ctx.input(|input| {
            let viewport = input.viewport();
            (viewport.outer_rect, viewport.inner_rect)
        });
        let mut layout =
            self.config
                .map_window_layouts
                .get(task)
                .copied()
                .unwrap_or(MapWindowLayout {
                    x: 0.0,
                    y: 0.0,
                    width: MAP_WINDOW_SIZE.x,
                    height: MAP_WINDOW_SIZE.y,
                });
        let mut changed = false;
        {
            let runtime = self.map_windows.entry(task).or_default();
            if let Some(inner) = inner {
                let size = inner.size();
                if runtime
                    .last_size
                    .is_none_or(|last| (last - size).length() > 0.5)
                {
                    runtime.last_size = Some(size);
                    if (vec2(layout.width, layout.height) - size).length() > 0.5 {
                        layout.width = size.x;
                        layout.height = size.y;
                        changed = true;
                    }
                }
            }
            if let Some(outer) = outer {
                let position = outer.min;
                if runtime
                    .last_outer
                    .is_none_or(|last| last.distance(position) > 0.5)
                {
                    runtime.last_outer = Some(position);
                    if pos2(layout.x, layout.y).distance(position) > 0.5 {
                        layout.x = position.x;
                        layout.y = position.y;
                        changed = true;
                    }
                }
            }
        }
        if changed {
            self.config
                .map_window_layouts
                .insert(task.to_string(), layout);
            self.persist();
        }
    }

    /// 主窗口里的占位：地图已经弹到浮窗，这里只留说明和各窗口的开关。
    pub(super) fn map_window_placeholder(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_rect_before_wrap();
        let (rect, _) = ui.allocate_exact_size(available.size(), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(8), theme::inset());
        painter.text(
            pos2(rect.center().x, rect.center().y - 56.0),
            Align2::CENTER_CENTER,
            "地图已在独立浮窗中显示",
            FontId::proportional(18.0),
            theme::text(),
        );
        // 提示文字按可用宽度换行，主窗口变窄时不会冲出面板。
        let hint = painter.layout(
            "每个任务一个浮窗，可分别拖动、缩放；透明度、置顶和锁定在「显示与布局」里统一调整"
                .to_owned(),
            FontId::proportional(13.0),
            theme::muted(),
            (rect.width() - 48.0).max(80.0),
        );
        painter.galley(
            pos2(
                rect.center().x - hint.size().x / 2.0,
                rect.center().y - 24.0,
            ),
            hint,
            theme::muted(),
        );

        let tasks = self.active_tasks();
        if tasks.is_empty() {
            painter.text(
                pos2(rect.center().x, rect.center().y + 24.0),
                Align2::CENTER_CENTER,
                "未勾选任务类型",
                FontId::proportional(14.0),
                theme::muted(),
            );
        } else {
            let button_size = vec2(108.0, 30.0);
            let gap = 8.0;
            let total = tasks.len() as f32 * button_size.x + (tasks.len() - 1) as f32 * gap;
            let start_x = rect.center().x - total / 2.0;
            let row_y = rect.center().y + 12.0;
            for (index, task) in tasks.iter().enumerate() {
                let open = self.map_window_open(task);
                let label = format!("{task}浮窗：{}", if open { "开" } else { "关" });
                let button_rect = Rect::from_min_size(
                    pos2(start_x + index as f32 * (button_size.x + gap), row_y),
                    button_size,
                );
                if ui
                    .put(button_rect, egui::Button::new(label).selected(open))
                    .on_hover_text(if open {
                        "点击关闭这个任务的浮窗"
                    } else {
                        "点击重新打开这个任务的浮窗"
                    })
                    .clicked()
                {
                    self.set_map_window_visible(task, !open);
                }
            }
        }
        let exit_rect = Rect::from_center_size(rect.center() + vec2(0.0, 64.0), vec2(150.0, 30.0));
        if ui
            .put(exit_rect, egui::Button::new("退出地图浮窗模式"))
            .clicked()
        {
            self.set_map_window(false);
        }
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
