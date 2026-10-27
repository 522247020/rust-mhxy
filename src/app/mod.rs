//! egui 界面与识别调度。只保留「抓鬼 / 鬼王」任务。
use crate::capture::{self, WindowInfo};
use crate::config::{assets_dir, region_to_string, Config};
use crate::core::{
    clamp_to_map, coordinate_in_map, find_map_definition, high_chance_areas, map_area,
    map_coordinate, map_point, map_rect, parse_tasks, MapDefinition, TaskFilter, TaskLocation,
    HIGH_CHANCE_SIZE, MAP_COORDINATE_TOLERANCE,
};
use crate::flight_flags::{self, FlightFlagStyle, FlightFlags, FlightPoint, SUPPORTED_MAPS};
use crate::gate::{self, ColorPoint, GateMode};
use crate::hotkey::{self, GlobalHotkey, Hotkey};
use crate::meditation::{MeditationTimer, State as MeditationState};
use crate::ocr::{self, LocalWorker, OcrEvent, OcrText};
use crate::preprocess;
use crate::theme;
use crate::update::UpdateInfo;
use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, Sense, TextureHandle, Vec2};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

mod flight;
mod meditation;
mod region;
mod shell;
mod sidebar;
mod task;
mod update;

use update::UpdateEvent;
use update::{UpdatePhase, UPDATE_CHECK_INTERVAL, UPDATE_FLASH_DURATION, UPDATE_WINDOW_SIZE};

use region::REGION_WINDOW_SIZE;
#[cfg(test)]
use region::{place_region_window, sample_image};

const TASK_ORDER: [&str; 3] = ["抓鬼", "鬼王", "通用"];
/// 每次注册全局快捷键都换一个 id：旧监听线程还在退出时，
/// 复用同一个 id 会撞上「热键已被注册」，导致新快捷键要等重试才生效。
static NEXT_HOTKEY_ID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(100);
/// 快捷键文本变化后的注册节流，避免输入过程中每个字符注册一次。
const HOTKEY_CHANGE_DELAY: Duration = Duration::from_millis(500);
/// 注册失败后的重试间隔。
const HOTKEY_RETRY_DELAY: Duration = Duration::from_secs(5);
/// 打坐悬浮窗胶囊的最小尺寸。
const NEARBY_RADIUS: f32 = 50.0;
/// 侧栏展开 / 收起时的最小窗口尺寸。
const MIN_WINDOW_SIZE_EXPANDED: Vec2 = Vec2::new(760.0, 540.0);
const MIN_WINDOW_SIZE_COLLAPSED: Vec2 = Vec2::new(460.0, 460.0);
/// 说明 / 更新窗口的默认尺寸。
const HELP_WINDOW_SIZE: Vec2 = Vec2::new(560.0, 640.0);
const FLIGHT_FLAGS_WINDOW_SIZE: Vec2 = Vec2::new(900.0, 760.0);
/// 高级设置与打坐设置的独立窗口尺寸。
const ADVANCED_WINDOW_SIZE: Vec2 = Vec2::new(560.0, 640.0);
const MEDITATION_WINDOW_SIZE: Vec2 = Vec2::new(480.0, 560.0);
#[derive(Default)]
struct TaskState {
    location: Option<TaskLocation>,
    map_index: Option<usize>,
    /// 多帧投票缓冲，抑制单帧错认。
    votes: VoteBuffer,
}

/// 多帧投票：地图取最近几帧的众数（连续 2 帧相同则立即切换），
/// 坐标取该地图最近 3 帧的中位数。
#[derive(Default)]
struct VoteBuffer {
    recent: std::collections::VecDeque<(String, i32, i32)>,
}

impl VoteBuffer {
    fn vote(&mut self, map: &str, x: i32, y: i32) -> (String, i32, i32) {
        self.recent.push_back((map.to_string(), x, y));
        while self.recent.len() > 5 {
            self.recent.pop_front();
        }
        // 连续两帧同一地图：视为切换或稳定跟踪，不再等众数。
        let consecutive = self
            .recent
            .iter()
            .rev()
            .take(2)
            .all(|(name, _, _)| name == map);
        let best_map = if consecutive {
            map.to_string()
        } else {
            let mut counts: Vec<(String, usize)> = Vec::new();
            for (name, _, _) in &self.recent {
                match counts.iter_mut().find(|(known, _)| known == name) {
                    Some((_, count)) => *count += 1,
                    None => counts.push((name.clone(), 1)),
                }
            }
            let (best, best_count) = counts
                .into_iter()
                .max_by_key(|(_, count)| *count)
                .unwrap_or_else(|| (map.to_string(), 1));
            if best_count < 2 {
                return (map.to_string(), x, y);
            }
            best
        };
        // 同一地图最近 3 帧的坐标中位数。
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for (name, px, py) in self.recent.iter().rev().take(3) {
            if name == &best_map {
                xs.push(*px);
                ys.push(*py);
            }
        }
        if xs.is_empty() {
            return (best_map, x, y);
        }
        xs.sort_unstable();
        ys.sort_unstable();
        (best_map, xs[xs.len() / 2], ys[ys.len() / 2])
    }
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
    /// 最近一次 OCR 快捷键注册失败的原因。
    hotkey_error: Option<String>,
    hotkey_tried_at: Option<Instant>,
    recording_hotkey: bool,
    last_manual_trigger: Option<Instant>,
    /// 识别还没结束时又按了快捷键：记下来，等这一轮结束立刻补一次，
    /// 不再像以前那样直接丢掉（看起来就像快捷键没反应）。
    pending_manual_trigger: bool,
    /// 精简模式：只留地图，隐藏顶部按钮栏、状态栏和侧栏；按 Esc 退出。
    compact: bool,
    /// 进入精简模式后的提示（按 Esc 退出），到点自动消失。
    compact_hint_until: Option<Instant>,
    /// 当前实际应用的亮色 / 暗色，用于跟随系统主题时即时切换。
    applied_light: Option<bool>,
    /// 打坐倒计时状态机。
    meditation: MeditationTimer,
    meditation_hotkey_tx: Sender<()>,
    meditation_hotkey_rx: Receiver<()>,
    meditation_hotkey_listener: Option<GlobalHotkey>,
    meditation_hotkey_attempted: Option<String>,
    /// 最近一次注册失败的原因，显示在设置里。
    meditation_hotkey_error: Option<String>,
    /// 上次尝试注册的时间，失败后定期重试。
    meditation_hotkey_tried_at: Option<Instant>,
    meditation_applied_on_top: Option<bool>,
    meditation_applied_passthrough: Option<bool>,
    meditation_applied_size: Option<Vec2>,
    /// 窗口尺寸的防抖：待应用的尺寸与第一次观察到的时间。
    meditation_pending_size: Option<(Vec2, Instant)>,
    /// 上一次识别时的游戏客户区尺寸，用来在分辨率变化时切换识别区域。
    last_client_size: Option<(u32, u32)>,
    /// 悬浮窗去阴影的尝试次数；窗口重建后归零重试。
    meditation_shadow_attempts: u8,
    /// 悬浮窗上一帧是否处于最小化；用于只在「可见→最小化」边沿发一次还原命令。
    meditation_overlay_minimized: bool,
    /// 正在录制打坐计时的全局快捷键。
    meditation_recording_hotkey: bool,
    /// 合成旗点位单独保存在用户本地目录，不进入主配置和更新包。
    flight_flags: FlightFlags,
    flight_flags_path: PathBuf,
    show_flight_flags_window: bool,
    /// 高级设置窗口（识别方式、方案、显示、合成旗）。
    show_advanced_window: bool,
    advanced_window_pos: Option<egui::Pos2>,
    applied_advanced_on_top: Option<bool>,
    /// 打坐计时设置窗口。
    show_meditation_window: bool,
    meditation_window_pos: Option<egui::Pos2>,
    applied_meditation_window_on_top: Option<bool>,
    /// 日志落盘路径（不再在界面里展示）。
    log_path: PathBuf,
    flight_map_edit: String,
    flight_flag_index: usize,
    flight_flag_name_edit: String,
    flight_feedback: Option<Result<String, String>>,
}

/// 日志落盘位置：配置目录下的 logs/rust-mhxy.log。
fn default_log_path(config_path: &std::path::Path) -> PathBuf {
    let dir = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.join("logs"))
        .unwrap_or_else(|| std::env::temp_dir().join("rust-mhxy-logs"));
    dir.join("rust-mhxy.log")
}

/// 追加一行日志；文件超过 2MB 时轮转成 `.1`，避免无限增长。
fn append_log(path: &std::path::Path, line: &str) {
    const MAX_BYTES: u64 = 2 * 1024 * 1024;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if std::fs::metadata(path)
        .map(|meta| meta.len() > MAX_BYTES)
        .unwrap_or(false)
    {
        let rotated = path.with_extension("log.1");
        let _ = std::fs::rename(path, rotated);
    }
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        Ok(mut file) => {
            use std::io::Write;
            let _ = file.write_all(line.as_bytes());
        }
        Err(err) => eprintln!("写入日志失败（{}）：{err}", path.display()),
    }
}

fn next_hotkey_id() -> i32 {
    NEXT_HOTKEY_ID.fetch_add(1, Ordering::Relaxed)
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
        let (meditation_hotkey_tx, meditation_hotkey_rx) = channel::<()>();
        let mut tasks = HashMap::new();
        for task in TASK_ORDER {
            tasks.insert(task, TaskState::default());
        }
        let flight_flags_path = if cfg!(test) {
            PathBuf::from("/unused/flight_flags.json")
        } else {
            flight_flags::default_config_path()
        };
        let (flight_flags, flight_feedback) = match FlightFlags::load(&flight_flags_path) {
            Ok(flags) => (flags, None),
            Err(err) => (FlightFlags::default(), Some(Err(err))),
        };
        let meditation_duration = config.meditation_duration_seconds;
        let log_path = if cfg!(test) {
            std::env::temp_dir().join(format!("rust-mhxy-test-log-{}.log", std::process::id()))
        } else {
            default_log_path(&config_path)
        };
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
            hotkey_error: None,
            hotkey_tried_at: None,
            recording_hotkey: false,
            last_manual_trigger: None,
            pending_manual_trigger: false,
            compact: false,
            compact_hint_until: None,
            applied_light: None,
            meditation: MeditationTimer::new(meditation_duration),
            meditation_hotkey_tx,
            meditation_hotkey_rx,
            meditation_hotkey_listener: None,
            meditation_hotkey_attempted: None,
            meditation_hotkey_error: None,
            meditation_hotkey_tried_at: None,
            meditation_applied_on_top: None,
            meditation_applied_passthrough: None,
            meditation_applied_size: None,
            meditation_pending_size: None,
            last_client_size: None,
            meditation_shadow_attempts: 0,
            meditation_overlay_minimized: false,
            meditation_recording_hotkey: false,
            flight_flags,
            flight_flags_path,
            show_flight_flags_window: false,
            show_advanced_window: false,
            advanced_window_pos: None,
            applied_advanced_on_top: None,
            show_meditation_window: false,
            meditation_window_pos: None,
            applied_meditation_window_on_top: None,
            log_path,
            flight_map_edit: SUPPORTED_MAPS[0].to_string(),
            flight_flag_index: 0,
            flight_flag_name_edit: String::new(),
            flight_feedback,
        };
        app.log(format!("程序启动，加载 {} 张地图", app.maps.len()));
        app.log_detail(format!("资源目录：{}", assets_dir().display()));
        if let Some(err) = map_error {
            app.log(err);
        }
        app
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
            // 换窗口后重新按新窗口的分辨率套用识别区域。
            self.last_client_size = None;
            self.gate_result = None;
            self.gate_active =
                !self.config.color_gate_enabled || self.config.color_gate_points.is_empty();
            self.last_gate_check =
                Instant::now() - Duration::from_millis(self.config.color_gate_interval_ms as u64);
        }
    }

    /// 进入精简模式：首次进入时在窗口底部提示「按 Esc 退出」，几秒后自动消失。
    fn enter_compact_mode(&mut self) {
        self.compact = true;
        self.compact_hint_until = Some(Instant::now() + Duration::from_secs(4));
    }

    fn toggle_running(&mut self) {
        if self.running {
            self.running = false;
            self.invalidate_scan();
            self.hotkey_listener = None;
            self.hotkey_attempted = None;
            self.hotkey_error = None;
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
            self.sync_meditation_hotkey();
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
        // 通用任务是独立模式：只要它开启，就不解析抓鬼 / 鬼王。
        if self.config.generic_enabled {
            return task_type == "通用";
        }
        match task_type {
            "抓鬼" => self.config.zhuagui_enabled,
            "鬼王" => self.config.ghost_king_enabled,
            "通用" => false,
            _ => true,
        }
    }

    fn task_filter(&self) -> TaskFilter {
        if self.config.generic_enabled {
            return TaskFilter {
                zhuagui: false,
                ghost_king: false,
                generic: true,
            };
        }
        TaskFilter {
            zhuagui: self.config.zhuagui_enabled,
            ghost_king: self.config.ghost_king_enabled,
            generic: false,
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
        // 自动模式下手动按快捷键也强制识别一次，不用先切到手动模式。
        if !self.running {
            // 没开始监听时按到快捷键（例如窗口聚焦回退）就明确说清楚，
            // 免得看起来像快捷键坏了。
            self.pending_manual_trigger = false;
            self.log_detail("未开始识别，快捷键不触发");
            return;
        }
        if self.busy {
            // 上一轮还没结束：先记下，等结果回来立刻补一次。
            self.pending_manual_trigger = true;
            self.log_detail("上一轮识别尚未结束，结束后立即补识别");
            return;
        }
        if let Some(last) = self.last_manual_trigger {
            if last.elapsed() < Duration::from_millis(400) {
                return;
            }
        }
        self.pending_manual_trigger = false;
        self.last_manual_trigger = Some(Instant::now());
        self.log_detail("手动触发识别");
        self.trigger_scan(true);
    }

    /// 上一轮识别结束后，把排队的那一次快捷键触发补上。
    fn flush_pending_manual_trigger(&mut self) {
        if self.pending_manual_trigger && !self.busy {
            self.pending_manual_trigger = false;
            self.manual_trigger();
        }
    }

    fn poll_hotkey(&mut self) {
        let mut pressed = false;
        while self.hotkey_rx.try_recv().is_ok() {
            pressed = true;
        }
        if pressed {
            self.log("全局快捷键触发");
            self.manual_trigger();
        }
    }

    /// 采纳一个新的快捷键：写入配置并立即重新注册（用于“改用空闲快捷键”）。
    fn adopt_hotkey(&mut self, label: String) {
        if self.config.ocr_hotkey == label {
            return;
        }
        self.config.ocr_hotkey = label;
        self.hotkey_attempted = None;
        self.hotkey_tried_at = None;
        self.hotkey_error = None;
        self.persist();
        self.sync_hotkey();
    }

    /// 按当前运行状态 / 模式 / 快捷键字符串同步全局快捷键注册。
    fn sync_hotkey(&mut self) {
        // 只要在运行就注册（自动模式下手动按一下也会立即识别一次），
        // 不再依赖「手动模式」开关，避免切模式后忘记重新注册。
        let desired = if self.running {
            Hotkey::parse(&self.config.ocr_hotkey)
                .ok()
                .map(|hotkey| (hotkey, hotkey.display()))
        } else {
            None
        };
        let desired_label = desired.as_ref().map(|(_, label)| label.clone());
        let changed = desired_label != self.hotkey_attempted;
        if !changed {
            // 监听线程意外退出时，系统已经把热键注销了，但界面还写着「已注册」。
            // 这里主动发现并重新注册，否则就会一直「提示已注册、按键没反应」。
            if self.hotkey_listener.as_ref().is_some_and(|l| l.is_alive()) {
                return;
            }
            if self.hotkey_listener.is_some() {
                self.log_detail("全局快捷键监听线程已退出，重新注册");
                self.hotkey_listener = None;
                self.hotkey_error = None;
            } else if !self.running {
                return;
            }
        }
        // 输入过程中每 500ms 才注册一次；注册失败或线程退出则每 5 秒重试。
        let delay = if changed {
            HOTKEY_CHANGE_DELAY
        } else {
            HOTKEY_RETRY_DELAY
        };
        if self.hotkey_tried_at.is_some_and(|at| at.elapsed() < delay) {
            return;
        }
        self.hotkey_attempted = desired_label;
        self.hotkey_tried_at = Some(Instant::now());
        self.hotkey_listener = None;
        let Some((hotkey, label)) = desired else {
            self.hotkey_error = None;
            return;
        };
        match hotkey::spawn(hotkey, next_hotkey_id(), self.hotkey_tx.clone()) {
            Ok(listener) => {
                match &listener.warning {
                    Some(warn) => self.log_detail(format!("快捷键提示：{warn}")),
                    None => self.log_detail(format!("已注册全局快捷键：{label}")),
                }
                self.hotkey_listener = Some(listener);
                self.hotkey_error = None;
            }
            Err(err) => {
                self.hotkey_listener = None;
                // 同一个错误只记一次，避免每 5 秒刷屏。
                if self.hotkey_error.as_deref() != Some(err.as_str()) {
                    self.log(err.clone());
                }
                self.hotkey_error = Some(err);
            }
        }
    }

    /// 分辨率变化时自动切换识别区域：
    /// 把调整过的区域记到上一个分辨率，再套用新分辨率保存过的区域。
    fn sync_region_for_size(&mut self, size: Option<(u32, u32)>) {
        let Some((width, height)) = size else {
            return;
        };
        if self.last_client_size == Some((width, height)) {
            return;
        }
        if let Some((old_width, old_height)) = self.last_client_size {
            self.config.region_presets.insert(
                format!("{old_width}x{old_height}"),
                region_to_string(self.config.task_region),
            );
        }
        let key = format!("{width}x{height}");
        if let Some(saved) = self
            .config
            .region_presets
            .get(&key)
            .and_then(|saved| crate::core::parse_region(saved))
        {
            self.config.task_region = saved;
        }
        self.last_client_size = Some((width, height));
        self.persist();
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
        // 分辨率变了就切换识别区域（每个分辨率记住一套）。
        self.sync_region_for_size(capture::client_size(handle));
        let region = self.config.task_region;
        let full = match capture::capture_client(handle, region) {
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
            capture::crop_task_region(&full, region),
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
        // 放大后再送 OCR：任务栏文字小的时候，检测模型更容易漏掉。
        let panel = preprocess::upscale(&capture::stack_images(&panels), self.config.ocr_upscale);
        self.update_preview(full);
        self.notice = None;
        let signature = hash_image(&panel);
        // 快捷键 / 「立即识别一次」是明确的用户意图，不能因为画面没变就跳过，
        // 否则按下去毫无反应（自动识别才需要去重）。
        if !force && self.last_signature == Some(signature) {
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
            if let Err(err) = self
                .worker
                .submit(generation, panel, self.config.ocr_high_recall)
            {
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
        let panel = preprocess::upscale(&capture::stack_images(&panels), self.config.ocr_upscale);
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
            let mut location = locations
                .iter()
                .find(|location| location.task_type == task_type)
                .cloned();
            // 多帧投票：地图取众数、坐标取中位数，减少单帧错认造成的跳动。
            if self.config.ocr_vote_frames {
                if let (Some(location), Some(state)) =
                    (location.as_mut(), self.tasks.get_mut(task_type))
                {
                    let (map, x, y) = state.votes.vote(&location.map_name, location.x, location.y);
                    location.map_name = map;
                    location.x = x;
                    location.y = y;
                }
            }
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
                (None, Some(location)) => {
                    // 不是「详细」结构匹配时标出来，方便判断这条结果可信度。
                    let hint = if location.matched == crate::core::TaskMatch::Detailed {
                        String::new()
                    } else {
                        format!(" [{}匹配]", location.matched.label())
                    };
                    format!(
                        "{task_type} {} ({}, {}){hint}",
                        location.map_name, location.x, location.y
                    )
                }
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

fn to_color_image(image: &image::RgbImage) -> egui::ColorImage {
    egui::ColorImage::from_rgb(
        [image.width() as usize, image.height() as usize],
        image.as_raw(),
    )
}

fn hash_image(image: &image::RgbImage) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    image.dimensions().hash(&mut hasher);
    image.as_raw().hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests;
