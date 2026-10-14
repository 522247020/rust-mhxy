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
use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, Sense, TextureHandle, Vec2};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

const TASK_ORDER: [&str; 2] = ["抓鬼", "鬼王"];
/// 左侧栏底部日志框的固定高度（像素）。
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
    show_region_window: bool,
    pick_points: bool,
    gate_active: bool,
    last_gate_check: Instant,
    gate_result: Option<gate::GateResult>,
    testing_remote: bool,
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
        let worker = LocalWorker::spawn(sender.clone());
        let mut tasks = HashMap::new();
        for task in TASK_ORDER {
            tasks.insert(task, TaskState::default());
        }
        let _ = cc;
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
            show_region_window: false,
            pick_points: true,
            gate_active: true,
            last_gate_check: Instant::now(),
            gate_result: None,
            testing_remote: false,
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
        app.refresh_windows(true);
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

    fn persist(&self) {
        let _ = self.config.save(&self.config_path);
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
                self.selected_handle = still.or_else(|| self.windows.first().map(|window| window.handle));
                if self.selected_handle != previous {
                    match self.selected_handle {
                        Some(handle) => {
                            let title = self
                                .windows
                                .iter()
                                .find(|window| window.handle == handle)
                                .map(|window| window.title.clone())
                                .unwrap_or_default();
                            self.log_detail(format!("自动选择游戏窗口：{title}（可选 {count} 个）"));
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
                        self.last_scan = None;
                    } else {
                        self.log("任务停止（图色未命中）");
                    }
                }
            }
            Err(err) => {
                if self.gate_active {
                    self.log_detail(format!("图色采样失败：{err}"));
                }
                self.gate_active = false;
            }
        }
    }

    fn trigger_scan(&mut self) {
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
        let (panel, full) = match capture::capture_task_panel(handle, self.config.task_region, extra)
        {
            Ok(result) => result,
            Err(err) => {
                self.log_detail(format!("截图失败（{window_title}）：{err}"));
                self.mark_all_stale("本轮识别失败");
                if err.contains("关闭") || err.contains("最小化") {
                    self.selected_handle = None;
                }
                return;
            }
        };
        self.preview_full = Some(to_color_image(&full));

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
                    self.busy = false;
                    return;
                }
            };
            let url = self.config.api_url.clone();
            let token = self.config.api_token.clone();
            let format = self.config.api_format.clone();
            let sender = self.sender.clone();
            self.log_detail(format!("{window_title}：请求远端 PP-OCR {url}"));
            thread::spawn(move || {
                let result = ocr::remote_recognize(&url, &token, &format, &png);
                let _ = sender.send(OcrEvent::Remote { generation, result });
            });
        } else {
            self.log_detail(format!("{window_title}：本地 PP-OCR 识别中"));
            self.worker.submit(generation, panel);
        }
    }

    fn poll_events(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            let (generation, result) = match event {
                OcrEvent::Local { generation, result } => (generation, result),
                OcrEvent::Remote { generation, result } => (generation, result),
                OcrEvent::Test { result } => {
                    self.testing_remote = false;
                    match result {
                        Ok(text) => self.log_detail(format!(
                            "远端 PP-OCR 连接正常（返回 {} 字）",
                            text.text.chars().count()
                        )),
                        Err(err) => self.log_detail(format!("远端 PP-OCR 连接失败：{err}")),
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
                    self.mark_all_stale("本轮识别失败");
                }
                Ok(text) => {
                    self.last_signature = self.pending_signature.take();
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
            self.log_detail("远端 PP-OCR 测试失败：未填写 API 地址");
            return;
        }
        let image = image::RgbImage::from_pixel(16, 16, image::Rgb([255, 255, 255]));
        let png = match ocr::encode_png(&image) {
            Ok(png) => png,
            Err(err) => {
                self.log_detail(format!("远端 PP-OCR 测试失败：{err}"));
                return;
            }
        };
        let url = self.config.api_url.clone();
        let token = self.config.api_token.clone();
        let format = self.config.api_format.clone();
        let sender = self.sender.clone();
        self.testing_remote = true;
        self.log_detail(format!("正在测试远端 PP-OCR：{url}"));
        thread::spawn(move || {
            let result = ocr::remote_recognize(&url, &token, &format, &png);
            let _ = sender.send(OcrEvent::Test { result });
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
                                let index = self
                                    .maps
                                    .iter()
                                    .position(|map| map.name == definition.name);
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
                                if find_map_definition(&location.map_name, &self.maps, None).is_some()
                                {
                                    (
                                        None,
                                        Some(format!("本轮地图不属于{task_type}任务，忽略")),
                                    )
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

    fn map_texture(&mut self, ctx: &egui::Context, definition: &MapDefinition) -> Option<TextureHandle> {
        let key = definition.image.to_string_lossy().to_string();
        if let Some(handle) = self.map_textures.get(&key) {
            return Some(handle.clone());
        }
        let image = match crate::core::load_rgb_image(&definition.image) {
            Ok(image) => image,
            Err(err) => {
                if self.map_texture_failed.insert(definition.name.clone()) {
                    self.log_detail(format!(
                        "地图 {} 图片加载失败：{}（{err}）",
                        definition.name,
                        definition.image.display()
                    ));
                }
                return None;
            }
        };
        let rgba = image::DynamicImage::ImageRgb8(image).to_rgba8();
        let size = [rgba.width() as usize, rgba.height() as usize];
        let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
        let handle = ctx.load_texture(key.clone(), color, egui::TextureOptions::LINEAR);
        self.map_textures.insert(key, handle.clone());
        Some(handle)
    }

    fn render_map(&mut self, ui: &mut egui::Ui, definition: &MapDefinition, location: &TaskLocation) {
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
        painter.rect_filled(rect, CornerRadius::same(8), Color32::from_rgb(18, 20, 24));
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
            for (chance_x, chance_y) in
                high_chance_areas(&definition.name, location.x as f32, location.y as f32, HIGH_CHANCE_SIZE)
            {
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

        if let Some(point) = map_point(definition, point_x, point_y, texture_size.x, texture_size.y) {
            let center = to_screen(point);
            painter.circle_filled(center, 8.0, Color32::from_rgb(255, 51, 78));
            painter.circle_stroke(center, 8.0, egui::Stroke::new(2.0, Color32::WHITE));
            let label = format!("({}, {})", location.x, location.y);
            painter.text(
                center + vec2(10.0, -20.0),
                Align2::LEFT_TOP,
                label,
                FontId::proportional(14.0),
                Color32::WHITE,
            );
        }

        if let Some(pointer) = response.hover_pos() {
            if image_rect.contains(pointer) {
                let pixel_x = (pointer.x - image_rect.min.x) / scale;
                let pixel_y = (pointer.y - image_rect.min.y) / scale;
                if let Some((game_x, game_y)) = map_coordinate(
                    definition,
                    pixel_x,
                    pixel_y,
                    texture_size.x,
                    texture_size.y,
                ) {
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
        ui.add_space(8.0);
    }

    fn controls(&mut self, ui: &mut egui::Ui) {
        // 整栏一个竖向瀑布流：设置与日志顺序往下排，整体滚动，不会互相遮挡。
        egui::ScrollArea::vertical()
            .id_salt("sidebar")
            .auto_shrink([false, false])
            .show(ui, |ui| {
            ui.heading("抓鬼 / 鬼王");
            ui.label(egui::RichText::new("任务坐标辅助").weak());
            ui.add_space(8.0);
            ui.separator();

            ui.label(egui::RichText::new("游戏窗口").strong());
            ui.horizontal(|ui| {
                let current = self
                    .selected_handle
                    .and_then(|handle| {
                        self.windows
                            .iter()
                            .find(|window| window.handle == handle)
                            .map(|window| window.title.clone())
                    })
                    .unwrap_or_else(|| "请选择".to_string());
                egui::ComboBox::from_id_salt("window")
                    .selected_text(current)
                    .width(ui.available_width() - 70.0)
                    .show_ui(ui, |ui| {
                        for window in self.windows.clone() {
                            let selected = self.selected_handle == Some(window.handle);
                            if ui.selectable_label(selected, &window.title).clicked() {
                                self.selected_handle = Some(window.handle);
                                self.last_signature = None;
                                self.log_detail(format!("手动选择窗口：{}", window.title));
                            }
                        }
                    });
                if ui.button("刷新").clicked() {
                    self.refresh_windows(true);
                }
            });
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                let label = if self.running { "暂停识别" } else { "开始识别" };
                if ui.button(label).clicked() {
                    if self.running {
                        self.running = false;
                        self.busy = false;
                        self.generation = self.generation.wrapping_add(1);
                        self.log("已暂停识别");
                    } else {
                        if self.selected_handle.is_none() {
                            self.refresh_windows(false);
                        }
                        if self.selected_handle.is_none() {
                            self.log("未找到游戏窗口，无法开始");
                        } else {
                            self.running = true;
                            self.last_scan = None;
                            self.last_gate_check = Instant::now();
                            self.last_signature = None;
                            self.log("开始自动识别");
                        }
                    }
                }
                let label = if self.show_region_window {
                    "关闭识别区域"
                } else {
                    "显示识别区域"
                };
                if ui.button(label).clicked() {
                    self.show_region_window = !self.show_region_window;
                }
            });
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.label("识别间隔");
                let mut interval = self.config.interval_seconds;
                let response = ui.add(
                    egui::DragValue::new(&mut interval)
                        .speed(0.2)
                        .range(1..=60)
                        .suffix(" 秒"),
                );
                self.config.interval_seconds = interval;
                if response.changed() {
                    self.persist();
                }
            });
            ui.separator();

            ui.label(egui::RichText::new("识别方式").strong());
            let mut mode = self.config.ocr_mode.clone();
            let mut mode_changed = false;
            ui.horizontal(|ui| {
                mode_changed |= ui
                    .radio_value(&mut mode, "local".to_string(), "本地 PP-OCR")
                    .changed();
                mode_changed |= ui
                    .radio_value(&mut mode, "remote".to_string(), "远端 PP-OCR")
                    .changed();
            });
            if mode_changed {
                self.config.ocr_mode = mode;
                self.generation = self.generation.wrapping_add(1);
                self.busy = false;
                self.last_signature = None;
                self.persist();
                let label = if self.config.ocr_mode == "remote" {
                    "远端 PP-OCR"
                } else {
                    "本地 PP-OCR"
                };
                self.log_detail(format!("切换识图模式：{label}"));
            }
            if self.config.ocr_mode == "remote" {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("API");
                    if ui
                        .text_edit_singleline(&mut self.api_url_edit)
                        .lost_focus()
                    {
                        self.config.api_url = self.api_url_edit.trim().to_string();
                        self.persist();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Token");
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut self.api_token_edit).password(true),
                        )
                        .lost_focus()
                    {
                        self.config.api_token = self.api_token_edit.trim().to_string();
                        self.persist();
                    }
                });
                let mut format = self.config.api_format.clone();
                let mut format_changed = false;
                ui.horizontal(|ui| {
                    ui.label("请求格式");
                    format_changed |= ui
                        .radio_value(&mut format, "multipart".to_string(), "multipart")
                        .changed();
                    format_changed |= ui
                        .radio_value(&mut format, "raw".to_string(), "raw")
                        .changed();
                });
                if format_changed {
                    self.config.api_format = format;
                    self.persist();
                }
                ui.horizontal(|ui| {
                    let button = egui::Button::new(if self.testing_remote {
                        "测试中…"
                    } else {
                        "测试连接"
                    });
                    if ui.add_enabled(!self.testing_remote, button).clicked() {
                        self.test_remote_ocr();
                    }
                    ui.label(egui::RichText::new("multipart → /ocr，raw → /ocr/raw").weak());
                });
            } else {
                ui.label(
                    egui::RichText::new("本地模型首次使用时会自动下载（约 25MB）").weak(),
                );
            }
            ui.separator();

            ui.label(egui::RichText::new("任务").strong());
            if ui
                .checkbox(&mut self.config.ghost_king_enabled, "识别鬼王")
                .changed()
            {
                self.last_signature = None;
                self.persist();
            }
            if ui
                .checkbox(&mut self.config.high_chance, "显示捉鬼高概率区域")
                .changed()
            {
                self.persist();
            }
            let layout_label = if self.config.horizontal_layout {
                "布局：左右等分（点击切换上下）"
            } else {
                "布局：上下等分（点击切换左右）"
            };
            if ui.button(layout_label).clicked() {
                self.config.horizontal_layout = !self.config.horizontal_layout;
                self.persist();
            }
            ui.separator();
            self.log_ui(ui);
            ui.add_space(6.0);
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
        egui::Frame::group(ui.style()).show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("log_scroll")
                .stick_to_bottom(true)
                .max_height(LOG_PANEL_HEIGHT)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    if self.logs.is_empty() {
                        ui.label(egui::RichText::new("暂无日志").weak());
                        return;
                    }
                    for line in &self.logs {
                        ui.label(egui::RichText::new(line).monospace().size(11.0));
                    }
                });
        });
    }

    /// 画客户区预览：红框主区域、蓝框备用区域、图色点位；点击时返回 (x, y, 颜色)。
    fn preview(&self, ui: &mut egui::Ui) -> Option<(i32, i32, [u8; 3])> {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("识别区域预览").strong());
            ui.label(egui::RichText::new("红框=主区域，蓝框=备用区域；点击图片可取点").weak());
        });
        let image = self.preview_full.as_ref()?;
        let size = Vec2::new(image.width() as f32, image.height() as f32);
        let available = ui.available_size();
        let scale = (available.x / size.x).min(available.y / size.y).max(0.05);
        let (rect, response) = ui.allocate_exact_size(size * scale, Sense::click());
        let painter = ui.painter_at(rect);
        let target = fit_rect(rect, size);
        let texture = ui
            .ctx()
            .load_texture("preview", image.clone(), egui::TextureOptions::LINEAR);
        painter.image(
            texture.id(),
            target,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        let draw_scale = target.width() / size.x;
        let to_screen = |x: f32, y: f32| {
            pos2(target.min.x + x * draw_scale, target.min.y + y * draw_scale)
        };
        for (region, color) in [
            (self.config.task_region, Color32::from_rgb(255, 50, 50)),
            (self.config.extra_task_region, Color32::from_rgb(60, 150, 255)),
        ] {
            if region == self.config.extra_task_region && !self.config.extra_region_enabled {
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
        // 子窗口的根 Ui 默认没有背景，这里铺一层不透明的面板底色。
        egui::Frame::central_panel(ui.style()).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.horizontal(|ui| {
                if ui.button("手动截图一次").clicked() {
                    self.capture_preview_once();
                }
                ui.label(egui::RichText::new("只截屏更新预览，不做 OCR").weak());
                ui.separator();
                ui.checkbox(&mut self.pick_points, "点击预览取点");
            });
            ui.separator();
            let mut changed = region_row(ui, "主区域", &mut self.config.task_region);
            let mut extra_enabled = self.config.extra_region_enabled;
            ui.horizontal(|ui| {
                if ui.checkbox(&mut extra_enabled, "启用备用区域").changed() {
                    self.config.extra_region_enabled = extra_enabled;
                    changed = true;
                }
            });
            if self.config.extra_region_enabled {
                changed |= region_row(ui, "备用", &mut self.config.extra_task_region);
            }
            if changed {
                self.last_signature = None;
                self.persist();
                self.log_detail("已更新识别区域设置");
            }
            ui.label(egui::RichText::new("单位 %，相对游戏客户区；改动下一轮识别生效").weak());
            ui.separator();
            self.color_gate_ui(ui);
            ui.separator();
            if let Some((x, y, color)) = self.preview(ui) {
                let label = format!("点{}", self.config.color_gate_points.len() + 1);
                let mut point =
                    ColorPoint::new(x, y, color, self.config.color_gate_tolerance, 1);
                point.label = label;
                self.config.color_gate_points.push(point);
                self.persist();
                self.log_detail(format!("新增图色点位 ({x}, {y}) {}", gate::to_hex(color)));
            }
        });
    }

    fn color_gate_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("图色门控").strong());
            if ui
                .checkbox(&mut self.config.color_gate_enabled, "启用")
                .changed()
            {
                self.last_gate_check = Instant::now();
                self.persist();
            }
            ui.label("间隔");
            let mut interval = self.config.color_gate_interval_ms;
            if ui
                .add(egui::DragValue::new(&mut interval).range(50..=2000).suffix(" ms"))
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
            ui.horizontal(|ui| {
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
            self.log_detail("手动截图失败：未找到游戏窗口");
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
                self.preview_full = Some(to_color_image(&full));
                self.log_detail(format!("手动截取截图：{title}"));
                // 立即刷新一次图色门控状态，方便在取点工具里看命中情况。
                self.check_gate();
            }
            Err(err) => {
                self.log_detail(format!("手动截图失败：{err}"));
                if err.contains("关闭") || err.contains("最小化") {
                    self.selected_handle = None;
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
        ui.horizontal(|ui| {
            ui.heading(task_type);
            if let Some(location) = &location {
                ui.label(format!(
                    "{}  ({}, {})",
                    location.map_name, location.x, location.y
                ));
            } else {
                ui.label(egui::RichText::new("等待识别").weak());
            }
            if let Some(captured) = captured {
                ui.label(
                    egui::RichText::new(format!("{} 秒前", captured.elapsed().as_secs())).weak(),
                );
            }
        });
        if let Some(status) = &status {
            ui.label(egui::RichText::new(status).weak());
        }
        match (location, map_index) {
            (Some(location), Some(index)) => {
                let definition = self.maps[index].clone();
                self.render_map(ui, &definition, &location);
            }
            _ => {
                let (rect, _) =
                    ui.allocate_exact_size(ui.available_size(), Sense::hover());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, CornerRadius::same(8), Color32::from_rgb(28, 31, 38));
                painter.text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    status.as_deref().unwrap_or("等待识别"),
                    FontId::proportional(15.0),
                    Color32::from_gray(150),
                );
            }
        }
    }
}

impl eframe::App for MhxyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_events();
        // 定时重新扫描游戏窗口（游戏可能后开 / 关掉）。
        if (self.running || self.selected_handle.is_none())
            && self.last_window_refresh.elapsed() >= Duration::from_secs(3)
        {
            self.refresh_windows(false);
            self.last_window_refresh = Instant::now();
        }
        if self.running && self.config.color_gate_enabled {
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
            let due = self.last_scan.map_or(true, |last| last.elapsed() >= interval);
            if due {
                self.trigger_scan();
            }
        }
        if self.running || self.busy {
            ctx.request_repaint_after(Duration::from_millis(150));
        }

        egui::Panel::top("header").show(ui, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let label = if self.config.sidebar_expanded {
                    "◀ 隐藏侧栏"
                } else {
                    "▶ 显示侧栏"
                };
                if ui.button(label).clicked() {
                    self.config.sidebar_expanded = !self.config.sidebar_expanded;
                    self.persist();
                }
                ui.heading("梦幻西游任务坐标辅助");
                ui.label(egui::RichText::new("Rust + egui").weak());
            });
            ui.add_space(4.0);
        });
        if self.config.sidebar_expanded {
            egui::Panel::left("controls")
                .resizable(true)
                .default_size(320.0)
                .min_size(260.0)
                .max_size(460.0)
                .show(ui, |ui| self.controls(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| {
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
                let mut child = ui.new_child(
                    egui::UiBuilder::new().max_rect(child_rect.shrink(4.0)),
                );
                self.task_panel(&mut child, task_type);
            }
        });
        if self.show_region_window {
            let builder = egui::ViewportBuilder::default()
                .with_title("识别区域")
                .with_inner_size([900.0, 640.0])
                .with_min_inner_size([600.0, 420.0]);
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("region_window"),
                builder,
                |ui, _class| {
                    self.region_window_ui(ui);
                },
            );
        }
    }
}

fn region_row(ui: &mut egui::Ui, label: &str, region: &mut crate::core::TaskRegion) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        let mut values = [region.0 * 100.0, region.1 * 100.0, region.2 * 100.0, region.3 * 100.0];
        let mut edited = false;
        for value in values.iter_mut() {
            let response = ui.add(
                egui::DragValue::new(value)
                    .speed(0.2)
                    .range(0.0..=100.0)
                    .suffix("%"),
            );
            edited |= response.changed();
        }
        if edited {
            *region = (values[0] / 100.0, values[1] / 100.0, values[2] / 100.0, values[3] / 100.0);
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
    let size = [image.width() as usize, image.height() as usize];
    let mut rgba = Vec::with_capacity(size[0] * size[1] * 4);
    for pixel in image.pixels() {
        rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
    }
    egui::ColorImage::from_rgba_unmultiplied(size, &rgba)
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
