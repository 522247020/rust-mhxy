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
    /// 传给 `ViewportBuilder` 的几何值，创建窗口时固定下来。
    /// 如果每帧把实际窗口大小 / 位置写回 builder，DPI 换算的取整会让
    /// 窗口反复收到 InnerSize / OuterPosition 命令，表现为拖动缩放时不停抖动。
    spawn_layout: Option<MapWindowLayout>,
    /// 是否已经按地图图片大小收紧过窗口；只在第一次拿到地图时做一次。
    sized_to_map: bool,
    /// 当前地图图片的宽高比，用来锁定等比缩放。
    map_aspect: Option<f32>,
    /// 正在拖边缩放的起点信息；系统 BeginResize 不能锁比例，所以自己算。
    resize_drag: Option<ResizeDrag>,
    shadow_attempts: u8,
    /// 上一帧是否最小化：无边框工具窗口没有任务栏按钮，Win+D 之后必须自己还原。
    minimized: bool,
}

/// 一次等比缩放手势的起点。
#[derive(Clone, Copy)]
struct ResizeDrag {
    direction: egui::ResizeDirection,
    /// 按下时鼠标在屏幕上的位置（逻辑像素）。
    start_pointer: egui::Pos2,
    /// 按下时的窗口外框，用来固定对边 / 对角。
    start_outer: Rect,
    /// 要锁定的宽高比（地图图片的宽 / 高）。
    aspect: f32,
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

    /// 测试用：查看传给 ViewportBuilder 的几何值是否保持稳定。
    #[cfg(test)]
    pub(super) fn map_window_spawn_layout(&self, task: &str) -> Option<MapWindowLayout> {
        self.map_windows
            .get(task)
            .and_then(|runtime| runtime.spawn_layout)
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
            let tasks = self.active_tasks();
            let image_sizes = self.task_map_image_sizes(&tasks, ctx);
            for (index, task) in tasks.iter().copied().enumerate() {
                let image_size = image_sizes.get(&task).copied();
                // 清空过运行时状态，下一帧会按新的默认布局重建 builder。
                self.ensure_map_window_layout(task, index, ctx, image_size);
            }
        }
        self.persist();
    }

    /// 当前任务地图图片的尺寸（逻辑像素）；还没识别到任务时为 None。
    fn task_map_image_size(&mut self, task: &str, ctx: &egui::Context) -> Option<Vec2> {
        let index = self.tasks.get(task)?.map_index?;
        let definition = self.maps.get(index)?.clone();
        let texture = self.map_texture(ctx, &definition)?;
        Some(texture.size_vec2())
    }

    /// 批量取各任务的地图尺寸；同一张地图只会加载一次纹理。
    fn task_map_image_sizes(
        &mut self,
        tasks: &[&'static str],
        ctx: &egui::Context,
    ) -> HashMap<&'static str, Vec2> {
        tasks
            .iter()
            .filter_map(|task| {
                self.task_map_image_size(task, ctx)
                    .map(|size| (*task, size))
            })
            .collect()
    }

    /// 读取任务浮窗的保存位置；第一次出现时按地图图片大小和错开的默认位置摆放。
    fn ensure_map_window_layout(
        &mut self,
        task: &'static str,
        index: usize,
        ctx: &egui::Context,
        image_size: Option<Vec2>,
    ) -> MapWindowLayout {
        if let Some(layout) = self.config.map_window_layouts.get(task) {
            return *layout;
        }
        let monitor = ctx.input(|input| {
            input
                .viewport()
                .monitor_size
                .unwrap_or_else(|| vec2(1920.0, 1080.0))
        });
        let size = image_size
            .map(|size| fit_window_size(size, monitor))
            .unwrap_or(MAP_WINDOW_SIZE);
        let base = centered_window_position(ctx, size);
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
        let image_sizes = self.task_map_image_sizes(&tasks, ctx);
        for (index, task) in tasks.iter().copied().enumerate() {
            if self.closed_map_windows.contains(task) {
                continue;
            }
            let image_size = image_sizes.get(&task).copied();
            let mut layout = self.ensure_map_window_layout(task, index, ctx, image_size);
            // 地图图片确定后：首次贴合图片大小；之后保持图片宽高比。
            // 旧版本存下来的固定 560×460 视为未调整过，迁移时也贴合图片。
            let mut layout_changed = false;
            {
                let legacy_default = (layout.width - MAP_WINDOW_SIZE.x).abs() < 1.0
                    && (layout.height - MAP_WINDOW_SIZE.y).abs() < 1.0;
                let runtime = self
                    .map_windows
                    .entry(task)
                    .or_insert_with(|| MapWindowRuntime {
                        spawn_layout: Some(layout),
                        sized_to_map: image_size.is_some() && !legacy_default,
                        ..Default::default()
                    });
                if let Some(size) = image_size {
                    let aspect = size.x / size.y.max(1.0);
                    let aspect_changed = runtime
                        .map_aspect
                        .is_none_or(|old| (old - aspect).abs() > 0.005);
                    if aspect_changed {
                        let monitor = ctx.input(|input| {
                            input
                                .viewport()
                                .monitor_size
                                .unwrap_or_else(|| vec2(1920.0, 1080.0))
                        });
                        // 第一次拿到地图且窗口还是默认尺寸：直接用图片大小；
                        // 否则保留用户已经调好的宽度，只把高度修正成等比。
                        let current_aspect = layout.width / layout.height.max(1.0);
                        if !runtime.sized_to_map && legacy_default {
                            let size = fit_window_size(size, monitor);
                            layout.width = size.x;
                            layout.height = size.y;
                            layout_changed = true;
                        } else if (current_aspect - aspect).abs() > 0.005 {
                            let size = aspect_size(layout.width, aspect, monitor);
                            layout.width = size.x;
                            layout.height = size.y;
                            layout_changed = true;
                        }
                        runtime.sized_to_map = true;
                        runtime.map_aspect = Some(aspect);
                        if layout_changed {
                            runtime.spawn_layout = Some(layout);
                        }
                    }
                }
            }
            if layout_changed {
                self.config
                    .map_window_layouts
                    .insert(task.to_string(), layout);
                self.persist();
            }
            // builder 的几何值只在创建 / 重置 / 首次贴合图片时更新。
            // 不能每帧把窗口实际大小写回去：DPI 取整会让窗口反复收到
            // InnerSize / OuterPosition，拖动缩放时就会不停抖动。
            let spawn = self
                .map_windows
                .get(&task)
                .and_then(|runtime| runtime.spawn_layout)
                .unwrap_or(layout);
            let title = map_window_title(task);
            let mut builder = egui::ViewportBuilder::default()
                .with_title(title.as_str())
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(true)
                .with_inner_size(vec2(spawn.width, spawn.height))
                .with_min_inner_size(MAP_WINDOW_MIN_SIZE)
                .with_position(pos2(spawn.x, spawn.y));
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
        // 不要背景：窗口里画什么就是地图图片本身。
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        self.task_panel_with(&mut child, task, alpha, false, true);
        if !self
            .tasks
            .get(task)
            .is_some_and(|state| state.location.is_some())
        {
            // 透明背景下文字直接叠在桌面 / 游戏上，加一层阴影保证可读。
            let text_pos = rect.center_bottom() - vec2(0.0, 16.0);
            let text = "等待识别到任务…";
            let font = FontId::proportional(14.0);
            child.painter().text(
                text_pos + vec2(1.0, 1.0),
                Align2::CENTER_CENTER,
                text,
                font.clone(),
                Color32::from_black_alpha(190).gamma_multiply(opacity),
            );
            child.painter().text(
                text_pos,
                Align2::CENTER_CENTER,
                text,
                font,
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
        let pointer = ctx
            .input(|input| input.pointer.interact_pos())
            .or_else(|| response.hover_pos());
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
            if let Some(pos) = pointer {
                if !close_rect.contains(pos) {
                    match resize_direction(rect, pos, RESIZE_BORDER) {
                        Some(direction) => {
                            // 系统 BeginResize 只能自由缩放，这里自己记录起点，
                            // 后续每帧按地图宽高比算目标尺寸。
                            if let Some(outer) = ctx.input(|input| input.viewport().outer_rect) {
                                let aspect = self
                                    .map_windows
                                    .get(&task)
                                    .and_then(|runtime| runtime.map_aspect)
                                    .filter(|aspect| *aspect > 0.05)
                                    .unwrap_or_else(|| outer.width() / outer.height().max(1.0));
                                let drag = ResizeDrag {
                                    direction,
                                    start_pointer: outer.min + pos.to_vec2(),
                                    start_outer: outer,
                                    aspect,
                                };
                                self.map_windows.entry(task).or_default().resize_drag = Some(drag);
                            } else {
                                ctx.send_viewport_cmd(egui::ViewportCommand::BeginResize(
                                    direction,
                                ));
                            }
                        }
                        None => ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag),
                    }
                }
            }
        }
        // 等比缩放：拖动过程中每帧按鼠标位置算尺寸，对边 / 对角保持不动。
        let drag = self
            .map_windows
            .get(&task)
            .and_then(|runtime| runtime.resize_drag);
        if let Some(drag) = drag {
            if response.dragged() {
                if let Some(pos) = ctx.input(|input| input.pointer.interact_pos()) {
                    let outer = ctx
                        .input(|input| input.viewport().outer_rect)
                        .unwrap_or(drag.start_outer);
                    let monitor = ctx.input(|input| {
                        input
                            .viewport()
                            .monitor_size
                            .unwrap_or_else(|| vec2(1920.0, 1080.0))
                    });
                    let target = aspect_resize_rect(&drag, outer.min + pos.to_vec2(), monitor);
                    // 先移动再改大小：尺寸以左上角为基准，移动后的左上角才是最终位置。
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(target.min));
                    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(target.size()));
                }
            } else if response.drag_stopped() {
                self.map_windows.entry(task).or_default().resize_drag = None;
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
            "每个任务一个浮窗，可分别拖动、等比缩放；透明度、置顶和锁定在「显示与布局」里统一调整"
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

/// 按图片大小开窗，但不超过当前显示器、也不小于浮窗下限。
fn fit_window_size(size: Vec2, monitor: Vec2) -> Vec2 {
    // 按图片大小开窗时也要保持宽高比，避免被显示器 / 下限裁得比例不对。
    aspect_size(size.x.max(1.0), size.x / size.y.max(1.0), monitor)
}

/// 保持宽高比，把宽度限制在浮窗下限和当前显示器范围内。
fn aspect_size(width: f32, aspect: f32, monitor: Vec2) -> Vec2 {
    let aspect = aspect.max(0.05);
    let min_width = MAP_WINDOW_MIN_SIZE.x.max(MAP_WINDOW_MIN_SIZE.y * aspect);
    let max_width = monitor.x.min(monitor.y * aspect).max(min_width);
    let width = if width.is_finite() {
        width.clamp(min_width, max_width)
    } else {
        min_width
    };
    vec2(width, width / aspect)
}

/// 测试用：直接按手势参数验证等比缩放计算。
#[cfg(test)]
pub(super) fn resize_rect_keeping_aspect(
    direction: egui::ResizeDirection,
    start_outer: Rect,
    start_pointer: egui::Pos2,
    aspect: f32,
    pointer: egui::Pos2,
    monitor: Vec2,
) -> Rect {
    aspect_resize_rect(
        &ResizeDrag {
            direction,
            start_pointer,
            start_outer,
            aspect,
        },
        pointer,
        monitor,
    )
}

/// 根据拖边方向和鼠标屏幕位置，算出等比缩放后的窗口外框。
/// 拖左边 / 上边时固定右 / 下边，其余方向固定左上角。
fn aspect_resize_rect(drag: &ResizeDrag, pointer: egui::Pos2, monitor: Vec2) -> Rect {
    use egui::ResizeDirection as Direction;
    let start = drag.start_outer;
    let dx = pointer.x - drag.start_pointer.x;
    let dy = pointer.y - drag.start_pointer.y;
    let has_left = matches!(
        drag.direction,
        Direction::West | Direction::NorthWest | Direction::SouthWest
    );
    let has_right = matches!(
        drag.direction,
        Direction::East | Direction::NorthEast | Direction::SouthEast
    );
    let has_top = matches!(
        drag.direction,
        Direction::North | Direction::NorthWest | Direction::NorthEast
    );
    let has_bottom = matches!(
        drag.direction,
        Direction::South | Direction::SouthWest | Direction::SouthEast
    );
    let start_width = start.width().max(1.0);
    let start_height = start.height().max(1.0);

    let scale_x = if has_left {
        (start_width - dx) / start_width
    } else if has_right {
        (start_width + dx) / start_width
    } else {
        1.0
    };
    let scale_y = if has_top {
        (start_height - dy) / start_height
    } else if has_bottom {
        (start_height + dy) / start_height
    } else {
        1.0
    };
    let scale = match (has_left || has_right, has_top || has_bottom) {
        // 角落：以鼠标位移更明显的那个方向为准，避免两轴比例互相打架。
        (true, true) => {
            if (scale_x - 1.0).abs() >= (scale_y - 1.0).abs() {
                scale_x
            } else {
                scale_y
            }
        }
        (true, false) => scale_x,
        (false, true) => scale_y,
        (false, false) => 1.0,
    };
    let size = aspect_size(start_width * scale, drag.aspect, monitor);
    let x = if has_left {
        start.right() - size.x
    } else {
        start.left()
    };
    let y = if has_top {
        start.bottom() - size.y
    } else {
        start.top()
    };
    Rect::from_min_size(pos2(x, y), size)
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
