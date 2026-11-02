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
}

struct Job {
    rx: Receiver<JobMsg>,
    total: u32,
    done: u32,
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
        let cfg = MatchConfig::default();
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
            let mut learned = 0usize;
            for region in &ocr.regions {
                let Some((x, y, width, height)) = coord_fix::padded_box(region, 2, &panel) else {
                    continue;
                };
                let Ok(crop) = font_lib::crop(&panel, x, y, width, height) else {
                    continue;
                };
                let gray = font_lib::binarize(&crop);
                let cells = font_lib::analyze(&gray, cfg);
                let Some(labels) = labels_for(&region.text, cells.len(), plan.learn_all) else {
                    skipped += 1;
                    continue;
                };
                for (b, label) in cells.iter().zip(labels) {
                    let Some(label) = label else {
                        continue;
                    };
                    learner.observe(
                        font_lib::cell_of(&gray, b, plan.cell_w, plan.cell_h),
                        &label.to_string(),
                    );
                    learned += 1;
                }
            }
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

// ---------------------------------------------------------------- 界面状态

struct Preview {
    texture: egui::TextureHandle,
    size: egui::Vec2,
    /// 预览时切出来的字模（检查切字是否正常）。
    cells: Vec<Vec<u8>>,
    ocr: String,
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
    lib_dirty: bool,
    seed_existing: bool,
    selected_glyph: Option<usize>,
    relabel: String,

    job: Option<Job>,
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
            lib_dirty: false,
            seed_existing: true,
            selected_glyph: None,
            relabel: String::new(),
            job: None,
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
                self.preview = Some(Preview {
                    texture: texture_of(ctx, "preview", &panel),
                    size: egui::vec2(panel.width() as f32, panel.height() as f32),
                    cells,
                    ocr: String::new(),
                });
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
                    if let Some(preview) = &mut self.preview {
                        preview.texture = texture_of(ctx, "preview", &panel);
                        preview.ocr = ocr;
                    } else {
                        let gray = font_lib::binarize(&panel);
                        let cells = cells_of(&gray);
                        self.preview = Some(Preview {
                            texture: texture_of(ctx, "preview", &panel),
                            size: egui::vec2(panel.width() as f32, panel.height() as f32),
                            cells,
                            ocr,
                        });
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
            }
        }
        if finished {
            self.job = None;
        }
    }

    fn busy(&self) -> bool {
        self.job.is_some()
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
        if let Some(preview) = &self.preview {
            let available = ui.available_width().min(preview.size.x.max(1.0));
            let scale = (available / preview.size.x).clamp(0.2, 3.0);
            let size = preview.size * scale;
            let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            ui.painter().image(
                preview.texture.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            ui.add_space(6.0);
            if !preview.ocr.is_empty() {
                ui.label(format!("OCR：{}", preview.ocr));
            }
            if !preview.cells.is_empty() {
                ui.label(
                    egui::RichText::new(format!("切出 {} 个字模", preview.cells.len()))
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

        ui.separator();
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("字库字模").strong());
            ui.label(
                egui::RichText::new("点一个字模可改标签或删除，错的样本千万别留着")
                    .small()
                    .weak(),
            );
        });
        if self.lib.is_empty() {
            ui.label("空字库：采集后这里会显示每个学到的字模。");
            return;
        }
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
                        if ui.small_button("删").clicked() {
                            delete = Some(index);
                        }
                    });
                }
            });
        });
        if let Some(index) = self.selected_glyph {
            ui.separator();
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
