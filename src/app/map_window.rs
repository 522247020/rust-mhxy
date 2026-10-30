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
#[cfg(not(windows))]
const RESIZE_BORDER: f32 = 6.0;
/// 右上角关闭按钮的尺寸与边距。
const CLOSE_BUTTON_SIZE: Vec2 = Vec2::new(20.0, 20.0);
const CLOSE_BUTTON_MARGIN: f32 = 6.0;
pub(super) const MAP_WIDTH_PRESETS: [(&str, f32); 5] = [
    ("420 px", 420.0),
    ("560 px", 560.0),
    ("700 px", 700.0),
    ("840 px", 840.0),
    ("1120 px", 1120.0),
];
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
    /// 上一帧是否处于移动/缩放循环；期间延迟写入最终几何。
    interacting: bool,
    /// 上一帧鼠标是否按在浮窗上，用于首次按下激活窗口。
    pointer_down: bool,
    pending_size: Option<Vec2>,
    #[cfg(not(windows))]
    resize_drag: Option<ResizeDrag>,
    #[cfg(not(windows))]
    last_resize_at: Option<Instant>,

    /// 浮窗最近的几何值（内框大小 / 外框位置），用来发现「窗口变了但还没重绘」。
    painted_geometry: (Option<Vec2>, Option<egui::Pos2>),

    /// 窗口是否已经显示过。第一帧先隐藏着画好，再显示，
    /// 避免创建 / 重新打开浮窗的瞬间闪出 Windows 默认的白底和边框。
    visible: bool,
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

    pub(super) fn set_map_window_width(&mut self, task: &'static str, width: f32) {
        if !width.is_finite() {
            return;
        }
        let aspect = self
            .map_windows
            .get(task)
            .and_then(|runtime| runtime.map_aspect)
            .or_else(|| {
                self.tasks
                    .get(task)
                    .and_then(|state| state.map_index)
                    .and_then(|index| self.maps.get(index))
                    .and_then(|definition| crate::core::load_rgb_image(&definition.image).ok())
                    .map(|image| image.width() as f32 / image.height().max(1) as f32)
            })
            .or_else(|| {
                self.config
                    .map_window_layouts
                    .get(task)
                    .map(|layout| layout.width / layout.height.max(1.0))
            })
            .filter(|aspect| aspect.is_finite() && *aspect > 0.05)
            .unwrap_or(MAP_WINDOW_SIZE.x / MAP_WINDOW_SIZE.y);
        let monitor = self.ctx.input(|input| {
            input
                .viewport()
                .monitor_size
                .unwrap_or_else(|| vec2(1920.0, 1080.0))
        });
        let size = aspect_size(width.max(MAP_WINDOW_MIN_SIZE.x), aspect, monitor);
        let layout = self
            .config
            .map_window_layouts
            .entry(task.to_string())
            .or_insert(MapWindowLayout {
                x: 0.0,
                y: 0.0,
                width: MAP_WINDOW_SIZE.x,
                height: MAP_WINDOW_SIZE.y,
            });
        layout.width = size.x;
        layout.height = size.y;
        if let Some(runtime) = self.map_windows.get_mut(task) {
            runtime.spawn_layout = Some(*layout);
            runtime.last_size = Some(size);
            runtime.pending_size = Some(size);
        }
        self.ctx.send_viewport_cmd_to(
            map_window_id(task),
            egui::ViewportCommand::InnerSize(vec2(size.x, size.y)),
        );
        self.persist();
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
        if !enabled {
            // 先把窗口隐藏掉再让 eframe 销毁，避免关闭瞬间闪出系统白底。
            for task in self.active_tasks() {
                self.ctx.send_viewport_cmd_to(
                    map_window_id(task),
                    egui::ViewportCommand::Visible(false),
                );
            }
        }
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
            // 先隐藏再销毁，避免关闭瞬间闪出系统白底。
            self.ctx
                .send_viewport_cmd_to(map_window_id(task), egui::ViewportCommand::Visible(false));
        }
        // 重新打开时按配置里的位置 / 大小重建，需要重置缓存。
        self.map_windows.remove(task);
        self.persist();
    }

    /// 把所有任务浮窗恢复到默认大小，并逐个错开摆回主窗口中间。
    pub(super) fn reset_map_window_geometry(&mut self, ctx: &egui::Context) {
        self.config.map_window_layouts.clear();
        self.closed_map_windows.clear();
        if self.config.map_window_enabled {
            let tasks = self.active_tasks();
            let image_sizes = self.task_map_image_sizes(&tasks, ctx);
            for (index, task) in tasks.iter().copied().enumerate() {
                let image_size = image_sizes.get(&task).copied();
                let layout = self.ensure_map_window_layout(task, index, ctx, image_size);
                // 保留现有窗口的 visible 状态，只更新 builder 几何，避免重置时
                // 窗口先隐藏再显示造成闪烁。
                if let Some(runtime) = self.map_windows.get_mut(&task) {
                    runtime.spawn_layout = Some(layout);
                    runtime.last_size = None;
                    runtime.last_outer = None;
                    runtime.sized_to_map = image_size.is_some();
                }
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
            let visible = self
                .map_windows
                .get(&task)
                .is_some_and(|runtime| runtime.visible);
            let title = map_window_title(task);
            let mut builder = egui::ViewportBuilder::default()
                .with_title(title.as_str())
                .with_decorations(false)
                .with_transparent(true)
                .with_resizable(true)
                // 新视口默认是 active 的：浮窗一创建就抢走焦点，主窗口里刚打开的
                // 「更多」菜单会立刻收起，浮窗设置根本来不及点。浮窗不需要键盘焦点。
                .with_active(false)
                .with_visible(visible)
                .with_inner_size(vec2(spawn.width, spawn.height))
                .with_min_inner_size(MAP_WINDOW_MIN_SIZE)
                .with_position(pos2(spawn.x, spawn.y));
            if desired_on_top(self.config.map_window_on_top, self.app_window_in_foreground) {
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
        // 浮窗的尺寸 / 位置一旦变化（原生拖拽缩放、程序化改宽度、系统移动窗口），
        // eframe 的交换链会重建，但根视口如果没有新的重绘请求，子视口就会停在
        // 上一帧甚至一片空白：看起来就是「拖的时候画面不动 / 窗口消失」。
        // 这里按帧对几何变化补一次重绘请求，保证缩放过程中画面跟着走。
        let geometry = ctx.input(|input| {
            let viewport = input.viewport();
            (
                viewport.inner_rect.map(|rect| rect.size()),
                viewport.outer_rect.map(|rect| rect.min),
            )
        });
        {
            let runtime = self.map_windows.entry(task).or_default();
            // 用容差比较：DPI 取整会让逻辑尺寸在小数位上抖，不能因此每帧重绘。
            if !geometry_close(runtime.painted_geometry, geometry) {
                runtime.painted_geometry = geometry;
                self.ctx.request_repaint_of(egui::ViewportId::ROOT);
                ctx.request_repaint();
            }
        }
        let title = map_window_title(task);
        let aspect = self
            .map_windows
            .get(&task)
            .and_then(|runtime| runtime.map_aspect)
            .or_else(|| {
                self.config
                    .map_window_layouts
                    .get(task)
                    .map(|layout| layout.width / layout.height.max(1.0))
            })
            .filter(|aspect| aspect.is_finite() && *aspect > 0.05)
            .or(Some(MAP_WINDOW_SIZE.x / MAP_WINDOW_SIZE.y));
        borderless::set_resize_aspect(&title, aspect);
        let attempts = self.map_windows.entry(task).or_default().shadow_attempts;
        let mut prepared = true;
        if attempts < 30 {
            // Windows 会给无边框窗口加 DWM 投影和圆角，透明背景下看得到；
            // 窗口刚创建时可能还找不到，下一帧再试（其他平台是空操作）。
            let found = borderless::prepare(&title, true, aspect, true);
            // 正常第一帧就能找到窗口；万一找不到，也别一直藏着不显示。
            prepared = found || attempts >= 2;
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
        // 还没识别到地图时先给一个深色底，避免窗口完全透明像一块空洞；
        // 识别到地图后不再画背景，整窗就是地图图片本身。
        let has_location = self
            .tasks
            .get(task)
            .is_some_and(|state| state.location.is_some());
        if !has_location {
            ui.painter().rect_filled(
                rect,
                CornerRadius::ZERO,
                theme::inset().gamma_multiply(opacity),
            );
        }
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        self.task_panel_with(&mut child, task, alpha, false, true);
        if !has_location {
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

        // 第一帧先隐藏着画好，再显示；避免窗口创建 / 重新打开时
        // 闪出 Windows 默认的白色底和边框。
        if prepared
            && !self
                .map_windows
                .get(&task)
                .is_some_and(|runtime| runtime.visible)
        {
            if let Some(runtime) = self.map_windows.get_mut(&task) {
                runtime.visible = true;
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            // 刚显示出来的窗口需要再走一帧，否则可能停在一片透明上。
            self.ctx.request_repaint_of(egui::ViewportId::ROOT);
            ctx.request_repaint();
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
        let hovering_close = pointer.is_some_and(|pos| close_rect.contains(pos));
        if hovering_close {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        } else {
            #[cfg(not(windows))]
            if let Some(pointer) = pointer {
                if let Some(direction) = resize_direction(rect, pointer, RESIZE_BORDER) {
                    ctx.set_cursor_icon(cursor_for(direction));
                }
            }
        }
        // 关闭按钮直接画在地图上，不再嵌套 Button：整窗拖动热区会和它抢点击，
        // 之前按钮刚出现就被热区顶掉，看起来就是“点了没反应”。
        if pointer.is_some_and(|pos| rect.contains(pos)) {
            let painter = ui.painter();
            let alpha = if hovering_close { 210 } else { 140 };
            painter.rect_filled(
                close_rect,
                CornerRadius::same(4),
                Color32::from_black_alpha(alpha).gamma_multiply(opacity),
            );
            // 用两条线画 X，不依赖字体字形，避免显示成方块。
            let stroke = egui::Stroke::new(
                (close_rect.width() * 0.11).max(1.2),
                Color32::WHITE.gamma_multiply(opacity),
            );
            let inset = close_rect.width() * 0.28;
            painter.line_segment(
                [
                    close_rect.left_top() + vec2(inset, inset),
                    close_rect.right_bottom() - vec2(inset, inset),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    close_rect.right_top() + vec2(-inset, inset),
                    close_rect.left_bottom() + vec2(inset, -inset),
                ],
                stroke,
            );
        }
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos().or(pointer) {
                if close_rect.contains(pos) {
                    // 只关掉这个任务的浮窗，其他任务的窗口保持不动。
                    self.set_map_window_visible(task, false);
                    return;
                }
            }
        }
        let pointer_down = response.is_pointer_button_down_on();
        let was_down = self
            .map_windows
            .get(&task)
            .is_some_and(|runtime| runtime.pointer_down);
        if pointer_down && !was_down {
            if let Some(pos) = pointer.filter(|pos| !close_rect.contains(*pos)) {
                #[cfg(windows)]
                {
                    let border = 8.0;
                    let left = pos.x - rect.left() <= border;
                    let right = rect.right() - pos.x <= border;
                    let top = pos.y - rect.top() <= border;
                    let bottom = rect.bottom() - pos.y <= border;
                    if !(left || right || top || bottom) {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                }
                #[cfg(not(windows))]
                match resize_direction(rect, pos, RESIZE_BORDER) {
                    Some(direction) => {
                        if let Some(outer) = ctx.input(|input| input.viewport().outer_rect) {
                            let aspect = self
                                .map_windows
                                .get(task)
                                .and_then(|runtime| runtime.map_aspect)
                                .unwrap_or_else(|| outer.width() / outer.height().max(1.0));
                            let runtime = self.map_windows.entry(task).or_default();
                            runtime.resize_drag = Some(ResizeDrag {
                                direction,
                                start_pointer: outer.min + pos.to_vec2(),
                                start_outer: outer,
                                aspect,
                            });
                            runtime.last_resize_at = None;
                        }
                    }
                    None => {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                        ctx.send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }
                }
            }
        }
        #[cfg(not(windows))]
        self.update_manual_resize(ui, task, pointer_down);
        let interacting = if cfg!(windows) {
            borderless::is_interacting(&title)
        } else {
            pointer_down
        };
        if interacting {
            // 原生移动 / 缩放循环里继续按帧重绘，地图跟着鼠标走，不会出现卡顿的残影。
            self.ctx.request_repaint_of(egui::ViewportId::ROOT);
            ctx.request_repaint();
        }
        if let Some(runtime) = self.map_windows.get_mut(&task) {
            runtime.interacting = interacting;
            runtime.pointer_down = pointer_down;
        }
    }

    #[cfg(not(windows))]
    fn update_manual_resize(&mut self, ui: &egui::Ui, task: &'static str, pointer_down: bool) {
        let drag = self
            .map_windows
            .get(task)
            .and_then(|runtime| runtime.resize_drag);
        if let Some(drag) = drag {
            if pointer_down {
                if let Some(pointer) = ui.ctx().input(|input| input.pointer.interact_pos()) {
                    let outer = ui
                        .ctx()
                        .input(|input| input.viewport().outer_rect)
                        .unwrap_or(drag.start_outer);
                    let current_size = ui
                        .ctx()
                        .input(|input| input.viewport().inner_rect)
                        .map(|rect| rect.size())
                        .unwrap_or_else(|| outer.size());
                    let monitor = ui
                        .ctx()
                        .input(|input| input.viewport().monitor_size)
                        .unwrap_or_else(|| vec2(1920.0, 1080.0));
                    let target = aspect_resize_rect(&drag, outer.min + pointer.to_vec2(), monitor);
                    let now = Instant::now();
                    let due = self
                        .map_windows
                        .get(task)
                        .and_then(|runtime| runtime.last_resize_at)
                        .is_none_or(|last| now.duration_since(last) >= Duration::from_millis(16));
                    if due {
                        if (target.min - outer.min).length() > 1.0 {
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::OuterPosition(
                                    target.min,
                                ));
                        }
                        if (target.size() - current_size).length() > 1.0 {
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::InnerSize(target.size()));
                        }
                        if let Some(runtime) = self.map_windows.get_mut(task) {
                            runtime.last_resize_at = Some(now);
                        }
                    }
                    ui.ctx().request_repaint();
                }
            } else if let Some(runtime) = self.map_windows.get_mut(task) {
                runtime.resize_drag = None;
            }
        }
    }

    /// 同步置顶 / 鼠标穿透，并把当前任务浮窗的位置和大小写回配置。
    fn sync_map_window_state(&mut self, ctx: &egui::Context, task: &'static str) {
        // 主窗口在前台（用户在调设置、看菜单）时，浮窗让位：降成普通窗口并压到
        // 主窗口下面。否则置顶浮窗会盖住「更多」菜单、下拉框和滑块，点了等于没点。
        let on_top = desired_on_top(self.config.map_window_on_top, self.app_window_in_foreground);
        let locked = self.config.map_window_locked;
        let title = map_window_title(task);
        {
            let runtime = self.map_windows.entry(task).or_default();
            if runtime.applied_on_top != Some(on_top) {
                let level = if on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                };
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
                if !on_top {
                    // 只靠 WindowLevel 不够：从置顶降下来时窗口会停在原先的层叠位置，
                    // 仍在主窗口之上。这里把它直接插到主窗口下面。
                    borderless::tuck_below(&title, &crate::main_window_title());
                }
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
        // 拖动 / 缩放过程中不写配置；Windows 在 WM_EXITSIZEMOVE 后读取最终几何，
        // 其他后端则由 map_window_ui 的 egui drag 状态控制。
        let title = map_window_title(task);
        let completed = borderless::completed_size(&title);
        let interacting = if cfg!(windows) {
            borderless::is_interacting(&title)
        } else {
            self.map_windows
                .get(task)
                .is_some_and(|runtime| runtime.interacting)
        };
        if interacting {
            return;
        }
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
        if let Some([width, height]) = completed {
            if (layout.width - width as f32).abs() > 0.5
                || (layout.height - height as f32).abs() > 0.5
            {
                layout.width = width as f32;
                layout.height = height as f32;
                changed = true;
            }
        }
        {
            let runtime = self.map_windows.entry(task).or_default();
            if completed.is_none() {
                if let Some(inner) = inner {
                    let size = inner.size();
                    if let Some(pending) = runtime.pending_size {
                        if (pending - size).length() <= 1.0 {
                            runtime.pending_size = None;
                        }
                    } else if runtime
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
        }
        if changed {
            self.config
                .map_window_layouts
                .insert(task.to_string(), layout);
            self.persist();
        }
    }

    /// 主窗口里的占位：地图已经弹到浮窗，这里只留说明和设置入口。
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
            "每个任务的浮窗开关、不透明度、宽度、置顶和锁定都在「地图浮窗设置」窗口里；\
             也可以从顶栏「更多 → 显示 → 浮窗设置…」打开"
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

        let button_rect =
            Rect::from_center_size(rect.center() + vec2(0.0, 40.0), vec2(150.0, 32.0));
        if ui
            .put(button_rect, egui::Button::new("浮窗设置…"))
            .on_hover_text("每个任务的浮窗开关、不透明度、宽度、置顶和锁定")
            .clicked()
        {
            self.toggle_map_window_settings(ui.ctx());
        }
    }
}

/// 浮窗实际是否置顶：配置要置顶，而且本进程没有窗口在前台。
pub(super) fn desired_on_top(configured: bool, app_in_foreground: bool) -> bool {
    configured && !app_in_foreground
}

/// 两次浮窗几何值是否足够接近：容差吸收 DPI 取整带来的小数抖动。
pub(super) fn geometry_close(
    previous: (Option<Vec2>, Option<egui::Pos2>),
    current: (Option<Vec2>, Option<egui::Pos2>),
) -> bool {
    fn size_close(a: Option<Vec2>, b: Option<Vec2>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => (a - b).length() <= 0.5,
            (None, None) => true,
            _ => false,
        }
    }
    fn pos_close(a: Option<egui::Pos2>, b: Option<egui::Pos2>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => a.distance(b) <= 0.5,
            (None, None) => true,
            _ => false,
        }
    }
    size_close(previous.0, current.0) && pos_close(previous.1, current.1)
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

/// 非 Windows 手工等比缩放回退的数据；Windows 直接由原生 WM_SIZING 处理。
/// 纯数学部分测试也要跑，所以测试构建同样编译。
#[cfg(any(test, not(windows)))]
#[derive(Clone, Copy)]
struct ResizeDrag {
    direction: egui::ResizeDirection,
    start_pointer: egui::Pos2,
    start_outer: Rect,
    aspect: f32,
}

/// 测试用：验证非 Windows 手工缩放回退的纯数学约束。
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
#[cfg(any(test, not(windows)))]
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
#[cfg(any(test, not(windows)))]
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

#[cfg(not(windows))]
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
