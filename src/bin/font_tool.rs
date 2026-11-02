//! 字库工具：采集 / 建库 / 读字，不启动 UI。
//!
//! ```text
//! cargo run --bin font-tool -- capture --out assets/fontlib.json
//! cargo run --bin font-tool -- build   --samples samples --out assets/fontlib.json
//! cargo run --bin font-tool -- read    --lib assets/fontlib.json --image panel.png
//! ```
//!
//! `capture` 是给「不想手动裁剪+手打标注」准备的：抓当前游戏窗口的任务栏截图，
//! 用本地 PP-OCR 拿到文字和检测框，再按框切成字模、用 OCR 文字当标签，
//! 多帧聚类投票后直接写出字库。切片/归一化和运行时复核完全同一套代码。
//!
//! 为了不把整个 config / meditation / update 依赖链拖进来，这里只按 `#[path]`
//! 引入采集需要的模块，并给 `ocr` 提供一个最小的 `config::assets_dir` 实现。

// 这些模块是从主程序共享过来的，工具只用到其中一小部分，不必为工具裁剪。
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
/// 和 `font-studio` 共用的窗口 / 选区 / 配置逻辑。
#[path = "../tool_shared.rs"]
mod shared;

use shared::*;

/// `ocr.rs` 只用它定位随包的 PP-OCR 模型；这里给一个最小实现，
/// 免得为了一个命令行工具把整个 config 模块链都编译进来。
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
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn usage() {
    eprintln!(
        "用法：
  font-tool windows
  font-tool pick    [--window 梦幻西游] [--out <校验小图.png>] [--write-config]
  font-tool crop    --image <大图.png> --crop x,y,w,h --out <小图.png>
  font-tool info    --lib <字库.json>
  font-tool verify  --lib <字库.json> [--frames 6] [--interval 900]
  font-tool sheet   --lib <字库.json> --out <对照图.png> [--scale 3]
  font-tool region  --region 左,上,右,下 [--size 1218x994] [--config <config.json>]
                    [--window 梦幻西游] [--dry-run]
  font-tool snap    --out <面板.png> [--desktop] [--window 梦幻西游] [--region 左,上,右,下]
                    [--preprocess off|color,gray,binary,invert] [--color #eaff65]
                    [--tolerance 24] [--upscale 2]
  font-tool capture --out <字库.json> [--frames 12] [--interval 700]
                    [--window 梦幻西游] [--region 左,上,右,下]
                    [--preprocess off|color,gray,binary,invert] [--color #eaff65]
                    [--tolerance 24] [--upscale 2] [--min-similarity 0.9]
                    [--cell 16x24] [--all] [--dump <目录>]
  font-tool build   --samples <目录> --out <字库.json> [--cell 16x24]
  font-tool read    --lib <字库.json> --image <图片.png> [--line] [--crop x,y,w,h]
                    [--save-binary <二值化.png>]

snap   ：只截一张面板存 PNG，不跑 OCR——调区域/预处理时秒出结果。
windows：列出可截图的窗口和客户区坐标，排查位置不对时先看它。
pick   ：整屏截图 + 拖拽框选，自动算出相对客户区的百分比区域；
         默认只打印结果并拍一张校验图，--write-config 直接写回 config.json。
crop   ：从图片里裁一块存出来。
region ：不开界面直接写区域：更新 ocr/task_region 和该分辨率的预设；
         --dry-run 只打印改完的配置，不落盘。
         （位置还不对就用 snap --region 0,0,1,1 截整块客户区看。）
capture：自动采集——截当前游戏窗口、OCR 拿标签、多帧聚类投票后直接写字库。
         默认只学数字和分隔符（坐标用），--all 连中文一起学。
         不给 --region/--preprocess 时，读 config.json 里的现有设置（含分辨率预设）。
         目标写库已存在时默认合并（累积补全数字），--fresh 从零重建。
info   ：体检字库：字符覆盖 / 每个字的字模数 / 还缺哪些坐标字符。
sheet  ：把字库渲染成对照图（一行一个标签），肉眼检查标签对不对。
build  ：目录下每个 xxx.png（一行文字的截图）+ 同名 xxx.txt（这行的准确文字）。
read   ：默认按行切分整块面板；--line 表示整张图就是一行。"
    );
}

fn main() {
    // 必须在任何窗口/截图之前：系统开了缩放时，不感知 DPI 的进程拿到的是「逻辑像素」，
    // 而主程序（egui/winit）是感知 DPI 的，两边的客户区尺寸/坐标会差一个缩放倍数。
    enable_dpi_awareness();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("windows") => windows(&args[1..]),
        Some("pick") => pick(&args[1..]),
        Some("crop") => crop_image(&args[1..]),
        Some("region") => region(&args[1..]),
        Some("info") => info(&args[1..]),
        Some("verify") => verify(&args[1..]),
        Some("debug-rows") => debug_rows(&args[1..]),
        Some("sheet") => sheet(&args[1..]),
        Some("align") => align(&args[1..]),
        Some("snap") => snap(&args[1..]),
        Some("capture") => capture_frames(&args[1..]),
        Some("build") => build(&args[1..]),
        Some("read") => read(&args[1..]),
        Some("help") | Some("-h") | Some("--help") | None => {
            usage();
            return;
        }
        Some(other) => {
            eprintln!("未知子命令：{other}");
            usage();
            std::process::exit(2);
        }
    };
    if let Err(err) = result {
        eprintln!("错误：{err}");
        std::process::exit(1);
    }
}

// ------------------------------------------------------------------ 参数解析

fn parse_flags(args: &[String]) -> HashMap<String, String> {
    let mut flags = HashMap::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        let Some(name) = arg.strip_prefix("--") else {
            index += 1;
            continue;
        };
        let (name, value) = match name.split_once('=') {
            Some((name, value)) => (name.to_string(), Some(value.to_string())),
            None => (name.to_string(), None),
        };
        match value {
            Some(value) => {
                flags.insert(name, value);
                index += 1;
            }
            None => {
                let next = args.get(index + 1).filter(|next| !next.starts_with("--"));
                flags.insert(name, next.cloned().unwrap_or_else(|| "1".to_string()));
                index += if next.is_some() { 2 } else { 1 };
            }
        }
    }
    flags
}

fn required(flags: &HashMap<String, String>, name: &str) -> Result<String, String> {
    flags
        .get(name)
        .cloned()
        .ok_or_else(|| format!("缺少 --{name}（看 font-tool --help）"))
}

fn number<T: std::str::FromStr>(
    flags: &HashMap<String, String>,
    name: &str,
    default: T,
) -> Result<T, String> {
    match flags.get(name) {
        Some(raw) => raw
            .parse::<T>()
            .map_err(|_| format!("--{name} 取值不合法：{raw}")),
        None => Ok(default),
    }
}

fn cell_size(flags: &HashMap<String, String>) -> Result<(u32, u32), String> {
    let Some(raw) = flags.get("cell") else {
        return Ok((font_lib::DEFAULT_CELL_W, font_lib::DEFAULT_CELL_H));
    };
    let (width, height) = raw
        .split_once(['x', 'X', '*'])
        .ok_or_else(|| format!("--cell 需要 宽x高，例如 16x24：{raw}"))?;
    let width = width
        .parse::<u32>()
        .map_err(|err| format!("--cell 宽解析失败：{err}"))?;
    let height = height
        .parse::<u32>()
        .map_err(|err| format!("--cell 高解析失败：{err}"))?;
    if width == 0 || height == 0 {
        return Err("--cell 不能为 0".into());
    }
    Ok((width, height))
}

// -------------------------------------------------------------------- 采集

/// 一次采集需要的全部参数（窗口 + 区域 + 预处理）。`capture` 和 `snap` 共用。
struct CapturePlan {
    window_title: String,
    handle: i64,
    client_size: Option<(u32, u32)>,
    region: core::TaskRegion,
    region_source: String,
    pipeline: preprocess::Pipeline,
    color: [u8; 3],
    tolerance: u8,
    upscale: u32,
    high_recall: bool,
}

impl CapturePlan {
    fn prepare(flags: &HashMap<String, String>) -> Result<Self, String> {
        let defaults = read_capture_defaults();
        let windows = game_windows()?;
        let window = pick_window(&windows, flags.get("window"))?;
        let client_size = capture::client_size(window.handle);
        let (region, region_source) = match flags.get("region") {
            Some(spec) => (
                core::parse_region(spec).ok_or_else(|| {
                    format!(
                        "--region 需要 0~1 之间的 左,上,右,下（百分比），例如 0.70,0.24,0.99,0.70：{spec}"
                    )
                })?,
                "--region".to_string(),
            ),
            None => {
                let preset = client_size
                    .map(|(width, height)| format!("{width}x{height}"))
                    .filter(|key| defaults.region_presets.contains_key(key));
                match preset {
                    Some(key) => (
                        defaults.region_for(client_size),
                        format!("config.json 的 {key} 预设"),
                    ),
                    None => (defaults.region, "config.json 的 ocr/task_region".to_string()),
                }
            }
        };
        let pipeline = preprocess::Pipeline::from_config(
            flags
                .get("preprocess")
                .map(String::as_str)
                .unwrap_or(&defaults.preprocess),
        );
        let color = match flags.get("color") {
            Some(spec) => {
                gate::parse_hex(spec).ok_or_else(|| format!("--color 需要 #rrggbb：{spec}"))?
            }
            None => defaults.color,
        };
        let tolerance: u8 = number(flags, "tolerance", defaults.tolerance)?;
        let upscale: u32 = number(flags, "upscale", defaults.upscale)?.clamp(1, 3);
        Ok(Self {
            window_title: window.title.clone(),
            handle: window.handle,
            client_size,
            region,
            region_source,
            pipeline,
            color,
            tolerance,
            upscale,
            high_recall: defaults.high_recall,
        })
    }

    /// 截一张面板（已预处理 + 放大），和主程序送 OCR 的图完全同一条流水线。
    fn grab(&self) -> Result<image::RgbImage, String> {
        let full = capture::capture_client(self.handle, self.region)?;
        let mut panel = preprocess::apply(
            capture::crop_task_region(&full, self.region),
            self.pipeline,
            self.color,
            self.tolerance,
        );
        if self.upscale > 1 {
            panel = preprocess::upscale(&panel, self.upscale);
        }
        Ok(panel)
    }

    fn describe(&self) -> String {
        format!(
            "窗口：{}（客户区 {:?}）｜区域 {:?}（来自 {}）｜预处理 {}｜放大 {}x",
            self.window_title,
            self.client_size,
            self.region,
            self.region_source,
            self.pipeline.summary(),
            self.upscale
        )
    }
}

/// 从图片里裁一块存出来（排查坐标/区域时用）。
fn crop_image(args: &[String]) -> Result<(), String> {
    let flags = parse_flags(args);
    let source: PathBuf = required(&flags, "image")?.into();
    let out: PathBuf = required(&flags, "out")?.into();
    let spec = required(&flags, "crop")?;
    let parts: Vec<u32> = spec
        .split(',')
        .filter_map(|part| part.trim().parse().ok())
        .collect();
    if parts.len() != 4 {
        return Err(format!("--crop 需要 x,y,w,h：{spec}"));
    }
    let image = load_rgb(&source)?;
    let cropped = font_lib::crop(&image, parts[0], parts[1], parts[2], parts[3])?;
    cropped
        .save(&out)
        .map_err(|err| format!("写入 {} 失败：{err}", out.display()))?;
    println!(
        "已保存 {}（{}x{}，取自 {}）",
        out.display(),
        cropped.width(),
        cropped.height(),
        source.display()
    );
    Ok(())
}

// ------------------------------------------------------------------ 框选取点

/// 整屏截图 + 拖拽框选，算出相对客户区的百分比区域，并当场校验一次。
fn pick(args: &[String]) -> Result<(), String> {
    let flags = parse_flags(args);
    let filter = flags.get("window").cloned();
    let mut candidates: Vec<Candidate> = Vec::new();
    for window in game_windows()? {
        if filter
            .as_deref()
            .is_some_and(|needle| !window.title.contains(needle))
        {
            continue;
        }
        if let Some(rect) = capture::client_rect(window.handle) {
            candidates.push(Candidate {
                handle: window.handle,
                title: window.title,
                rect,
            });
        }
    }
    candidates.sort_by(|a, b| b.area().total_cmp(&a.area()));
    if candidates.is_empty() {
        return Err("没有找到带客户区坐标的窗口（非 Windows 平台不支持）".into());
    }
    println!("候选窗口（按面积从大到小）：");
    for candidate in &candidates {
        let (x, y, width, height) = candidate.rect;
        println!(
            "  ({x:>5}, {y:>4})  {width:>4}x{height:<4}  {}",
            candidate.title
        );
    }
    println!("请在整屏截图上拖拽框住「任务栏文字」，Enter 确认、Esc 取消");

    let desktop = capture::capture_desktop()?;
    let Some(selection) = run_picker(desktop.clone(), candidates.clone()) else {
        return Err("已取消".into());
    };
    let Some(candidate) = best_candidate(&candidates, selection).cloned() else {
        return Err(
            "选中的区域不在任何候选窗口的客户区里：先用 windows 命令确认窗口，或用 --window 缩小范围"
                .into(),
        );
    };
    let client = candidate.rect;
    let region = region_from_selection(selection, client)?;
    let text = format!(
        "{:.4},{:.4},{:.4},{:.4}",
        region.0, region.1, region.2, region.3
    );
    println!("匹配窗口：{}", candidate.title);
    println!("区域（相对客户区）：{text}");

    // 被其他窗口盖住时截到的是别人的画面，先提醒一句。
    let center_x = (selection.0 + selection.2) / 2.0;
    let center_y = (selection.1 + selection.3) / 2.0;
    if let Some((handle, title)) = capture::window_at(center_x as i32, center_y as i32) {
        if handle != candidate.handle {
            println!(
                "警告：选中区域现在被「{title}」盖住了，截图会拿到它的画面。\
                 请先把游戏窗口切到前台，重跑一次 pick。"
            );
        }
    }

    // 当场校验：按这个区域截一张，和整屏截图里选中的那块比一下。
    let check = capture::capture_client(candidate.handle, region)?;
    let out: PathBuf = flags
        .get("out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/region-check.png"));
    check
        .save(&out)
        .map_err(|err| format!("写入 {} 失败：{err}", out.display()))?;
    println!(
        "已按该区域截图：{}（{}x{}）",
        out.display(),
        check.width(),
        check.height()
    );
    let expected = font_lib::crop(
        &desktop,
        selection.0 as u32,
        selection.1 as u32,
        (selection.2 - selection.0) as u32,
        (selection.3 - selection.1) as u32,
    )
    .ok();
    match expected.map(|expected| mean_abs_diff(&check, &expected)) {
        Some(Some(diff)) if diff <= 12.0 => {
            println!("校验：与框选画面一致（平均像素差 {diff:.1}）")
        }
        Some(Some(diff)) => {
            println!("校验：与框选画面差很多（平均像素差 {diff:.1}），可能被遮挡或窗口位置变了")
        }
        _ => println!("校验：尺寸不一致，没法对比（窗口大小变了吗）"),
    }

    if flags.contains_key("write-config") {
        let path = config_path();
        write_config_region(&path, &text, &format!("{}x{}", client.2, client.3))?;
        println!("已写入 {}", path.display());
    } else {
        println!("要直接写进配置加 --write-config；或手动填 config.json 的 \"ocr/task_region\"");
    }
    Ok(())
}

/// 选区：整屏像素矩形 `(x0, y0, x1, y1)`。
type Selection = (f32, f32, f32, f32);

struct PickerApp {
    image: std::sync::Arc<image::RgbImage>,
    texture: Option<egui::TextureHandle>,
    candidates: Vec<Candidate>,
    /// 拖拽起点/终点：整屏像素坐标。
    start: Option<(f32, f32)>,
    current: Option<(f32, f32)>,
    result: std::sync::Arc<std::sync::Mutex<Option<Selection>>>,
    cancelled: std::sync::Arc<std::sync::Mutex<bool>>,
}

impl PickerApp {
    fn selection(&self) -> Option<(f32, f32, f32, f32)> {
        let (start_x, start_y) = self.start?;
        let (current_x, current_y) = self.current?;
        let rect = (
            start_x.min(current_x),
            start_y.min(current_y),
            start_x.max(current_x),
            start_y.max(current_y),
        );
        (rect.2 - rect.0 >= 4.0 && rect.3 - rect.1 >= 4.0).then_some(rect)
    }

    /// 当前选区落在哪个窗口里、对应的百分比区域是多少。
    fn verdict(&self) -> Option<(&Candidate, core::TaskRegion)> {
        let selection = self.selection()?;
        let candidate = best_candidate(&self.candidates, selection)?;
        let region = region_from_selection(selection, candidate.rect).ok()?;
        Some((candidate, region))
    }

    fn confirm(&self, ctx: &egui::Context) {
        let Some(rect) = self.selection() else {
            return;
        };
        if self.verdict().is_none() {
            return;
        }
        *self.result.lock().unwrap() = Some(rect);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn cancel(&self, ctx: &egui::Context) {
        *self.cancelled.lock().unwrap() = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn texture(&mut self, ctx: &egui::Context) -> egui::TextureHandle {
        if let Some(texture) = &self.texture {
            return texture.clone();
        }
        let image = self.image.clone();
        let mut rgba = Vec::with_capacity((image.width() * image.height() * 4) as usize);
        for pixel in image.pixels() {
            rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
        }
        let color = egui::ColorImage::from_rgba_unmultiplied(
            [image.width() as usize, image.height() as usize],
            &rgba,
        );
        let texture = ctx.load_texture("desktop", color, egui::TextureOptions::LINEAR);
        self.texture = Some(texture.clone());
        texture
    }
}

impl eframe::App for PickerApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let texture = self.texture(&ctx);
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(egui::Color32::from_rgb(22, 22, 26))
                    .inner_margin(8),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("拖拽框住「任务栏文字」");
                    ui.separator();
                    match self.verdict() {
                        Some((candidate, region)) => ui.label(format!(
                            "区域 {:.4},{:.4},{:.4},{:.4}（窗口：{}）",
                            region.0, region.1, region.2, region.3, candidate.title
                        )),
                        None => ui.label("（还没框选到窗口内）"),
                    };
                    ui.separator();
                    let ready = self.verdict().is_some();
                    if ui
                        .add_enabled(ready, egui::Button::new("确认 (Enter)"))
                        .clicked()
                    {
                        self.confirm(&ctx);
                    }
                    if ui.button("取消 (Esc)").clicked() {
                        self.cancel(&ctx);
                    }
                });
                ui.separator();

                let (image_w, image_h) = (self.image.width() as f32, self.image.height() as f32);
                let available = ui.available_size();
                let scale = (available.x / image_w).min(available.y / image_h).min(1.0);
                let size = egui::vec2(image_w * scale, image_h * scale);
                let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
                let to_desktop =
                    |pos: egui::Pos2| ((pos.x - rect.min.x) / scale, (pos.y - rect.min.y) / scale);
                if response.drag_started() {
                    self.start = response.interact_pointer_pos().map(to_desktop);
                    self.current = self.start;
                } else if response.dragged() {
                    self.current = response.interact_pointer_pos().map(to_desktop);
                }
                ui.painter().image(
                    texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                if let Some((x0, y0, x1, y1)) = self.selection() {
                    let selected = egui::Rect::from_min_max(
                        rect.min + egui::vec2(x0 * scale, y0 * scale),
                        rect.min + egui::vec2(x1 * scale, y1 * scale),
                    );
                    // 选区外压暗，避免误看相邻区域。
                    let shade = egui::Color32::from_black_alpha(120);
                    let full = rect;
                    ui.painter().rect_filled(
                        egui::Rect::from_min_max(full.min, egui::pos2(full.max.x, selected.min.y)),
                        0.0,
                        shade,
                    );
                    ui.painter().rect_filled(
                        egui::Rect::from_min_max(egui::pos2(full.min.x, selected.max.y), full.max),
                        0.0,
                        shade,
                    );
                    ui.painter().rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(full.min.x, selected.min.y),
                            egui::pos2(selected.min.x, selected.max.y),
                        ),
                        0.0,
                        shade,
                    );
                    ui.painter().rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(selected.max.x, selected.min.y),
                            egui::pos2(full.max.x, selected.max.y),
                        ),
                        0.0,
                        shade,
                    );
                    ui.painter().rect_stroke(
                        selected,
                        0.0,
                        egui::Stroke::new(2.0, egui::Color32::from_rgb(0x4c, 0xd9, 0x8a)),
                        egui::StrokeKind::Outside,
                    );
                }
            });

        if ctx.input(|input| input.key_pressed(egui::Key::Enter)) {
            self.confirm(&ctx);
        }
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.cancel(&ctx);
        }
    }
}

fn run_picker(
    desktop: image::RgbImage,
    candidates: Vec<Candidate>,
) -> Option<(f32, f32, f32, f32)> {
    let result = std::sync::Arc::new(std::sync::Mutex::new(None));
    let cancelled = std::sync::Arc::new(std::sync::Mutex::new(false));
    let app = PickerApp {
        image: std::sync::Arc::new(desktop),
        texture: None,
        candidates,
        start: None,
        current: None,
        result: result.clone(),
        cancelled: cancelled.clone(),
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("font-tool · 框选任务区域")
            .with_inner_size([1280.0, 820.0])
            .with_min_inner_size([640.0, 400.0]),
        ..Default::default()
    };
    let started = eframe::run_native(
        "font-tool-pick",
        options,
        Box::new(move |cc| {
            install_cjk(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    );
    if let Err(err) = started {
        eprintln!("打开框选窗口失败：{err}");
        return None;
    }
    let picked = {
        let guard = result.lock().unwrap();
        *guard
    };
    picked
}
// -------------------------------------------------------------- 写回 config.json

/// 列一下截得到的窗口和它们的客户区坐标（排查「位置不对」时先看这个）。
fn windows(_args: &[String]) -> Result<(), String> {
    if let Some(dpi) = capture::dpi_info() {
        println!(
            "屏幕：{}x{}｜系统缩放 {}%｜本进程 DPI 感知：{}",
            dpi.screen.0,
            dpi.screen.1,
            dpi.system_dpi * 100 / 96,
            dpi.awareness
        );
        if dpi.system_dpi > 96 && dpi.awareness.starts_with("unaware") {
            println!(
                "注意：系统有缩放、本进程却「不感知 DPI」，窗口/屏幕坐标是逻辑像素，\
                 和截图像素不是同一套。"
            );
        }
        println!();
    }
    let windows = capture::list_windows()?;
    if windows.is_empty() {
        println!("没有枚举到可截图的窗口");
        return Ok(());
    }
    for window in &windows {
        match capture::client_rect(window.handle) {
            Some((x, y, width, height)) => println!(
                "客户区 ({x:>5}, {y:>4})  {width:>4}x{height:<4}  {}",
                window.title
            ),
            None => println!("客户区 未知                  {}", window.title),
        }
    }
    println!("\n提示：用 snap --region 0,0,1,1 可以截整块客户区，看区域该往哪切。");
    Ok(())
}

/// 不开 GUI 直接写区域（脚本/手动调参用）：`region --region 0.7,0.24,0.99,0.7`。
fn region(args: &[String]) -> Result<(), String> {
    let flags = parse_flags(args);
    let spec = required(&flags, "region")?;
    let region = core::parse_region(&spec)
        .ok_or_else(|| format!("--region 需要 0~1 之间、左<右、上<下的四个百分比：{spec}"))?;
    let text = format!(
        "{:.4},{:.4},{:.4},{:.4}",
        region.0, region.1, region.2, region.3
    );
    let size_key = match flags.get("size") {
        Some(size) => size.clone(),
        None => {
            let windows = game_windows()?;
            let window = pick_window(&windows, flags.get("window"))?;
            match capture::client_size(window.handle) {
                Some((width, height)) => format!("{width}x{height}"),
                None => return Err("读不到窗口客户区尺寸，用 --size 指定（例如 1218x994）".into()),
            }
        }
    };
    println!("区域：{text}（分辨率键 {size_key}）");
    let path = flags
        .get("config")
        .map(PathBuf::from)
        .unwrap_or_else(config_path);
    if flags.contains_key("dry-run") {
        let original = std::fs::read_to_string(&path)
            .map_err(|err| format!("读取 {} 失败：{err}", path.display()))?;
        let patched = patch_preset(
            &replace_key_value(&original, "ocr/task_region", &text),
            &size_key,
            &text,
        );
        print!("{patched}");
        return Ok(());
    }
    write_config_region(&path, &text, &size_key)?;
    println!("已写入 {}", path.display());
    Ok(())
}

/// 只截一张面板存成 PNG（不跑 OCR）：调区域 / 预处理时用，秒出结果。
/// 加 `--desktop` 则截整个屏幕（看游戏窗口在屏幕上到底长什么样）。
fn snap(args: &[String]) -> Result<(), String> {
    let flags = parse_flags(args);
    let out: PathBuf = required(&flags, "out")?.into();
    if flags.contains_key("desktop") {
        let screen = capture::capture_desktop()?;
        screen
            .save(&out)
            .map_err(|err| format!("写入 {} 失败：{err}", out.display()))?;
        println!(
            "已保存整屏 {}（{}x{}）",
            out.display(),
            screen.width(),
            screen.height()
        );
        return Ok(());
    }
    let plan = CapturePlan::prepare(&flags)?;
    println!("{}", plan.describe());
    let panel = plan.grab()?;
    panel
        .save(&out)
        .map_err(|err| format!("写入 {} 失败：{err}", out.display()))?;
    println!(
        "已保存 {}（{}x{}）—— 打开看一眼区域对不对",
        out.display(),
        panel.width(),
        panel.height()
    );
    Ok(())
}

fn capture_frames(args: &[String]) -> Result<(), String> {
    let flags = parse_flags(args);
    let out: PathBuf = required(&flags, "out")?.into();
    let (cell_w, cell_h) = cell_size(&flags)?;
    let frames: u32 = number(&flags, "frames", 12)?;
    if frames == 0 {
        return Err("--frames 至少 1".into());
    }
    let interval_ms: u64 = number(&flags, "interval", 700)?;
    let min_similarity: f32 = number(&flags, "min-similarity", 0.9)?;
    let padding: u32 = number(&flags, "padding", coord_fix::DEFAULT_PADDING)?;
    let learn_all = flags.contains_key("all");
    let dump: Option<PathBuf> = flags.get("dump").map(PathBuf::from);

    let plan = CapturePlan::prepare(&flags)?;
    println!(
        "{}｜{} 帧，间隔 {} ms",
        plan.describe(),
        frames,
        interval_ms
    );
    if let Some(dir) = &dump {
        std::fs::create_dir_all(dir)
            .map_err(|err| format!("创建 {} 失败：{err}", dir.display()))?;
        println!("样本图会另存到 {}", dir.display());
    }

    let mut learner = Learner::new(
        cell_w,
        cell_h,
        LearnConfig {
            min_similarity,
            shift: 1,
        },
    );
    // 默认合并已有字库：分几次采集能逐步补全数字，而不是把上次的成果覆盖掉。
    if !flags.contains_key("fresh") && out.exists() {
        match GlyphLib::load(&out) {
            Ok(existing) => match learner.seed(&existing) {
                Ok(()) => println!(
                    "合并已有字库：{} 个字模（要重建加 --fresh）",
                    existing.len()
                ),
                Err(err) => eprintln!("忽略已有字库：{err}"),
            },
            Err(err) => eprintln!("忽略已有字库：{err}"),
        }
    }
    let mut learned_frames = 0usize;
    let mut skipped = 0usize;

    for frame in 0..frames {
        if frame > 0 {
            std::thread::sleep(Duration::from_millis(interval_ms));
        }
        let panel = plan.grab()?;
        if let Some(dir) = &dump {
            // 面板本身也存一份：没采到字时先看这张图就知道是区域错了还是被遮挡了。
            let _ = panel.save(dir.join(format!("f{frame:02}-panel.png")));
        }
        let ocr = ocr::recognize_panel(&panel, plan.high_recall)?;
        let used = learn_panel(
            &mut learner,
            &panel,
            &ocr,
            learn_all,
            &mut skipped,
            padding,
            dump.as_deref().map(|dir| (dir, frame)),
        );
        let text = ocr.text.trim().replace('\n', " / ");
        println!(
            "第 {}/{} 帧：面板 {}x{}，采到 {} 个字模｜OCR {}",
            frame + 1,
            frames,
            panel.width(),
            panel.height(),
            used,
            if text.is_empty() {
                "无文字".to_string()
            } else {
                text
            }
        );
        if used > 0 {
            learned_frames += 1;
        }
    }

    if learner.cluster_count() == 0 {
        return Err(format!(
            "一帧都没采到字模（{learned_frames}/{frames} 帧有内容，{skipped} 段文字切字数量对不上）。\n\
             检查：游戏窗口是否在前台可见、任务栏里是否有任务文字、用 --region 调整区域，\
             或者用 --preprocess color,binary + --color 把文字颜色提出来。"
        ));
    }

    let lib = learner.finish();
    lib.save(&out)?;
    println!(
        "已写入 {}（{}x{}，{} 个字模，{} 个样本，来自 {}/{} 帧）",
        out.display(),
        cell_w,
        cell_h,
        lib.len(),
        learner.sample_count(),
        learned_frames,
        frames
    );
    for summary in learner.summary() {
        print!(
            "{}×{}（{} 票）  ",
            summary.label, summary.clusters, summary.votes
        );
    }
    println!();
    for warning in learner.conflicts(2) {
        eprintln!("提醒：{warning}");
    }
    if skipped > 0 {
        eprintln!("有 {skipped} 段文字的字数与切出的字模数对不上，已跳过");
    }
    report_coverage(&lib);
    Ok(())
}

/// 把覆盖情况打印出来：齐了就说齐了，缺了就把缺的字符列出来。
fn report_coverage(lib: &GlyphLib) {
    match coverage_report(lib) {
        Ok(note) => println!("覆盖：{note}"),
        Err(missing) => eprintln!(
            "提醒：还缺这些坐标字符：{missing}\n\
             采集时任务栏里要有带坐标的任务（抓鬼/鬼王/降妖…），或者多跑几次；\
             重复运行会自动合并到同一份字库（从零重建加 --fresh）。"
        ),
    }
}

/// 调试：看一段文字怎么对齐到切出来的跨度（排查「学不到/学错」时用）。
fn align(args: &[String]) -> Result<(), String> {
    let flags = parse_flags(args);
    let image_path: PathBuf = required(&flags, "image")?.into();
    let text = required(&flags, "text")?;
    let image = load_rgb(&image_path)?;
    // 可选：先在原始裁剪图上跑一遍预处理，和采集时完全同一条流水线。
    let pipeline = preprocess::Pipeline::from_config(
        flags.get("preprocess").map(String::as_str).unwrap_or("off"),
    );
    let color = match flags.get("color") {
        Some(spec) => {
            gate::parse_hex(spec).ok_or_else(|| format!("--color 需要 #rrggbb：{spec}"))?
        }
        None => [0xea, 0xff, 0x65],
    };
    let tolerance: u8 = number(&flags, "tolerance", 24)?;
    let upscale: u32 = number(&flags, "upscale", 1)?;
    let processed = preprocess::upscale(
        &preprocess::apply(image, pipeline, color, tolerance),
        upscale,
    );
    if let Some(path) = flags.get("out") {
        processed
            .save(path)
            .map_err(|err| format!("写入 {path} 失败：{err}"))?;
        println!("预处理结果已写到 {path}");
    }
    let gray = font_lib::binarize(&processed);
    let image = processed;
    let mut cfg = MatchConfig::default();
    cfg.gap_merge = number(&flags, "gap", cfg.gap_merge)?;
    let rows = font_lib::row_spans(&gray, cfg.row_gap);
    println!(
        "图片 {}x{}，预处理 {}｜切出 {} 行",
        image.width(),
        image.height(),
        pipeline.summary(),
        rows.len()
    );
    println!(
        "图片 {}x{}，切出 {} 行",
        image.width(),
        image.height(),
        rows.len()
    );
    if rows.len() != 1 {
        println!("不是单行，学习时会被跳过");
    }
    let boxes = if flags.contains_key("components") {
        font_lib::analyze_components(&gray, cfg)
    } else {
        font_lib::analyze(&gray, cfg)
    };
    let widths: Vec<u32> = boxes.iter().map(|b| b.span.width()).collect();
    println!("跨度宽度：{widths:?}");
    match font_lib::align_chars_to_spans(&text, &widths) {
        Some(blocks) => {
            for (index, (b, block)) in boxes.iter().zip(blocks).enumerate() {
                let chars: String = block.iter().collect();
                println!(
                    "  #{index} x={:<4} 宽={:<4} → 「{chars}」",
                    b.span.x0,
                    b.span.width()
                );
            }
        }
        None => println!("对齐不上：「{text}」"),
    }
    Ok(())
}

/// 复现 font-studio 的预览：截图 → 预处理 → OCR → 按行切字，把结果落盘。
///
/// ```bash
/// cargo run --bin font-tool -- debug-rows --out target/dbg
/// ```
/// 输出 `dbg/panel.png` 和 `dbg/rows.json`，用来排查「框和文字对不上」。
fn debug_rows(args: &[String]) -> Result<(), String> {
    let flags = parse_flags(args);
    let out: PathBuf = flags
        .get("out")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/dbg"));
    std::fs::create_dir_all(&out).map_err(|err| format!("创建 {} 失败：{err}", out.display()))?;
    let plan = CapturePlan::prepare(&flags)?;
    println!("{}", plan.describe());
    let panel = plan.grab()?;
    panel
        .save(out.join("panel.png"))
        .map_err(|err| format!("写 panel.png 失败：{err}"))?;
    let gray = font_lib::binarize(&panel);
    let ocr = ocr::recognize_panel(&panel, plan.high_recall)?;
    println!(
        "面板 {}x{}，OCR 文本：{}",
        panel.width(),
        panel.height(),
        ocr.text.trim().replace('\n', " / ")
    );
    let mut lines: Vec<String> = Vec::new();
    for region in &ocr.regions {
        let raw_band = (region.left, region.top, region.right, region.bottom);
        let trimmed = font_lib::trim_to_ink(
            &gray,
            (
                raw_band.0.max(0.0) as u32,
                raw_band.1.max(0.0) as u32,
                (raw_band.2 as u32).min(gray.width()),
                (raw_band.3 as u32).min(gray.height()),
            ),
        );
        let cells = trimmed
            .map(|band| font_lib::split_row_by_text(&gray, band, &region.text))
            .unwrap_or_default();
        let cell_text: String = cells.iter().map(|(_, ch)| *ch).collect();
        let cell_desc: Vec<String> = cells
            .iter()
            .map(|(b, _)| format!("{}@{}w{}", "", b.span.x0, b.span.width()))
            .collect();
        println!(
            "  区域「{}」rect=({:.0},{:.0},{:.0},{:.0}) 墨迹={:?} 切出 {} 格（{}）",
            region.text,
            region.left,
            region.top,
            region.right,
            region.bottom,
            trimmed,
            cells.len(),
            cell_text
        );
        let _ = cell_desc;
        // 把每个字的框画到图上，直观对比。
        lines.push(format!(
            "{{\"text\":\"{}\",\"rect\":[{:.1},{:.1},{:.1},{:.1}],\"ink\":{:?},\"cells\":{}}}",
            region.text.replace('"', "'"),
            region.left,
            region.top,
            region.right,
            region.bottom,
            trimmed,
            cells.len()
        ));
    }
    let json = format!(
        "{{\n  \"panel\": [{}, {}],\n  \"upscale\": {},\n  \"rows\": [\n    {}\n  ]\n}}\n",
        panel.width(),
        panel.height(),
        plan.upscale,
        lines.join(",\n    ")
    );
    std::fs::write(out.join("rows.json"), json)
        .map_err(|err| format!("写 rows.json 失败：{err}"))?;
    println!(
        "已写入 {}/panel.png 和 {}/rows.json",
        out.display(),
        out.display()
    );
    Ok(())
}

/// 字库自测：按当前区域抓 N 帧，对比「字库读出的坐标」和「OCR 认出的坐标」。
///
/// 这是判断字库到底有没有用的唯一可靠办法：一致率高就说明它认的数字是对的，
/// 也就能在运行时替换 OCR 的错读；一致率低就别开复核开关。
fn verify(args: &[String]) -> Result<(), String> {
    let flags = parse_flags(args);
    let lib_path: PathBuf = required(&flags, "lib")?.into();
    let lib = GlyphLib::load(&lib_path)?;
    if lib.is_empty() {
        return Err(format!("字库 {} 是空的", lib_path.display()));
    }
    let frames: u32 = number(&flags, "frames", 6)?;
    let interval_ms: u64 = number(&flags, "interval", 900)?;
    let plan = CapturePlan::prepare(&flags)?;
    println!("{}｜字库 {} 个字模", plan.describe(), lib.len());

    let cfg = MatchConfig::default();
    let (mut read_ok, mut same, mut total) = (0u32, 0u32, 0u32);
    let mut examples: Vec<String> = Vec::new();
    for frame in 0..frames {
        if frame > 0 {
            std::thread::sleep(std::time::Duration::from_millis(interval_ms));
        }
        let panel = plan.grab()?;
        let ocr = ocr::recognize_panel(&panel, plan.high_recall)?;
        let mut frame_reads = 0;
        for region in &ocr.regions {
            let Some(shape) = coord_fix::coordinate_shape(&region.text) else {
                continue;
            };
            let ocr_coord: String = region
                .text
                .chars()
                .skip(shape.start)
                .take(shape.len())
                .collect();
            let Some((x, y, width, height)) = coord_fix::padded_box(region, 2, &panel) else {
                continue;
            };
            let Ok(crop) = font_lib::crop(&panel, x, y, width, height) else {
                continue;
            };
            let gray = font_lib::binarize(&crop);
            let runs = font_lib::read_narrow_runs(&gray, &lib, cfg);
            let lib_coord = runs
                .iter()
                .map(|run| run.iter().map(|m| m.label.as_str()).collect::<String>())
                .find(|text| {
                    let chars = text.chars().count();
                    coord_fix::coordinate_shape(text) == Some(0..chars)
                });
            total += 1;
            match lib_coord {
                Some(lib_coord) => {
                    read_ok += 1;
                    frame_reads += 1;
                    if lib_coord == ocr_coord {
                        same += 1;
                    } else if examples.len() < 8 {
                        examples.push(format!("OCR「{ocr_coord}」 vs 字库「{lib_coord}」"));
                    }
                }
                None => {
                    if examples.len() < 8 {
                        let saw: Vec<String> = runs
                            .iter()
                            .map(|run| {
                                format!(
                                    "「{}」",
                                    run.iter().map(|m| m.label.as_str()).collect::<String>()
                                )
                            })
                            .collect();
                        examples.push(format!(
                            "OCR「{ocr_coord}」→ 字库读到 {}（不成坐标）",
                            if saw.is_empty() {
                                "空".to_string()
                            } else {
                                saw.join(" ")
                            }
                        ));
                    }
                }
            }
        }
        println!(
            "第 {}/{} 帧：OCR 坐标段 {} 个，字库读出 {} 个",
            frame + 1,
            frames,
            total,
            frame_reads
        );
    }
    if total == 0 {
        println!("没有任何一帧出现坐标形状的文字：区域不对，或任务追踪没显示带坐标的任务");
        return Ok(());
    }
    println!(
        "\n合计：坐标段 {total} 个，字库读出 {read_ok} 个，与 OCR 一致 {same} 个（{:.0}%）",
        same as f32 * 100.0 / total as f32
    );
    for example in examples {
        println!("  例：{example}");
    }
    Ok(())
}

/// 把字库渲染成对照图：**一行一个标签**，同一标签的多个字模并排。
/// 肉眼就能看出「标签对不对」——OCR 标错的样本在图上很明显。
fn sheet(args: &[String]) -> Result<(), String> {
    let flags = parse_flags(args);
    let lib_path: PathBuf = required(&flags, "lib")?.into();
    let out: PathBuf = required(&flags, "out")?.into();
    let scale = number(&flags, "scale", 3u32)?.clamp(1, 8);
    let lib = GlyphLib::load(&lib_path)?;
    if lib.is_empty() {
        return Err(format!("字库 {} 是空的", lib_path.display()));
    }

    // 分组：按标签排序，标签内部保持入库顺序。
    let mut groups: Vec<(String, Vec<Vec<u8>>)> = Vec::new();
    for glyph in &lib.glyphs {
        match groups.iter_mut().find(|(label, _)| *label == glyph.label) {
            Some((_, cells)) => cells.push(glyph.pixels.clone()),
            None => groups.push((glyph.label.clone(), vec![glyph.pixels.clone()])),
        }
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));

    let cell_w = lib.cell_w * scale;
    let cell_h = lib.cell_h * scale;
    let gap = 4u32;
    let row_gap = 6u32;
    let widest = groups
        .iter()
        .map(|(_, cells)| cells.len() as u32 * (cell_w + gap))
        .max()
        .unwrap_or(cell_w);
    let height = groups.len() as u32 * (cell_h + gap + row_gap);
    let mut sheet =
        image::RgbImage::from_pixel(widest.max(1), height.max(1), image::Rgb([32, 32, 36]));

    for (row, (_label, cells)) in groups.iter().enumerate() {
        let y0 = row as u32 * (cell_h + gap + row_gap);
        for (column, pixels) in cells.iter().enumerate() {
            let x0 = column as u32 * (cell_w + gap);
            for y in 0..lib.cell_h * scale {
                for x in 0..lib.cell_w * scale {
                    let sx = x / scale;
                    let sy = y / scale;
                    let value = pixels[(sy * lib.cell_w + sx) as usize];
                    if let Some(pixel) = sheet.get_pixel_mut_checked(x0 + x, y0 + y) {
                        *pixel = image::Rgb([value, value, value]);
                    }
                }
            }
        }
    }
    sheet
        .save(&out)
        .map_err(|err| format!("写入 {} 失败：{err}", out.display()))?;
    println!(
        "已写入 {}（{}x{}，{} 个标签）。每行一个标签，顺序如下：",
        out.display(),
        sheet.width(),
        sheet.height(),
        groups.len()
    );
    for (row, (label, cells)) in groups.iter().enumerate() {
        println!("  第 {:>2} 行：{label} ×{}", row + 1, cells.len());
    }
    Ok(())
}

/// 体检已有字库：字符覆盖、每个字的字模数，以及缺哪些坐标字符。
fn info(args: &[String]) -> Result<(), String> {
    let flags = parse_flags(args);
    let path: PathBuf = required(&flags, "lib")?.into();
    let lib = GlyphLib::load(&path)?;
    if lib.is_empty() {
        return Err(format!("字库 {} 是空的", path.display()));
    }
    println!(
        "{}：{}x{}，{} 个字模",
        path.display(),
        lib.cell_w,
        lib.cell_h,
        lib.len()
    );
    for (label, count) in lib.label_counts() {
        print!("{label}×{count}  ");
    }
    println!();
    report_coverage(&lib);
    Ok(())
}

// -------------------------------------------------------------------- 建库

fn build(args: &[String]) -> Result<(), String> {
    let flags = parse_flags(args);
    let samples: PathBuf = required(&flags, "samples")?.into();
    let out: PathBuf = required(&flags, "out")?.into();
    let (cell_w, cell_h) = cell_size(&flags)?;
    if !samples.is_dir() {
        return Err(format!(
            "样本目录 {} 不存在。\n\
             两种做法：\n\
             1) 直接自动采集（推荐）：font-tool capture --out {}\n\
             2) 手动准备样本：在 {} 下放 xxx.png（一行文字截图）+ 同名 xxx.txt（这行的准确文字）",
            samples.display(),
            out.display(),
            samples.display()
        ));
    }
    let (lib, warnings) =
        font_lib::build_from_dir(&samples, cell_w, cell_h, MatchConfig::default())?;
    for warning in &warnings {
        eprintln!("警告：{warning}");
    }
    if lib.is_empty() {
        return Err("没有产出任何字模".into());
    }
    lib.save(&out)?;
    println!(
        "已写入 {}（{}x{}，{} 个字模）",
        out.display(),
        cell_w,
        cell_h,
        lib.len()
    );
    for summary in lib.label_counts() {
        print!("{}×{}  ", summary.0, summary.1);
    }
    println!();
    Ok(())
}

// -------------------------------------------------------------------- 读字

fn read(args: &[String]) -> Result<(), String> {
    let flags = parse_flags(args);
    let lib_path: PathBuf = required(&flags, "lib")?.into();
    let image_path: PathBuf = required(&flags, "image")?.into();
    let lib = GlyphLib::load(&lib_path)?;
    if lib.is_empty() {
        return Err(format!("字库 {} 是空的", lib_path.display()));
    }
    let mut image = load_rgb(&image_path)?;
    if let Some(spec) = flags.get("crop") {
        let parts: Vec<u32> = spec
            .split(',')
            .filter_map(|part| part.trim().parse().ok())
            .collect();
        if parts.len() != 4 {
            return Err(format!("--crop 需要 x,y,w,h：{spec}"));
        }
        image = font_lib::crop(&image, parts[0], parts[1], parts[2], parts[3])?;
    }
    if let Some(path) = flags.get("save-binary") {
        let gray = font_lib::binarize(&image);
        gray.save(path)
            .map_err(|err| format!("写入二值化图 {path} 失败：{err}"))?;
        eprintln!("二值化结果已写到 {path}");
    }
    let single_line = flags.contains_key("line");
    let started = std::time::Instant::now();
    if single_line {
        let gray = font_lib::binarize(&image);
        let line = font_lib::read_line_gray(&gray, &lib, MatchConfig::default());
        print_line(&line);
    } else {
        for line in font_lib::read_lines(&image, &lib, MatchConfig::default()) {
            print_line(&line);
        }
    }
    eprintln!(
        "耗时 {:.2} ms，字库 {} 个字模",
        started.elapsed().as_secs_f64() * 1000.0,
        lib.len()
    );
    Ok(())
}

fn print_line(line: &font_lib::LineRead) {
    let mut detail = String::new();
    for glyph in &line.glyphs {
        detail.push_str(&format!(
            "{}({:.2}/{:.2}) ",
            glyph.label, glyph.score, glyph.margin
        ));
    }
    println!(
        "y={:<4} {:<12} {}{}",
        line.top,
        line.text,
        if line.confident { "" } else { "不可信 " },
        detail
    );
}

fn load_rgb(path: &Path) -> Result<image::RgbImage, String> {
    let image =
        image::open(path).map_err(|err| format!("读取图片 {} 失败：{err}", path.display()))?;
    Ok(image.to_rgb8())
}
