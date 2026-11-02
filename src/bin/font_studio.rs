//! `font-studio`：点阵字库的图形化采集工具。
//!
//! 命令行那套（`font-tool capture`）要背参数、看不到画面；这个界面把同一个流程
//! 摊开：选窗口 → 框区域 → 看预处理和切字效果 → 采集 → 逐个检查字模。
//!
//! 和 `font-tool` 共用 `tool_shared`，切片/归一化/标签对齐与运行时复核是同一套代码。

// 和 font-tool 一样：这些模块是从主程序共享过来的，界面只用到其中一部分。
#![allow(dead_code)]

#[path = "../capture.rs"]
mod capture;
#[path = "../coord_fix.rs"]
mod coord_fix;
#[path = "../core.rs"]
mod core;
#[path = "../font_lib.rs"]
mod font_lib;
#[path = "../gate.rs"]
mod gate;
#[path = "../ocr.rs"]
mod ocr;
#[path = "../preprocess.rs"]
mod preprocess;
#[path = "../tool_shared.rs"]
mod shared;

/// `ocr.rs` 只用它定位随包的 PP-OCR 模型；这里给一个最小实现，
/// 免得为了一个 GUI 工具把整个 config 模块链编译进来。
mod config {
    use std::path::PathBuf;

    pub fn assets_dir() -> PathBuf {
        if let Ok(override_path) = std::env::var("MHXY_ASSETS_DIR") {
            if !override_path.is_empty() {
                return PathBuf::from(override_path);
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            if cwd.join("assets").join("maps.json").exists() {
                return cwd.join("assets");
            }
            if cwd.join("maps.json").exists() {
                return cwd;
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                if dir.join("assets").join("maps.json").exists() {
                    return dir.join("assets");
                }
                return dir.to_path_buf();
            }
        }
        PathBuf::from(".")
    }
}

use font_lib::{GlyphLib, LearnConfig, Learner, MatchConfig};
use shared::*;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

fn main() -> eframe::Result {
    // 必须在创建窗口/截图之前：系统有缩放时两套坐标会差一个倍数。
    enable_dpi_awareness();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("font-studio · 点阵字库采集")
            .with_inner_size([1360.0, 900.0])
            .with_min_inner_size([980.0, 640.0]),
        ..Default::default()
    };
    eframe::run_native(
        "font-studio",
        options,
        Box::new(|cc| {
            install_cjk(&cc.egui_ctx);
            Ok(Box::new(Studio::new()))
        }),
    )
}

// ---------------------------------------------------------------- 后台采集

enum JobMsg {
    Frame {
        index: u32,
        total: u32,
        learned: usize,
        ocr: String,
        panel: image::RgbImage,
    },
    Done {
        lib: GlyphLib,
        summary: Vec<font_lib::LabelSummary>,
        conflicts: Vec<String>,
        samples: usize,
        frames_used: usize,
        skipped: usize,
    },
    Failed(String),
    /// 后台积累：每轮一报。`error` 是这一轮的错误（窗口被关/最小化），不影响继续跑。
    Progress {
        attempts: u32,
        frames_used: u32,
        samples: usize,
        learned: usize,
        ocr: String,
        panel: Option<image::RgbImage>,
        lib: GlyphLib,
        summary: Vec<font_lib::LabelSummary>,
        complete: bool,
        error: Option<String>,
        unchanged: bool,
    },
    /// 后台积累被停掉（或出错退出），带上当前成果。
    AccStopped {
        lib: GlyphLib,
        summary: Vec<font_lib::LabelSummary>,
        samples: usize,
        frames_used: u32,
        attempts: u32,
    },
}

struct Job {
    rx: Receiver<JobMsg>,
    total: u32,
    done: u32,
    started: Instant,
}

/// 后台积累模式的 UI 句柄。
struct AccState {
    rx: Receiver<JobMsg>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    attempts: u32,
    frames_used: u32,
    samples: usize,
    complete: bool,
    started: Instant,
}

/// 一次采集需要的全部参数（复制一份丢给后台线程）。
#[derive(Clone)]
struct JobPlan {
    handle: i64,
    region: core::TaskRegion,
    pipeline: preprocess::Pipeline,
    color: [u8; 3],
    tolerance: u8,
    upscale: u32,
    high_recall: bool,
    frames: u32,
    interval_ms: u64,
    min_similarity: f32,
    learn_all: bool,
    cell_w: u32,
    cell_h: u32,
    seed: Option<GlyphLib>,
}

fn spawn_job(plan: JobPlan, ctx: egui::Context) -> Receiver<JobMsg> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut learner = Learner::new(
            plan.cell_w,
            plan.cell_h,
            LearnConfig {
                min_similarity: plan.min_similarity,
                shift: 1,
            },
        );
        if let Some(seed) = &plan.seed {
            let _ = learner.seed(seed);
        }
        let mut frames_used = 0usize;
        let mut skipped = 0usize;
        for index in 0..plan.frames {
            if index > 0 {
                std::thread::sleep(Duration::from_millis(plan.interval_ms));
            }
            let panel = (|| -> Result<image::RgbImage, String> {
                let full = capture::capture_client(plan.handle, plan.region)?;
                let mut panel = preprocess::apply(
                    capture::crop_task_region(&full, plan.region),
                    plan.pipeline,
                    plan.color,
                    plan.tolerance,
                );
                if plan.upscale > 1 {
                    panel = preprocess::upscale(&panel, plan.upscale);
                }
                Ok(panel)
            })();
            let panel = match panel {
                Ok(panel) => panel,
                Err(err) => {
                    let _ = tx.send(JobMsg::Failed(err));
                    return;
                }
            };
            let ocr = match ocr::recognize_panel(&panel, plan.high_recall) {
                Ok(ocr) => ocr,
                Err(err) => {
                    let _ = tx.send(JobMsg::Failed(err));
                    return;
                }
            };
            let learned = learn_panel(
                &mut learner,
                &panel,
                &ocr,
                plan.learn_all,
                &mut skipped,
                coord_fix::DEFAULT_PADDING,
                None,
            );
            if learned > 0 {
                frames_used += 1;
            }
            let _ = tx.send(JobMsg::Frame {
                index: index + 1,
                total: plan.frames,
                learned,
                ocr: ocr.text.trim().replace('\n', " / "),
                panel,
            });
            ctx.request_repaint();
        }
        let _ = tx.send(JobMsg::Done {
            lib: learner.finish(),
            summary: learner.summary(),
            conflicts: learner.conflicts(2),
            samples: learner.sample_count(),
            frames_used,
            skipped,
        });
        ctx.request_repaint();
    });
    rx
}

/// 面板指纹：画面没变就不重复识别/学习（后台积累时游戏可能一直停在同一个界面）。
fn panel_hash(panel: &image::RgbImage) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let step_x = (panel.width() / 64).max(1) as usize;
    for y in (0..panel.height()).step_by(3) {
        for x in (0..panel.width()).step_by(step_x) {
            let pixel = panel.get_pixel(x, y);
            for channel in 0..3 {
                hash ^= pixel[channel] as u64;
                hash = hash.wrapping_mul(0x100_0000_01b3);
            }
        }
    }
    hash
}

/// 可被 `stop` 打断的等待：不要一觉睡到间隔结束才响应停止。
fn sleep_while(stop: &std::sync::atomic::AtomicBool, total_ms: u64) {
    let chunk = 200u64;
    let mut left = total_ms;
    while left > 0 {
        if stop.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        let step = left.min(chunk);
        std::thread::sleep(Duration::from_millis(step));
        left -= step;
    }
}

/// 后台积累：一直跑，识别到带坐标的任务就学；覆盖齐了就通知一次。
///
/// 和固定帧数采集的区别：不设终点，出错（窗口最小化/被关）不会退出，
/// 画面没变化会跳过，成果每轮回传，UI 随时可以停止。
fn spawn_accumulate(
    plan: JobPlan,
    ctx: egui::Context,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Receiver<JobMsg> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut learner = Learner::new(
            plan.cell_w,
            plan.cell_h,
            LearnConfig {
                min_similarity: plan.min_similarity,
                shift: 1,
            },
        );
        if let Some(seed) = &plan.seed {
            let _ = learner.seed(seed);
        }
        let mut attempts = 0u32;
        let mut frames_used = 0u32;
        let mut last_hash = 0u64;
        let mut complete_notified = false;
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            attempts += 1;
            let captured = (|| -> Result<image::RgbImage, String> {
                let full = capture::capture_client(plan.handle, plan.region)?;
                let mut panel = preprocess::apply(
                    capture::crop_task_region(&full, plan.region),
                    plan.pipeline,
                    plan.color,
                    plan.tolerance,
                );
                if plan.upscale > 1 {
                    panel = preprocess::upscale(&panel, plan.upscale);
                }
                Ok(panel)
            })();

            let panel = match captured {
                Ok(panel) => panel,
                Err(err) => {
                    let _ = tx.send(JobMsg::Progress {
                        attempts,
                        frames_used,
                        samples: learner.sample_count(),
                        learned: 0,
                        ocr: String::new(),
                        panel: None,
                        lib: learner.finish(),
                        summary: learner.summary(),
                        complete: false,
                        error: Some(err),
                        unchanged: false,
                    });
                    ctx.request_repaint();
                    sleep_while(&stop, plan.interval_ms);
                    continue;
                }
            };

            let hash = panel_hash(&panel);
            if hash == last_hash {
                let _ = tx.send(JobMsg::Progress {
                    attempts,
                    frames_used,
                    samples: learner.sample_count(),
                    learned: 0,
                    ocr: String::new(),
                    panel: None,
                    lib: learner.finish(),
                    summary: learner.summary(),
                    complete: false,
                    error: None,
                    unchanged: true,
                });
                ctx.request_repaint();
                sleep_while(&stop, plan.interval_ms);
                continue;
            }
            last_hash = hash;

            let ocr = match ocr::recognize_panel(&panel, plan.high_recall) {
                Ok(ocr) => ocr,
                Err(err) => {
                    let _ = tx.send(JobMsg::Progress {
                        attempts,
                        frames_used,
                        samples: learner.sample_count(),
                        learned: 0,
                        ocr: String::new(),
                        panel: Some(panel),
                        lib: learner.finish(),
                        summary: learner.summary(),
                        complete: false,
                        error: Some(err),
                        unchanged: false,
                    });
                    ctx.request_repaint();
                    sleep_while(&stop, plan.interval_ms);
                    continue;
                }
            };

            let mut skipped_regions = 0usize;
            let learned = learn_panel(
                &mut learner,
                &panel,
                &ocr,
                plan.learn_all,
                &mut skipped_regions,
                coord_fix::DEFAULT_PADDING,
                None,
            );
            if learned > 0 {
                frames_used += 1;
            }
            let lib = learner.finish();
            let complete = font_lib::missing_coordinate_labels(&lib).is_empty();
            let _ = tx.send(JobMsg::Progress {
                attempts,
                frames_used,
                samples: learner.sample_count(),
                learned,
                ocr: ocr.text.trim().replace('\n', " / "),
                panel: Some(panel),
                lib,
                summary: learner.summary(),
                complete,
                error: None,
                unchanged: false,
            });
            ctx.request_repaint();
            if complete && !complete_notified {
                complete_notified = true;
            }
            sleep_while(&stop, plan.interval_ms);
        }
        let _ = tx.send(JobMsg::AccStopped {
            lib: learner.finish(),
            summary: learner.summary(),
            samples: learner.sample_count(),
            frames_used,
            attempts,
        });
        ctx.request_repaint();
    });
    rx
}

// ---------------------------------------------------------------- 界面状态
// ---------------------------------------------------------------- 界面状态

struct Preview {
    /// 原始面板（点图取色、重新二值化都要用）。
    image: std::sync::Arc<image::RgbImage>,
    texture: egui::TextureHandle,
    binary_texture: egui::TextureHandle,
    size: egui::Vec2,
    /// 预览时切出来的字模（检查切字是否正常）。
    cells: Vec<Vec<u8>>,
    ocr: String,
}

impl Preview {
    fn new(ctx: &egui::Context, panel: image::RgbImage, ocr: String) -> Self {
        let image = std::sync::Arc::new(panel);
        let texture = texture_of(ctx, "preview", &image);
        let gray = font_lib::binarize(&image);
        let binary_texture = texture_of_gray(ctx, "preview-binary", &gray);
        let cells = cells_of(&gray);
        Self {
            size: egui::vec2(image.width() as f32, image.height() as f32),
            image,
            texture,
            binary_texture,
            cells,
            ocr,
        }
    }

    /// 画面变了就一起刷新两张图。
    fn update(&mut self, ctx: &egui::Context, panel: image::RgbImage, ocr: String) {
        *self = Self::new(ctx, panel, ocr);
    }
}

struct Picker {
    texture: egui::TextureHandle,
    image_size: egui::Vec2,
    start: Option<(f32, f32)>,
    current: Option<(f32, f32)>,
    /// 框选结束后要写回的窗口（和它的客户区）。
    candidate: Candidate,
}

struct Studio {
    defaults: CaptureDefaults,
    windows: Vec<capture::WindowInfo>,
    window_index: usize,
    dpi_line: String,

    region: core::TaskRegion,
    region_text: String,
    color_key: bool,
    grayscale: bool,
    binarize: bool,
    invert: bool,
    color: [u8; 3],
    color_hex: String,
    tolerance: u8,
    upscale: u32,
    high_recall: bool,

    frames: u32,
    interval_ms: u64,
    min_similarity: f32,
    learn_all: bool,

    lib_path: String,
    lib: GlyphLib,
    /// 预览显示二值化结果（看文字有没有被干净地提出来）。
    show_binary: bool,
    lib_dirty: bool,
    seed_existing: bool,
    selected_glyph: Option<usize>,
    relabel: String,

    job: Option<Job>,
    /// 后台积累模式（一直跑，直到手动停止）。
    acc: Option<AccState>,
    preview: Option<Preview>,
    picker: Option<Picker>,
    logs: VecDeque<String>,
    summary: Vec<font_lib::LabelSummary>,
}

impl Studio {
    fn new() -> Self {
        let defaults = read_capture_defaults();
        let pipeline = preprocess::Pipeline::from_config(&defaults.preprocess);
        let mut studio = Self {
            region: defaults.region,
            region_text: region_text(defaults.region),
            color_key: pipeline.color_key,
            grayscale: pipeline.grayscale,
            binarize: pipeline.binarize,
            invert: pipeline.invert,
            color: defaults.color,
            color_hex: format!(
                "#{:02x}{:02x}{:02x}",
                defaults.color[0], defaults.color[1], defaults.color[2]
            ),
            tolerance: defaults.tolerance,
            upscale: defaults.upscale,
            high_recall: defaults.high_recall,
            frames: 15,
            interval_ms: 1200,
            min_similarity: 0.9,
            learn_all: false,
            lib_path: default_lib_path().display().to_string(),
            lib: GlyphLib::new(font_lib::DEFAULT_CELL_W, font_lib::DEFAULT_CELL_H),
            show_binary: false,
            lib_dirty: false,
            seed_existing: true,
            selected_glyph: None,
            relabel: String::new(),
            job: None,
            acc: None,
            preview: None,
            picker: None,
            logs: VecDeque::new(),
            summary: Vec::new(),
            defaults,
            windows: Vec::new(),
            window_index: 0,
            dpi_line: String::new(),
        };
        studio.reload_windows();
        studio.load_lib();
        studio.log("提示：先刷新窗口、框好区域，再点「截图预览」确认画面，最后采集。");
        studio
    }

    fn log(&mut self, message: impl Into<String>) {
        let time = chrono::Local::now().format("%H:%M:%S");
        self.logs.push_back(format!("[{time}] {}", message.into()));
        while self.logs.len() > 200 {
            self.logs.pop_front();
        }
    }

    fn reload_windows(&mut self) {
        self.windows.clear();
        self.window_index = 0;
        self.dpi_line = match capture::dpi_info() {
            Some(dpi) => format!(
                "屏幕 {}x{}｜缩放 {}%｜{}",
                dpi.screen.0,
                dpi.screen.1,
                dpi.system_dpi * 100 / 96,
                dpi.awareness
            ),
            None => String::new(),
        };
        match game_windows() {
            Ok(windows) => {
                self.log(format!("找到 {} 个游戏窗口", windows.len()));
                self.windows = windows;
                // 默认选中第一个，区域按它的分辨率取预设。
                if let Some(window) = self.windows.first() {
                    if let Some(rect) = capture::client_rect(window.handle) {
                        let preset = self.defaults.region_for(Some((rect.2, rect.3)));
                        self.region = preset;
                        self.region_text = region_text(preset);
                    }
                }
            }
            Err(err) => self.log(err),
        }
    }

    fn current_window(&self) -> Option<&capture::WindowInfo> {
        self.windows.get(self.window_index)
    }

    fn pipeline(&self) -> preprocess::Pipeline {
        preprocess::Pipeline {
            color_key: self.color_key,
            grayscale: self.grayscale,
            binarize: self.binarize,
            invert: self.invert,
        }
    }

    fn load_lib(&mut self) {
        let path = PathBuf::from(self.lib_path.trim());
        match GlyphLib::load(&path) {
            Ok(lib) if !lib.is_empty() => {
                let labels = lib.label_counts();
                self.log(format!(
                    "已加载 {}：{} 个字模（{}）",
                    path.display(),
                    lib.len(),
                    labels
                        .iter()
                        .map(|(label, count)| format!("{label}×{count}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
                self.lib = lib;
                self.lib_dirty = false;
                self.selected_glyph = None;
            }
            Ok(_) => self.log(format!("{} 是空字库，先采集", path.display())),
            Err(err) => self.log(format!("没有加载字库：{err}")),
        }
    }

    fn save_lib(&mut self) {
        let path = PathBuf::from(self.lib_path.trim());
        if self.lib.is_empty() {
            self.log("字库是空的，没有可保存的内容");
            return;
        }
        match self.lib.save(&path) {
            Ok(()) => {
                self.lib_dirty = false;
                self.log(format!(
                    "已保存 {}：{} 个字模",
                    path.display(),
                    self.lib.len()
                ));
            }
            Err(err) => self.log(err),
        }
    }

    /// 抓一张面板做预览（不走 OCR），同时把切出来的字模也显示出来。
    fn refresh_preview(&mut self, ctx: &egui::Context) {
        let Some(window) = self.current_window().cloned() else {
            self.log("还没有选中窗口，点「刷新窗口」");
            return;
        };
        let captured = (|| -> Result<image::RgbImage, String> {
            let full = capture::capture_client(window.handle, self.region)?;
            let mut panel = preprocess::apply(
                capture::crop_task_region(&full, self.region),
                self.pipeline(),
                self.color,
                self.tolerance,
            );
            if self.upscale > 1 {
                panel = preprocess::upscale(&panel, self.upscale);
            }
            Ok(panel)
        })();
        match captured {
            Ok(panel) => {
                let gray = font_lib::binarize(&panel);
                let cells = cells_of(&gray);
                self.log(format!(
                    "预览：{}x{}，切出 {} 个字模",
                    panel.width(),
                    panel.height(),
                    cells.len()
                ));
                self.preview = Some(Preview::new(ctx, panel, String::new()));
            }
            Err(err) => self.log(err),
        }
    }

    fn start_pick(&mut self, ctx: &egui::Context) {
        let filter = self.current_window().map(|window| window.title.clone());
        let candidates = match candidates(filter.as_ref()) {
            Ok(candidates) => candidates,
            Err(err) => {
                self.log(err);
                return;
            }
        };
        let candidate = candidates[0].clone();
        match capture::capture_desktop() {
            Ok(desktop) => {
                let image_size = egui::vec2(desktop.width() as f32, desktop.height() as f32);
                self.picker = Some(Picker {
                    texture: texture_of(ctx, "desktop", &desktop),
                    image_size,
                    start: None,
                    current: None,
                    candidate,
                });
                self.log("拖动鼠标框住任务栏文字，Enter 确认、Esc 取消");
            }
            Err(err) => self.log(err),
        }
    }

    fn start_capture(&mut self, ctx: &egui::Context) {
        let Some(window) = self.current_window().cloned() else {
            self.log("还没有选中窗口，点「刷新窗口」");
            return;
        };
        let seed = if self.seed_existing && !self.lib.is_empty() {
            Some(self.lib.clone())
        } else {
            None
        };
        let plan = JobPlan {
            handle: window.handle,
            region: self.region,
            pipeline: self.pipeline(),
            color: self.color,
            tolerance: self.tolerance,
            upscale: self.upscale,
            high_recall: self.high_recall,
            frames: self.frames,
            interval_ms: self.interval_ms,
            min_similarity: self.min_similarity,
            learn_all: self.learn_all,
            cell_w: font_lib::DEFAULT_CELL_W,
            cell_h: font_lib::DEFAULT_CELL_H,
            seed,
        };
        self.log(format!(
            "开始采集：{} 帧，间隔 {} ms{}",
            plan.frames,
            plan.interval_ms,
            if plan.seed.is_some() {
                "（合并已有字库）"
            } else {
                ""
            }
        ));
        self.job = Some(Job {
            rx: spawn_job(plan, ctx.clone()),
            total: self.frames,
            done: 0,
            started: Instant::now(),
        });
        self.summary.clear();
    }

    fn poll_job(&mut self, ctx: &egui::Context) {
        let Some(job) = &self.job else {
            return;
        };
        let total = job.total;
        let mut messages = Vec::new();
        while let Ok(message) = job.rx.try_recv() {
            messages.push(message);
        }
        if messages.is_empty() {
            return;
        }
        let mut finished = false;
        for message in messages {
            match message {
                JobMsg::Frame {
                    index,
                    total,
                    learned,
                    ocr,
                    panel,
                } => {
                    if let Some(job) = &mut self.job {
                        job.done = index;
                    }
                    self.log(format!(
                        "第 {index}/{total} 帧：采到 {learned} 个字模｜OCR {}",
                        if ocr.is_empty() { "无文字" } else { &ocr }
                    ));
                    match &mut self.preview {
                        Some(preview) => preview.update(ctx, panel, ocr),
                        None => self.preview = Some(Preview::new(ctx, panel, ocr)),
                    }
                }
                JobMsg::Done {
                    lib,
                    summary,
                    conflicts,
                    samples,
                    frames_used,
                    skipped,
                } => {
                    finished = true;
                    self.log(format!(
                        "采集完成：{} 个字模，{} 个样本，来自 {frames_used}/{total} 帧{}",
                        lib.len(),
                        samples,
                        if skipped > 0 {
                            format!("，{skipped} 段字数对不上已跳过")
                        } else {
                            String::new()
                        }
                    ));
                    for warning in conflicts {
                        self.log(format!("提醒：{warning}"));
                    }
                    self.lib = lib;
                    self.lib_dirty = true;
                    self.selected_glyph = None;
                    self.summary = summary;
                }
                JobMsg::Failed(err) => {
                    finished = true;
                    self.log(format!("采集中断：{err}"));
                }
                // 后台积累的消息在 poll_acc 里处理。
                JobMsg::Progress { .. } | JobMsg::AccStopped { .. } => {}
            }
        }
        if finished {
            self.job = None;
        }
    }

    fn busy(&self) -> bool {
        self.job.is_some() || self.acc.is_some()
    }

    /// 开始后台积累：一直跑，直到点停止。
    fn start_accumulate(&mut self, ctx: &egui::Context) {
        let Some(window) = self.current_window().cloned() else {
            self.log("还没有选中窗口，点「刷新窗口」");
            return;
        };
        if self.job.is_some() {
            self.log("先等当前采集跑完");
            return;
        }
        let seed = if !self.lib.is_empty() {
            Some(self.lib.clone())
        } else {
            None
        };
        let plan = JobPlan {
            handle: window.handle,
            region: self.region,
            pipeline: self.pipeline(),
            color: self.color,
            tolerance: self.tolerance,
            upscale: self.upscale,
            high_recall: self.high_recall,
            frames: 0,
            interval_ms: self.interval_ms,
            min_similarity: self.min_similarity,
            learn_all: self.learn_all,
            cell_w: font_lib::DEFAULT_CELL_W,
            cell_h: font_lib::DEFAULT_CELL_H,
            seed,
        };
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.log(format!(
            "开始后台积累：每 {} ms 抓一帧，识别到带坐标的任务就学；覆盖齐了会提示",
            plan.interval_ms
        ));
        self.acc = Some(AccState {
            rx: spawn_accumulate(plan, ctx.clone(), stop.clone()),
            stop,
            attempts: 0,
            frames_used: 0,
            samples: 0,
            complete: false,
            started: Instant::now(),
        });
    }

    fn stop_accumulate(&mut self) {
        if let Some(acc) = &self.acc {
            acc.stop.store(true, std::sync::atomic::Ordering::Relaxed);
            self.log("正在停止后台积累…");
        }
    }

    /// 处理后台积累的回传：更新字库、覆盖状态和预览。
    fn poll_acc(&mut self, ctx: &egui::Context) {
        let Some(acc) = &self.acc else {
            return;
        };
        let mut messages = Vec::new();
        while let Ok(message) = acc.rx.try_recv() {
            messages.push(message);
        }
        if messages.is_empty() {
            return;
        }
        let mut stopped = false;
        for message in messages {
            match message {
                JobMsg::Progress {
                    attempts,
                    frames_used,
                    samples,
                    learned,
                    ocr,
                    panel,
                    lib,
                    summary,
                    complete,
                    error,
                    unchanged,
                } => {
                    if let Some(acc) = &mut self.acc {
                        acc.attempts = attempts;
                        acc.frames_used = frames_used;
                        acc.samples = samples;
                    }
                    let was_complete = self.acc.as_ref().is_some_and(|acc| acc.complete);
                    if let Some(acc) = &mut self.acc {
                        acc.complete = complete;
                    }
                    self.lib = lib;
                    self.lib_dirty = true;
                    self.summary = summary;
                    if complete && !was_complete {
                        self.log(
                            "覆盖齐了：0-9 和 , . 都有了 —— 可以点「保存」写盘，然后去主程序勾上复核",
                        );
                    }
                    if let Some(err) = error {
                        self.log(format!("这一轮跳过：{err}"));
                    } else if unchanged {
                        self.log("画面没变化，跳过");
                    } else if learned > 0 {
                        self.log(format!(
                            "第 {attempts} 轮：采到 {learned} 个字模（累计 {frames_used} 帧有收获、{samples} 个样本）｜OCR {}",
                            if ocr.is_empty() { "无文字" } else { &ocr }
                        ));
                    } else {
                        self.log(format!(
                            "第 {attempts} 轮：没采到字模｜OCR {}",
                            if ocr.is_empty() { "无文字" } else { &ocr }
                        ));
                    }
                    if let Some(panel) = panel {
                        match &mut self.preview {
                            Some(preview) => preview.update(ctx, panel, ocr),
                            None => self.preview = Some(Preview::new(ctx, panel, ocr)),
                        }
                    }
                }
                JobMsg::AccStopped {
                    lib,
                    summary,
                    samples,
                    frames_used,
                    attempts,
                } => {
                    stopped = true;
                    self.lib = lib;
                    self.lib_dirty = true;
                    self.summary = summary;
                    let elapsed = self
                        .acc
                        .as_ref()
                        .map(|acc| acc.started.elapsed().as_secs())
                        .unwrap_or(0);
                    self.log(format!(
                        "已停止后台积累：{attempts} 轮（{elapsed}s）、{frames_used} 帧有收获、{samples} 个样本；记得保存"
                    ));
                }
                _ => {}
            }
        }
        if stopped {
            self.acc = None;
        }
    }
    fn glyph_texture(
        &self,
        ctx: &egui::Context,
        pixels: &[u8],
        index: usize,
    ) -> egui::TextureHandle {
        let (w, h) = (self.lib.cell_w, self.lib.cell_h);
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for value in pixels {
            rgba.extend_from_slice(&[*value, *value, *value, 255]);
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
        // 用索引+内容长度当名字，字模改了自然会重建。
        ctx.load_texture(
            format!("glyph-{index}-{}", pixels.len()),
            image,
            egui::TextureOptions::NEAREST,
        )
    }
}

fn default_lib_path() -> PathBuf {
    crate::config::assets_dir().join("fontlib.json")
}

/// 把二值化后的面板切成一个个 16x24 字模（预览用，只关心形态）。
fn cells_of(gray: &image::GrayImage) -> Vec<Vec<u8>> {
    font_lib::analyze(gray, MatchConfig::default())
        .iter()
        .map(|b| font_lib::cell_of(gray, b, font_lib::DEFAULT_CELL_W, font_lib::DEFAULT_CELL_H))
        .collect()
}

fn texture_of(ctx: &egui::Context, name: &str, image: &image::RgbImage) -> egui::TextureHandle {
    let mut rgba = Vec::with_capacity((image.width() * image.height() * 4) as usize);
    for pixel in image.pixels() {
        rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
    }
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        &rgba,
    );
    ctx.load_texture(name, color, egui::TextureOptions::LINEAR)
}

impl eframe::App for Studio {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.06, 0.07, 0.09, 1.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll_job(&ctx);
        self.poll_acc(&ctx);
        // 采集中要持续刷新进度。
        if self.busy() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }

        if self.picker.is_some() {
            self.picker_ui(ui);
            return;
        }

        egui::Panel::top("toolbar")
            .frame(egui::Frame::new().inner_margin(8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("font-studio").strong());
                    ui.separator();
                    if ui.button("刷新窗口").clicked() {
                        self.reload_windows();
                    }
                    let current = self
                        .current_window()
                        .map(|window| window.title.clone())
                        .unwrap_or_else(|| "（没有可用窗口）".into());
                    egui::ComboBox::from_id_salt("window")
                        .selected_text(current)
                        .width(360.0)
                        .show_ui(ui, |ui| {
                            let titles: Vec<String> =
                                self.windows.iter().map(|w| w.title.clone()).collect();
                            for (index, title) in titles.iter().enumerate() {
                                ui.selectable_value(&mut self.window_index, index, title);
                            }
                        });
                    ui.separator();
                    ui.label(egui::RichText::new(&self.dpi_line).small().weak());
                });
            });

        egui::Panel::left("controls")
            .resizable(true)
            .default_size(360.0)
            .frame(egui::Frame::new().inner_margin(10))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    self.controls_ui(ui, &ctx);
                });
            });

        egui::Panel::bottom("log")
            .resizable(true)
            .default_size(150.0)
            .frame(egui::Frame::new().inner_margin(8))
            .show(ui, |ui| {
                ui.label(egui::RichText::new("日志").small().weak());
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.logs {
                            ui.label(egui::RichText::new(line).small().monospace());
                        }
                    });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::new().inner_margin(10))
            .show(ui, |ui| {
                self.preview_ui(ui, &ctx);
            });
    }
}

impl Studio {
    fn controls_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.label(egui::RichText::new("识别区域").strong());
        ui.horizontal(|ui| {
            if ui.button("框选区域").clicked() {
                self.start_pick(ctx);
            }
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.region_text)
                        .desired_width(180.0)
                        .hint_text("左,上,右,下（0~1）"),
                )
                .lost_focus()
            {
                match core::parse_region(&self.region_text) {
                    Some(region) => self.region = region,
                    None => self.log("区域格式不对：要 0~1 之间的四个百分比"),
                }
            }
        });
        ui.horizontal(|ui| {
            if ui.button("写回 config.json").clicked() {
                let size_key = self
                    .current_window()
                    .and_then(|window| capture::client_size(window.handle))
                    .map(|(width, height)| format!("{width}x{height}"))
                    .unwrap_or_default();
                if size_key.is_empty() {
                    self.log("读不到客户区尺寸，先刷新窗口");
                } else {
                    match write_config_region(&config_path(), &self.region_text, &size_key) {
                        Ok(()) => self.log(format!("已写入 config.json（{size_key}）")),
                        Err(err) => self.log(err),
                    }
                }
            }
            ui.label(egui::RichText::new("等价于主程序里的取点").small().weak());
        });

        ui.add_space(8.0);
        ui.label(egui::RichText::new("预处理").strong());
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            changed |= ui.checkbox(&mut self.color_key, "颜色提取").changed();
            changed |= ui.checkbox(&mut self.grayscale, "灰度").changed();
            changed |= ui.checkbox(&mut self.binarize, "二值化").changed();
            changed |= ui.checkbox(&mut self.invert, "反色").changed();
        });
        ui.horizontal(|ui| {
            ui.label("文字颜色");
            if ui
                .add(egui::TextEdit::singleline(&mut self.color_hex).desired_width(80.0))
                .changed()
            {
                match gate::parse_hex(&self.color_hex) {
                    Some(color) => {
                        self.color = color;
                        changed = true;
                    }
                    None => self.log("颜色要 #rrggbb"),
                }
            }
            ui.add(
                egui::Slider::new(&mut self.tolerance, 0..=80)
                    .text("容差")
                    .clamping(egui::SliderClamping::Always),
            );
        });
        ui.horizontal(|ui| {
            ui.label("放大");
            for value in 1..=3 {
                changed |= ui
                    .selectable_value(&mut self.upscale, value, format!("{value}x"))
                    .changed();
            }
            changed |= ui.checkbox(&mut self.high_recall, "高召回").changed();
        });
        ui.horizontal(|ui| {
            if ui.button("截图预览").clicked() {
                self.refresh_preview(ctx);
            }
            ui.label(
                egui::RichText::new(self.pipeline().summary())
                    .small()
                    .weak(),
            );
        });
        if changed {
            self.log(format!("预处理：{}", self.pipeline().summary()));
        }

        ui.add_space(8.0);
        ui.label(egui::RichText::new("采集").strong());
        ui.horizontal(|ui| {
            ui.add(egui::Slider::new(&mut self.frames, 1..=60).text("帧数"));
            ui.add(egui::Slider::new(&mut self.interval_ms, 200..=3000).text("间隔ms"));
        });
        ui.horizontal(|ui| {
            ui.add(egui::Slider::new(&mut self.min_similarity, 0.6..=0.99).text("聚类相似度"));
            ui.checkbox(&mut self.learn_all, "连中文一起学");
        });
        ui.checkbox(&mut self.seed_existing, "合并已有字库（不勾=从零重建）");
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.busy(), egui::Button::new("开始采集"))
                .clicked()
            {
                self.start_capture(ctx);
            }
            if self.busy() {
                ui.spinner();
                if let Some(job) = &self.job {
                    ui.label(format!(
                        "{}/{}（{:.0}s）",
                        job.done,
                        job.total,
                        job.started.elapsed().as_secs_f32()
                    ));
                }
            }
        });
        ui.label(
            egui::RichText::new(
                "采集时任务栏里要有带坐标的任务（抓鬼/鬼王/降妖…），否则学不到数字。",
            )
            .small()
            .weak(),
        );

        ui.add_space(6.0);
        ui.separator();
        ui.horizontal(|ui| {
            if self.acc.is_none() {
                if ui
                    .add_enabled(self.job.is_none(), egui::Button::new("后台积累"))
                    .on_hover_text("不设终点一直跑：识别到带坐标的任务就学，画面没变化自动跳过")
                    .clicked()
                {
                    self.start_accumulate(ctx);
                }
            } else if ui.button("停止积累").clicked() {
                self.stop_accumulate();
            }
            if let Some(acc) = &self.acc {
                ui.spinner();
                ui.label(format!(
                    "{} 轮 / 有收获 {} 帧 / {} 样本（{}s）",
                    acc.attempts,
                    acc.frames_used,
                    acc.samples,
                    acc.started.elapsed().as_secs()
                ));
            }
        });
        if self.acc.as_ref().is_some_and(|acc| acc.complete) {
            ui.label(
                egui::RichText::new("覆盖齐了：记得点「保存」写盘")
                    .color(OK_GREEN)
                    .strong(),
            );
        }
        ui.label(
            egui::RichText::new(
                "后台积累适合边做任务边攒：窗口最小化/被盖住只会跳过那一轮，不会退出。",
            )
            .small()
            .weak(),
        );

        ui.add_space(8.0);
        ui.label(egui::RichText::new("字库").strong());
        ui.horizontal(|ui| {
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.lib_path)
                        .desired_width(230.0)
                        .hint_text("assets/fontlib.json"),
                )
                .lost_focus()
            {
                self.load_lib();
            }
            if ui.button("读取").clicked() {
                self.load_lib();
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.lib.is_empty(), egui::Button::new("保存"))
                .clicked()
            {
                self.save_lib();
            }
            if self.lib_dirty {
                ui.label(egui::RichText::new("有未保存的改动").small().color(EARLY));
            }
            if ui.button("清空").clicked() {
                self.lib = GlyphLib::new(font_lib::DEFAULT_CELL_W, font_lib::DEFAULT_CELL_H);
                self.lib_dirty = true;
                self.selected_glyph = None;
                self.summary.clear();
                self.log("已清空字库（还没写盘）");
            }
        });
        ui.label(format!(
            "{} 个字模，{}x{}",
            self.lib.len(),
            self.lib.cell_w,
            self.lib.cell_h
        ));
        match coverage_report(&self.lib) {
            Ok(note) => {
                ui.label(egui::RichText::new(format!("覆盖：{note}")).color(OK_GREEN));
            }
            Err(missing) => {
                ui.label(
                    egui::RichText::new(format!("还缺坐标字符：{missing}"))
                        .color(WARN)
                        .strong(),
                );
            }
        }
        if !self.summary.is_empty() {
            ui.add_space(4.0);
            ui.label(
                egui::RichText::new(
                    self.summary
                        .iter()
                        .map(|item| format!("{}×{}（{}票）", item.label, item.clusters, item.votes))
                        .collect::<Vec<_>>()
                        .join("  "),
                )
                .small()
                .weak(),
            );
        }
    }

    fn preview_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let mut picked_color: Option<[u8; 3]> = None;
        if let Some(preview) = &self.preview {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.show_binary, "显示二值化");
                ui.label(
                    egui::RichText::new("在图上点一下可以取文字颜色（自动勾选颜色提取）")
                        .small()
                        .weak(),
                );
            });
            let available = ui.available_width().min(preview.size.x.max(1.0));
            let scale = (available / preview.size.x).clamp(0.2, 3.0);
            let size = preview.size * scale;
            let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
            let texture = if self.show_binary {
                preview.binary_texture.id()
            } else {
                preview.texture.id()
            };
            ui.painter().image(
                texture,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            if response.clicked() {
                if let Some(pos) = response.interact_pointer_pos() {
                    let x = ((pos.x - rect.min.x) / scale) as u32;
                    let y = ((pos.y - rect.min.y) / scale) as u32;
                    if let Some(pixel) = preview.image.get_pixel_checked(x, y) {
                        picked_color = Some([pixel[0], pixel[1], pixel[2]]);
                    }
                }
            }
            ui.add_space(6.0);
            if !preview.ocr.is_empty() {
                ui.label(format!("OCR：{}", preview.ocr));
            }
            if !preview.cells.is_empty() {
                ui.label(
                    egui::RichText::new(format!(
                        "切出 {} 个字模（每个格子应该正好一个字）",
                        preview.cells.len()
                    ))
                    .small()
                    .weak(),
                );
                ui.horizontal_wrapped(|ui| {
                    for (index, cell) in preview.cells.iter().enumerate() {
                        let texture = self.glyph_texture(ctx, cell, index);
                        let (cell_rect, _) = ui.allocate_exact_size(
                            egui::vec2(
                                font_lib::DEFAULT_CELL_W as f32 * 2.0,
                                font_lib::DEFAULT_CELL_H as f32 * 2.0,
                            ),
                            egui::Sense::hover(),
                        );
                        ui.painter().image(
                            texture.id(),
                            cell_rect,
                            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                });
            }
        } else {
            ui.label("还没有预览：左边点「截图预览」抓一张看看区域和预处理对不对。");
        }
        if let Some(color) = picked_color {
            self.color = color;
            self.color_hex = format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]);
            if !self.color_key {
                self.color_key = true;
                self.log("已勾选「颜色提取」，点「截图预览」看效果（容差可调）");
            }
            self.log(format!("取色：{}", self.color_hex));
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("字库字模").strong());
            let note = if self.acc.is_some() {
                "积累中：每轮会回传最新字库，停止后才能改标签/删除"
            } else {
                "点一个字模可改标签或删除，错的样本千万别留着"
            };
            ui.label(egui::RichText::new(note).small().weak());
        });
        if self.lib.is_empty() {
            ui.label("空字库：采集后这里会显示每个学到的字模。");
            return;
        }
        let locked = self.acc.is_some();
        let mut delete: Option<usize> = None;
        let mut relabel: Option<(usize, String)> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (index, glyph) in self.lib.glyphs.iter().enumerate() {
                    let texture = self.glyph_texture(ctx, &glyph.pixels, index);
                    let selected = self.selected_glyph == Some(index);
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(self.lib.cell_w as f32 * 3.0, self.lib.cell_h as f32 * 3.0),
                        egui::Sense::click(),
                    );
                    if response.clicked() {
                        self.selected_glyph = Some(index);
                        self.relabel = glyph.label.clone();
                    }
                    ui.painter().image(
                        texture.id(),
                        rect,
                        egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                    if selected {
                        ui.painter().rect_stroke(
                            rect,
                            2.0,
                            egui::Stroke::new(2.0, OK_GREEN),
                            egui::StrokeKind::Outside,
                        );
                    }
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new(&glyph.label).strong());
                        if ui
                            .add_enabled(!locked, egui::Button::new("删").small())
                            .clicked()
                        {
                            delete = Some(index);
                        }
                    });
                }
            });
        });
        if let Some(index) = self.selected_glyph {
            ui.separator();
            ui.add_enabled_ui(!locked, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!("选中第 {index} 个字模，标签："));
                    ui.add(egui::TextEdit::singleline(&mut self.relabel).desired_width(40.0));
                    if ui.button("改标签").clicked() {
                        let label: String = self.relabel.trim().chars().take(1).collect();
                        if !label.is_empty() {
                            relabel = Some((index, label));
                        }
                    }
                    if ui.button("删除选中").clicked() {
                        delete = Some(index);
                    }
                });
            });
        }
        if let Some((index, label)) = relabel {
            if let Some(glyph) = self.lib.glyphs.get_mut(index) {
                glyph.label = label;
                self.lib_dirty = true;
                self.log(format!("第 {index} 个字模改标签为「{}」", self.relabel));
            }
        }
        if let Some(index) = delete {
            if index < self.lib.glyphs.len() {
                let removed = self.lib.glyphs.remove(index);
                self.lib_dirty = true;
                self.selected_glyph = None;
                self.log(format!("删除字模「{}」", removed.label));
            }
        }
    }

    fn picker_ui(&mut self, ui: &mut egui::Ui) {
        let Some(picker) = &mut self.picker else {
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        egui::Panel::top("picker_bar")
            .frame(egui::Frame::new().inner_margin(8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "框住任务栏文字（窗口：{}）",
                        picker.candidate.title
                    ));
                    ui.separator();
                    match selection_of(picker) {
                        Some(selection) => {
                            match region_from_selection(selection, picker.candidate.rect) {
                                Ok(region) => ui.label(format!("区域 {}", region_text(region))),
                                Err(err) => ui.label(egui::RichText::new(err).color(WARN)),
                            };
                        }
                        None => {
                            ui.label("（还没框选）");
                        }
                    }
                    ui.separator();
                    confirm = ui.button("确认 (Enter)").clicked();
                    cancel = ui.button("取消 (Esc)").clicked();
                });
            });
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::BLACK)
                    .inner_margin(6),
            )
            .show(ui, |ui| {
                let available = ui.available_size();
                let scale = (available.x / picker.image_size.x)
                    .min(available.y / picker.image_size.y)
                    .min(1.0);
                let size = picker.image_size * scale;
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
                let to_desktop =
                    |pos: egui::Pos2| ((pos.x - rect.min.x) / scale, (pos.y - rect.min.y) / scale);
                if response.drag_started() {
                    picker.start = response.interact_pointer_pos().map(to_desktop);
                    picker.current = picker.start;
                } else if response.dragged() {
                    picker.current = response.interact_pointer_pos().map(to_desktop);
                }
                ui.painter().image(
                    picker.texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                if let Some((x0, y0, x1, y1)) = selection_of(picker) {
                    let selected = egui::Rect::from_min_max(
                        rect.min + egui::vec2(x0 * scale, y0 * scale),
                        rect.min + egui::vec2(x1 * scale, y1 * scale),
                    );
                    ui.painter().rect_stroke(
                        selected,
                        0.0,
                        egui::Stroke::new(2.0, OK_GREEN),
                        egui::StrokeKind::Outside,
                    );
                }
            });

        let ctx = ui.ctx().clone();
        if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
            confirm = true;
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }
        if confirm {
            if let Some(selection) = self.picker.as_ref().and_then(selection_of) {
                let candidate = self.picker.as_ref().map(|p| p.candidate.clone());
                if let Some(candidate) = candidate {
                    match region_from_selection(selection, candidate.rect) {
                        Ok(region) => {
                            self.region = region;
                            self.region_text = region_text(region);
                            self.log(format!("区域改为 {}", self.region_text));
                            self.picker = None;
                        }
                        Err(err) => {
                            self.log(err);
                        }
                    }
                }
            }
        } else if cancel {
            self.picker = None;
            self.log("已取消框选");
        }
    }
}

fn selection_of(picker: &Picker) -> Option<Selection> {
    let (start_x, start_y) = picker.start?;
    let (current_x, current_y) = picker.current?;
    let selection = (
        start_x.min(current_x),
        start_y.min(current_y),
        start_x.max(current_x),
        start_y.max(current_y),
    );
    (selection.2 - selection.0 >= 4.0 && selection.3 - selection.1 >= 4.0).then_some(selection)
}

const OK_GREEN: egui::Color32 = egui::Color32::from_rgb(0x4c, 0xd9, 0x8a);
const WARN: egui::Color32 = egui::Color32::from_rgb(0xf0, 0xb4, 0x4c);
const EARLY: egui::Color32 = egui::Color32::from_rgb(0x8a, 0xb4, 0xf8);

#[cfg(test)]
mod tests {
    use super::*;
    use font_lib::fixture;

    /// 合成一块面板 + 一份“OCR 结果”，验证采集核心：裁图→切字→打标签→学习。
    #[test]
    fn learns_glyphs_from_one_panel() {
        let line = fixture::render("79,26", 5, 1);
        let mut panel = image::RgbImage::from_pixel(220, 60, image::Rgb([255, 255, 255]));
        image::imageops::overlay(&mut panel, &line, 20, 10);
        let region = ocr::OcrRegion {
            text: "79,26".into(),
            score: 0.95,
            left: 20.0,
            top: 10.0,
            right: 20.0 + line.width() as f32,
            bottom: 10.0 + line.height() as f32,
            range: 0..5,
        };
        let ocr = ocr::OcrText {
            text: "79,26".into(),
            elapsed_secs: 0.0,
            regions: vec![region],
            notes: Vec::new(),
        };
        let mut learner = Learner::new(
            font_lib::DEFAULT_CELL_W,
            font_lib::DEFAULT_CELL_H,
            LearnConfig::default(),
        );
        let mut skipped = 0usize;
        let learned = learn_panel(
            &mut learner,
            &panel,
            &ocr,
            false,
            &mut skipped,
            coord_fix::DEFAULT_PADDING,
            None,
        );
        assert_eq!(learned, 5, "应当学到 5 个字模");
        assert_eq!(skipped, 0);
        let labels: Vec<String> = learner.summary().into_iter().map(|s| s.label).collect();
        assert_eq!(labels, [",", "2", "6", "7", "9"]);
    }

    /// OCR 只认出一半、切字数对不上时，宁可跳过，也不打错标签。
    #[test]
    fn skips_regions_whose_cell_count_disagrees() {
        let line = fixture::render("79,26", 5, 1);
        let mut panel = image::RgbImage::from_pixel(220, 60, image::Rgb([255, 255, 255]));
        image::imageops::overlay(&mut panel, &line, 20, 10);
        let region = ocr::OcrRegion {
            text: "79".into(),
            score: 0.9,
            left: 20.0,
            top: 10.0,
            right: 20.0 + line.width() as f32,
            bottom: 10.0 + line.height() as f32,
            range: 0..2,
        };
        let ocr = ocr::OcrText {
            text: "79".into(),
            elapsed_secs: 0.0,
            regions: vec![region],
            notes: Vec::new(),
        };
        let mut learner = Learner::new(
            font_lib::DEFAULT_CELL_W,
            font_lib::DEFAULT_CELL_H,
            LearnConfig::default(),
        );
        let mut skipped = 0usize;
        assert_eq!(
            learn_panel(
                &mut learner,
                &panel,
                &ocr,
                false,
                &mut skipped,
                coord_fix::DEFAULT_PADDING,
                None
            ),
            0
        );
        assert_eq!(skipped, 1);
        assert_eq!(learner.cluster_count(), 0);
    }

    #[test]
    fn panel_hash_is_stable_and_sensitive() {
        let a = image::RgbImage::from_pixel(120, 40, image::Rgb([255, 255, 255]));
        let mut b = a.clone();
        assert_eq!(panel_hash(&a), panel_hash(&b));
        // 改一个会被采样到的像素（y 每 3 行采一次）。
        b.put_pixel(60, 21, image::Rgb([0, 0, 0]));
        assert_ne!(panel_hash(&a), panel_hash(&b), "画面变了指纹就得变");
    }

    #[test]
    fn sleep_while_returns_immediately_when_stopped() {
        use std::sync::atomic::AtomicBool;
        let stop = AtomicBool::new(true);
        let started = Instant::now();
        sleep_while(&stop, 5_000);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "停止要立即生效"
        );
    }
}

/// 二值化图的贴图（黑白，单通道存成灰度 RGBA）。
fn texture_of_gray(
    ctx: &egui::Context,
    name: &str,
    image: &image::GrayImage,
) -> egui::TextureHandle {
    let mut rgba = Vec::with_capacity((image.width() * image.height() * 4) as usize);
    for pixel in image.pixels() {
        let value = pixel[0];
        rgba.extend_from_slice(&[value, value, value, 255]);
    }
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [image.width() as usize, image.height() as usize],
        &rgba,
    );
    ctx.load_texture(name, color, egui::TextureOptions::NEAREST)
}
