//! egui 界面与识别调度。只保留「抓鬼 / 鬼王」任务。
use crate::capture::{self, WindowInfo};
use crate::config::{assets_dir, Config};
use crate::core::{
    clamp_to_map, coordinate_in_map, find_map_definition, high_chance_areas, map_area,
    map_coordinate, map_point, map_rect, parse_tasks, MapDefinition, TaskFilter, TaskLocation,
    HIGH_CHANCE_SIZE, MAP_COORDINATE_TOLERANCE,
};
use crate::gate::{self, ColorPoint, GateMode};
use crate::hotkey::{self, GlobalHotkey, Hotkey};
use crate::ocr::{self, LocalWorker, OcrEvent, OcrText};
use crate::preprocess;
use crate::theme;
use crate::update::{self, UpdateInfo};
use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, Sense, TextureHandle, Vec2};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

const TASK_ORDER: [&str; 3] = ["抓鬼", "鬼王", "通用"];
/// 「附近」范围框的半径（游戏坐标），所以框的边长是 2 倍。
const NEARBY_RADIUS: f32 = 50.0;
/// 日志框的最小高度；实际高度随窗口高度变化。
const LOG_PANEL_MIN_HEIGHT: f32 = 140.0;
/// 日志框占窗口可用高度的比例。
const LOG_PANEL_HEIGHT_RATIO: f32 = 0.34;
/// 侧栏展开 / 收起时的最小窗口尺寸。
const MIN_WINDOW_SIZE_EXPANDED: Vec2 = Vec2::new(760.0, 540.0);
const MIN_WINDOW_SIZE_COLLAPSED: Vec2 = Vec2::new(460.0, 460.0);
/// 定时检查更新的间隔。
const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(30 * 60);
/// 说明 / 更新窗口的默认尺寸。
const HELP_WINDOW_SIZE: Vec2 = Vec2::new(560.0, 640.0);
const UPDATE_WINDOW_SIZE: Vec2 = Vec2::new(480.0, 540.0);
/// 检查结果在按钮上停留的时间。
const UPDATE_FLASH_DURATION: Duration = Duration::from_secs(4);

/// 自动更新的进度阶段。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdatePhase {
    Idle,
    Checking,
    Available,
    Downloading,
    Downloaded,
    Failed,
}

enum UpdateEvent {
    Checked(Result<Option<UpdateInfo>, String>),
    Downloaded(Result<PathBuf, String>),
}

#[derive(Default)]
struct TaskState {
    location: Option<TaskLocation>,
    map_index: Option<usize>,
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
    preview_image: Option<image::RgbImage>,
    preview_texture: Option<TextureHandle>,
    /// 预处理后的任务区域缩略图（只在取点窗口打开时存在）。
    preprocess_preview: Option<TextureHandle>,
    preprocess_preview_extra: Option<TextureHandle>,
    /// 预处理设置当前编辑主区域（false）还是备用区域（true）。
    preprocess_target_extra: bool,
    client_size: Option<(u32, u32)>,
    region_window_pos: Option<egui::Pos2>,
    show_region_window: bool,
    pick_points: bool,
    /// 开启后点击预览取的是“文字颜色”而不是图色点位。
    pick_text_color: bool,
    /// 取点窗口「测试 OCR」的状态与结果。
    ocr_test_running: bool,
    ocr_test_result: Option<Result<String, String>>,
    gate_active: bool,
    last_gate_check: Instant,
    gate_result: Option<gate::GateResult>,
    testing_remote: bool,
    applied_min_inner_size: Option<Vec2>,
    applied_on_top: Option<bool>,
    /// 取点窗口自己的置顶状态。子视口默认不会被父窗口的置顶带上，
    /// 不同步的话主窗口置顶时会把取点窗口整个压住。
    applied_region_on_top: Option<bool>,
    remote_feedback: Option<Result<String, String>>,
    notice: Option<String>,
    show_help: bool,
    last_ocr_secs: Option<f64>,
    config_dirty_since: Option<Instant>,
    save_error: Option<String>,
    ctx: egui::Context,
    last_window_refresh: Instant,
    api_url_edit: String,
    api_token_edit: String,
    /// 预处理文字颜色的十六进制输入框。
    preprocess_color_edit: String,
    extra_preprocess_color_edit: String,
    update_tx: Sender<UpdateEvent>,
    update_rx: Receiver<UpdateEvent>,
    update_phase: UpdatePhase,
    update_info: Option<UpdateInfo>,
    update_progress: Arc<AtomicU64>,
    update_cancel: Arc<AtomicBool>,
    update_zip: Option<PathBuf>,
    update_message: Option<String>,
    update_message_at: Option<Instant>,
    update_error: Option<String>,
    last_update_check: Option<Instant>,
    show_update_dialog: bool,
    profile_saving: bool,
    profile_name_edit: String,
    help_window_pos: Option<egui::Pos2>,
    update_window_pos: Option<egui::Pos2>,
    applied_help_on_top: Option<bool>,
    applied_update_on_top: Option<bool>,
    started_at: Instant,
    worker: LocalWorker,
    receiver: Receiver<OcrEvent>,
    sender: Sender<OcrEvent>,
    hotkey_tx: Sender<()>,
    hotkey_rx: Receiver<()>,
    hotkey_listener: Option<GlobalHotkey>,
    hotkey_attempted: Option<String>,
    recording_hotkey: bool,
    last_manual_trigger: Option<Instant>,
    logs: Vec<String>,
    /// 精简模式：只留地图，隐藏顶部按钮栏、状态栏和侧栏；按 Esc 退出。
    compact: bool,
    /// 当前实际应用的亮色 / 暗色，用于跟随系统主题时即时切换。
    applied_light: Option<bool>,
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
        let (hotkey_tx, hotkey_rx) = channel::<()>();
        let (update_tx, update_rx) = channel::<UpdateEvent>();
        let mut tasks = HashMap::new();
        for task in TASK_ORDER {
            tasks.insert(task, TaskState::default());
        }
        let mut app = Self {
            api_url_edit: config.api_url.clone(),
            api_token_edit: config.api_token.clone(),
            preprocess_color_edit: gate::to_hex(config.preprocess_color),
            extra_preprocess_color_edit: gate::to_hex(config.extra_preprocess_color),
            update_tx: update_tx.clone(),
            update_rx,
            update_phase: UpdatePhase::Idle,
            update_info: None,
            update_progress: Arc::new(AtomicU64::new(0)),
            update_cancel: Arc::new(AtomicBool::new(false)),
            update_zip: None,
            update_message: None,
            update_message_at: None,
            update_error: None,
            last_update_check: None,
            show_update_dialog: false,
            profile_saving: false,
            profile_name_edit: String::new(),
            help_window_pos: None,
            update_window_pos: None,
            applied_help_on_top: None,
            applied_update_on_top: None,
            started_at: Instant::now(),
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
            preview_image: None,
            preview_texture: None,
            preprocess_preview: None,
            preprocess_preview_extra: None,
            preprocess_target_extra: false,
            client_size: None,
            region_window_pos: None,
            show_region_window: false,
            pick_points: true,
            pick_text_color: false,
            ocr_test_running: false,
            ocr_test_result: None,
            gate_active: true,
            last_gate_check: Instant::now(),
            gate_result: None,
            testing_remote: false,
            applied_min_inner_size: None,
            applied_on_top: None,
            applied_region_on_top: None,
            remote_feedback: None,
            notice: None,
            show_help: false,
            last_ocr_secs: None,
            config_dirty_since: None,
            save_error: None,
            ctx,
            last_window_refresh: Instant::now(),
            worker,
            receiver,
            sender,
            hotkey_tx,
            hotkey_rx,
            hotkey_listener: None,
            hotkey_attempted: None,
            recording_hotkey: false,
            last_manual_trigger: None,
            logs: Vec::new(),
            compact: false,
            applied_light: None,
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

    // ---- 自动更新 ----

    fn start_update_check(&mut self, manual: bool) {
        if matches!(
            self.update_phase,
            UpdatePhase::Checking | UpdatePhase::Downloading
        ) {
            return;
        }
        self.last_update_check = Some(Instant::now());
        self.update_error = None;
        if manual {
            self.update_message = None;
        }
        if !self.config.update_enabled && !manual {
            return;
        }
        if self.config.update_manifest_url.trim().is_empty() {
            if manual {
                self.update_phase = UpdatePhase::Failed;
                self.update_error =
                    Some("更新地址为空，请在 config.json 设置 update/manifest_url".into());
            }
            return;
        }
        self.update_phase = UpdatePhase::Checking;
        let url = self.config.update_manifest_url.clone();
        let sender = self.update_tx.clone();
        let ctx = self.ctx.clone();
        thread::spawn(move || {
            let result = update::check(&url, crate::APP_VERSION);
            let _ = sender.send(UpdateEvent::Checked(result));
            ctx.request_repaint();
        });
    }

    fn start_update_download(&mut self) {
        let Some(info) = self.update_info.clone() else {
            return;
        };
        self.update_phase = UpdatePhase::Downloading;
        self.update_error = None;
        self.update_progress.store(0, Ordering::Relaxed);
        self.update_cancel.store(false, Ordering::Relaxed);
        let destination = update::temp_package(&info.version);
        let sender = self.update_tx.clone();
        let ctx = self.ctx.clone();
        let progress = self.update_progress.clone();
        let cancel = self.update_cancel.clone();
        thread::spawn(move || {
            let result =
                update::download_to(&info.url, &destination, &info.sha256, &progress, &cancel)
                    .map(|()| destination);
            let _ = sender.send(UpdateEvent::Downloaded(result));
            ctx.request_repaint();
        });
    }

    fn update_dialog_ui(&mut self, ui: &mut egui::Ui) {
        let Some(info) = self.update_info.clone() else {
            ui.label("暂时没有可用的更新信息。");
            return;
        };
        ui.label(
            egui::RichText::new(format!(
                "当前 {} → 最新 {}",
                crate::APP_VERSION,
                info.version
            ))
            .strong(),
        );
        ui.label(
            egui::RichText::new(format!("安装包 {:.1} MB", info.size as f64 / 1_048_576.0))
                .small()
                .weak(),
        );
        ui.add_space(6.0);
        ui.label(egui::RichText::new("更新内容").strong());
        egui::Frame::new()
            .fill(theme::inset())
            .corner_radius(6)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("update_notes")
                    .max_height(200.0)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let notes = info.notes.trim();
                        if notes.is_empty() {
                            ui.label("本次更新以修复和优化为主。");
                        } else {
                            for line in notes.lines() {
                                let line = line.trim();
                                if !line.is_empty() {
                                    ui.label(format!("• {line}"));
                                }
                            }
                        }
                    });
            });
        if let Some(err) = &self.update_error {
            ui.label(egui::RichText::new(err).small().color(theme::red()));
        }
        ui.label(
            egui::RichText::new(format!("清单：{}", self.config.update_manifest_url))
                .small()
                .weak(),
        );
        ui.add_space(6.0);
        match self.update_phase {
            UpdatePhase::Downloading => {
                let total = info.size;
                let downloaded = self.update_progress.load(Ordering::Relaxed);
                let ratio = if total > 0 {
                    (downloaded as f32 / total as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                ui.add(egui::ProgressBar::new(ratio).text(format!(
                    "下载中 {:.0}% · {:.1}/{:.1} MB",
                    ratio * 100.0,
                    downloaded as f64 / 1_048_576.0,
                    total as f64 / 1_048_576.0
                )));
                if ui.button("取消下载").clicked() {
                    self.update_cancel.store(true, Ordering::Relaxed);
                }
            }
            UpdatePhase::Downloaded => {
                ui.horizontal(|ui| {
                    if ui.add(theme::primary_button("立即重启并更新")).clicked() {
                        self.install_update();
                    }
                    if ui.button("稍后").clicked() {
                        self.show_update_dialog = false;
                    }
                });
            }
            _ => {
                ui.horizontal(|ui| {
                    if ui.add(theme::primary_button("下载并更新")).clicked() {
                        self.start_update_download();
                    }
                    if ui.button("稍后").clicked() {
                        self.show_update_dialog = false;
                    }
                    ui.hyperlink_to(egui::RichText::new("打开发布页").small(), update::HOME_URL);
                });
            }
        }
    }

    fn install_update(&mut self) {
        let Some(zip) = self.update_zip.clone() else {
            return;
        };
        self.show_update_dialog = false;
        self.flush_config();
        match update::apply_and_restart(&zip) {
            Ok(()) => {
                self.notice = Some("正在重启以完成更新…".to_string());
                self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Err(err) => {
                self.update_phase = UpdatePhase::Failed;
                self.update_error = Some(err);
            }
        }
    }

    fn poll_updates(&mut self) {
        while let Ok(event) = self.update_rx.try_recv() {
            match event {
                UpdateEvent::Checked(Ok(Some(info))) => {
                    self.update_message = Some(format!("发现新版本 {}", info.version));
                    self.update_message_at = Some(Instant::now());
                    self.log(format!(
                        "发现新版本 {}，可在右上角「检查更新」按钮更新",
                        info.version
                    ));
                    self.update_info = Some(info);
                    self.update_phase = UpdatePhase::Available;
                    self.update_error = None;
                    if self
                        .notice
                        .as_deref()
                        .is_some_and(|notice| notice.starts_with("检查更新失败"))
                    {
                        self.notice = None;
                    }
                }
                UpdateEvent::Checked(Ok(None)) => {
                    self.update_message = Some("已是最新版本".to_string());
                    self.update_message_at = Some(Instant::now());
                    self.log_detail(format!("已是最新版本（{}）", crate::APP_VERSION));
                    self.update_info = None;
                    self.update_phase = UpdatePhase::Idle;
                    self.update_error = None;
                    if self
                        .notice
                        .as_deref()
                        .is_some_and(|notice| notice.starts_with("检查更新失败"))
                    {
                        self.notice = None;
                    }
                }
                UpdateEvent::Checked(Err(err)) => {
                    self.update_phase = UpdatePhase::Failed;
                    self.update_message = Some("检查失败".to_string());
                    self.update_message_at = Some(Instant::now());
                    let url = self.config.update_manifest_url.clone();
                    let message = format!("检查更新失败：{err}（{url}）");
                    self.log_detail(message.clone());
                    self.notice = Some(message);
                    self.update_error = Some(err);
                }
                UpdateEvent::Downloaded(Ok(path)) => {
                    self.update_zip = Some(path);
                    self.update_phase = UpdatePhase::Downloaded;
                    self.update_message = Some("安装包已就绪".to_string());
                    self.update_message_at = Some(Instant::now());
                    self.update_error = None;
                }
                UpdateEvent::Downloaded(Err(err)) => {
                    self.update_phase = UpdatePhase::Failed;
                    self.update_error = Some(err);
                }
            }
        }
    }

    fn sidebar_section_open(&self, id: &str) -> bool {
        !self
            .config
            .sidebar_folded
            .split(',')
            .any(|item| item.trim() == id)
    }

    fn set_sidebar_section_open(&mut self, id: &str, open: bool) {
        let mut folded: Vec<String> = self
            .config
            .sidebar_folded
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_string)
            .collect();
        if open {
            folded.retain(|item| item != id);
        } else if !folded.iter().any(|item| item == id) {
            folded.push(id.to_string());
        }
        self.config.sidebar_folded = folded.join(",");
        self.persist();
    }

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
            self.preview_image = None;
            self.preview_texture = None;
            self.client_size = None;
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
            self.hotkey_listener = None;
            self.hotkey_attempted = None;
            self.log(if self.manual_mode() {
                "已停止监听快捷键"
            } else {
                "已暂停识别"
            });
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
            self.sync_hotkey();
            self.log(if self.manual_mode() {
                "开始监听快捷键"
            } else {
                "开始自动识别"
            });
        }
    }

    /// 记录客户区尺寸；仅在打开识别区域窗口时才保留整张截图，避免常驻内存。
    fn update_preview(&mut self, image: image::RgbImage) {
        self.client_size = Some(image.dimensions());
        if !self.show_region_window {
            return;
        }
        let color = to_color_image(&image);
        if let Some(texture) = &mut self.preview_texture {
            texture.set(color, egui::TextureOptions::LINEAR);
        } else {
            self.preview_texture = Some(self.ctx.load_texture(
                "client_preview",
                color,
                egui::TextureOptions::LINEAR,
            ));
        }
        self.preview_image = Some(image);
        self.refresh_preprocess_preview();
    }

    /// 关闭取点工具后释放截图和纹理。
    fn release_preview(&mut self) {
        self.preview_image = None;
        self.preview_texture = None;
        self.preprocess_preview = None;
        self.preprocess_preview_extra = None;
    }

    /// 用当前截图和预处理设置生成小预览，方便在取点窗口里确认效果。
    fn refresh_preprocess_preview(&mut self) {
        let Some(full) = self.preview_image.as_ref() else {
            self.preprocess_preview = None;
            self.preprocess_preview_extra = None;
            return;
        };
        let main = capture::crop_task_region(full, self.config.task_region);
        let main = preprocess::apply(
            main,
            self.preprocess_pipeline_for(false),
            self.config.preprocess_color,
            self.config.preprocess_tolerance,
        );
        let main_color = to_color_image(&main);
        if let Some(texture) = &mut self.preprocess_preview {
            texture.set(main_color, egui::TextureOptions::LINEAR);
        } else {
            self.preprocess_preview = Some(self.ctx.load_texture(
                "preprocess_preview_main",
                main_color,
                egui::TextureOptions::LINEAR,
            ));
        }

        if self.config.extra_region_enabled {
            let extra = capture::crop_task_region(full, self.config.extra_task_region);
            let extra = preprocess::apply(
                extra,
                self.preprocess_pipeline_for(true),
                self.config.extra_preprocess_color,
                self.config.extra_preprocess_tolerance,
            );
            let extra_color = to_color_image(&extra);
            if let Some(texture) = &mut self.preprocess_preview_extra {
                texture.set(extra_color, egui::TextureOptions::LINEAR);
            } else {
                self.preprocess_preview_extra = Some(self.ctx.load_texture(
                    "preprocess_preview_extra",
                    extra_color,
                    egui::TextureOptions::LINEAR,
                ));
            }
        } else {
            self.preprocess_preview_extra = None;
        }
    }

    fn profile_dirty(&self) -> bool {
        let current = Config::profile_from_current(&self.config);
        self.config
            .profiles
            .iter()
            .find(|profile| profile.name == self.config.active_profile)
            .is_none_or(|profile| *profile != current)
    }

    fn apply_profile(&mut self, name: &str) {
        let Some(profile) = self
            .config
            .profiles
            .iter()
            .find(|profile| profile.name == name)
            .cloned()
        else {
            return;
        };
        self.config.active_profile = profile.name.clone();
        self.config.color_gate_enabled = profile.gate_enabled;
        self.config.color_gate_interval_ms = profile.gate_interval_ms;
        self.config.color_gate_mode = profile.gate_mode;
        self.config.color_gate_tolerance = profile.gate_tolerance;
        self.config.color_gate_points = profile.gate_points;
        self.config.preprocess_mode = profile.preprocess_mode;
        self.config.preprocess_color = profile.preprocess_color;
        self.config.preprocess_tolerance = profile.preprocess_tolerance;
        self.config.extra_preprocess_mode = profile.extra_preprocess_mode;
        self.config.extra_preprocess_color = profile.extra_preprocess_color;
        self.config.extra_preprocess_tolerance = profile.extra_preprocess_tolerance;
        self.preprocess_color_edit = gate::to_hex(self.config.preprocess_color);
        self.extra_preprocess_color_edit = gate::to_hex(self.config.extra_preprocess_color);
        self.invalidate_scan();
        self.check_gate();
        self.refresh_preprocess_preview();
        self.persist();
        self.log_detail(format!("切换识别方案：{name}"));
    }

    fn save_active_profile(&mut self) {
        let current = Config::profile_from_current(&self.config);
        if let Some(profile) = self
            .config
            .profiles
            .iter_mut()
            .find(|profile| profile.name == self.config.active_profile)
        {
            *profile = current;
            self.persist();
            self.notice = Some(format!("已保存方案「{}」", self.config.active_profile));
        }
    }

    fn create_profile(&mut self) {
        let mut name = self.profile_name_edit.trim().to_string();
        if name.is_empty() {
            name = format!("方案 {}", self.config.profiles.len() + 1);
        }
        let mut unique = name.clone();
        let mut index = 2;
        while self
            .config
            .profiles
            .iter()
            .any(|profile| profile.name == unique)
        {
            unique = format!("{name} {index}");
            index += 1;
        }
        self.config.active_profile = unique.clone();
        let profile = Config::profile_from_current(&self.config);
        self.config.profiles.push(profile);
        self.profile_saving = false;
        self.persist();
        self.notice = Some(format!("已新建方案「{unique}」"));
    }

    fn delete_active_profile(&mut self) {
        if self.config.profiles.len() <= 1 {
            return;
        }
        let name = self.config.active_profile.clone();
        self.config.profiles.retain(|profile| profile.name != name);
        if let Some(first) = self
            .config
            .profiles
            .first()
            .map(|profile| profile.name.clone())
        {
            self.apply_profile(&first);
        }
        self.notice = Some(format!("已删除方案「{name}」"));
    }

    fn profile_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new("识别方案").strong());
            let dirty = self.profile_dirty();
            let label = if dirty {
                format!("{} *", self.config.active_profile)
            } else {
                self.config.active_profile.clone()
            };
            let mut chosen: Option<String> = None;
            egui::ComboBox::from_id_salt("profile_select")
                .selected_text(label)
                .width(150.0)
                .show_ui(ui, |ui| {
                    for profile in &self.config.profiles {
                        if ui
                            .selectable_label(
                                profile.name == self.config.active_profile,
                                &profile.name,
                            )
                            .clicked()
                        {
                            chosen = Some(profile.name.clone());
                        }
                    }
                });
            if let Some(name) = chosen {
                self.apply_profile(&name);
            }
            if ui
                .small_button("保存")
                .on_hover_text("把当前的图色门控和预处理设置保存到当前方案")
                .clicked()
            {
                self.save_active_profile();
            }
            if ui
                .small_button("另存为")
                .on_hover_text("用当前设置新建一个方案")
                .clicked()
            {
                self.profile_saving = true;
                self.profile_name_edit = format!("方案 {}", self.config.profiles.len() + 1);
            }
            if ui
                .add_enabled(
                    self.config.profiles.len() > 1,
                    egui::Button::new("删除").small(),
                )
                .clicked()
            {
                self.delete_active_profile();
            }
            if dirty {
                ui.label(
                    egui::RichText::new("有未保存的修改")
                        .small()
                        .color(theme::gold()),
                );
            }
        });
        if self.profile_saving {
            ui.horizontal_wrapped(|ui| {
                ui.label("方案名称");
                ui.add(
                    egui::TextEdit::singleline(&mut self.profile_name_edit).desired_width(140.0),
                );
                if ui.button("确定").clicked() {
                    self.create_profile();
                }
                if ui.button("取消").clicked() {
                    self.profile_saving = false;
                }
            });
        }
    }

    fn set_on_top(&mut self, on: bool, ctx: &egui::Context) {
        self.config.on_top = on;
        self.applied_on_top = Some(on);
        apply_window_level(ctx, on);
        self.persist();
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
        match task_type {
            "抓鬼" => self.config.zhuagui_enabled,
            "鬼王" => self.config.ghost_king_enabled,
            "通用" => self.config.generic_enabled,
            _ => true,
        }
    }

    fn task_filter(&self) -> TaskFilter {
        TaskFilter {
            zhuagui: self.config.zhuagui_enabled,
            ghost_king: self.config.ghost_king_enabled,
            generic: self.config.generic_enabled,
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
        if let Some((client_w, client_h)) = self.client_size {
            let client_w = client_w as i32;
            let client_h = client_h as i32;
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

    fn manual_mode(&self) -> bool {
        self.config.trigger_mode == "manual"
    }

    /// 快捷键 / 「识别一次」触发：手动模式下一次只识别一次，忽略图色门控。
    fn manual_trigger(&mut self) {
        if !self.running || !self.manual_mode() || self.busy {
            return;
        }
        if let Some(last) = self.last_manual_trigger {
            if last.elapsed() < Duration::from_millis(400) {
                return;
            }
        }
        self.last_manual_trigger = Some(Instant::now());
        self.log_detail("手动触发识别");
        self.trigger_scan(true);
    }

    fn poll_hotkey(&mut self) {
        while self.hotkey_rx.try_recv().is_ok() {
            self.manual_trigger();
        }
    }

    /// 按当前运行状态 / 模式 / 快捷键字符串同步全局快捷键注册。
    fn sync_hotkey(&mut self) {
        let desired = if self.running && self.manual_mode() {
            Hotkey::parse(&self.config.ocr_hotkey)
                .ok()
                .map(|hotkey| (hotkey, hotkey.display()))
        } else {
            None
        };
        let desired_label = desired.as_ref().map(|(_, label)| label.clone());
        if desired_label == self.hotkey_attempted {
            return;
        }
        self.hotkey_attempted = desired_label;
        self.hotkey_listener = None;
        let Some((hotkey, label)) = desired else {
            return;
        };
        match hotkey::spawn(hotkey, self.hotkey_tx.clone()) {
            Ok(listener) => {
                self.hotkey_listener = Some(listener);
                self.log_detail(format!("已注册全局快捷键：{label}"));
            }
            Err(err) => {
                self.hotkey_listener = None;
                self.log_detail(format!("{err}，窗口聚焦时仍可使用 {label}"));
            }
        }
    }

    fn trigger_scan(&mut self, force: bool) {
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
        if !force && self.config.color_gate_enabled && !self.gate_active {
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
        self.last_scan = Some(Instant::now());
        let full = match capture::capture_client(handle, self.config.task_region) {
            Ok(full) => full,
            Err(err) => {
                self.log_detail(format!("截图失败（{window_title}）：{err}"));
                self.notice = Some(err.clone());
                if err.contains("关闭") || err.contains("最小化") {
                    self.set_window(None);
                }
                return;
            }
        };
        // 主区域和备用区域分别预处理，再上下拼接送一次 OCR。
        let mut panels = vec![preprocess::apply(
            capture::crop_task_region(&full, self.config.task_region),
            self.preprocess_pipeline_for(false),
            self.config.preprocess_color,
            self.config.preprocess_tolerance,
        )];
        if self.config.extra_region_enabled {
            panels.push(preprocess::apply(
                capture::crop_task_region(&full, self.config.extra_task_region),
                self.preprocess_pipeline_for(true),
                self.config.extra_preprocess_color,
                self.config.extra_preprocess_tolerance,
            ));
        }
        let panel = capture::stack_images(&panels);
        self.update_preview(full);
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
                OcrEvent::PanelTest { result } => {
                    self.ocr_test_running = false;
                    self.ocr_test_result = Some(match result {
                        Ok(text) => Ok(self.format_test_result(&text)),
                        Err(err) => Err(err),
                    });
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

    /// 取点窗口的「测试 OCR」：用当前截图与预处理设置识别一次，结果只显示在窗口里。
    fn run_panel_test(&mut self) {
        if self.ocr_test_running {
            return;
        }
        if self.preview_image.is_none() {
            self.capture_preview_once();
        }
        let Some(full) = self.preview_image.as_ref() else {
            self.ocr_test_result = Some(Err("暂无截图：请打开游戏窗口后点击「更新截图」".into()));
            return;
        };
        let mut panels = vec![preprocess::apply(
            capture::crop_task_region(full, self.config.task_region),
            self.preprocess_pipeline_for(false),
            self.config.preprocess_color,
            self.config.preprocess_tolerance,
        )];
        if self.config.extra_region_enabled {
            panels.push(preprocess::apply(
                capture::crop_task_region(full, self.config.extra_task_region),
                self.preprocess_pipeline_for(true),
                self.config.extra_preprocess_color,
                self.config.extra_preprocess_tolerance,
            ));
        }
        let panel = capture::stack_images(&panels);
        self.ocr_test_running = true;
        self.ocr_test_result = None;
        if self.config.ocr_mode == "remote" {
            let png = match ocr::encode_png(&panel) {
                Ok(png) => png,
                Err(err) => {
                    self.ocr_test_running = false;
                    self.ocr_test_result = Some(Err(err));
                    return;
                }
            };
            let url = self.config.api_url.clone();
            let token = self.config.api_token.clone();
            let format = self.config.api_format.clone();
            let sender = self.sender.clone();
            let ctx = self.ctx.clone();
            thread::spawn(move || {
                let result = ocr::remote_recognize(&url, &token, &format, &png);
                let _ = sender.send(OcrEvent::PanelTest { result });
                ctx.request_repaint();
            });
        } else if let Err(err) = self.worker.submit_test(panel) {
            self.ocr_test_running = false;
            self.ocr_test_result = Some(Err(err));
        }
    }

    /// 把测试 OCR 结果整理成几行可读文本：耗时、识别文字、解析出的任务。
    fn format_test_result(&self, text: &OcrText) -> String {
        let tasks = parse_tasks(
            &text.text,
            &self.maps,
            TaskFilter {
                zhuagui: true,
                ghost_king: true,
                generic: true,
            },
        );
        let mut lines = vec![format!("耗时 {:.2} 秒", text.elapsed_secs)];
        let raw = text.text.trim().replace('\n', " | ");
        if raw.is_empty() {
            lines.push("未识别到文字".to_string());
        } else {
            lines.push(format!("文字：{}", truncate_text(&raw, 240)));
        }
        if tasks.is_empty() {
            lines.push("未解析到任务（可检查截图区域或预处理）".to_string());
        }
        for task in tasks {
            lines.push(format!(
                "{} {} ({}, {}){}",
                task.task_type,
                task.map_name,
                task.x,
                task.y,
                if task.nearby { " 附近" } else { "" }
            ));
        }
        lines.join("\n")
    }

    fn apply_ocr(&mut self, ocr_text: &OcrText) {
        let locations = parse_tasks(&ocr_text.text, &self.maps, self.task_filter());
        // 详细日志里始终留一份 OCR 原文：同时有两个任务时，能直接看出是
        // 只截到一段文字（区域 / 预处理）还是文字都在、只是没解析出来（解析）。
        let raw = ocr_text.text.trim().replace('\n', " | ");
        if raw.is_empty() {
            self.log_detail("OCR 没有识别到文字");
        } else {
            self.log_detail(format!("OCR 文字：{}", truncate_text(&raw, 240)));
        }
        if !self.task_active("抓鬼") && !self.task_active("鬼王") && !self.task_active("通用")
        {
            self.log_detail("没有勾选任何任务类型，不会解析任务");
        }
        for task_type in TASK_ORDER {
            if !self.task_active(task_type) {
                continue;
            }
            let location = locations
                .iter()
                .find(|location| location.task_type == task_type)
                .cloned();
            let mut clamp_note = None;
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
                                let x = location.x as f32;
                                let y = location.y as f32;
                                let strict =
                                    coordinate_in_map(definition, x, y, MAP_COORDINATE_TOLERANCE);
                                // 地图截图可能只截了地图的一部分，坐标超出较多时仍贴边显示；
                                // 超出 50% 以上才当作 OCR 噪声丢弃。
                                if !coordinate_in_map(definition, x, y, 0.5) {
                                    (None, Some("本轮坐标无效，忽略".to_string()))
                                } else {
                                    if !strict {
                                        clamp_note = Some(format!(
                                            "{task_type} 坐标 ({}, {}) 超出地图范围，已贴边显示",
                                            location.x, location.y
                                        ));
                                    }
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
            if let Some(note) = clamp_note {
                self.log_detail(note);
            }
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
        show_high_chance: bool,
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
        painter.rect_filled(rect, CornerRadius::same(8), theme::inset());
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

        // 高概率格子：相对任务坐标偏移的 50×50 区域。
        let high_chance_rects: Vec<Rect> = if show_high_chance {
            high_chance_areas(
                &definition.name,
                location.x as f32,
                location.y as f32,
                HIGH_CHANCE_SIZE,
            )
            .into_iter()
            .filter_map(|(chance_x, chance_y)| {
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
                (chance.width() > 0.0 && chance.height() > 0.0).then_some(chance)
            })
            .collect()
        } else {
            Vec::new()
        };

        let nearby_area: Option<Rect> = if location.nearby {
            // 用实际任务坐标算范围框（不是贴边后的 point_x/point_y），
            // 超出地图的部分由 map_rect 裁到地图边缘，这样黄框位置才和真实坐标对应。
            let (left, top, right, bottom) = map_area(
                definition,
                location.x as f32,
                location.y as f32,
                NEARBY_RADIUS * 2.0,
                texture_size.x,
                texture_size.y,
            );
            Some(
                Rect::from_min_max(to_screen((left, top)), to_screen((right, bottom)))
                    .intersect(image_rect),
            )
        } else {
            None
        };

        if let Some(area) = nearby_area {
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

        for chance in &high_chance_rects {
            // 只显示落在黄色「附近」框内的部分，避免红框超出黄框。
            let chance = match nearby_area {
                Some(area) => (*chance).intersect(area),
                None => *chance,
            };
            if chance.width() <= 0.0 || chance.height() <= 0.0 {
                continue;
            }
            painter.rect_filled(
                chance,
                CornerRadius::ZERO,
                Color32::from_rgba_unmultiplied(255, 40, 40, 48),
            );
            painter.rect_stroke(
                chance,
                CornerRadius::ZERO,
                egui::Stroke::new(2.0, Color32::from_rgba_unmultiplied(255, 40, 40, 170)),
                egui::StrokeKind::Inside,
            );
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
                let fold_window = self.sidebar_section_open("window");
                let now = theme::collapsible_section(ui, "sidebar_window", "游戏窗口", fold_window, |ui| {
                    let current = self
                        .windows
                        .iter()
                        .find(|window| Some(window.handle) == self.selected_handle)
                        .map(|window| window.title.clone())
                        .unwrap_or_else(|| "未找到游戏窗口".into());
                    let mut selected = self.selected_handle;
                    ui.horizontal(|ui| {
                        // ComboBox::width() 只是「最小宽度」：egui 用父 Ui 的 available_width
                        // 作为文本换行宽度，默认 Extend 模式下选中项文本多长它就有多宽。
                        // 游戏窗口标题很长时，它会撑破卡片右缘并把「刷新」按钮挤出可视区，
                        // 所以这里用一个限宽的子 Ui 把可用宽度钉死，并让选中项文本截断。
                        let combo_width = (ui.available_width() - 66.0).max(100.0);
                        ui.allocate_ui_with_layout(
                            egui::vec2(combo_width, ui.spacing().interact_size.y),
                            egui::Layout::left_to_right(egui::Align::Center),
                            |ui| {
                                egui::ComboBox::from_id_salt("window")
                                    .selected_text(current)
                                    .width(combo_width)
                                    .truncate()
                                    .show_ui(ui, |ui| {
                                        for window in &self.windows {
                                            ui.selectable_value(
                                                &mut selected,
                                                Some(window.handle),
                                                &window.title,
                                            );
                                        }
                                    });
                            },
                        );
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
                                .color(theme::gold()),
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
                            // 取点窗口优先放在主窗口右侧；右侧不够就放左侧，
                            // 两边都放不下才贴屏幕右缘（此时靠置顶层级保证不被主窗口压住）。
                            self.region_window_pos =
                                Some(region_window_position(ui.ctx(), REGION_WINDOW_SIZE));
                            self.applied_region_on_top = None;
                            self.capture_preview_once();
                        } else {
                            self.region_window_pos = None;
                            self.applied_region_on_top = None;
                            self.release_preview();
                        }
                    }
                });
                if now != fold_window {
                    self.set_sidebar_section_open("window", now);
                }
                let fold_trigger = self.sidebar_section_open("trigger");
                let now = theme::collapsible_section(ui, "sidebar_trigger", "识别模式", fold_trigger, |ui| {
                    let previous = self.config.trigger_mode.clone();
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.config.trigger_mode,
                            "auto".into(),
                            "自动（按间隔）",
                        );
                        ui.selectable_value(
                            &mut self.config.trigger_mode,
                            "manual".into(),
                            "手动（快捷键）",
                        );
                    });
                    if previous != self.config.trigger_mode {
                        self.invalidate_scan();
                        self.sync_hotkey();
                        self.persist();
                    }
                    if self.manual_mode() {
                        ui.horizontal(|ui| {
                            ui.label("快捷键");
                            let response = ui.add(
                                egui::TextEdit::singleline(&mut self.config.ocr_hotkey)
                                    .desired_width(96.0)
                                    .hint_text("F8"),
                            );
                            if response.lost_focus() {
                                self.sync_hotkey();
                                self.persist();
                            }
                            let record_label =
                                if self.recording_hotkey { "按键…" } else { "录制" };
                            if ui.button(record_label).clicked() {
                                self.recording_hotkey = !self.recording_hotkey;
                            }
                        });
                        match Hotkey::parse(&self.config.ocr_hotkey) {
                            Ok(hotkey) => {
                                let status = if self.hotkey_listener.is_some() {
                                    "全局快捷键已注册"
                                } else if self.running {
                                    "窗口聚焦时可用"
                                } else {
                                    "开始监听后生效"
                                };
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} · {status}",
                                        hotkey.display()
                                    ))
                                    .small()
                                    .weak(),
                                );
                            }
                            Err(err) => {
                                ui.colored_label(theme::gold(), format!("快捷键无效：{err}"));
                            }
                        }
                        ui.horizontal(|ui| {
                            if ui.button("立即识别一次").clicked() {
                                self.manual_trigger();
                            }
                            ui.label(
                                egui::RichText::new("手动模式下按键触发一次 OCR")
                                    .small()
                                    .weak(),
                            );
                        });
                    }
                });
                if now != fold_trigger {
                    self.set_sidebar_section_open("trigger", now);
                }
                let fold_ocr = self.sidebar_section_open("ocr");
                let now = theme::collapsible_section(ui, "sidebar_ocr", "识别方式", fold_ocr, |ui| {
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
                                Ok(message) => (message, theme::jade()),
                                Err(message) => (message, theme::red()),
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
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("预处理").small().weak());
                        let main_pipeline = self.preprocess_pipeline_for(false);
                        let extra_pipeline = self.preprocess_pipeline_for(true);
                        let summary = if self.config.extra_region_enabled {
                            format!(
                                "主 {} / 备 {}",
                                main_pipeline.summary(),
                                extra_pipeline.summary()
                            )
                        } else {
                            main_pipeline.summary()
                        };
                        let all_off = main_pipeline.is_off()
                            && (!self.config.extra_region_enabled || extra_pipeline.is_off());
                        ui.label(
                            egui::RichText::new(summary)
                                .small()
                                .color(if all_off { theme::muted() } else { theme::jade() }),
                        );
                    });
                });
                if now != fold_ocr {
                    self.set_sidebar_section_open("ocr", now);
                }
                let fold_task = self.sidebar_section_open("task");
                let now = theme::collapsible_section(ui, "sidebar_task", "任务显示", fold_task, |ui| {
                    // 抓鬼 / 鬼王 按任务名解析；通用任务不看任务类型，两边互斥。
                    if ui
                        .checkbox(&mut self.config.zhuagui_enabled, "识别抓鬼任务")
                        .changed()
                    {
                        if self.config.zhuagui_enabled {
                            self.config.generic_enabled = false;
                        }
                        self.invalidate_scan();
                        self.persist();
                    }
                    if self.config.zhuagui_enabled {
                        ui.indent("high_chance", |ui| {
                            if ui
                                .checkbox(&mut self.config.high_chance, "显示高概率区域")
                                .on_hover_text(
                                    "按 175LG 捉鬼省眼规则，在抓鬼地图上叠加 50×50 的高概率格子",
                                )
                                .changed()
                            {
                                self.persist();
                            }
                        });
                    }
                    if ui
                        .checkbox(&mut self.config.ghost_king_enabled, "同时识别鬼王任务")
                        .changed()
                    {
                        self.invalidate_scan();
                        self.persist();
                    }
                    if self.config.ghost_king_enabled {
                        ui.indent("high_chance_king", |ui| {
                            if ui
                                .checkbox(
                                    &mut self.config.ghost_king_high_chance,
                                    "显示高概率区域",
                                )
                                .on_hover_text(
                                    "按 175LG 捉鬼省眼规则，在鬼王地图上叠加 50×50 的高概率格子",
                                )
                                .changed()
                            {
                                self.persist();
                            }
                        });
                    }
                    if ui
                        .checkbox(&mut self.config.generic_enabled, "通用任务（只看地图名和坐标）")
                        .on_hover_text(
                            "不识别任务类型，只按地图名和坐标标注；坐标不一定在第一个截图区域里，勾选后会自动启用备用区域",
                        )
                        .changed()
                    {
                        if self.config.generic_enabled {
                            // 通用任务与抓鬼/鬼王互斥；坐标可能落在第二块截图里，顺手打开备用区域。
                            self.config.zhuagui_enabled = false;
                            self.config.ghost_king_enabled = false;
                            if !self.config.extra_region_enabled {
                                self.config.extra_region_enabled = true;
                            }
                        } else if !self.config.zhuagui_enabled && !self.config.ghost_king_enabled {
                            // 关掉通用任务后一个任务都不剩，就把抓鬼打开，免得没有面板。
                            self.config.zhuagui_enabled = true;
                        }
                        self.invalidate_scan();
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
                if now != fold_task {
                    self.set_sidebar_section_open("task", now);
                }
                let fold_log = self.sidebar_section_open("log");
                let now = theme::collapsible_section(
                    ui,
                    "sidebar_log",
                    "日志",
                    fold_log,
                    |ui| self.log_ui(ui),
                );
                if now != fold_log {
                    self.set_sidebar_section_open("log", now);
                }
                ui.add_space(4.0);
            });
    }

    fn help_contents(&self, ui: &mut egui::Ui) {
        ui.label(egui::RichText::new("基本流程").strong());
        for line in [
            "1. 打开游戏，程序会自动扫描并选中游戏窗口（也可用下拉框选择、点「刷新」）。",
            "2. 顶部「开始识别」/「开始监听」启动，「暂停」停止。",
            "3. 「识别方式」选择本地 PP-OCRv5 或远端 PP-OCR；远端可点「测试连接」。",
            "4. 「设置识别区域」打开独立窗口，可调整任务栏截图区域并点击取点配置图色门控。",
        ] {
            ui.label(egui::RichText::new(line).small());
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new("识别模式").strong());
        for line in [
            "自动（按间隔）：按设定间隔定时识别。",
            "手动（快捷键）：点「开始监听」后，按快捷键触发一次识别；快捷键可填写或点「录制」。",
        ] {
            ui.label(egui::RichText::new(line).small());
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new("界面").strong());
        for line in [
            "界面主题：顶栏「界面主题」可选跟随系统 / 亮色 / 暗色，默认跟随系统，系统切换亮暗时立即跟随。",
            "精简界面：顶栏「精简界面」隐藏按钮栏、状态栏和设置栏，只保留地图；按 Esc 恢复默认界面。",
            "置顶：让本窗口始终保持在其他窗口之上，方便对照游戏。",
        ] {
            ui.label(egui::RichText::new(line).small());
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new("图色门控").strong());
        for line in [
            "在「设置识别区域」窗口里点「手动截图一次」，再点击预览图取点取色。",
            "默认每 300ms 比对一次点位颜色：命中（有任务）才允许 OCR，未命中（战斗 / 任务完成）暂停 OCR。",
            "支持全部命中 / 任一命中，并可调每个点的容差与半径；未配置点位时默认允许 OCR。",
            "手动模式下手动触发会忽略图色门控。",
        ] {
            ui.label(egui::RichText::new(line).small());
        }
        ui.add_space(4.0);
        ui.label(egui::RichText::new("其它").strong());
        for line in [
            "任务显示：勾选抓鬼 / 鬼王 / 通用任务；高概率区域可分别开关。",
            "日志：简单只记启动 / 任务开始 / 停止，详细额外记录 OCR 过程与错误。",
            "设置会自动保存到程序目录的 config.json。",
        ] {
            ui.label(egui::RichText::new(line).small());
        }
        ui.add_space(6.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("项目地址").small().weak());
            ui.hyperlink_to("GitHub", crate::APP_REPO_URL)
                .on_hover_text(crate::APP_REPO_URL);
        });
    }

    fn log_ui(&mut self, ui: &mut egui::Ui) {
        // 日志高度跟随窗口高度，窗口变高时能显示更多记录。
        let log_height = log_panel_height(ui.ctx().content_rect().height());
        ui.horizontal(|ui| {
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
            .fill(theme::inset())
            .corner_radius(8)
            .inner_margin(10)
            .show(ui, |ui| {
                if self.logs.is_empty() {
                    ui.label(egui::RichText::new("暂无日志").small().weak());
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("log_scroll")
                        .stick_to_bottom(true)
                        .max_height(log_height)
                        .auto_shrink([false, true])
                        .show_rows(ui, 15.0, self.logs.len(), |ui, rows| {
                            for index in rows {
                                ui.add(
                                    egui::Label::new(
                                        egui::RichText::new(&self.logs[index])
                                            .monospace()
                                            .size(11.0)
                                            .color(theme::muted()),
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
    fn preview(&self, ui: &mut egui::Ui, max_height: f32) -> Option<(i32, i32, [u8; 3])> {
        let Some(image) = self.preview_image.as_ref() else {
            ui.centered_and_justified(|ui| {
                ui.label(
                    egui::RichText::new("暂无截图\n选择游戏窗口后，点击「更新截图」")
                        .color(theme::muted()),
                );
            });
            return None;
        };
        let texture = self.preview_texture.as_ref()?;
        let size = Vec2::new(image.width() as f32, image.height() as f32);
        let available = vec2(
            ui.available_width(),
            ui.available_height().min(max_height).max(1.0),
        );
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
            (self.config.task_region, theme::red(), true),
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
                if (self.pick_points || self.pick_text_color) && response.clicked() {
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

    /// 识别区域窗口：
    /// 上排「截图范围 / 图色门控 | 识别区域预览」，分隔线下面是「识别预处理 | 预处理预览」。
    fn region_window_ui(&mut self, ui: &mut egui::Ui) {
        if ui.ctx().input(|input| input.viewport().close_requested()) {
            self.show_region_window = false;
            self.region_window_pos = None;
            self.release_preview();
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
                    if ui
                        .add_enabled(
                            !self.ocr_test_running,
                            egui::Button::new(if self.ocr_test_running {
                                "识别中…"
                            } else {
                                "测试 OCR"
                            }),
                        )
                        .on_hover_text("用当前截图和预处理设置识别一次，不影响正式识别")
                        .clicked()
                    {
                        self.run_panel_test();
                    }
                    ui.checkbox(&mut self.pick_points, "点击预览取点");
                    if self.pick_text_color {
                        ui.label(
                            egui::RichText::new("取色模式：点击右侧预览设置文字颜色")
                                .small()
                                .color(theme::jade()),
                        );
                    }
                });
                ui.separator();
                self.profile_ui(ui);
                if let Some(message) = &self.notice {
                    ui.label(egui::RichText::new(message).small().color(theme::gold()));
                }
                if self.ocr_test_running {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(egui::RichText::new("正在识别…").small().weak());
                    });
                } else if let Some(result) = self.ocr_test_result.clone() {
                    let mut clear = false;
                    egui::Frame::new()
                        .fill(theme::inset())
                        .corner_radius(6)
                        .inner_margin(8)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new("测试结果").small().strong());
                                if ui.small_button("清空").clicked() {
                                    clear = true;
                                }
                            });
                            egui::ScrollArea::vertical()
                                .id_salt("test_result")
                                .max_height(80.0)
                                .show(ui, |ui| match &result {
                                    Ok(text) => {
                                        ui.add(
                                            egui::Label::new(
                                                egui::RichText::new(text)
                                                    .monospace()
                                                    .size(11.0)
                                                    .color(theme::text()),
                                            )
                                            .wrap(),
                                        );
                                    }
                                    Err(err) => {
                                        ui.label(
                                            egui::RichText::new(err).small().color(theme::red()),
                                        );
                                    }
                                });
                        });
                    if clear {
                        self.ocr_test_result = None;
                    }
                }

                let total_height = ui.available_height();
                let separator = 16.0;
                let top_height = ((total_height - separator) * 0.62).max(240.0);
                let bottom_height = (total_height - separator - top_height).max(140.0);
                let settings_width = (ui.available_width() * 0.36).clamp(300.0, 430.0);

                // 上排：截图范围 / 图色门控 | 识别区域预览
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), top_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal_top(|ui| {
                            ui.vertical(|ui| {
                                ui.set_width(settings_width);
                                egui::ScrollArea::vertical()
                                    .id_salt("region_top_settings")
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        let mut fold_changed = false;
                                        let scope_open = theme::collapsible_section(
                                            ui,
                                            "region_scope",
                                            "截图范围",
                                            self.config.region_scope_open,
                                            |ui| {
                                                let mut changed = region_fields(
                                                    ui,
                                                    "main",
                                                    "主区域",
                                                    &mut self.config.task_region,
                                                );
                                                ui.add_space(6.0);
                                                ui.label(egui::RichText::new("备用区域").strong());
                                                changed |= ui
                                                    .checkbox(
                                                        &mut self.config.extra_region_enabled,
                                                        "启用备用区域",
                                                    )
                                                    .changed();
                                                ui.add_enabled_ui(
                                                    self.config.extra_region_enabled,
                                                    |ui| {
                                                        changed |= region_grid(
                                                            ui,
                                                            "extra",
                                                            &mut self.config.extra_task_region,
                                                        );
                                                    },
                                                );
                                                if changed {
                                                    self.invalidate_scan();
                                                    self.persist();
                                                    self.refresh_preprocess_preview();
                                                }
                                            },
                                        );
                                        if scope_open != self.config.region_scope_open {
                                            self.config.region_scope_open = scope_open;
                                            fold_changed = true;
                                        }
                                        let gate_open = theme::collapsible_section(
                                            ui,
                                            "region_gate",
                                            "图色门控设置",
                                            self.config.region_gate_open,
                                            |ui| self.color_gate_ui(ui),
                                        );
                                        if gate_open != self.config.region_gate_open {
                                            self.config.region_gate_open = gate_open;
                                            fold_changed = true;
                                        }
                                        if fold_changed {
                                            self.persist();
                                        }
                                    });
                            });
                            ui.separator();
                            ui.vertical(|ui| {
                                ui.label(egui::RichText::new("识别区域预览").strong());
                                ui.label(
                                    egui::RichText::new(
                                        "红框=主区域，蓝框=备用区域；点击图片可取点",
                                    )
                                    .small()
                                    .weak(),
                                );
                                let preview_height = (ui.available_height() - 8.0).max(120.0);
                                if let Some((x, y, color)) = self.preview(ui, preview_height) {
                                    self.apply_preview_click(x, y, color);
                                }
                            });
                        });
                    },
                );
                ui.separator();
                // 下排：识别预处理 | 预处理预览
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), bottom_height),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.horizontal_top(|ui| {
                            let total = ui.available_width();
                            let preprocess_width = (total * 0.38).clamp(260.0, 420.0);
                            // 识别预处理
                            ui.vertical(|ui| {
                                ui.set_width(preprocess_width);
                                egui::ScrollArea::vertical()
                                    .id_salt("region_preprocess_settings")
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        let open = theme::collapsible_section(
                                            ui,
                                            "region_preprocess",
                                            "识别预处理",
                                            self.config.region_preprocess_open,
                                            |ui| self.preprocess_ui(ui),
                                        );
                                        if open != self.config.region_preprocess_open {
                                            self.config.region_preprocess_open = open;
                                            self.persist();
                                        }
                                    });
                            });
                            ui.separator();
                            // 预处理预览（主区域 / 备用区域左右排列）
                            ui.vertical(|ui| {
                                self.preprocess_preview_ui(
                                    ui,
                                    (ui.available_height() - 8.0).max(80.0),
                                );
                            });
                        });
                    },
                );
            });
    }

    fn apply_preview_click(&mut self, x: i32, y: i32, color: [u8; 3]) {
        if self.pick_text_color {
            let extra = self.preprocess_target_extra && self.config.extra_region_enabled;
            if extra {
                self.config.extra_preprocess_color = color;
            } else {
                self.config.preprocess_color = color;
            }
            self.preprocess_color_edit = gate::to_hex(color);
            self.pick_text_color = false;
            let mut pipeline = self.preprocess_pipeline_for(extra);
            if !pipeline.color_key {
                pipeline.color_key = true;
                if extra {
                    self.config.extra_preprocess_mode = pipeline.config_value();
                } else {
                    self.config.preprocess_mode = pipeline.config_value();
                }
            }
            self.invalidate_scan();
            self.persist();
            self.refresh_preprocess_preview();
            self.log_detail(format!(
                "取文字颜色 {}，预处理：{}",
                gate::to_hex(color),
                pipeline.summary()
            ));
        } else {
            let mut point = ColorPoint::new(x, y, color, self.config.color_gate_tolerance, 1);
            point.label = format!("点{}", self.config.color_gate_points.len() + 1);
            self.config.color_gate_points.push(point);
            self.check_gate();
            self.persist();
            self.log_detail(format!("新增图色点位 ({x}, {y}) {}", gate::to_hex(color)));
        }
    }

    fn preprocess_preview_ui(&self, ui: &mut egui::Ui, max_height: f32) {
        ui.label(egui::RichText::new("预处理预览").small().weak());
        let width = ui.available_width();
        let column = ((width - 16.0) / 2.0).max(80.0);
        let image_height = (max_height - 22.0).max(60.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(column);
                ui.label(egui::RichText::new("主区域").small().color(theme::muted()));
                preview_texture(
                    ui,
                    &self.preprocess_preview,
                    column,
                    image_height,
                    "点击「更新截图」后可预览",
                );
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.set_width(column);
                ui.label(
                    egui::RichText::new("备用区域")
                        .small()
                        .color(theme::muted()),
                );
                if self.config.extra_region_enabled {
                    preview_texture(
                        ui,
                        &self.preprocess_preview_extra,
                        column,
                        image_height,
                        "暂无备用区域截图",
                    );
                } else {
                    ui.label(egui::RichText::new("未启用备用区域").small().weak());
                }
            });
        });
    }

    fn preprocess_pipeline_for(&self, extra: bool) -> preprocess::Pipeline {
        let mode = if extra {
            &self.config.extra_preprocess_mode
        } else {
            &self.config.preprocess_mode
        };
        preprocess::Pipeline::from_config(mode)
    }

    fn preprocess_ui(&mut self, ui: &mut egui::Ui) {
        self.preprocess_block(ui, false, true);
        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        self.preprocess_block(ui, true, self.config.extra_region_enabled);
    }

    /// 单个区域的预处理设置。`extra` 未启用时整块灰色不可选。
    fn preprocess_block(&mut self, ui: &mut egui::Ui, extra: bool, enabled: bool) {
        let title = if extra {
            "备用区域预处理"
        } else {
            "主区域预处理"
        };
        ui.add_enabled_ui(enabled, |ui| {
            ui.label(egui::RichText::new(title).strong().color(if enabled {
                theme::text()
            } else {
                theme::muted()
            }));
            if !enabled {
                ui.label(
                    egui::RichText::new("先在截图范围勾选「启用备用区域」")
                        .small()
                        .weak(),
                );
                return;
            }
            let previous_mode = if extra {
                self.config.extra_preprocess_mode.clone()
            } else {
                self.config.preprocess_mode.clone()
            };
            let mut pipeline = preprocess::Pipeline::from_config(&previous_mode);
            let mut color = if extra {
                self.config.extra_preprocess_color
            } else {
                self.config.preprocess_color
            };
            let mut tolerance = if extra {
                self.config.extra_preprocess_tolerance
            } else {
                self.config.preprocess_tolerance
            };
            let mut edit_value = if extra {
                self.extra_preprocess_color_edit.clone()
            } else {
                self.preprocess_color_edit.clone()
            };
            let mut changed = false;
            ui.horizontal_wrapped(|ui| {
                changed |= ui.checkbox(&mut pipeline.color_key, "颜色提取").changed();
                changed |= ui.checkbox(&mut pipeline.grayscale, "灰度").changed();
                changed |= ui.checkbox(&mut pipeline.binarize, "二值化").changed();
                changed |= ui
                    .checkbox(&mut pipeline.invert, "反色")
                    .on_hover_text("最终黑白颠倒：二值化把文字判成白色时勾选")
                    .changed();
            });
            if pipeline.color_key {
                let mut new_color: Option<[u8; 3]> = None;
                ui.horizontal_wrapped(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(3),
                        Color32::from_rgb(color[0], color[1], color[2]),
                    );
                    ui.painter().rect_stroke(
                        rect,
                        CornerRadius::same(3),
                        egui::Stroke::new(1.0, theme::border()),
                        egui::StrokeKind::Outside,
                    );
                    if ui
                        .selectable_label(self.pick_text_color, "点击预览取色")
                        .on_hover_text("在右侧预览图上点任务文字取色；点不准时用色值或调色板")
                        .clicked()
                    {
                        self.pick_text_color = !self.pick_text_color;
                        if self.pick_text_color {
                            self.preprocess_target_extra = extra;
                        }
                    }
                    if !self.config.color_gate_points.is_empty()
                        && ui.small_button("用图色点位颜色").clicked()
                    {
                        new_color = Some(self.config.color_gate_points[0].color);
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label("色值");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut edit_value)
                            .desired_width(84.0)
                            .hint_text("#eaff65"),
                    );
                    if response.changed() {
                        if let Some(picked) = gate::parse_hex(&edit_value) {
                            new_color = Some(picked);
                        }
                    } else if !response.has_focus() {
                        edit_value = gate::to_hex(color);
                    }
                    if ui
                        .small_button("默认")
                        .on_hover_text("恢复默认颜色 234, 255, 101（#eaff65）")
                        .clicked()
                    {
                        new_color = Some(crate::config::DEFAULT_PREPROCESS_COLOR);
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("容差");
                    changed |= ui
                        .add(egui::DragValue::new(&mut tolerance).range(0..=255))
                        .changed();
                });
                ui.collapsing("更多颜色", |ui| {
                    let mut color32 = Color32::from_rgb(color[0], color[1], color[2]);
                    if egui::color_picker::color_picker_color32(
                        ui,
                        &mut color32,
                        egui::color_picker::Alpha::Opaque,
                    ) {
                        new_color = Some([color32.r(), color32.g(), color32.b()]);
                    }
                });
                if let Some(picked) = new_color {
                    color = picked;
                    edit_value = gate::to_hex(picked);
                    changed = true;
                }
            }
            let summary = pipeline.summary();
            ui.label(
                egui::RichText::new(format!(
                    "顺序：{}（二值化已包含灰度，颜色提取自带反色；结果黑白颠倒时再勾反色）",
                    if summary == "关闭" {
                        "未启用"
                    } else {
                        &summary
                    }
                ))
                .small()
                .weak(),
            );
            let mode_value = pipeline.config_value();
            if changed || mode_value != previous_mode {
                if extra {
                    self.config.extra_preprocess_mode = mode_value;
                    self.config.extra_preprocess_color = color;
                    self.config.extra_preprocess_tolerance = tolerance;
                } else {
                    self.config.preprocess_mode = mode_value;
                    self.config.preprocess_color = color;
                    self.config.preprocess_tolerance = tolerance;
                }
                self.invalidate_scan();
                self.persist();
                self.refresh_preprocess_preview();
            }
            if extra {
                self.extra_preprocess_color_edit = edit_value;
            } else {
                self.preprocess_color_edit = edit_value;
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
                self.update_preview(full);
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

    /// 任务区域只显示地图；地图名称、坐标和识别状态不再单独占一行。
    fn task_panel(&mut self, ui: &mut egui::Ui, task_type: &str) {
        let (location, map_index) = {
            let state = self.tasks.get(task_type).expect("task state");
            (state.location.clone(), state.map_index)
        };
        let accent = if task_type == "抓鬼" {
            theme::jade()
        } else {
            theme::gold()
        };
        let show_high_chance = match task_type {
            "抓鬼" => self.config.high_chance,
            "鬼王" => self.config.ghost_king_high_chance,
            _ => false,
        };
        let compact = self.compact;
        let draw = |app: &mut Self, ui: &mut egui::Ui| {
            ui.set_min_size(ui.available_size());
            match (&location, map_index) {
                (Some(location), Some(index)) => {
                    let definition = app.maps[index].clone();
                    app.render_map(ui, &definition, location, show_high_chance);
                }
                _ => {
                    let size = ui.available_size().max(vec2(1.0, 1.0));
                    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                    let painter = ui.painter_at(rect);
                    painter.rect_filled(rect, CornerRadius::same(8), theme::inset());
                    let center = rect.center();
                    painter.circle_stroke(center, 18.0, egui::Stroke::new(1.0, theme::border()));
                    painter.circle_stroke(
                        center,
                        9.0,
                        egui::Stroke::new(1.0, accent.gamma_multiply(0.6)),
                    );
                    painter.line_segment(
                        [center - vec2(25.0, 0.0), center + vec2(25.0, 0.0)],
                        egui::Stroke::new(1.0, theme::border()),
                    );
                    painter.line_segment(
                        [center - vec2(0.0, 25.0), center + vec2(0.0, 25.0)],
                        egui::Stroke::new(1.0, theme::border()),
                    );
                }
            }
        };
        if compact {
            // 精简模式不套任务卡片的边框和背景，地图直接铺满窗口。
            draw(self, ui);
        } else {
            theme::card().show(ui, |ui| draw(self, ui));
        }
    }
}

impl eframe::App for MhxyApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // 跟随系统主题时，操作系统切换亮暗要即时生效。
        let light = theme::resolve_light(&self.config.theme, &ctx);
        if self.applied_light != Some(light) {
            theme::set_light(&ctx, light);
            self.applied_light = Some(light);
        }
        self.poll_updates();
        // 默认开启的定时检查：启动 2 秒后检查一次，之后每 30 分钟一次。
        // 有更新/正在下载时不再重复检查；测试环境不联网。
        if !cfg!(test)
            && self.config.update_enabled
            && self.started_at.elapsed() >= Duration::from_secs(2)
            && !matches!(
                self.update_phase,
                UpdatePhase::Checking
                    | UpdatePhase::Downloading
                    | UpdatePhase::Available
                    | UpdatePhase::Downloaded
            )
            && self
                .last_update_check
                .is_none_or(|last| last.elapsed() >= UPDATE_CHECK_INTERVAL)
        {
            self.start_update_check(false);
        }
        // 让按钮上的“已是最新/检查失败”提示自动消失。
        if let Some(at) = self.update_message_at {
            if at.elapsed() < UPDATE_FLASH_DURATION {
                ctx.request_repaint_after(UPDATE_FLASH_DURATION - at.elapsed());
            }
        }
        if matches!(
            self.update_phase,
            UpdatePhase::Checking | UpdatePhase::Downloading
        ) {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        // 收起侧栏后放宽最小宽度，让窗口可以继续缩小。
        let desired_min_size = if self.config.sidebar_expanded && !self.compact {
            MIN_WINDOW_SIZE_EXPANDED
        } else {
            MIN_WINDOW_SIZE_COLLAPSED
        };
        if self.applied_min_inner_size != Some(desired_min_size) {
            ctx.send_viewport_cmd(egui::ViewportCommand::MinInnerSize(desired_min_size));
            self.applied_min_inner_size = Some(desired_min_size);
        }
        // 启动首帧也把置顶状态同步给窗口，避免只依赖创建时的参数。
        if self.applied_on_top != Some(self.config.on_top) {
            apply_window_level(&ctx, self.config.on_top);
            self.applied_on_top = Some(self.config.on_top);
        }
        if self
            .config_dirty_since
            .is_some_and(|since| since.elapsed() >= Duration::from_millis(400))
        {
            self.flush_config();
        }
        self.poll_events();
        self.poll_hotkey();
        // 精简模式下按 Esc 回到默认界面；正在录制快捷键时 Esc 只用于取消录制。
        if self.compact && !self.recording_hotkey && ctx.input(|i| i.key_pressed(egui::Key::Escape))
        {
            self.compact = false;
            self.notice = Some("已退出精简模式".to_string());
        }
        if self.recording_hotkey {
            if let Some(captured) = hotkey::capture_egui(&ctx) {
                self.recording_hotkey = false;
                if captured != "Escape" {
                    self.config.ocr_hotkey = captured;
                    self.persist();
                }
            }
        }
        self.sync_hotkey();
        // 非 Windows 或全局注册失败时，窗口聚焦也能用快捷键。
        if self.running && self.manual_mode() && self.hotkey_listener.is_none() && !self.busy {
            if let Ok(hotkey) = Hotkey::parse(&self.config.ocr_hotkey) {
                if hotkey.matches_egui(&ctx) {
                    self.manual_trigger();
                }
            }
        }
        // 定时重新扫描游戏窗口（游戏可能后开 / 关掉）。
        if (self.running || self.selected_handle.is_none())
            && self.last_window_refresh.elapsed() >= Duration::from_secs(3)
        {
            self.refresh_windows(false);
            self.last_window_refresh = Instant::now();
        }
        if (self.show_region_window || (self.running && !self.manual_mode()))
            && self.config.color_gate_enabled
        {
            let gate_interval =
                Duration::from_millis(self.config.color_gate_interval_ms.max(50) as u64);
            if self.last_gate_check.elapsed() >= gate_interval {
                self.check_gate();
                self.last_gate_check = Instant::now();
            }
            ctx.request_repaint_after(gate_interval);
        }
        if self.running && !self.manual_mode() && !self.busy {
            let interval = Duration::from_secs(self.config.interval_seconds.max(1) as u64);
            let due = self.last_scan.is_none_or(|last| last.elapsed() >= interval);
            if due {
                self.trigger_scan(false);
            }
        }
        // 即使暂停也定时唤醒：刷新窗口列表、连接测试结果和坐标更新时间。
        ctx.request_repaint_after(Duration::from_secs(1));
        if self.running || self.busy {
            ctx.request_repaint_after(Duration::from_millis(150));
        }
        if !self.compact {
            egui::Panel::top("header")
                .frame(
                    egui::Frame::new()
                        .fill(theme::surface())
                        .inner_margin(egui::Margin::symmetric(16, 12)),
                )
                .show(ui, |ui| {
                    let narrow = ui.available_width() < 560.0;
                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(if self.config.sidebar_expanded {
                                    "收起设置"
                                } else {
                                    "展开设置"
                                })
                                .min_size(vec2(0.0, 34.0)),
                            )
                            .clicked()
                        {
                            self.config.sidebar_expanded = !self.config.sidebar_expanded;
                            self.persist();
                        }
                        ui.heading("梦幻西游");
                        if !narrow {
                            ui.label(egui::RichText::new("任务坐标辅助").color(theme::muted()));
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(theme::primary_button(if self.running {
                                    if self.manual_mode() {
                                        "暂停监听"
                                    } else {
                                        "暂停识别"
                                    }
                                } else if self.manual_mode() {
                                    "开始监听"
                                } else {
                                    "开始识别"
                                }))
                                .clicked()
                            {
                                self.toggle_running();
                            }
                            if ui
                                .add(header_button("精简界面", false))
                                .on_hover_text(
                                    "只显示地图，隐藏顶部按钮、状态栏和设置侧栏；按 Esc 退出",
                                )
                                .clicked()
                            {
                                self.compact = true;
                            }
                            let on_top = self.config.on_top;
                            if ui
                                .add(header_button("置顶", on_top))
                                .on_hover_text("让本窗口始终显示在游戏窗口前面")
                                .clicked()
                            {
                                self.set_on_top(!on_top, &ctx);
                            }
                            ui.scope(|ui| {
                                ui.spacing_mut().interact_size.y = 34.0;
                                ui.menu_button("界面主题", |ui| {
                                    for (value, label) in [
                                        ("system", "跟随系统"),
                                        ("light", "亮色"),
                                        ("dark", "暗色"),
                                    ] {
                                        if ui.radio(self.config.theme == value, label).clicked() {
                                            self.config.theme = value.to_string();
                                            self.persist();
                                            ui.close();
                                        }
                                    }
                                })
                                .response
                                .on_hover_text("默认跟随系统，也可以固定亮色或暗色");
                            });
                            if ui
                                .add(header_button("说明", self.show_help))
                                .on_hover_text("使用说明")
                                .clicked()
                            {
                                let opening = !self.show_help;
                                self.show_help = opening;
                                if opening {
                                    self.help_window_pos =
                                        Some(centered_window_position(ui.ctx(), HELP_WINDOW_SIZE));
                                }
                            }
                            // 更新按钮：无更新时是普通「检查更新」；发现更新后变成金色按钮，
                            // 点击弹出更新说明，不主动弹窗。
                            let total =
                                self.update_info.as_ref().map(|info| info.size).unwrap_or(0);
                            let downloaded = self.update_progress.load(Ordering::Relaxed);
                            let percent = if total > 0 {
                                (downloaded as f32 / total as f32 * 100.0).clamp(0.0, 100.0)
                            } else {
                                0.0
                            };
                            let flash_active = self
                                .update_message_at
                                .is_some_and(|at| at.elapsed() < UPDATE_FLASH_DURATION);
                            let flash_text = self.update_message.clone().unwrap_or_default();
                            let (text, color, enabled, highlight) = match self.update_phase {
                                UpdatePhase::Checking => {
                                    ("检查中…".to_string(), theme::muted(), false, false)
                                }
                                UpdatePhase::Available => (
                                    format!(
                                        "{} {}",
                                        if narrow { "更新" } else { "发现新版本" },
                                        self.update_info
                                            .as_ref()
                                            .map(|info| info.version.as_str())
                                            .unwrap_or("")
                                    ),
                                    theme::on_accent(),
                                    true,
                                    true,
                                ),
                                UpdatePhase::Downloading => {
                                    (format!("下载 {percent:.0}%"), theme::text(), true, false)
                                }
                                UpdatePhase::Downloaded => (
                                    if narrow {
                                        "重启更新"
                                    } else {
                                        "重启并更新"
                                    }
                                    .to_string(),
                                    theme::on_accent(),
                                    true,
                                    true,
                                ),
                                UpdatePhase::Failed if flash_active => {
                                    (flash_text, theme::red(), true, false)
                                }
                                UpdatePhase::Failed => {
                                    ("检查更新".to_string(), theme::text(), true, false)
                                }
                                UpdatePhase::Idle if flash_active => {
                                    (flash_text, theme::muted(), true, false)
                                }
                                UpdatePhase::Idle => (
                                    if narrow { "更新" } else { "检查更新" }.to_string(),
                                    theme::text(),
                                    true,
                                    false,
                                ),
                            };
                            let tooltip = match self.update_phase {
                                UpdatePhase::Available => format!(
                                    "发现新版本 {}\n点击查看更新说明",
                                    self.update_info
                                        .as_ref()
                                        .map(|info| info.version.as_str())
                                        .unwrap_or("")
                                ),
                                UpdatePhase::Downloading => "点击查看下载进度".to_string(),
                                UpdatePhase::Downloaded => "安装包已就绪，点击完成更新".to_string(),
                                UpdatePhase::Failed => self
                                    .update_error
                                    .clone()
                                    .unwrap_or_else(|| "检查更新失败".to_string()),
                                UpdatePhase::Checking => "正在检查更新".to_string(),
                                UpdatePhase::Idle => self
                                    .update_message
                                    .clone()
                                    .unwrap_or_else(|| "检查更新".to_string()),
                            };
                            let mut label = egui::RichText::new(text).color(color);
                            if highlight {
                                label = label.strong();
                            }
                            let mut button = egui::Button::new(label).min_size(vec2(0.0, 34.0));
                            if highlight {
                                button = button.fill(theme::gold());
                            }
                            if ui
                                .add_enabled(enabled, button)
                                .on_hover_text(tooltip)
                                .clicked()
                            {
                                match self.update_phase {
                                    UpdatePhase::Available
                                    | UpdatePhase::Downloading
                                    | UpdatePhase::Downloaded => {
                                        self.show_update_dialog = true;
                                        self.update_window_pos = Some(centered_window_position(
                                            ui.ctx(),
                                            UPDATE_WINDOW_SIZE,
                                        ));
                                    }
                                    _ => self.start_update_check(true),
                                }
                            }
                        });
                    });
                });
        }
        if !self.compact {
            egui::Panel::bottom("status_bar")
                .frame(
                    egui::Frame::new()
                        .fill(theme::surface())
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
                                    egui::RichText::new(message).small().color(theme::gold()),
                                )
                                .truncate(),
                            )
                            .on_hover_text(message);
                        } else if self.manual_mode() {
                            let hotkey = Hotkey::parse(&self.config.ocr_hotkey)
                                .map(|hotkey| hotkey.display())
                                .unwrap_or_else(|_| self.config.ocr_hotkey.clone());
                            let text = if self.running {
                                format!("手动识别 · 按 {hotkey} 触发")
                            } else {
                                format!("手动识别 · 快捷键 {hotkey}（未开始监听）")
                            };
                            ui.label(egui::RichText::new(text).small().weak());
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
                                ("配置保存失败", theme::red())
                            } else if self.config_dirty_since.is_some() {
                                ("正在保存…", theme::muted())
                            } else {
                                ("设置自动保存", theme::muted())
                            };
                            ui.label(egui::RichText::new(text).small().color(color))
                                .on_hover_text(self.save_error.as_deref().unwrap_or(text));
                            ui.separator();
                            ui.hyperlink_to(
                                egui::RichText::new("发布页").small(),
                                update::HOME_URL,
                            )
                            .on_hover_text(update::HOME_URL);
                            ui.separator();
                            ui.hyperlink_to(
                                egui::RichText::new("GitHub").small(),
                                crate::APP_REPO_URL,
                            )
                            .on_hover_text(crate::APP_REPO_URL);
                        });
                    });
                });
        }
        if self.config.sidebar_expanded && !self.compact {
            // 侧栏最多占可用宽度的一半减一点，保证右侧地图区域不会被挤压。
            let max_sidebar = (ui.available_width() - 360.0).clamp(240.0, 400.0);
            egui::Panel::left("controls")
                .resizable(true)
                .default_size(320.0f32.min(max_sidebar).max(240.0))
                .min_size(240.0f32.min(max_sidebar))
                .max_size(max_sidebar)
                .frame(
                    egui::Frame::new()
                        .fill(theme::background())
                        .inner_margin(12),
                )
                .show(ui, |ui| self.controls(ui));
        }
        let central_margin = if self.compact { 0 } else { 12 };
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::background())
                    .inner_margin(central_margin),
            )
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
                    let gap = if self.compact { 0.0 } else { 5.0 };
                    let mut child =
                        ui.new_child(egui::UiBuilder::new().max_rect(child_rect.shrink(gap)));
                    self.task_panel(&mut child, task_type);
                }
            });
        if self.show_update_dialog {
            let on_top = self.config.on_top;
            let mut builder = egui::ViewportBuilder::default()
                .with_title(format!("发现新版本 · rust-mhxy {}", crate::APP_VERSION))
                .with_inner_size(UPDATE_WINDOW_SIZE)
                .with_min_inner_size([400.0, 340.0]);
            if on_top {
                builder = builder.with_always_on_top();
            }
            if let Some(position) = self.update_window_pos {
                builder = builder.with_position(position);
            }
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("update_window"),
                builder,
                |ui, _class| {
                    if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
                        // 视口还没建好时先画在主窗口里（只出现一帧）。
                        egui::Window::new("发现新版本")
                            .collapsible(false)
                            .resizable(false)
                            .default_width(460.0)
                            .show(ui.ctx(), |ui| self.update_dialog_ui(ui));
                    } else {
                        if ui.ctx().input(|input| input.viewport().close_requested()) {
                            self.show_update_dialog = false;
                        }
                        if self.applied_update_on_top != Some(on_top) {
                            let level = if on_top {
                                egui::WindowLevel::AlwaysOnTop
                            } else {
                                egui::WindowLevel::Normal
                            };
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
                            self.applied_update_on_top = Some(on_top);
                        }
                        egui::Frame::central_panel(ui.style())
                            .inner_margin(16)
                            .show(ui, |ui| {
                                ui.set_min_size(ui.available_size());
                                egui::ScrollArea::vertical().show(ui, |ui| {
                                    self.update_dialog_ui(ui);
                                });
                            });
                    }
                },
            );
        }
        if self.show_help {
            let on_top = self.config.on_top;
            let mut builder = egui::ViewportBuilder::default()
                .with_title("使用说明 · rust-mhxy")
                .with_inner_size(HELP_WINDOW_SIZE)
                .with_min_inner_size([420.0, 360.0]);
            if on_top {
                builder = builder.with_always_on_top();
            }
            if let Some(position) = self.help_window_pos {
                builder = builder.with_position(position);
            }
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("help_window"),
                builder,
                |ui, _class| {
                    if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
                        egui::Window::new("使用说明")
                            .collapsible(false)
                            .resizable(true)
                            .default_width(480.0)
                            .show(ui.ctx(), |ui| self.help_contents(ui));
                    } else {
                        if ui.ctx().input(|input| input.viewport().close_requested()) {
                            self.show_help = false;
                        }
                        if self.applied_help_on_top != Some(on_top) {
                            let level = if on_top {
                                egui::WindowLevel::AlwaysOnTop
                            } else {
                                egui::WindowLevel::Normal
                            };
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
                            self.applied_help_on_top = Some(on_top);
                        }
                        egui::Frame::central_panel(ui.style())
                            .inner_margin(16)
                            .show(ui, |ui| {
                                ui.set_min_size(ui.available_size());
                                egui::ScrollArea::vertical().show(ui, |ui| self.help_contents(ui));
                            });
                    }
                },
            );
        }
        if !self.show_region_window {
            self.release_preview();
        }
        if self.show_region_window {
            let on_top = self.config.on_top;
            let mut builder = egui::ViewportBuilder::default()
                .with_title("识别区域")
                .with_inner_size(REGION_WINDOW_SIZE)
                .with_min_inner_size([720.0, 600.0]);
            if on_top {
                // 主窗口置顶时取点窗口也必须置顶，否则会被主窗口整个盖住。
                builder = builder.with_always_on_top();
            }
            // 只在建立窗口时定位，之后用户拖动不会被抢回。
            if let Some(position) = self.region_window_pos {
                builder = builder.with_position(position);
            }
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("region_window"),
                builder,
                |ui, _class| {
                    if ui.ctx().viewport_id() == egui::ViewportId::ROOT {
                        // 独立视口还没建好，先画在主窗口里（只出现在第一帧）。
                        egui::Window::new("识别区域")
                            .default_size([800.0, 640.0])
                            .show(ui.ctx(), |ui| self.region_window_ui(ui));
                    } else {
                        // 跟随「置顶」开关同步取点窗口的层级，开着置顶时它才不会被主窗口压住。
                        if self.applied_region_on_top != Some(on_top) {
                            let level = if on_top {
                                egui::WindowLevel::AlwaysOnTop
                            } else {
                                egui::WindowLevel::Normal
                            };
                            ui.ctx()
                                .send_viewport_cmd(egui::ViewportCommand::WindowLevel(level));
                            self.applied_region_on_top = Some(on_top);
                        }
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

/// 左/上/右/下 四个百分比，两行两列对齐。
fn region_grid(ui: &mut egui::Ui, id: &str, region: &mut crate::core::TaskRegion) -> bool {
    let mut values = [
        region.0 * 100.0,
        region.1 * 100.0,
        region.2 * 100.0,
        region.3 * 100.0,
    ];
    let mut edited = false;
    egui::Grid::new(("region_grid", id))
        .num_columns(4)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (index, (name, value)) in ["左", "上", "右", "下"]
                .into_iter()
                .zip(values.iter_mut())
                .enumerate()
            {
                ui.label(egui::RichText::new(name).small().weak());
                edited |= ui
                    .add(
                        egui::DragValue::new(value)
                            .speed(0.2)
                            .range(0.0..=100.0)
                            .suffix("%"),
                    )
                    .changed();
                if index % 2 == 1 {
                    ui.end_row();
                }
            }
        });
    if !edited {
        return false;
    }
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
    true
}

/// 带标题的区域设置（标题单独一行）。
fn region_fields(
    ui: &mut egui::Ui,
    id: &str,
    title: &str,
    region: &mut crate::core::TaskRegion,
) -> bool {
    ui.label(egui::RichText::new(title).strong());
    region_grid(ui, id, region)
}

/// 取点窗口的默认尺寸。
const REGION_WINDOW_SIZE: Vec2 = Vec2::new(1000.0, 820.0);

/// 顶部栏统一按钮格式：统一样式与高度，用 selected 表示开关状态。
fn header_button(text: &str, selected: bool) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_owned()))
        .selected(selected)
        .min_size(vec2(0.0, 34.0))
}

/// 说明 / 更新窗口在主窗口中间打开，并限制在屏幕内。
fn centered_window_position(ctx: &egui::Context, size: Vec2) -> egui::Pos2 {
    ctx.input(|input| {
        let viewport = input.viewport();
        let monitor = viewport
            .monitor_size
            .unwrap_or_else(|| Vec2::new(1920.0, 1080.0));
        let main = viewport.outer_rect;
        let x = main
            .map(|rect| rect.center().x - size.x / 2.0)
            .unwrap_or(0.0);
        let y = main
            .map(|rect| rect.center().y - size.y / 2.0)
            .unwrap_or(0.0);
        egui::pos2(
            x.clamp(0.0, (monitor.x - size.x).max(0.0)),
            y.clamp(0.0, (monitor.y - size.y).max(0.0)),
        )
    })
}

/// 取点窗口优先放在主窗口右侧，右侧空间不够就改放左侧；两边都放不下时才贴屏幕右缘，
/// 此时不可避免会和主窗口重叠，靠窗口层级（置顶）保证它显示在主窗口之上。
fn region_window_position(ctx: &egui::Context, size: Vec2) -> egui::Pos2 {
    ctx.input(|input| {
        let viewport = input.viewport();
        let monitor = viewport
            .monitor_size
            .unwrap_or_else(|| Vec2::new(1920.0, 1080.0));
        match viewport.outer_rect {
            Some(main) => place_region_window(main, monitor, size),
            // 拿不到主窗口位置时退化成屏幕右上角，至少保证整窗可见。
            None => egui::pos2((monitor.x - size.x).max(0.0), 0.0),
        }
    })
}

/// [`region_window_position`] 的纯计算部分：主窗口外框 + 显示器尺寸 → 取点窗口外框左上角。
fn place_region_window(main: Rect, monitor: Vec2, size: Vec2) -> egui::Pos2 {
    const GAP: f32 = 8.0;
    // with_position 给的是窗口外框左上角，多留一个标题栏高度，免得下沿出屏。
    const TITLE_BAR: f32 = 32.0;
    let outer = size + Vec2::new(0.0, TITLE_BAR);
    let max_x = (monitor.x - outer.x).max(0.0);
    let max_y = (monitor.y - outer.y).max(0.0);
    let right = main.max.x + GAP;
    let left = main.min.x - GAP - outer.x;
    let x = if right <= max_x {
        right
    } else if left >= 0.0 {
        left
    } else {
        max_x
    };
    let y = main.min.y.clamp(0.0, max_y);
    egui::pos2(x, y)
}

/// 按可用宽高等比绘制一张预览纹理；没有纹理时显示提示文字。
fn preview_texture(
    ui: &mut egui::Ui,
    texture: &Option<TextureHandle>,
    max_width: f32,
    max_height: f32,
    empty_text: &str,
) {
    match texture {
        Some(texture) => {
            let size = texture.size_vec2();
            let scale = (max_width / size.x).min(max_height / size.y).min(1.0);
            let (rect, _) = ui.allocate_exact_size(size * scale, Sense::hover());
            ui.painter().image(
                texture.id(),
                rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            ui.label(egui::RichText::new(empty_text).small().weak());
        }
    }
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let mut output: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        output.push('…');
    }
    output
}

fn fit_rect(outer: Rect, size: Vec2) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return outer;
    }
    let scale = (outer.width() / size.x).min(outer.height() / size.y);
    Rect::from_center_size(outer.center(), size * scale)
}

/// 应用窗口置顶状态。
fn apply_window_level(ctx: &egui::Context, on: bool) {
    ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if on {
        egui::WindowLevel::AlwaysOnTop
    } else {
        egui::WindowLevel::Normal
    }));
}

/// 日志框高度随窗口高度线性变化，并限制在最小值和最大值之间。
fn log_panel_height(content_height: f32) -> f32 {
    (content_height * LOG_PANEL_HEIGHT_RATIO).clamp(LOG_PANEL_MIN_HEIGHT, 620.0)
}

fn to_color_image(image: &image::RgbImage) -> egui::ColorImage {
    egui::ColorImage::from_rgb(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    )
}

fn sample_image(image: &image::RgbImage, x: i32, y: i32) -> Option<[u8; 3]> {
    if x < 0 || y < 0 || x as u32 >= image.width() || y as u32 >= image.height() {
        return None;
    }
    let pixel = image.get_pixel(x as u32, y as u32);
    Some([pixel[0], pixel[1], pixel[2]])
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
        theme::install(&ctx, false);
        MhxyApp::from_config(Config::default(), PathBuf::from("/unused/config.json"), ctx)
    }

    /// 收集这一帧渲染出的所有文字，用于断言哪些界面元素出现了。
    fn collect_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.text().to_string()),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(shape, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    /// 收集这一帧每段文字的位置，用于断言顶栏按钮顺序。
    fn text_positions(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Pos2)> {
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Pos2)>) {
            match shape {
                egui::Shape::Text(text) => out.push((text.galley.text().to_string(), text.pos)),
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        walk(shape, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    fn escape_input() -> egui::RawInput {
        let mut input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
            ..Default::default()
        };
        input.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::default(),
        });
        input
    }

    #[test]
    fn task_panel_has_no_high_chance_checkbox() {
        let mut app = app();
        app.config.high_chance = true;
        app.config.ghost_king_high_chance = true;
        let ctx = app.ctx.clone();
        let mut frame = eframe::Frame::_new_kittest();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
        let texts = collect_texts(&output.shapes);
        assert!(
            !texts.iter().any(|text| text == "高概率区域"),
            "地图上方不应再有高概率勾选框：{texts:?}"
        );
        // 侧栏「任务显示」里的开关还在，只是换成了完整文案。
        assert!(
            texts.iter().any(|text| text.contains("显示高概率区域")),
            "侧栏仍应保留高概率开关：{texts:?}"
        );
        output.textures_delta.clear();
    }

    #[test]
    fn header_orders_update_help_theme_top_compact_start() {
        let mut app = app();
        let ctx = app.ctx.clone();
        let mut frame = eframe::Frame::_new_kittest();
        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| app.ui(ui, &mut frame));
        let positions = text_positions(&output.shapes);
        let find = |needle: &str| {
            positions
                .iter()
                .find(|(text, _)| text == needle)
                .map(|(_, pos)| *pos)
                .unwrap_or_else(|| panic!("顶栏缺少「{needle}」：{positions:?}"))
        };
        let update = find("检查更新");
        let help = find("说明");
        let theme = find("界面主题");
        let on_top = find("置顶");
        let compact = find("精简界面");
        let start = find("开始识别");
        assert!(
            update.x < help.x
                && help.x < theme.x
                && theme.x < on_top.x
                && on_top.x < compact.x
                && compact.x < start.x,
            "顶栏顺序应为 检查更新 | 说明 | 界面主题 | 置顶 | 精简界面 | 开始识别：\
             update={update:?} help={help:?} theme={theme:?} on_top={on_top:?} \
             compact={compact:?} start={start:?}"
        );
        output.textures_delta.clear();
    }

    #[test]
    fn compact_mode_hides_chrome_and_esc_restores_it() {
        let mut app = app();
        let ctx = app.ctx.clone();
        let mut frame = eframe::Frame::_new_kittest();
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
            ..Default::default()
        };

        app.compact = true;
        let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
        let texts = collect_texts(&output.shapes);
        for hidden in [
            "开始识别",
            "精简界面",
            "置顶",
            "界面主题",
            "说明",
            "梦幻西游",
        ] {
            assert!(
                !texts.iter().any(|text| text == hidden),
                "精简模式不应显示 {hidden}：{texts:?}"
            );
        }
        assert!(
            !texts.iter().any(|text| text.contains("检查更新")),
            "精简模式不应显示更新按钮：{texts:?}"
        );
        output.textures_delta.clear();

        // Esc 回到默认界面，顶部按钮和侧栏重新出现。
        let mut output = ctx.run_ui(escape_input(), |ui| app.ui(ui, &mut frame));
        assert!(!app.compact, "按 Esc 应退出精简模式");
        let texts = collect_texts(&output.shapes);
        assert!(
            texts.iter().any(|text| text == "开始识别"),
            "退出精简模式后应重新出现开始识别按钮：{texts:?}"
        );
        output.textures_delta.clear();
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
    fn region_window_renders_all_preprocess_modes() {
        let mut app = app();
        app.show_region_window = true;
        app.config.preprocess_mode = "color".into();
        app.config.preprocess_color = [240, 210, 120];
        app.update_preview(image::RgbImage::from_pixel(
            320,
            240,
            image::Rgb([24, 24, 24]),
        ));
        let ctx = app.ctx.clone();
        for mode in [
            "off",
            "gray",
            "binary",
            "color",
            "color,binary",
            "gray,binary",
            "binary,invert",
        ] {
            app.config.preprocess_mode = mode.into();
            app.refresh_preprocess_preview();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 700.0))),
                    ..Default::default()
                },
                |ui| app.region_window_ui(ui),
            );
            assert!(!output.shapes.is_empty(), "预处理模式 {mode} 渲染失败");
            output.textures_delta.clear();
        }
    }

    #[test]
    fn update_events_move_through_phases() {
        let mut app = app();
        let info = UpdateInfo {
            version: "v9.9.9".into(),
            notes: "测试".into(),
            url: "https://example.com/a.zip".into(),
            sha256: "abc".into(),
            size: 100,
        };
        app.update_tx
            .send(UpdateEvent::Checked(Ok(Some(info))))
            .unwrap();
        app.poll_updates();
        assert_eq!(app.update_phase, UpdatePhase::Available);
        assert_eq!(
            app.update_info.as_ref().map(|info| info.version.as_str()),
            Some("v9.9.9")
        );

        app.update_tx
            .send(UpdateEvent::Downloaded(Ok(PathBuf::from("/tmp/a.zip"))))
            .unwrap();
        app.poll_updates();
        assert_eq!(app.update_phase, UpdatePhase::Downloaded);
        assert!(app.update_zip.is_some());

        app.update_tx
            .send(UpdateEvent::Checked(Err("网络错误".into())))
            .unwrap();
        app.poll_updates();
        assert_eq!(app.update_phase, UpdatePhase::Failed);
        assert_eq!(app.update_error.as_deref(), Some("网络错误"));
    }

    #[test]
    fn update_section_renders_every_phase() {
        let mut app = app();
        let ctx = app.ctx.clone();
        let mut frame = eframe::Frame::_new_kittest();
        app.update_info = Some(UpdateInfo {
            version: "v9.9.9".into(),
            notes: "测试说明".into(),
            url: "https://example.com/a.zip".into(),
            sha256: "abc".into(),
            size: 1024,
        });
        app.update_message = Some("已是最新版本".into());
        app.update_message_at = Some(Instant::now());
        app.update_error = Some("测试错误".into());
        app.update_progress.store(512, Ordering::Relaxed);
        for phase in [
            UpdatePhase::Idle,
            UpdatePhase::Checking,
            UpdatePhase::Available,
            UpdatePhase::Downloading,
            UpdatePhase::Downloaded,
            UpdatePhase::Failed,
        ] {
            app.update_phase = phase;
            // 同时渲染顶部按钮和更新说明弹窗。
            app.show_update_dialog = true;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
                    ..Default::default()
                },
                |ui| app.ui(ui, &mut frame),
            );
            assert!(!output.shapes.is_empty(), "更新阶段 {phase:?} 渲染失败");
            app.show_update_dialog = false;
            output.textures_delta.clear();
        }
    }

    #[test]
    fn panel_test_result_is_formatted_for_region_window() {
        let mut app = app();
        app.ocr_test_running = true;
        app.sender
            .send(OcrEvent::PanelTest {
                result: Ok(OcrText {
                    text: "抓鬼 长寿村 12,34".into(),
                    elapsed_secs: 0.2,
                }),
            })
            .unwrap();
        app.poll_events();
        assert!(!app.ocr_test_running, "测试结束后应复位状态");
        let text = app
            .ocr_test_result
            .as_ref()
            .expect("应保存测试结果")
            .as_ref()
            .expect("应识别成功");
        assert!(text.contains("耗时 0.20 秒"));
        assert!(text.contains("抓鬼"));
        assert!(text.contains("长寿村"));
        assert!(text.contains("(12, 34)"));

        app.ocr_test_running = true;
        app.sender
            .send(OcrEvent::PanelTest {
                result: Err("模型加载失败".into()),
            })
            .unwrap();
        app.poll_events();
        assert_eq!(app.ocr_test_result, Some(Err("模型加载失败".to_string())));
    }

    #[test]
    fn region_window_uses_left_right_layout() {
        fn mesh_bounds(shape: &egui::Shape, texture: egui::TextureId) -> Option<Rect> {
            match shape {
                egui::Shape::Mesh(mesh) if mesh.texture_id == texture => Some(mesh.calc_bounds()),
                egui::Shape::Vec(shapes) => {
                    shapes.iter().find_map(|shape| mesh_bounds(shape, texture))
                }
                _ => None,
            }
        }
        let mut app = app();
        app.show_region_window = true;
        app.config.preprocess_mode = "binary".into();
        app.config.extra_region_enabled = true;
        app.update_preview(image::RgbImage::from_pixel(
            640,
            360,
            image::Rgb([20, 20, 20]),
        ));
        let ctx = app.ctx.clone();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 820.0))),
                ..Default::default()
            },
            |ui| app.region_window_ui(ui),
        );
        let preview_id = app.preview_texture.as_ref().expect("预览纹理").id();
        let preprocess_id = app.preprocess_preview.as_ref().expect("预处理预览").id();
        let extra_preview_id = app
            .preprocess_preview_extra
            .as_ref()
            .expect("备用区域预处理预览")
            .id();
        let preview = output
            .shapes
            .iter()
            .find_map(|shape| mesh_bounds(&shape.shape, preview_id))
            .expect("识别区域预览未绘制");
        let preprocess = output
            .shapes
            .iter()
            .find_map(|shape| mesh_bounds(&shape.shape, preprocess_id))
            .expect("预处理预览未绘制");
        assert!(preview.min.x >= 300.0, "识别区域预览应在右侧：{preview:?}");
        assert!(
            preprocess.min.x >= 300.0,
            "预处理预览应在右侧：{preprocess:?}"
        );
        assert!(
            preprocess.min.y > preview.max.y,
            "预处理预览应在识别区域预览下方：preview={preview:?} preprocess={preprocess:?}"
        );
        let extra_preprocess = output
            .shapes
            .iter()
            .find_map(|shape| mesh_bounds(&shape.shape, extra_preview_id))
            .expect("备用区域预处理预览未绘制");
        assert!(
            preprocess.min.x < extra_preprocess.min.x,
            "主区域在左、备用区域在右：main={preprocess:?} extra={extra_preprocess:?}"
        );
        assert!(
            preprocess.min.y < extra_preprocess.max.y && extra_preprocess.min.y < preprocess.max.y,
            "两张预处理预览应左右排列：main={preprocess:?} extra={extra_preprocess:?}"
        );

        // 文字位置：截图范围、门控设置在上排左侧；识别预处理在分隔线下方。
        fn text_pos(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<egui::Pos2> {
            fn walk(shape: &egui::Shape, needle: &str) -> Option<egui::Pos2> {
                match shape {
                    egui::Shape::Text(text) => {
                        text.galley.text().contains(needle).then_some(text.pos)
                    }
                    egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| walk(shape, needle)),
                    _ => None,
                }
            }
            shapes.iter().find_map(|shape| walk(&shape.shape, needle))
        }
        let scope = text_pos(&output.shapes, "截图范围").expect("缺少截图范围标题");
        let extra_region = text_pos(&output.shapes, "备用区域").expect("缺少备用区域标题");
        let gate = text_pos(&output.shapes, "图色门控设置").expect("缺少门控标题");
        let preprocess_title = text_pos(&output.shapes, "识别预处理").expect("缺少预处理标题");
        let preview_title = text_pos(&output.shapes, "识别区域预览").expect("缺少预览标题");
        assert!(
            scope.y < extra_region.y && extra_region.y < gate.y,
            "顺序应为 截图范围 → 备用区域 → 门控设置：{scope:?} {extra_region:?} {gate:?}"
        );
        let main_region = text_pos(&output.shapes, "主区域").expect("缺少主区域标题");
        let enable_extra = text_pos(&output.shapes, "启用备用区域").expect("缺少启用备用区域");
        let main_preprocess = text_pos(&output.shapes, "主区域预处理").expect("缺少主区域预处理");
        let extra_preprocess =
            text_pos(&output.shapes, "备用区域预处理").expect("缺少备用区域预处理");
        assert!(
            main_region.y < extra_region.y && extra_region.y < enable_extra.y,
            "截图范围内顺序应为 主区域 → 备用区域 → 启用备用区域"
        );
        assert!(
            main_preprocess.y > gate.y && main_preprocess.y < extra_preprocess.y,
            "预处理应为 主区域在上、备用区域在下，且在门控设置之后：{main_preprocess:?} {extra_preprocess:?}"
        );
        assert!(
            preprocess_title.y > gate.y,
            "识别预处理应在分隔线下方、门控设置之后"
        );
        assert!(
            preview_title.x > scope.x + 120.0,
            "识别区域预览标题应在右侧列"
        );
        output.textures_delta.clear();
    }

    #[test]
    fn preprocess_settings_are_independent_per_region() {
        let mut app = app();
        app.config.preprocess_mode = "binary".into();
        app.config.extra_preprocess_mode = "color".into();
        app.config.extra_preprocess_color = [1, 2, 3];
        app.config.extra_region_enabled = true;
        app.show_region_window = true;
        app.update_preview(image::RgbImage::from_pixel(
            200,
            100,
            image::Rgb([30, 30, 30]),
        ));
        assert_eq!(app.preprocess_pipeline_for(false).config_value(), "binary");
        assert_eq!(app.preprocess_pipeline_for(true).config_value(), "color");
        assert!(app.preprocess_preview.is_some(), "主区域预览应生成");
        assert!(app.preprocess_preview_extra.is_some(), "备用区域预览应生成");

        // 修改主区域设置不影响备用区域。
        app.config.preprocess_mode = "gray".into();
        app.refresh_preprocess_preview();
        assert_eq!(app.preprocess_pipeline_for(true).config_value(), "color");
    }

    #[test]
    fn preprocess_preview_tracks_settings() {
        let mut app = app();
        app.show_region_window = true;
        app.config.preprocess_mode = "binary".into();
        app.update_preview(image::RgbImage::from_pixel(
            60,
            40,
            image::Rgb([10, 10, 10]),
        ));
        assert!(
            app.preprocess_preview.is_some(),
            "取点窗口打开时应生成预处理预览"
        );
        let first = app.preprocess_preview.as_ref().map(|texture| texture.id());
        app.config.preprocess_mode = "gray".into();
        app.refresh_preprocess_preview();
        assert_eq!(
            app.preprocess_preview.as_ref().map(|texture| texture.id()),
            first,
            "切换模式应复用同一纹理"
        );

        app.show_region_window = false;
        app.release_preview();
        assert!(app.preprocess_preview.is_none(), "关闭取点窗口后释放预览");
    }

    #[test]
    fn preview_updates_reuse_the_existing_texture() {
        let mut app = app();
        app.show_region_window = true;
        app.update_preview(image::RgbImage::from_pixel(100, 100, image::Rgb([0, 0, 0])));
        let texture_id = app.preview_texture.as_ref().unwrap().id();
        app.update_preview(image::RgbImage::from_pixel(
            120,
            100,
            image::Rgb([255, 0, 0]),
        ));
        assert_eq!(app.preview_texture.as_ref().unwrap().id(), texture_id);
        assert_eq!(
            sample_image(app.preview_image.as_ref().unwrap(), 119, 99),
            Some([255, 0, 0])
        );
    }

    #[test]
    fn preview_is_released_when_region_window_is_closed() {
        let mut app = app();
        app.show_region_window = true;
        app.update_preview(image::RgbImage::from_pixel(64, 64, image::Rgb([1, 2, 3])));
        assert!(app.preview_image.is_some());
        assert!(app.preview_texture.is_some());
        assert_eq!(app.client_size, Some((64, 64)));

        app.show_region_window = false;
        app.release_preview();
        assert!(app.preview_image.is_none());
        assert!(app.preview_texture.is_none());
        assert_eq!(app.client_size, Some((64, 64)), "客户区尺寸仍用于图色取点");

        app.update_preview(image::RgbImage::from_pixel(80, 60, image::Rgb([4, 5, 6])));
        assert!(app.preview_image.is_none());
        assert!(app.preview_texture.is_none());
        assert_eq!(app.client_size, Some((80, 60)));
    }

    #[test]
    fn failed_recognition_keeps_previous_map_and_bounds() {
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
            for recognized in [true, false, true, false] {
                if recognized {
                    app.apply_ocr(&success);
                } else {
                    app.apply_ocr(&OcrText {
                        text: String::new(),
                        elapsed_secs: 0.1,
                    });
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
                assert!(
                    app.tasks["抓鬼"].location.is_some(),
                    "未识别到任务时应保留上次坐标"
                );
                if let Some(expected) = expected {
                    assert_eq!(bounds, expected, "宽度：{width}，本轮识别：{recognized}");
                }
                expected = Some(bounds);
                output.textures_delta.clear();
            }
        }
    }

    #[test]
    fn renders_in_light_theme() {
        let ctx = egui::Context::default();
        theme::install(&ctx, true);
        let mut app = MhxyApp::from_config(
            Config {
                theme: "light".into(),
                ..Default::default()
            },
            PathBuf::from("/unused/config.json"),
            ctx.clone(),
        );
        let mut frame = eframe::Frame::_new_kittest();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
                ..Default::default()
            },
            |ui| app.ui(ui, &mut frame),
        );
        assert!(!output.shapes.is_empty(), "亮色主题渲染失败");
        output.textures_delta.clear();
        // 恢复默认，避免影响并行运行的其他测试。
        theme::set_light(&ctx, false);
    }

    #[test]
    fn profiles_apply_save_and_delete() {
        let mut app = app();
        app.config.preprocess_mode = "binary".into();
        app.config.color_gate_enabled = true;
        app.save_active_profile();

        app.config.preprocess_mode = "off".into();
        app.config.color_gate_enabled = false;
        let name = app.config.active_profile.clone();
        app.apply_profile(&name);
        assert_eq!(app.config.preprocess_mode, "binary");
        assert!(app.config.color_gate_enabled);

        app.profile_name_edit = "测试方案".into();
        app.create_profile();
        assert_eq!(app.config.active_profile, "测试方案");
        assert!(app.config.profiles.iter().any(|p| p.name == "测试方案"));

        app.delete_active_profile();
        assert!(!app.config.profiles.iter().any(|p| p.name == "测试方案"));
        assert!(!app.config.profiles.is_empty());
    }

    #[test]
    fn sidebar_sections_can_collapse() {
        let mut app = app();
        app.config.sidebar_folded = "window,trigger,ocr,task,log".into();
        assert!(!app.sidebar_section_open("window"));
        assert!(!app.sidebar_section_open("log"));
        assert!(app.sidebar_section_open("unknown"));
        app.set_sidebar_section_open("window", true);
        assert!(app.sidebar_section_open("window"));
        assert!(app.config.sidebar_folded.contains("trigger"));
        app.set_sidebar_section_open("trigger", true);
        assert!(!app.config.sidebar_folded.contains("trigger"));

        // 全折叠时侧栏仍能正常渲染。
        let ctx = app.ctx.clone();
        let mut frame = eframe::Frame::_new_kittest();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
                ..Default::default()
            },
            |ui| app.ui(ui, &mut frame),
        );
        assert!(!output.shapes.is_empty());
        output.textures_delta.clear();
    }

    #[test]
    fn log_panel_height_grows_with_window() {
        assert_eq!(log_panel_height(200.0), LOG_PANEL_MIN_HEIGHT);
        assert!(log_panel_height(900.0) > log_panel_height(700.0));
        assert_eq!(log_panel_height(5000.0), 620.0);
    }

    #[test]
    fn region_window_stays_on_screen_and_prefers_the_free_side() {
        let size = vec2(900.0, 700.0);
        let monitor = vec2(1920.0, 1080.0);

        // 主窗口靠左、右侧放得下：贴主窗口右缘。
        let left_half = Rect::from_min_size(pos2(0.0, 0.0), vec2(900.0, 800.0));
        assert_eq!(
            place_region_window(left_half, monitor, size),
            pos2(908.0, 0.0)
        );

        // 主窗口靠右、右侧放不下但左侧放得下：改放主窗口左侧，两边都不重叠。
        let right_half = Rect::from_min_size(pos2(1000.0, 0.0), vec2(900.0, 800.0));
        assert_eq!(
            place_region_window(right_half, monitor, size),
            pos2(92.0, 0.0)
        );

        // 两侧都放不下（默认窗口尺寸 + 1080p 正是这种情况）：贴屏幕右缘，整窗仍在屏内。
        let wide = Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0));
        assert_eq!(place_region_window(wide, monitor, size), pos2(1020.0, 0.0));

        // 主窗口贴近屏幕下沿时整体上移，底边不越出屏幕（算上了标题栏高度）。
        let low = Rect::from_min_size(pos2(0.0, 900.0), vec2(1120.0, 180.0));
        assert_eq!(place_region_window(low, monitor, size), pos2(1020.0, 348.0));
    }

    #[test]
    fn generic_task_panel_follows_config() {
        let mut app = app();
        assert!(app.task_active("抓鬼"), "抓鬼默认开启");
        assert!(!app.task_active("通用"), "默认不显示通用任务面板");
        app.config.generic_enabled = true;
        assert!(app.task_active("通用"), "勾选后应显示通用任务面板");
        assert_eq!(TASK_ORDER, ["抓鬼", "鬼王", "通用"]);

        // 通用任务与抓鬼互斥：切到通用后抓鬼不再显示。
        app.config.zhuagui_enabled = false;
        assert!(!app.task_active("抓鬼"));
        assert!(app.task_active("通用"));
    }

    #[test]
    fn long_window_titles_do_not_push_sidebar_controls_out_of_the_card() {
        /// 在绘制出来的图元里按文本找控件的实际位置。
        fn text_bounds(shapes: &[egui::epaint::ClippedShape], needle: &str) -> Option<Rect> {
            fn walk(shape: &egui::Shape, needle: &str) -> Option<Rect> {
                match shape {
                    egui::Shape::Text(text) if text.galley.text().contains(needle) => {
                        Some(text.visual_bounding_rect())
                    }
                    egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| walk(shape, needle)),
                    _ => None,
                }
            }
            shapes
                .iter()
                .find_map(|clipped| walk(&clipped.shape, needle))
        }

        let mut app = app();
        let ctx = app.ctx.clone();
        let mut frame = eframe::Frame::_new_kittest();
        app.config.sidebar_expanded = true;
        app.selected_handle = Some(7);
        app.windows = vec![WindowInfo {
            handle: 7,
            title: "梦幻西游 ONLINE - (山东1区[东海崂山]) - 守^一座空城[41] - 再补一段很长的标题"
                .into(),
        }];

        // 侧栏展开时的最小窗口：此时侧栏取到上限 400，最容易挤掉右侧控件。
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(760.0, 540.0))),
                ..Default::default()
            },
            |ui| app.ui(ui, &mut frame),
        );
        output.textures_delta.clear();

        // ComboBox::width() 只是最小宽度，长标题曾把「刷新」整个挤出卡片。
        // 注意：被挤出侧栏的控件会被 egui 按 clip_rect 剔除，所以「找不到」也算失败。
        for control in ["刷新", "设置识别区域"] {
            let bounds = text_bounds(&output.shapes, control)
                .unwrap_or_else(|| panic!("「{control}」没有绘制在侧栏可见区域内"));
            assert!(
                bounds.max.x <= 400.0,
                "「{control}」超出了侧栏右缘：{bounds:?}"
            );
        }
    }

    #[test]
    fn window_controls_send_viewport_commands() {
        let mut app = app();
        let ctx = app.ctx.clone();
        let mut frame = eframe::Frame::_new_kittest();
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
            ..Default::default()
        };

        app.config.on_top = false;
        app.set_on_top(true, &ctx);
        assert!(app.config.on_top);
        assert!(app.config_dirty_since.is_some(), "切换置顶后应触发配置保存");
        let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
        assert!(output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::WindowLevel(
                egui::WindowLevel::AlwaysOnTop
            )));
        output.textures_delta.clear();

        app.config.on_top = true;
        app.set_on_top(false, &ctx);
        let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
        assert!(output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .contains(&egui::ViewportCommand::WindowLevel(
                egui::WindowLevel::Normal
            )));
        output.textures_delta.clear();
    }

    #[test]
    fn collapsing_sidebar_lowers_minimum_window_size() {
        let mut app = app();
        let ctx = app.ctx.clone();
        let mut frame = eframe::Frame::_new_kittest();
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1120.0, 800.0))),
            ..Default::default()
        };
        let min_size = |output: &egui::FullOutput| {
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .iter()
                .find_map(|command| match command {
                    egui::ViewportCommand::MinInnerSize(size) => Some(*size),
                    _ => None,
                })
        };

        app.config.sidebar_expanded = true;
        let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
        assert_eq!(min_size(&output), Some(MIN_WINDOW_SIZE_EXPANDED));
        output.textures_delta.clear();

        app.config.sidebar_expanded = false;
        let mut output = ctx.run_ui(input(), |ui| app.ui(ui, &mut frame));
        let collapsed = min_size(&output).expect("收起侧栏后应放宽最小尺寸");
        assert_eq!(collapsed, MIN_WINDOW_SIZE_COLLAPSED);
        assert!(collapsed.x < MIN_WINDOW_SIZE_EXPANDED.x);
        output.textures_delta.clear();
    }

    #[test]
    fn renders_local_remote_and_map_layouts_at_supported_sizes() {
        /// 找到地图纹理的绘制区域。
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
            if sidebar {
                // 地图必须在侧栏右侧绘制，不能被侧栏区域覆盖。
                for texture in app.map_textures.values().map(|handle| handle.id()) {
                    if let Some(bounds) = output
                        .shapes
                        .iter()
                        .find_map(|shape| map_bounds(&shape.shape, texture))
                    {
                        assert!(
                            bounds.min.x >= 240.0,
                            "地图绘到了侧栏区域：{bounds:?}（窗口 {size:?}）"
                        );
                    }
                }
            }
            output.textures_delta.clear();
        }
        assert_eq!(app.map_textures.len(), 2);
    }
}
