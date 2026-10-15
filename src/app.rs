//! egui 界面与识别调度。只保留「抓鬼 / 鬼王」任务。
use crate::capture::{self, WindowInfo};
use crate::config::{assets_dir, Config};
use crate::core::{
    clamp_to_map, coordinate_in_map, find_map_definition, high_chance_areas, map_area,
    map_coordinate, map_point, map_rect, parse_tasks, MapDefinition, TaskLocation,
    HIGH_CHANCE_SIZE, MAP_COORDINATE_TOLERANCE,
};
use crate::gate::{self, ColorPoint, GateMode};
use crate::ocr::{self, LocalWorker, OcrEvent, OcrText};
use crate::theme;
use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, Sense, TextureHandle, Vec2};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

const TASK_ORDER: [&str; 2] = ["抓鬼", "鬼王"];
/// 侧栏日志框的最大高度。
const LOG_PANEL_HEIGHT: f32 = 240.0;

#[derive(Default)]
struct TaskState {
    location: Option<TaskLocation>,
    map_index: Option<usize>,
    status: Option<String>,
    captured_at: Option<Instant>,
}

pub struct MhxyApp {
    config: Config,
    config_path: PathBuf,
    maps: Vec<MapDefinition>,
    windows: Vec<WindowInfo>,
    selected_handle: Option<i64>,
    running: bool,
    busy: bool,
    /// 每次发起识别都会 +1，用来丢弃暂停/改设置后才返回的旧结果。
    generation: u64,
    last_scan: Option<Instant>,
    last_signature: Option<u64>,
    pending_signature: Option<u64>,
    tasks: HashMap<&'static str, TaskState>,
    map_textures: HashMap<String, TextureHandle>,
    map_texture_failed: HashSet<String>,
    preview_full: Option<egui::ColorImage>,
    preview_texture: Option<TextureHandle>,
    show_region_window: bool,
    pick_points: bool,
    gate_active: bool,
    last_gate_check: Instant,
    gate_result: Option<gate::GateResult>,
    testing_remote: bool,
    remote_feedback: Option<Result<String, String>>,
    notice: Option<String>,
    last_ocr_secs: Option<f64>,
    config_dirty_since: Option<Instant>,
    save_error: Option<String>,
    ctx: egui::Context,
    last_window_refresh: Instant,
    api_url_edit: String,
    api_token_edit: String,
    worker: LocalWorker,
    receiver: Receiver<OcrEvent>,
    sender: Sender<OcrEvent>,
    logs: Vec<String>,
}

impl MhxyApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let config_path = crate::config::default_config_path();
        let config = Config::load(&config_path);
        let mut app = Self::from_config(config, config_path, cc.egui_ctx.clone());
        app.refresh_windows(true);
        app
    }

    fn from_config(config: Config, config_path: PathBuf, ctx: egui::Context) -> Self {
        let mut map_error: Option<String> = None;
        let maps = match crate::core::load_maps(&assets_dir().join("maps.json")) {
            Ok(maps) => maps,
            Err(err) => {
                map_error = Some(err);
                Vec::new()
            }
        };
        let windows = Vec::new();
        let (sender, receiver) = channel::<OcrEvent>();
        let worker = LocalWorker::spawn(sender.clone(), ctx.clone());
        let mut tasks = HashMap::new();
        for task in TASK_ORDER {
            tasks.insert(task, TaskState::default());
        }
        let mut app = Self {
            api_url_edit: config.api_url.clone(),
            api_token_edit: config.api_token.clone(),
            config,
            config_path,
            maps,
            windows,
            selected_handle: None,
            running: false,
            busy: false,
            generation: 0,
            last_scan: None,
            last_signature: None,
            pending_signature: None,
            tasks,
            map_textures: HashMap::new(),
            map_texture_failed: HashSet::new(),
            preview_full: None,
            preview_texture: None,
            show_region_window: false,
            pick_points: true,
            gate_active: true,
            last_gate_check: Instant::now(),
            gate_result: None,
            testing_remote: false,
            remote_feedback: None,
            notice: None,
            last_ocr_secs: None,
            config_dirty_since: None,
            save_error: None,
            ctx,
            last_window_refresh: Instant::now(),
            worker,
            receiver,
            sender,
            logs: Vec::new(),
        };
        app.log(format!("程序启动，加载 {} 张地图", app.maps.len()));
        app.log_detail(format!("资源目录：{}", assets_dir().display()));
        if let Some(err) = map_error {
            app.log(err);
        }
        app
    }

    /// 简单日志：启动应用、任务开始/停止。详细模式下也显示。
    fn log(&mut self, message: impl Into<String>) {
        self.push_log(true, message.into());
    }

    /// 详细日志：只在「详细」级别显示（OCR 过程、错误、窗口变化等）。
    fn log_detail(&mut self, message: impl Into<String>) {
        self.push_log(false, message.into());
    }

    fn push_log(&mut self, simple: bool, message: String) {
        if !simple && self.config.log_level != "detailed" {
            return;
        }
        let time = chrono::Local::now().format("%H:%M:%S");
        self.logs.push(format!("[{time}] {message}"));
        if self.logs.len() > 1000 {
            let excess = self.logs.len() - 1000;
            self.logs.drain(0..excess);
        }
    }

    /// 拖动和连续输入只在停顿后落盘，避免每一帧都重写配置。
    fn persist(&mut self) {
        self.config_dirty_since = Some(Instant::now());
        self.ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn flush_config(&mut self) {
        if self.config_dirty_since.take().is_some() {
            self.save_error = self.config.save(&self.config_path).err();
        }
    }

    fn invalidate_scan(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.busy = false;
        self.pending_signature = None;
        self.last_signature = None;
        self.last_scan = None;
    }

    fn set_window(&mut self, handle: Option<i64>) {
        if self.selected_handle != handle {
            self.selected_handle = handle;
            self.invalidate_scan();
            self.tasks
                .values_mut()
                .for_each(|state| *state = TaskState::default());
            self.preview_full = None;
            self.preview_texture = None;
            self.gate_result = None;
            self.gate_active =
                !self.config.color_gate_enabled || self.config.color_gate_points.is_empty();
            self.last_gate_check =
                Instant::now() - Duration::from_millis(self.config.color_gate_interval_ms as u64);
        }
    }

    fn toggle_running(&mut self) {
        if self.running {
            self.running = false;
            self.invalidate_scan();
            self.log("已暂停识别");
        } else {
            if self.selected_handle.is_none() {
                self.refresh_windows(false);
            }
            if self.selected_handle.is_none() {
                self.notice = Some("未找到游戏窗口，请打开游戏后刷新窗口列表".into());
                return;
            }
            if self.config.ocr_mode == "remote" && self.config.api_url.trim().is_empty() {
                self.notice = Some("请先填写远端 API 地址，或切换到本地识别".into());
                return;
            }
            self.running = true;
            self.invalidate_scan();
            self.check_gate();
            self.last_gate_check = Instant::now();
            self.notice = None;
            self.log("开始自动识别");
        }
    }

    fn update_preview(&mut self, image: &image::RgbImage) {
        let color = to_color_image(image);
        if let Some(texture) = &mut self.preview_texture {
            texture.set(color.clone(), egui::TextureOptions::LINEAR);
        } else {
            self.preview_texture = Some(self.ctx.load_texture(
                "client_preview",
                color.clone(),
                egui::TextureOptions::LINEAR,
            ));
        }
        self.preview_full = Some(color);
    }

    fn runtime_status(&self) -> (&str, Color32) {
        if self.busy {
            ("正在识别", theme::JADE)
        } else if !self.running {
            ("已暂停", theme::MUTED)
        } else if self.selected_handle.is_none() {
            ("等待游戏窗口", theme::GOLD)
        } else if self.config.color_gate_enabled && !self.gate_active {
            ("图色未命中 · 等待任务", theme::GOLD)
        } else {
            ("自动识别中", theme::JADE)
        }
    }

    /// 扫描游戏窗口并自动选中一个（优先梦幻西游，其次任意可用窗口）。
    fn refresh_windows(&mut self, log_always: bool) {
        match capture::list_windows() {
            Ok(windows) => {
                let count = windows.len();
                let previous = self.selected_handle;
                self.windows = windows;
                let still = previous
                    .filter(|handle| self.windows.iter().any(|window| window.handle == *handle));
                self.set_window(still.or_else(|| self.windows.first().map(|window| window.handle)));
                if self.selected_handle != previous {
                    match self.selected_handle {
                        Some(handle) => {
                            let title = self
                                .windows
                                .iter()
                                .find(|window| window.handle == handle)
                                .map(|window| window.title.clone())
                                .unwrap_or_default();
                            self.log_detail(format!(
                                "自动选择游戏窗口：{title}（可选 {count} 个）"
                            ));
                        }
                        None => self.log_detail(format!("未找到游戏窗口（可选 {count} 个）")),
                    }
                } else if log_always {
                    self.log_detail(format!("扫描到 {count} 个可用窗口"));
                }
            }
            Err(err) => {
                self.log_detail(format!("枚举窗口失败：{err}"));
            }
        }
    }

    fn task_active(&self, task_type: &str) -> bool {
        task_type != "鬼王" || self.config.ghost_king_enabled
    }

    fn mark_all_stale(&mut self, message: &str) {
        for (task, state) in self.tasks.iter_mut() {
            if *task != "鬼王" || self.config.ghost_king_enabled {
                state.status = Some(message.to_string());
            }
        }
    }

    /// 图色门控：抓一小块屏幕比对点位颜色，判断当前是否有任务。
    fn check_gate(&mut self) {
        if !self.config.color_gate_enabled || self.config.color_gate_points.is_empty() {
            self.gate_active = true;
            self.gate_result = None;
            return;
        }
        if self.selected_handle.is_none() {
            self.gate_active = false;
            return;
        }
        let Some(handle) = self.selected_handle else {
            self.gate_active = false;
            return;
        };
        let Some((mut x, mut y, mut width, mut height)) =
            gate::bounding_box(&self.config.color_gate_points)
        else {
            self.gate_active = true;
            return;
        };
        if let Some(image) = &self.preview_full {
            let client_w = image.width() as i32;
            let client_h = image.height() as i32;
            x = x.clamp(0, (client_w - 1).max(0));
            y = y.clamp(0, (client_h - 1).max(0));
            width = width.clamp(1, (client_w - x).max(1));
            height = height.clamp(1, (client_h - y).max(1));
        }
        match capture::capture_client_rect(handle, x, y, width, height) {
            Ok(image) => {
                let mode = GateMode::from_str(&self.config.color_gate_mode);
                let result = gate::evaluate(&image, (x, y), &self.config.color_gate_points, mode);
                let previous = self.gate_active;
                self.gate_active = result.active;
                self.gate_result = Some(result);
                if self.gate_active != previous {
                    if self.gate_active {
                        self.log("任务开始（图色命中）");
                        // 立即识别一次，不用等下一个识别间隔。
                        self.invalidate_scan();
                    } else {
                        self.invalidate_scan();
                        self.log("任务停止（图色未命中）");
                    }
                }
            }
            Err(err) => {
                if self.gate_active {
                    self.log_detail(format!("图色采样失败：{err}"));
                    self.invalidate_scan();
                }
                self.gate_result = None;
                self.gate_active = false;
            }
        }
    }

    fn trigger_scan(&mut self) {
        if self.busy {
            return;
        }
        self.last_scan = Some(Instant::now());
        if self.selected_handle.is_none() {
            self.refresh_windows(false);
        }
        let Some(handle) = self.selected_handle else {
            self.log_detail("未找到游戏窗口，等待下次扫描");
            return;
        };
        if self.config.color_gate_enabled && !self.gate_active {
            let title = self
                .windows
                .iter()
                .find(|window| window.handle == handle)
                .map(|window| window.title.clone())
                .unwrap_or_default();
            self.log_detail(format!("{title}：图色未命中，暂停 OCR"));
            return;
        }
        let window_title = self
            .windows
            .iter()
            .find(|window| window.handle == handle)
            .map(|window| window.title.clone())
            .unwrap_or_else(|| format!("#{handle}"));
        let extra = if self.config.extra_region_enabled {
            Some(self.config.extra_task_region)
        } else {
            None
        };
        self.last_scan = Some(Instant::now());
        let (panel, full) =
            match capture::capture_task_panel(handle, self.config.task_region, extra) {
                Ok(result) => result,
                Err(err) => {
                    self.log_detail(format!("截图失败（{window_title}）：{err}"));
                    self.notice = Some(err.clone());
                    self.mark_all_stale("本轮截图失败，保留上次坐标");
                    if err.contains("关闭") || err.contains("最小化") {
                        self.set_window(None);
                    }
                    return;
                }
            };
        self.update_preview(&full);
        self.notice = None;

        let signature = hash_image(&panel);
        if self.last_signature == Some(signature) {
            self.log_detail(format!("{window_title}：识别区域未变化，跳过"));
            return;
        }
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.pending_signature = Some(signature);
        self.busy = true;

        if self.config.ocr_mode == "remote" {
            let png = match ocr::encode_png(&panel) {
                Ok(png) => png,
                Err(err) => {
                    self.log_detail(format!("编码截图失败：{err}"));
                    self.notice = Some(err);
                    self.pending_signature = None;
                    self.busy = false;
                    return;
                }
            };
            let url = self.config.api_url.clone();
            let token = self.config.api_token.clone();
            let format = self.config.api_format.clone();
            let sender = self.sender.clone();
            let ctx = self.ctx.clone();
            self.log_detail(format!("{window_title}：请求远端 PP-OCR {url}"));
            thread::spawn(move || {
                let result = ocr::remote_recognize(&url, &token, &format, &png);
                let _ = sender.send(OcrEvent::Remote { generation, result });
                ctx.request_repaint();
            });
        } else {
            self.log_detail(format!("{window_title}：本地 PP-OCR 识别中"));
            if let Err(err) = self.worker.submit(generation, panel) {
                self.busy = false;
                self.notice = Some(err);
            }
        }
    }

    fn poll_events(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            let (generation, result) = match event {
                OcrEvent::Local { generation, result } => (generation, result),
                OcrEvent::Remote { generation, result } => (generation, result),
                OcrEvent::Test { result } => {
                    self.testing_remote = false;
                    self.remote_feedback = Some(match result {
                        Ok(text) => Ok(format!("连接正常 · {:.2} 秒", text.elapsed_secs)),
                        Err(err) => Err(err),
                    });
                    if let Some(result) = &self.remote_feedback {
                        self.log_detail(match result {
                            Ok(message) => format!("远端 PP-OCR {message}"),
                            Err(err) => format!("远端 PP-OCR 连接失败：{err}"),
                        });
                    }
                    continue;
                }
            };
            if generation != self.generation {
                continue;
            }
            self.busy = false;
            match result {
                Err(err) => {
                    self.log_detail(format!("识别失败：{err}"));
                    self.notice = Some(err);
                    self.pending_signature = None;
                    self.mark_all_stale("本轮识别失败，保留上次坐标");
                }
                Ok(text) => {
                    self.last_signature = self.pending_signature.take();
                    self.last_ocr_secs = Some(text.elapsed_secs);
                    self.notice = None;
                    self.apply_ocr(&text);
                }
            }
        }
    }

    /// 测试远端 PP-OCR 连接：发一张 16×16 白图，把结果写日志。
    fn test_remote_ocr(&mut self) {
        self.config.api_url = self.api_url_edit.trim().to_string();
        self.config.api_token = self.api_token_edit.trim().to_string();
        self.persist();
        if self.config.api_url.is_empty() {
            self.remote_feedback = Some(Err("请先填写 API 地址".into()));
            return;
        }
        let image = image::RgbImage::from_pixel(16, 16, image::Rgb([255, 255, 255]));
        let png = match ocr::encode_png(&image) {
            Ok(png) => png,
            Err(err) => {
                self.remote_feedback = Some(Err(err));
                return;
            }
        };
        let url = self.config.api_url.clone();
        let token = self.config.api_token.clone();
        let format = self.config.api_format.clone();
        let sender = self.sender.clone();
        self.testing_remote = true;
        self.remote_feedback = None;
        let ctx = self.ctx.clone();
        self.log_detail(format!("正在测试远端 PP-OCR：{url}"));
        thread::spawn(move || {
            let result = ocr::remote_recognize(&url, &token, &format, &png);
            let _ = sender.send(OcrEvent::Test { result });
            ctx.request_repaint();
        });
    }

    fn apply_ocr(&mut self, ocr_text: &OcrText) {
        let locations = parse_tasks(&ocr_text.text, &self.maps, self.config.ghost_king_enabled);
        for task_type in TASK_ORDER {
            if !self.task_active(task_type) {
                continue;
            }
            let location = locations
                .iter()
                .find(|location| location.task_type == task_type)
                .cloned();
            let (map_index, task_status) = match &location {
                None => (None, Some("本轮未识别到任务".to_string())),
                Some(location) => {
                    if self.maps.is_empty() {
                        (None, Some("地图配置为空".to_string()))
                    } else {
                        match find_map_definition(&location.map_name, &self.maps, Some(task_type)) {
                            Some(definition) => {
                                let index =
                                    self.maps.iter().position(|map| map.name == definition.name);
                                if !coordinate_in_map(
                                    definition,
                                    location.x as f32,
                                    location.y as f32,
                                    MAP_COORDINATE_TOLERANCE,
                                ) {
                                    (None, Some("本轮坐标无效，忽略".to_string()))
                                } else {
                                    (index, None)
                                }
                            }
                            None => {
                                if find_map_definition(&location.map_name, &self.maps, None)
                                    .is_some()
                                {
                                    (None, Some(format!("本轮地图不属于{task_type}任务，忽略")))
                                } else {
                                    (None, Some("本轮地图名称无效，忽略".to_string()))
                                }
                            }
                        }
                    }
                }
            };
            let summary = match (&task_status, &location) {
                (None, Some(location)) => format!(
                    "{task_type} {} ({}, {})",
                    location.map_name, location.x, location.y
                ),
                (Some(message), _) => format!("{task_type} {message}"),
                (None, None) => format!("{task_type} 无结果"),
            };
            self.log_detail(format!("{summary}（{:.2}s）", ocr_text.elapsed_secs));
            if let Some(state) = self.tasks.get_mut(task_type) {
                if task_status.is_none() {
                    state.location = location;
                    state.map_index = map_index;
                    state.captured_at = Some(Instant::now());
                    state.status = None;
                } else {
                    state.status = task_status;
                }
            }
        }
    }

    fn map_texture(
        &mut self,
        ctx: &egui::Context,
        definition: &MapDefinition,
    ) -> Option<TextureHandle> {
        let key = definition.image.to_string_lossy().to_string();
        if let Some(handle) = self.map_textures.get(&key) {
            return Some(handle.clone());
        }
        if self.map_texture_failed.contains(&key) {
            return None;
        }
        let image = match crate::core::load_rgb_image(&definition.image) {
            Ok(image) => image,
            Err(err) => {
                if self.map_texture_failed.insert(key.clone()) {
                    self.log_detail(format!(
                        "地图 {} 图片加载失败：{}（{err}）",
                        definition.name,
                        definition.image.display()
                    ));
                }
                return None;
            }
        };
        let color = to_color_image(&image);
        let handle = ctx.load_texture(key.clone(), color, egui::TextureOptions::LINEAR);
        self.map_textures.insert(key, handle.clone());
        Some(handle)
    }

    fn render_map(
        &mut self,
        ui: &mut egui::Ui,
        definition: &MapDefinition,
        location: &TaskLocation,
    ) {
        let Some(texture) = self.map_texture(ui.ctx(), definition) else {
            ui.colored_label(Color32::from_rgb(240, 180, 80), "地图图片未配置");
            ui.add_space(6.0);
            return;
        };
        let texture_size = texture.size_vec2();
        let available = ui.available_size();
        if available.x < 8.0 || available.y < 8.0 {
            return;
        }
        let (rect, response) = ui.allocate_exact_size(available, Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(8), theme::INSET);
        let outer = rect.shrink(8.0);
        let image_rect = fit_rect(outer, texture_size);
        if image_rect.width() <= 1.0 {
            return;
        }
        painter.image(
            texture.id(),
            image_rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let scale = image_rect.width() / texture_size.x;
        let to_screen = |point: (f32, f32)| {
            pos2(
                image_rect.min.x + point.0 * scale,
                image_rect.min.y + point.1 * scale,
            )
        };
        let (point_x, point_y) = clamp_to_map(definition, location.x as f32, location.y as f32);

        if location.nearby {
            let (left, top, right, bottom) = map_area(
                definition,
                point_x,
                point_y,
                50.0,
                texture_size.x,
                texture_size.y,
            );
            let area = Rect::from_min_max(to_screen((left, top)), to_screen((right, bottom)))
                .intersect(image_rect);
            let shade = Color32::from_rgba_unmultiplied(0, 0, 0, 150);
            painter.rect_filled(
                Rect::from_min_max(image_rect.min, pos2(image_rect.max.x, area.min.y)),
                CornerRadius::ZERO,
                shade,
            );
            painter.rect_filled(
                Rect::from_min_max(pos2(image_rect.min.x, area.max.y), image_rect.max),
                CornerRadius::ZERO,
                shade,
            );
            painter.rect_filled(
                Rect::from_min_max(
                    pos2(image_rect.min.x, area.min.y),
                    pos2(area.min.x, area.max.y),
                ),
                CornerRadius::ZERO,
                shade,
            );
            painter.rect_filled(
                Rect::from_min_max(
                    pos2(area.max.x, area.min.y),
                    pos2(image_rect.max.x, area.max.y),
                ),
                CornerRadius::ZERO,
                shade,
            );
            painter.rect_stroke(
                area,
                CornerRadius::ZERO,
                egui::Stroke::new(2.0, Color32::from_rgb(255, 209, 102)),
                egui::StrokeKind::Inside,
            );
        }

        if self.config.high_chance {
            for (chance_x, chance_y) in high_chance_areas(
                &definition.name,
                location.x as f32,
                location.y as f32,
                HIGH_CHANCE_SIZE,
            ) {
                let (left, top, right, bottom) = map_rect(
                    definition,
                    chance_x,
                    chance_y,
                    chance_x + HIGH_CHANCE_SIZE,
                    chance_y + HIGH_CHANCE_SIZE,
                    texture_size.x,
                    texture_size.y,
                );
                let chance = Rect::from_min_max(to_screen((left, top)), to_screen((right, bottom)))
                    .intersect(image_rect);
                if chance.width() <= 0.0 || chance.height() <= 0.0 {
                    continue;
                }
                painter.rect_filled(
                    chance,
                    CornerRadius::ZERO,
                    Color32::from_rgba_unmultiplied(255, 0, 0, 112),
                );
                painter.rect_stroke(
                    chance,
                    CornerRadius::ZERO,
                    egui::Stroke::new(2.0, Color32::from_rgba_unmultiplied(255, 0, 0, 180)),
                    egui::StrokeKind::Inside,
                );
            }
        }

        if let Some(point) = map_point(definition, point_x, point_y, texture_size.x, texture_size.y)
        {
            let center = to_screen(point);
            painter.circle_filled(center, 8.0, Color32::from_rgb(255, 51, 78));
            painter.circle_stroke(center, 8.0, egui::Stroke::new(2.0, Color32::WHITE));
            let label = format!("({}, {})", location.x, location.y);
            let galley = painter.layout_no_wrap(label, FontId::proportional(14.0), Color32::WHITE);
            let label_size = galley.size() + vec2(12.0, 8.0);
            let label_min = pos2(
                (center.x + 12.0)
                    .clamp(rect.left(), (rect.right() - label_size.x).max(rect.left())),
                (center.y - 30.0).clamp(rect.top(), (rect.bottom() - label_size.y).max(rect.top())),
            );
            painter.rect_filled(
                Rect::from_min_size(label_min, label_size),
                CornerRadius::same(4),
                Color32::from_black_alpha(220),
            );
            painter.galley(label_min + vec2(6.0, 4.0), galley, Color32::WHITE);
        }

        if let Some(pointer) = response.hover_pos() {
            if image_rect.contains(pointer) {
                let pixel_x = (pointer.x - image_rect.min.x) / scale;
                let pixel_y = (pointer.y - image_rect.min.y) / scale;
                if let Some((game_x, game_y)) =
                    map_coordinate(definition, pixel_x, pixel_y, texture_size.x, texture_size.y)
                {
                    painter.text(
                        image_rect.right_top() + vec2(-8.0, 8.0),
                        Align2::RIGHT_TOP,
                        format!("鼠标坐标：{game_x}, {game_y}"),
                        FontId::proportional(13.0),
                        Color32::from_rgb(248, 250, 252),
                    );
                }
            }
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("sidebar")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                theme::section(ui, "游戏窗口", |ui| {
                    let current = self
                        .windows
                        .iter()
                        .find(|window| Some(window.handle) == self.selected_handle)
                        .map(|window| window.title.clone())
                        .unwrap_or_else(|| "未找到游戏窗口".into());
                    let mut selected = self.selected_handle;
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("window")
                            .selected_text(current)
                            .width((ui.available_width() - 66.0).max(100.0))
                            .show_ui(ui, |ui| {
                                for window in &self.windows {
                                    ui.selectable_value(
                                        &mut selected,
                                        Some(window.handle),
                                        &window.title,
                                    );
                                }
                            });
                        if ui.button("刷新").clicked() {
                            self.refresh_windows(true);
                            selected = self.selected_handle;
                        }
                    });
                    if selected != self.selected_handle {
                        self.set_window(selected);
                        self.log_detail("已切换游戏窗口，清除旧任务结果");
                    }
                    if self.windows.is_empty() {
                        ui.label(
                            egui::RichText::new("请先打开游戏，窗口列表会自动刷新")
                                .small()
                                .color(theme::GOLD),
                        );
                    }
                    ui.horizontal(|ui| {
                        ui.label("识别间隔");
                        if ui
                            .add(
                                egui::DragValue::new(&mut self.config.interval_seconds)
                                    .range(1..=60)
                                    .suffix(" 秒"),
                            )
                            .changed()
                        {
                            self.persist();
                        }
                    });
                    let label = if self.show_region_window {
                        "关闭识别区域"
                    } else {
                        "设置识别区域"
                    };
                    if ui
                        .add_sized([ui.available_width(), 32.0], egui::Button::new(label))
                        .clicked()
                    {
                        self.show_region_window = !self.show_region_window;
                        if self.show_region_window {
                            self.capture_preview_once();
                        }
                    }
                });
                theme::section(ui, "识别方式", |ui| {
                    let previous = self.config.ocr_mode.clone();
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.config.ocr_mode,
                            "local".into(),
                            "本地 PP-OCR",
                        );
                        ui.selectable_value(
                            &mut self.config.ocr_mode,
                            "remote".into(),
                            "远端 PP-OCR",
                        );
                    });
                    if previous != self.config.ocr_mode {
                        self.invalidate_scan();
                        self.notice = None;
                        self.persist();
                    }
                    if self.config.ocr_mode == "remote" {
                        ui.label(egui::RichText::new("服务地址").small().weak());
                        let url_changed = ui
                            .add(
                                egui::TextEdit::singleline(&mut self.api_url_edit)
                                    .desired_width(f32::INFINITY)
                                    .hint_text("http://192.168.1.100:8008"),
                            )
                            .changed();
                        ui.label(egui::RichText::new("访问令牌（可选）").small().weak());
                        let token_changed = ui
                            .add(
                                egui::TextEdit::singleline(&mut self.api_token_edit)
                                    .desired_width(f32::INFINITY)
                                    .password(true)
                                    .hint_text("Bearer Token"),
                            )
                            .changed();
                        if url_changed || token_changed {
                            self.config.api_url = self.api_url_edit.trim().into();
                            self.config.api_token = self.api_token_edit.trim().into();
                            self.remote_feedback = None;
                            self.invalidate_scan();
                            self.persist();
                        }
                        let previous = self.config.api_format.clone();
                        ui.horizontal(|ui| {
                            ui.label("格式");
                            ui.selectable_value(
                                &mut self.config.api_format,
                                "multipart".into(),
                                "multipart",
                            );
                            ui.selectable_value(&mut self.config.api_format, "raw".into(), "raw");
                        });
                        if previous != self.config.api_format {
                            self.invalidate_scan();
                            self.remote_feedback = None;
                            self.persist();
                        }
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(
                                    !self.testing_remote,
                                    egui::Button::new(if self.testing_remote {
                                        "测试中…"
                                    } else {
                                        "测试连接"
                                    }),
                                )
                                .clicked()
                            {
                                self.test_remote_ocr();
                            }
                            if self.testing_remote {
                                ui.spinner();
                            }
                        });
                        if let Some(result) = &self.remote_feedback {
                            let (message, color) = match result {
                                Ok(message) => (message, theme::JADE),
                                Err(message) => (message, theme::RED),
                            };
                            ui.label(egui::RichText::new(message).small().color(color));
                        }
                    } else {
                        ui.label(
                            egui::RichText::new("首次识别时加载模型；缺少模型时自动下载约 25 MB。")
                                .small()
                                .weak(),
                        );
                    }
                });
                theme::section(ui, "任务显示", |ui| {
                    if ui
                        .checkbox(&mut self.config.ghost_king_enabled, "同时识别鬼王任务")
                        .changed()
                    {
                        self.invalidate_scan();
                        self.persist();
                    }
                    if ui
                        .checkbox(&mut self.config.high_chance, "叠加高概率区域（50 × 50）")
                        .changed()
                    {
                        self.persist();
                    }
                    let previous = self.config.horizontal_layout;
                    ui.horizontal(|ui| {
                        ui.label("布局");
                        ui.selectable_value(&mut self.config.horizontal_layout, false, "上下");
                        ui.selectable_value(&mut self.config.horizontal_layout, true, "左右");
                    });
                    if previous != self.config.horizontal_layout {
                        self.persist();
                    }
                });
                theme::section(ui, "界面", |ui| {
                    ui.horizontal(|ui| {
                        ui.label("缩放");
                        let mut follow_system = self.config.window_scale == 0.0;
                        if ui.checkbox(&mut follow_system, "跟随系统").changed() {
                            self.config.window_scale = if follow_system { 0.0 } else { 1.0 };
                            self.persist();
                        }
                        if !follow_system
                            && ui
                                .add(
                                    egui::DragValue::new(&mut self.config.window_scale)
                                        .speed(0.05)
                                        .range(0.75..=2.5)
                                        .fixed_decimals(2)
                                        .suffix(" ×"),
                                )
                                .changed()
                        {
                            self.persist();
                        }
                    });
                });
                self.log_ui(ui);
                ui.add_space(4.0);
            });
    }

    fn log_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("日志").strong());
            let mut level = self.config.log_level.clone();
            let mut changed = false;
            changed |= ui
                .radio_value(&mut level, "simple".to_string(), "简单")
                .changed();
            changed |= ui
                .radio_value(&mut level, "detailed".to_string(), "详细")
                .changed();
            if changed {
                self.config.log_level = level;
                self.persist();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("清空").clicked() {
                    self.logs.clear();
                }
            });
        });
        egui::Frame::new()
            .fill(theme::INSET)
            .corner_radius(8)
            .inner_margin(10)
            .show(ui, |ui| {
                if self.logs.is_empty() {
                    ui.label(egui::RichText::new("暂无日志").small().weak());
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("log_scroll")
                        .stick_to_bottom(true)
                        .max_height(LOG_PANEL_HEIGHT)
                        .auto_shrink([false, true])
                        .show_rows(ui, 15.0, self.logs.len(), |ui, rows| {
                            for index in rows {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&self.logs[index])
                                            .monospace()
                                            .size(11.0)
                                            .color(theme::MUTED),
                                    )
                                    .truncate(),
                                )
                                .on_hover_text(&self.logs[index]);
                            }
                        });
                }
            });
    }

    /// 画客户区预览：红框主区域、蓝框备用区域、图色点位；点击时返回 (x, y, 颜色)。
    fn preview(&self, ui: &mut egui::Ui) -> Option<(i32, i32, [u8; 3])> {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("识别区域预览").strong());
            ui.label(egui::RichText::new("红框=主区域，蓝框=备用区域；点击图片可取点").weak());
        });
        let Some(image) = self.preview_full.as_ref() else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("暂无截图\n选择游戏窗口后，点击「更新截图」")
                        .color(theme::MUTED),
                );
            });
            return None;
        };
        let texture = self.preview_texture.as_ref()?;
        let size = Vec2::new(image.width() as f32, image.height() as f32);
        let available = ui.available_size();
        let scale = (available.x / size.x).min(available.y / size.y).max(0.001);
        let (rect, response) = ui.allocate_exact_size(size * scale, Sense::click());
        let painter = ui.painter_at(rect);
        let target = fit_rect(rect, size);
        painter.image(
            texture.id(),
            target,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let draw_scale = target.width() / size.x;
        let to_screen =
            |x: f32, y: f32| pos2(target.min.x + x * draw_scale, target.min.y + y * draw_scale);
        for (region, color, enabled) in [
            (self.config.task_region, theme::RED, true),
            (
                self.config.extra_task_region,
                Color32::from_rgb(100, 174, 255),
                self.config.extra_region_enabled,
            ),
        ] {
            if !enabled {
                continue;
            }
            let (left, top, right, bottom) = crate::core::task_region(size.x, size.y, region);
            painter.rect_stroke(
                Rect::from_min_max(to_screen(left, top), to_screen(right, bottom)),
                CornerRadius::ZERO,
                egui::Stroke::new(2.0, color),
                egui::StrokeKind::Inside,
            );
        }

        // 鼠标悬停：显示客户区坐标与该像素颜色；点击则可取点。
        let mut picked = None;
        if let Some(pos) = response.hover_pos().filter(|pos| target.contains(*pos)) {
            let px = ((pos.x - target.min.x) / draw_scale).floor() as i32;
            let py = ((pos.y - target.min.y) / draw_scale).floor() as i32;
            if let Some(color) = sample_image(image, px, py) {
                painter.circle_stroke(
                    to_screen(px as f32, py as f32),
                    5.0,
                    egui::Stroke::new(1.5, Color32::WHITE),
                );
                painter.text(
                    target.left_top() + vec2(8.0, 8.0),
                    Align2::LEFT_TOP,
                    format!("({px}, {py})  {}", gate::to_hex(color)),
                    FontId::proportional(13.0),
                    Color32::WHITE,
                );
                if self.pick_points && response.clicked() {
                    picked = Some((px, py, color));
                }
            }
        }

        // 已配置的点位，标在地图上。
        for point in &self.config.color_gate_points {
            let center = to_screen(point.x as f32, point.y as f32);
            let box_rect = Rect::from_center_size(center, vec2(9.0, 9.0));
            let color = Color32::from_rgb(point.color[0], point.color[1], point.color[2]);
            painter.rect_filled(box_rect, CornerRadius::ZERO, color);
            painter.rect_stroke(
                box_rect,
                CornerRadius::ZERO,
                egui::Stroke::new(1.5, Color32::WHITE),
                egui::StrokeKind::Outside,
            );
        }
        picked
    }

    fn region_window_ui(&mut self, ui: &mut egui::Ui) {
        if ui.ctx().input(|input| input.viewport().close_requested()) {
            self.show_region_window = false;
        }
        egui::Frame::central_panel(ui.style())
            .inner_margin(16)
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.horizontal_wrapped(|ui| {
                    ui.heading("识别区域");
                    if ui.button("更新截图").clicked() {
                        self.capture_preview_once();
                    }
                    ui.checkbox(&mut self.pick_points, "点击预览取点");
                });
                ui.label(
                    egui::RichText::new("截取游戏客户区，设置任务栏范围与图色点位")
                        .small()
                        .weak(),
                );
                let max_height = (ui.available_height() * 0.50).max(100.0);
                egui::ScrollArea::vertical()
                    .id_salt("region_settings")
                    .max_height(max_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        theme::section(ui, "截图范围 · 相对客户区百分比", |ui| {
                            let mut changed =
                                region_row(ui, "主区域", &mut self.config.task_region);
                            changed |= ui
                                .checkbox(&mut self.config.extra_region_enabled, "启用备用区域")
                                .changed();
                            if self.config.extra_region_enabled {
                                changed |=
                                    region_row(ui, "备用区", &mut self.config.extra_task_region);
                            }
                            if changed {
                                self.invalidate_scan();
                                self.persist();
                            }
                        });
                        theme::section(ui, "图色门控", |ui| self.color_gate_ui(ui));
                    });
                if let Some(message) = &self.notice {
                    ui.label(egui::RichText::new(message).small().color(theme::GOLD));
                }
                ui.add_space(4.0);
                if let Some((x, y, color)) = self.preview(ui) {
                    let mut point =
                        ColorPoint::new(x, y, color, self.config.color_gate_tolerance, 1);
                    point.label = format!("点{}", self.config.color_gate_points.len() + 1);
                    self.config.color_gate_points.push(point);
                    self.check_gate();
                    self.persist();
                    self.log_detail(format!("新增图色点位 ({x}, {y}) {}", gate::to_hex(color)));
                }
            });
    }

    fn color_gate_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .checkbox(&mut self.config.color_gate_enabled, "启用")
                .changed()
            {
                self.invalidate_scan();
                self.check_gate();
                self.last_gate_check = Instant::now();
                self.persist();
            }
            ui.label("间隔");
            let mut interval = self.config.color_gate_interval_ms;
            if ui
                .add(
                    egui::DragValue::new(&mut interval)
                        .range(50..=5000)
                        .suffix(" ms"),
                )
                .changed()
            {
                self.config.color_gate_interval_ms = interval;
                self.persist();
            }
            ui.label("默认容差");
            let mut tolerance = self.config.color_gate_tolerance;
            if ui
                .add(egui::DragValue::new(&mut tolerance).range(0..=255))
                .changed()
            {
                self.config.color_gate_tolerance = tolerance;
                self.persist();
            }
            let mut mode = self.config.color_gate_mode.clone();
            let mut mode_changed = false;
            mode_changed |= ui
                .radio_value(&mut mode, "all".to_string(), "全部命中")
                .changed();
            mode_changed |= ui
                .radio_value(&mut mode, "any".to_string(), "任一命中")
                .changed();
            if mode_changed {
                self.config.color_gate_mode = mode;
                self.check_gate();
                self.persist();
            }
        });
        let status = if !self.config.color_gate_enabled {
            "未启用".to_string()
        } else if self.config.color_gate_points.is_empty() {
            "未配置点位（默认允许 OCR）".to_string()
        } else if self.gate_active {
            "有任务：允许 OCR".to_string()
        } else {
            "未命中：暂停 OCR".to_string()
        };
        ui.label(egui::RichText::new(format!("状态：{status}")).weak());

        let mut points = self.config.color_gate_points.clone();
        let gate_result = self.gate_result.clone();
        let mut changed = false;
        let mut remove = None;
        for (index, point) in points.iter_mut().enumerate() {
            ui.horizontal_wrapped(|ui| {
                let (swatch, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                ui.painter().rect_filled(
                    swatch,
                    CornerRadius::same(2),
                    Color32::from_rgb(point.color[0], point.color[1], point.color[2]),
                );
                ui.label(format!("({}, {}) {}", point.x, point.y, point.hex()));
                ui.label("容差");
                let mut tolerance = point.tolerance;
                if ui
                    .add(egui::DragValue::new(&mut tolerance).range(0..=255))
                    .changed()
                {
                    point.tolerance = tolerance;
                    changed = true;
                }
                ui.label("半径");
                let mut radius = point.radius;
                if ui
                    .add(egui::DragValue::new(&mut radius).range(0..=20))
                    .changed()
                {
                    point.radius = radius;
                    changed = true;
                }
                if let Some(result) = &gate_result {
                    if let (Some(hit), Some(sampled)) =
                        (result.matched.get(index), result.sampled.get(index))
                    {
                        let text = match sampled {
                            Some(color) => format!(
                                "实测 {} {}",
                                gate::to_hex(*color),
                                if *hit { "命中" } else { "未命中" }
                            ),
                            None => "越界".to_string(),
                        };
                        let color = if *hit {
                            Color32::from_rgb(120, 220, 120)
                        } else {
                            Color32::from_rgb(240, 160, 160)
                        };
                        ui.colored_label(color, text);
                    }
                }
                if ui.small_button("删除").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = remove {
            points.remove(index);
            changed = true;
        }
        if ui.button("清空点位").clicked() {
            points.clear();
            changed = true;
        }
        if changed {
            self.config.color_gate_points = points;
            self.check_gate();
            self.last_gate_check = Instant::now();
            self.persist();
        }
    }

    /// 手动截取一次游戏客户区，只更新预览，不做 OCR。
    fn capture_preview_once(&mut self) {
        if self.selected_handle.is_none() {
            self.refresh_windows(false);
        }
        let Some(handle) = self.selected_handle else {
            self.notice = Some("未找到游戏窗口，请打开游戏后刷新窗口列表".into());
            return;
        };
        let extra = if self.config.extra_region_enabled {
            Some(self.config.extra_task_region)
        } else {
            None
        };
        let title = self
            .windows
            .iter()
            .find(|window| window.handle == handle)
            .map(|window| window.title.clone())
            .unwrap_or_else(|| format!("#{handle}"));
        match capture::capture_task_panel(handle, self.config.task_region, extra) {
            Ok((_panel, full)) => {
                self.update_preview(&full);
                self.notice = None;
                self.log_detail(format!("手动截取截图：{title}"));
                // 立即刷新一次图色门控状态，方便在取点工具里看命中情况。
                self.check_gate();
            }
            Err(err) => {
                self.log_detail(format!("手动截图失败：{err}"));
                self.notice = Some(err.clone());
                if err.contains("关闭") || err.contains("最小化") {
                    self.set_window(None);
                }
            }
        }
    }

    fn task_panel(&mut self, ui: &mut egui::Ui, task_type: &str) {
        let (location, map_index, status, captured) = {
            let state = self.tasks.get(task_type).expect("task state");
            (
                state.location.clone(),
                state.map_index,
                state.status.clone(),
                state.captured_at,
            )
        };
        let accent = if task_type == "抓鬼" {
            theme::JADE
        } else {
            theme::GOLD
        };
        theme::card().show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            // 标题与状态左右排列，始终只占固定一行，防止提示变化推挤地图。
            let (header, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::hover());
            let status_width = (header.width() * 0.40).min(240.0);
            let status_rect = Rect::from_min_max(
                pos2(header.right() - status_width, header.top()),
                header.max,
            );
            let title_rect =
                Rect::from_min_max(header.min, pos2(status_rect.left() - 8.0, header.bottom()));
            let mut title_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(title_rect)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
            );
            title_ui.set_clip_rect(title_rect.intersect(ui.clip_rect()));
            theme::badge(&mut title_ui, task_type, accent);
            if let Some(location) = &location {
                let coords = format!("({}, {})", location.x, location.y);
                let title = format!(
                    "{} {}{}",
                    location.map_name,
                    coords,
                    if location.nearby { " · 附近" } else { "" }
                );
                if title_ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(&title)
                                .size(20.0)
                                .strong()
                                .color(accent),
                        )
                        .frame(false)
                        .truncate(),
                    )
                    .on_hover_text(format!("{title}\n点击复制地图与坐标"))
                    .clicked()
                {
                    ui.ctx()
                        .copy_text(format!("{} {}", location.map_name, coords));
                }
            }
            let age = captured.map(|time| format!("{} 秒前更新", time.elapsed().as_secs()));
            let message = status.as_deref().or(age.as_deref()).unwrap_or("等待识别");
            let tooltip = match (&status, &age) {
                (Some(status), Some(age)) => format!("{status}\n{age}"),
                _ => message.to_string(),
            };
            let mut status_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(status_rect)
                    .layout(egui::Layout::right_to_left(egui::Align::Center)),
            );
            status_ui.set_clip_rect(status_rect.intersect(ui.clip_rect()));
            status_ui
                .add(
                    egui::Label::new(egui::RichText::new(message).size(12.0).color(
                        if status.is_some() {
                            theme::GOLD
                        } else {
                            theme::MUTED
                        },
                    ))
                    .truncate(),
                )
                .on_hover_text(tooltip);
            ui.add_space(2.0);
            match (location, map_index) {
                (Some(location), Some(index)) => {
                    let definition = self.maps[index].clone();
                    self.render_map(ui, &definition, &location);
                }
                _ => {
                    let size = ui.available_size().max(vec2(1.0, 1.0));
                    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(rect, CornerRadius::same(8), theme::INSET);
                    let center = rect.center() - vec2(0.0, 24.0);
                    painter.circle_stroke(center, 18.0, egui::Stroke::new(1.0, theme::BORDER));
                    painter.circle_stroke(
                        center,
                        9.0,
                        egui::Stroke::new(1.0, accent.gamma_multiply(0.6)),
                    );
                    painter.line_segment(
                        [center - vec2(25.0, 0.0), center + vec2(25.0, 0.0)],
                        egui::Stroke::new(1.0, theme::BORDER),
                    );
                    painter.line_segment(
                        [center - vec2(0.0, 25.0), center + vec2(0.0, 25.0)],
                        egui::Stroke::new(1.0, theme::BORDER),
                    );
                    painter.text(
                        center + vec2(0.0, 42.0),
                        Align2::CENTER_CENTER,
                        if self.running {
                            "正在等待任务信息"
                        } else {
                            "开始识别后，在这里查看地图"
                        },
                        FontId::proportional(14.0),
                        theme::MUTED,
                    );
                }
            }
        });
    }
}

impl eframe::App for MhxyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let desired_scale = self.config.window_scale;
        if desired_scale == 0.0 {
            if (ctx.zoom_factor() - 1.0).abs() > 0.001 {
                ctx.set_zoom_factor(1.0);
            }
        } else if (ctx.pixels_per_point() - desired_scale).abs() > 0.001 {
            ctx.set_pixels_per_point(desired_scale);
        }
        if self
            .config_dirty_since
            .is_some_and(|since| since.elapsed() >= Duration::from_millis(400))
        {
            self.flush_config();
        }
        self.poll_events();
        // 定时重新扫描游戏窗口（游戏可能后开 / 关掉）。
        if (self.running || self.selected_handle.is_none())
            && self.last_window_refresh.elapsed() >= Duration::from_secs(3)
        {
            self.refresh_windows(false);
            self.last_window_refresh = Instant::now();
        }
        if (self.running || self.show_region_window) && self.config.color_gate_enabled {
            let gate_interval =
                Duration::from_millis(self.config.color_gate_interval_ms.max(50) as u64);
            if self.last_gate_check.elapsed() >= gate_interval {
                self.check_gate();
                self.last_gate_check = Instant::now();
            }
            ctx.request_repaint_after(gate_interval);
        }
        if self.running && !self.busy {
            let interval = Duration::from_secs(self.config.interval_seconds.max(1) as u64);
            let due = self.last_scan.is_none_or(|last| last.elapsed() >= interval);
            if due {
                self.trigger_scan();
            }
        }
        // 即使暂停也定时唤醒：刷新窗口列表、连接测试结果和坐标更新时间。
        ctx.request_repaint_after(Duration::from_secs(1));
        if self.running || self.busy {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        egui::Panel::top("header")
            .frame(
                egui::Frame::new()
                    .fill(theme::SURFACE)
                    .inner_margin(egui::Margin::symmetric(16, 12)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui
                        .button(if self.config.sidebar_expanded {
                            "收起设置"
                        } else {
                            "展开设置"
                        })
                        .clicked()
                    {
                        self.config.sidebar_expanded = !self.config.sidebar_expanded;
                        self.persist();
                    }
                    ui.heading("梦幻西游");
                    ui.label(egui::RichText::new("任务坐标辅助").color(theme::MUTED));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add(theme::primary_button(if self.running {
                                "暂停识别"
                            } else {
                                "开始识别"
                            }))
                            .clicked()
                        {
                            self.toggle_running();
                        }
                        let (label, color) = self.runtime_status();
                        theme::status(ui, label, color);
                    });
                });
            });
        egui::Panel::bottom("status_bar")
            .frame(
                egui::Frame::new()
                    .fill(theme::SURFACE)
                    .inner_margin(egui::Margin::symmetric(16, 6)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(crate::APP_VERSION).small().weak());
                    ui.separator();
                    ui.label(
                        egui::RichText::new(if self.config.ocr_mode == "remote" {
                            "远端 PP-OCR"
                        } else {
                            "本地 PP-OCRv5"
                        })
                        .small()
                        .weak(),
                    );
                    ui.separator();
                    if let Some(message) = &self.notice {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(message).small().color(theme::GOLD),
                            )
                            .truncate(),
                        )
                        .on_hover_text(message);
                    } else {
                        ui.label(
                            egui::RichText::new(format!(
                                "每 {} 秒识别",
                                self.config.interval_seconds
                            ))
                            .small()
                            .weak(),
                        );
                    }
                    if let Some(seconds) = self.last_ocr_secs {
                        ui.separator();
                        ui.label(
                            egui::RichText::new(format!("上次耗时 {seconds:.2} 秒"))
                                .small()
                                .weak(),
                        );
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (text, color) = if self.save_error.is_some() {
                            ("配置保存失败", theme::RED)
                        } else if self.config_dirty_since.is_some() {
                            ("正在保存…", theme::MUTED)
                        } else {
                            ("设置自动保存", theme::MUTED)
                        };
                        ui.label(egui::RichText::new(text).small().color(color))
                            .on_hover_text(self.save_error.as_deref().unwrap_or(text));
                    });
                });
            });
        if self.config.sidebar_expanded {
            egui::Panel::left("controls")
                .resizable(true)
                .default_size(320.0)
                .min_size(290.0)
                .max_size(400.0)
                .frame(egui::Frame::new().fill(theme::BACKGROUND).inner_margin(12))
                .show(ui, |ui| self.controls(ui));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BACKGROUND).inner_margin(12))
            .show(ui, |ui| {
                let tasks: Vec<&'static str> = TASK_ORDER
                    .iter()
                    .copied()
                    .filter(|task| self.task_active(task))
                    .collect();
                if tasks.is_empty() {
                    return;
                }
                let rect = ui.available_rect_before_wrap();
                let count = tasks.len() as f32;
                for (index, task_type) in tasks.iter().enumerate() {
                    // 横向布局左右等分，纵向布局上下等分；只有一个任务时占满剩余区域。
                    let child_rect = if self.config.horizontal_layout {
                        let width = rect.width() / count;
                        Rect::from_min_size(
                            pos2(rect.min.x + width * index as f32, rect.min.y),
                            vec2(width, rect.height()),
                        )
                    } else {
                        let height = rect.height() / count;
                        Rect::from_min_size(
                            pos2(rect.min.x, rect.min.y + height * index as f32),
                            vec2(rect.width(), height),
                        )
                    };
                    let mut child =
                        ui.new_child(egui::UiBuilder::new().max_rect(child_rect.shrink(5.0)));
                    self.task_panel(&mut child, task_type);
                }
            });
        if self.show_region_window {
            let builder = egui::ViewportBuilder::default()
                .with_title("识别区域")
                .with_inner_size([960.0, 760.0])
                .with_min_inner_size([660.0, 480.0]);
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("region_window"),
                builder,
                |ui, _class| {
                    if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
                        egui::Window::new("识别区域")
                            .default_size([800.0, 640.0])
                            .show(ui.ctx(), |ui| self.region_window_ui(ui));
                    } else {
                        self.region_window_ui(ui);
                    }
                },
            );
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.flush_config();
    }
}

fn region_row(ui: &mut egui::Ui, label: &str, region: &mut crate::core::TaskRegion) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        let mut values = [
            region.0 * 100.0,
            region.1 * 100.0,
            region.2 * 100.0,
            region.3 * 100.0,
        ];
        let mut edited = false;
        for (name, value) in ["左", "上", "右", "下"].into_iter().zip(values.iter_mut()) {
            ui.label(egui::RichText::new(name).small().weak());
            let response = ui.add(
                egui::DragValue::new(value)
                    .speed(0.2)
                    .range(0.0..=100.0)
                    .suffix("%"),
            );
            edited |= response.changed();
        }
        if edited {
            *region = (
                values[0] / 100.0,
                values[1] / 100.0,
                values[2] / 100.0,
                values[3] / 100.0,
            );
            region.0 = region.0.clamp(0.0, 0.99);
            region.1 = region.1.clamp(0.0, 0.99);
            region.2 = region.2.clamp(region.0 + 0.01, 1.0);
            region.3 = region.3.clamp(region.1 + 0.01, 1.0);
            changed = true;
        }
    });
    changed
}

fn fit_rect(outer: Rect, size: Vec2) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return outer;
    }
    let scale = (outer.width() / size.x).min(outer.height() / size.y);
    Rect::from_center_size(outer.center(), size * scale)
}

fn to_color_image(image: &image::RgbImage) -> egui::ColorImage {
    egui::ColorImage::from_rgb(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    )
}

fn sample_image(image: &egui::ColorImage, x: i32, y: i32) -> Option<[u8; 3]> {
    if x < 0 || y < 0 || x as usize >= image.width() || y as usize >= image.height() {
        return None;
    }
    let pixel = image.pixels[y as usize * image.width() + x as usize];
    Some([pixel.r(), pixel.g(), pixel.b()])
}

fn hash_image(image: &image::RgbImage) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    image.dimensions().hash(&mut hasher);
    image.as_raw().hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;

    fn app() -> MhxyApp {
        let ctx = egui::Context::default();
        theme::install(&ctx);
        MhxyApp::from_config(Config::default(), PathBuf::from("/unused/config.json"), ctx)
    }

    fn result(generation: u64) -> OcrEvent {
        OcrEvent::Remote {
            generation,
            result: Ok(OcrText {
                text: "抓鬼 长寿村 12,34 鬼王 天宫 56,78".into(),
                elapsed_secs: 0.25,
            }),
        }
    }

    #[test]
    fn switching_windows_discards_inflight_results_and_previous_coordinates() {
        let mut app = app();
        app.set_window(Some(1));
        app.apply_ocr(&OcrText {
            text: "抓鬼 长寿村 12,34".into(),
            elapsed_secs: 0.1,
        });
        assert!(app.tasks["抓鬼"].location.is_some());
        app.busy = true;
        let old_generation = app.generation;
        app.set_window(Some(2));
        app.sender.send(result(old_generation)).unwrap();
        app.poll_events();
        assert!(!app.busy);
        assert!(app.tasks.values().all(|state| state.location.is_none()));
    }

    #[test]
    fn pausing_discards_inflight_results() {
        let mut app = app();
        app.running = true;
        app.busy = true;
        let generation = app.generation;
        app.toggle_running();
        app.sender.send(result(generation)).unwrap();
        app.poll_events();
        assert!(!app.running);
        assert!(!app.busy);
        assert!(app.tasks["抓鬼"].location.is_none());
    }

    #[test]
    fn remote_test_feedback_is_available_with_simple_logging() {
        let mut app = app();
        app.testing_remote = true;
        app.sender
            .send(OcrEvent::Test {
                result: Err("连接超时".into()),
            })
            .unwrap();
        app.poll_events();
        assert!(!app.testing_remote);
        assert_eq!(app.remote_feedback, Some(Err("连接超时".into())));
        assert_eq!(app.logs.len(), 1);
    }

    #[test]
    fn preview_updates_reuse_the_existing_texture() {
        let mut app = app();
        app.update_preview(&image::RgbImage::from_pixel(
            100,
            100,
            image::Rgb([0, 0, 0]),
        ));
        let texture_id = app.preview_texture.as_ref().unwrap().id();
        app.update_preview(&image::RgbImage::from_pixel(
            120,
            100,
            image::Rgb([255, 0, 0]),
        ));
        assert_eq!(app.preview_texture.as_ref().unwrap().id(), texture_id);
        assert_eq!(
            sample_image(app.preview_full.as_ref().unwrap(), 119, 99),
            Some([255, 0, 0])
        );
    }

    #[test]
    fn task_status_changes_do_not_move_or_resize_the_map() {
        fn map_bounds(shape: &egui::Shape, texture: egui::TextureId) -> Option<Rect> {
            match shape {
                egui::Shape::Mesh(mesh) if mesh.texture_id == texture => Some(mesh.calc_bounds()),
                egui::Shape::Vec(shapes) => {
                    shapes.iter().find_map(|shape| map_bounds(shape, texture))
                }
                _ => None,
            }
        }
        let mut app = app();
        let ctx = app.ctx.clone();
        let success = OcrText {
            text: "抓鬼 长寿村 12,34".into(),
            elapsed_secs: 0.1,
        };
        for width in [700.0, 320.0] {
            let mut expected = None;
            for message in [
                None,
                Some("等待识别"),
                Some("本轮未识别到任务"),
                None,
                Some("本轮识别失败，保留上次坐标"),
                None,
            ] {
                app.apply_ocr(&success);
                if message == Some("本轮未识别到任务") {
                    app.apply_ocr(&OcrText {
                        text: String::new(),
                        elapsed_secs: 0.1,
                    });
                } else {
                    app.tasks.get_mut("抓鬼").unwrap().status = message.map(str::to_string);
                }
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 400.0))),
                        ..Default::default()
                    },
                    |ui| app.task_panel(ui, "抓鬼"),
                );
                let texture = app.map_textures.values().next().unwrap().id();
                let bounds = output
                    .shapes
                    .iter()
                    .find_map(|shape| map_bounds(&shape.shape, texture))
                    .unwrap();
                assert!(app.tasks["抓鬼"].location.is_some());
                if let Some(expected) = expected {
                    assert_eq!(bounds, expected, "宽度：{width}，状态：{message:?}");
                }
                expected = Some(bounds);
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn renders_local_remote_and_map_layouts_at_supported_sizes() {
        let mut app = app();
        let ctx = app.ctx.clone();
        let mut frame = eframe::Frame::_new_kittest();
        for (size, remote, horizontal, sidebar, with_task) in [
            ([1120.0, 800.0], false, false, true, false),
            ([760.0, 540.0], true, false, true, false),
            ([1120.0, 800.0], false, true, true, true),
            ([760.0, 540.0], false, false, false, true),
        ] {
            app.config.ocr_mode = if remote { "remote" } else { "local" }.into();
            app.config.horizontal_layout = horizontal;
            app.config.sidebar_expanded = sidebar;
            if with_task {
                app.apply_ocr(&OcrText {
                    text: "抓鬼 长寿村 12,34 鬼王 天宫 56,78".into(),
                    elapsed_secs: 0.1,
                });
            }
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), Vec2::from(size))),
                    ..Default::default()
                },
                |ui| app.ui(ui, &mut frame),
            );
            assert!(!output.shapes.is_empty());
            assert!(output
                .shapes
                .iter()
                .all(|shape| shape.clip_rect.is_finite()));
            output.textures_delta.clear();
        }
        assert_eq!(app.map_textures.len(), 2);
    }
}
