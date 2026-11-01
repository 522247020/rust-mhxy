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

/// 把本进程设成 per-monitor-v2 DPI 感知，和主程序一致。
/// 失败（已经被框架设置过、旧系统不支持）就当没这回事，不能因为一个提示失败就不干活。
#[cfg(windows)]
fn enable_dpi_awareness() {
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

#[cfg(not(windows))]
fn enable_dpi_awareness() {}

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

/// `config.json` 里和采集相关的几项。读不出来就用默认值（配置本来也可以没有）。
struct CaptureDefaults {
    region: core::TaskRegion,
    /// 按分辨率保存的识别区域（和主程序一样：1920x1080 一套）。
    region_presets: HashMap<String, core::TaskRegion>,
    preprocess: String,
    color: [u8; 3],
    tolerance: u8,
    upscale: u32,
    high_recall: bool,
}

impl CaptureDefaults {
    /// 当前窗口分辨率下该用的区域：优先用分辨率预设，再退到通用区域。
    fn region_for(&self, size: Option<(u32, u32)>) -> core::TaskRegion {
        size.and_then(|(width, height)| {
            self.region_presets
                .get(&format!("{width}x{height}"))
                .copied()
        })
        .unwrap_or(self.region)
    }
}

fn read_capture_defaults() -> CaptureDefaults {
    let mut defaults = CaptureDefaults {
        region: core::DEFAULT_TASK_REGION,
        region_presets: HashMap::new(),
        preprocess: "off".into(),
        color: [0xea, 0xff, 0x65],
        tolerance: 24,
        upscale: 2,
        high_recall: true,
    };
    let path = config_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        return defaults;
    };
    // 配置是 json5（带注释、尾逗号），所以走 json5 而不是 serde_json。
    let Ok(value) = json5::from_str::<serde_json::Value>(&text) else {
        eprintln!("提示：{} 解析失败，采集参数用默认值", path.display());
        return defaults;
    };
    if let Some(region) = value
        .get("ocr/task_region")
        .and_then(|value| value.as_str())
        .and_then(core::parse_region)
    {
        defaults.region = region;
    }
    if let Some(presets) = value
        .get("ocr/region_presets")
        .and_then(|value| value.as_object())
    {
        for (key, region) in presets {
            if let Some(region) = region.as_str().and_then(core::parse_region) {
                defaults.region_presets.insert(key.clone(), region);
            }
        }
    }
    if let Some(mode) = value.get("ocr/preprocess").and_then(|value| value.as_str()) {
        defaults.preprocess = mode.to_string();
    }
    if let Some(color) = value
        .get("ocr/preprocess_color")
        .and_then(|value| value.as_str())
        .and_then(gate::parse_hex)
    {
        defaults.color = color;
    }
    if let Some(tolerance) = value
        .get("ocr/preprocess_tolerance")
        .and_then(|value| value.as_u64())
    {
        defaults.tolerance = tolerance.min(255) as u8;
    }
    if let Some(upscale) = value.get("ocr/upscale").and_then(|value| value.as_u64()) {
        defaults.upscale = (upscale as u32).clamp(1, 3);
    }
    if let Some(high_recall) = value
        .get("ocr/high_recall")
        .and_then(|value| value.as_bool())
    {
        defaults.high_recall = high_recall;
    }
    defaults
}

/// 和主程序同一条规则：优先用当前目录下的 config.json（开发期），否则用 exe 旁边的。
fn config_path() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("Cargo.toml").exists() {
            return cwd.join("config.json");
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join("config.json");
        }
    }
    PathBuf::from("config.json")
}

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
/// 一个候选游戏窗口：客户区在屏幕上的位置。
#[derive(Clone, Debug)]
struct Candidate {
    handle: i64,
    title: String,
    rect: (i32, i32, u32, u32),
}

impl Candidate {
    fn area(&self) -> f32 {
        self.rect.2 as f32 * self.rect.3 as f32
    }
}

/// 选区和哪个窗口重叠最多——不依赖窗口枚举顺序（多开时顺序不可靠）。
fn best_candidate(candidates: &[Candidate], selection: (f32, f32, f32, f32)) -> Option<&Candidate> {
    let mut best: Option<(&Candidate, f32)> = None;
    for candidate in candidates {
        let (x, y, width, height) = candidate.rect;
        let left = selection.0.max(x as f32);
        let top = selection.1.max(y as f32);
        let right = selection.2.min((x + width as i32) as f32);
        let bottom = selection.3.min((y + height as i32) as f32);
        let overlap = (right - left).max(0.0) * (bottom - top).max(0.0);
        if best.is_none_or(|(_, best_overlap)| overlap > best_overlap) {
            best = Some((candidate, overlap));
        }
    }
    let selection_area = ((selection.2 - selection.0) * (selection.3 - selection.1)).max(1.0);
    // 选区至少要有一半落在窗口里，否则说明选到的是别的程序。
    best.filter(|(_, overlap)| *overlap >= selection_area * 0.5)
        .map(|(candidate, _)| candidate)
}

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

fn region_from_selection(
    selection: (f32, f32, f32, f32),
    client: (i32, i32, u32, u32),
) -> Result<core::TaskRegion, String> {
    let (client_x, client_y) = (client.0 as f32, client.1 as f32);
    let (client_w, client_h) = (client.2 as f32, client.3 as f32);
    if client_w <= 0.0 || client_h <= 0.0 {
        return Err("客户区尺寸为 0".into());
    }
    let left = ((selection.0 - client_x) / client_w).clamp(0.0, 1.0);
    let top = ((selection.1 - client_y) / client_h).clamp(0.0, 1.0);
    let right = ((selection.2 - client_x) / client_w).clamp(0.0, 1.0);
    let bottom = ((selection.3 - client_y) / client_h).clamp(0.0, 1.0);
    if right - left < 0.01 || bottom - top < 0.01 {
        return Err("框太小了：请在图上拖一个大一点的矩形".into());
    }
    Ok((left, top, right, bottom))
}

/// 两张同尺寸图的平均像素差；尺寸差太多就没法比。
fn mean_abs_diff(a: &image::RgbImage, b: &image::RgbImage) -> Option<f32> {
    let width = a.width().min(b.width());
    let height = a.height().min(b.height());
    if width == 0 || height == 0 {
        return None;
    }
    if a.width().abs_diff(b.width()) > 8 || a.height().abs_diff(b.height()) > 8 {
        return None;
    }
    let mut total = 0f64;
    let mut count = 0f64;
    for y in 0..height {
        for x in 0..width {
            let pa = a.get_pixel(x, y);
            let pb = b.get_pixel(x, y);
            for channel in 0..3 {
                total += (pa[channel] as f64 - pb[channel] as f64).abs();
                count += 1.0;
            }
        }
    }
    (count > 0.0).then(|| (total / count) as f32)
}

/// 中文字体，让框选窗口的提示不显示成方块。
fn install_cjk(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/msyh.ttf",
        "C:/Windows/Fonts/simhei.ttf",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    ];
    for path in CANDIDATES {
        let Ok(bytes) = std::fs::read(path) else {
            continue;
        };
        let bytes: &'static [u8] = Box::leak(bytes.into_boxed_slice());
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "cjk".to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(bytes)),
        );
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "cjk".to_owned());
        fonts
            .families
            .entry(egui::FontFamily::Monospace)
            .or_default()
            .push("cjk".to_owned());
        ctx.set_fonts(fonts);
        return;
    }
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

/// 把区域写回 `config.json`：更新 `ocr/task_region`，并补上当前分辨率的预设。
/// 改写后会先解析一遍，解析不过就放弃（不碰原文件）。
fn write_config_region(path: &Path, region: &str, size_key: &str) -> Result<(), String> {
    let original = std::fs::read_to_string(path)
        .map_err(|err| format!("读取 {} 失败：{err}", path.display()))?;
    let mut text = replace_key_value(&original, "ocr/task_region", region);
    text = patch_preset(&text, size_key, region);
    if json5::from_str::<serde_json::Value>(&text).is_err() {
        return Err(format!(
            "改写后会破坏 {} 的格式，已放弃写入（原文件未动）——请手动把区域填进去",
            path.display()
        ));
    }
    std::fs::write(path, text).map_err(|err| format!("写入 {} 失败：{err}", path.display()))
}

/// 把某一行的 `"key": "..."` 值替换掉，保留缩进、注释和行尾逗号。
fn replace_key_value(text: &str, key: &str, value: &str) -> String {
    let needle = format!("\"{key}\"");
    let mut out = String::with_capacity(text.len() + 32);
    let mut done = false;
    for line in text.lines() {
        if !done {
            if let Some(colon) = line.find(':') {
                let (head, _) = line.split_at(colon);
                if head.contains(&needle) {
                    let indent: String = line.chars().take_while(|ch| ch.is_whitespace()).collect();
                    let comma = if line.trim_end().ends_with(',') {
                        ","
                    } else {
                        ""
                    };
                    out.push_str(&format!("{indent}{needle}: \"{value}\"{comma}\n"));
                    done = true;
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if !done {
        // 配置里还没有这个键：插到最后一个 '}' 前面。
        if let Some(pos) = out.rfind('}') {
            out.insert_str(pos, &format!("  \"{key}\": \"{value}\",\n"));
        }
    }
    out
}

/// 在 `ocr/region_presets` 里更新/插入当前分辨率的区域。
fn patch_preset(text: &str, size_key: &str, value: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let Some(header) = lines
        .iter()
        .position(|line| line.contains("\"ocr/region_presets\""))
    else {
        return text.to_string();
    };
    let size_needle = format!("\"{size_key}\"");
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 2);
    let mut finished = false;
    for (index, line) in lines.iter().enumerate() {
        if finished {
            out.push((*line).to_string());
            continue;
        }
        let indent: String = line.chars().take_while(|ch| ch.is_whitespace()).collect();
        if index == header {
            // 单行写法 `"ocr/region_presets": {}`：直接在花括号里塞一项。
            if line.contains("{}") {
                let filled = format!("{{\n{indent}  {size_needle}: \"{value}\",\n{indent}}}");
                out.push(line.replacen("{}", &filled, 1));
                finished = true;
                continue;
            }
            out.push((*line).to_string());
            continue;
        }
        if index > header {
            // 键已存在：改值。
            if line.contains(&size_needle) && line.contains(':') {
                out.push(format!("{indent}{size_needle}: \"{value}\","));
                finished = true;
                continue;
            }
            // 到了对象结尾还没有这个键：插在它前面。
            if line.trim_start().starts_with('}') {
                // 原文件最后一项可能没有逗号，不补就会写出非法 JSON5。
                if let Some(last) = out.iter_mut().rev().find(|line| !line.trim().is_empty()) {
                    let trimmed = last.trim_end();
                    if !trimmed.ends_with(',') && !trimmed.ends_with('{') {
                        last.push(',');
                    }
                }
                out.push(format!("{indent}  {size_needle}: \"{value}\","));
                out.push((*line).to_string());
                finished = true;
                continue;
            }
        }
        out.push((*line).to_string());
    }
    let mut result = out.join("\n");
    result.push('\n');
    result
}

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
        let mut used = 0usize;
        for (index, region) in ocr.regions.iter().enumerate() {
            let Some((x, y, width, height)) = coord_fix::padded_box(region, padding, &panel) else {
                continue;
            };
            let Ok(crop) = font_lib::crop(&panel, x, y, width, height) else {
                continue;
            };
            let gray = font_lib::binarize(&crop);
            let cells = font_lib::analyze(&gray, MatchConfig::default());
            let Some(labels) = labels_for(&region.text, cells.len(), learn_all) else {
                skipped += 1;
                continue;
            };
            if let Some(dir) = &dump {
                let stem = dir.join(format!("f{frame:02}-{index:02}"));
                let _ = crop.save(stem.with_extension("png"));
                let _ = std::fs::write(stem.with_extension("txt"), &region.text);
            }
            for (b, label) in cells.iter().zip(labels) {
                let Some(label) = label else {
                    continue;
                };
                learner.observe(
                    font_lib::cell_of(&gray, b, cell_w, cell_h),
                    &label.to_string(),
                );
                used += 1;
            }
        }
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

/// 报告坐标字符覆盖情况：缺哪个字直接列出来，比「只有 N 个字模」有用得多。
fn report_coverage(lib: &GlyphLib) {
    let missing = font_lib::missing_coordinate_labels(lib);
    if missing.is_empty() {
        println!("覆盖：0-9 和 , . 都有了");
        return;
    }
    let missing: String = missing.iter().collect();
    eprintln!(
        "提醒：还缺这些坐标字符：{missing}\n\
         采集时任务栏里要有带坐标的任务（抓鬼/鬼王/降妖…），或者多跑几次；\
         重复运行会自动合并到同一份字库（从零重建加 --fresh）。"
    );
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

fn is_game_title(title: &str) -> bool {
    if title.contains("梦幻西游") {
        return true;
    }
    // 英文客户端/多开工具可能写 MHXY，但本项目自己的终端窗口叫 rust-mhxy，得排掉。
    let lower = title.to_lowercase();
    lower.contains("mhxy") && !lower.contains("rust-mhxy")
}

/// 枚举真正可用的游戏窗口。
///
/// 不「猜一个」：游戏最小化时 `list_windows` 会返回所有窗口，
/// 随手取第一个只会截到终端 / 输入法。找不到就报错并列出可选项。
fn game_windows() -> Result<Vec<capture::WindowInfo>, String> {
    let all = capture::list_windows()?;
    let matched: Vec<capture::WindowInfo> = all
        .iter()
        .filter(|window| is_game_title(&window.title))
        .cloned()
        .collect();
    if !matched.is_empty() {
        return Ok(matched);
    }
    Err(format!(
        "没找到标题含「梦幻西游」的窗口：游戏可能最小化了。\n\
         当前可见窗口：{}\n可以用 --window <标题片段> 指定。",
        window_titles(&all)
    ))
}

fn pick_window<'a>(
    windows: &'a [capture::WindowInfo],
    filter: Option<&String>,
) -> Result<&'a capture::WindowInfo, String> {
    if windows.is_empty() {
        return Err("没有找到可截图的窗口：先把游戏窗口打开".into());
    }
    if let Some(filter) = filter {
        return windows
            .iter()
            .find(|window| window.title.contains(filter.as_str()))
            .ok_or_else(|| {
                format!(
                    "没有标题包含「{filter}」的窗口，可选：{}",
                    window_titles(windows)
                )
            });
    }
    // 默认优先游戏窗口，而不是枚举结果里的第一个（可能是个 IME 全屏窗口）。
    Ok(windows
        .iter()
        .find(|window| is_game_title(&window.title))
        .unwrap_or(&windows[0]))
}

fn window_titles(windows: &[capture::WindowInfo]) -> String {
    windows
        .iter()
        .map(|window| window.title.as_str())
        .collect::<Vec<_>>()
        .join("｜")
}

fn is_digitish(ch: char) -> bool {
    ch.is_ascii_digit() || matches!(ch, ',' | '，' | '、' | '.' | '．' | '。')
}

/// 把切出来的字模对到 OCR 文字上。
///
/// - 字数与切字数一致 → 逐字对应；
/// - 否则，若文字里有坐标片段、且切字数正好等于坐标长度 → 对到坐标那一段
///   （OCR 常把「去长寿村（79,26）附近」检成一个框）；
/// - 其余情况返回 None：宁可跳过这一段，也不给字模打错误标签。
///
/// `learn_all` 为 false 时只学数字和分隔符，避免把界面上的中文/噪声塞进坐标字库。
fn labels_for(text: &str, cells: usize, learn_all: bool) -> Option<Vec<Option<char>>> {
    let chars: Vec<char> = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    let labels: Vec<char> = if chars.len() == cells {
        chars
    } else {
        let shape = coord_fix::coordinate_shape(text)?;
        let sub: Vec<char> = text.chars().skip(shape.start).take(shape.len()).collect();
        if sub.len() != cells {
            return None;
        }
        sub
    };
    Some(
        labels
            .into_iter()
            .map(|ch| (learn_all || is_digitish(ch)).then_some(ch))
            .collect(),
    )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_align_with_cells_when_counts_match() {
        let labels = labels_for("79,26", 5, false).expect("应当对齐");
        let kept: String = labels.into_iter().flatten().collect();
        assert_eq!(kept, "79,26");
    }

    #[test]
    fn labels_fall_back_to_coordinate_segment() {
        // OCR 把整行检成一个框，只有坐标那一段切成 5 个字模。
        let labels = labels_for("去长寿村（79,26）附近", 5, false).expect("应当对上坐标段");
        let kept: String = labels.into_iter().flatten().collect();
        assert_eq!(kept, "79,26");
    }

    #[test]
    fn labels_skip_chinese_unless_learning_everything() {
        // 字数对得上但混着中文：默认只学数字和分隔符。
        let labels = labels_for("长寿村79,26", 8, false).expect("应当对齐");
        let kept: String = labels.into_iter().flatten().collect();
        assert_eq!(kept, "79,26");

        let all = labels_for("长寿村79,26", 8, true).expect("应当对齐");
        assert_eq!(all.iter().filter(|label| label.is_some()).count(), 8);
    }

    #[test]
    fn labels_reject_count_mismatch() {
        assert!(labels_for("79,26", 4, false).is_none());
        assert!(labels_for("没有数字也没有坐标", 3, false).is_none());
    }

    #[test]
    fn maps_screen_selection_to_client_percent() {
        let client = (900, 700, 800, 600);
        // 客户区内 80..400 像素 → 10%..50%
        let region = region_from_selection((980.0, 760.0, 1300.0, 1000.0), client).unwrap();
        assert!((region.0 - 0.10).abs() < 1e-6, "{region:?}");
        assert!((region.1 - 0.10).abs() < 1e-6, "{region:?}");
        assert!((region.2 - 0.50).abs() < 1e-6, "{region:?}");
        assert!((region.3 - 0.50).abs() < 1e-6, "{region:?}");
        // 框太小要报错，避免写出一个没法用的区域。
        assert!(region_from_selection((980.0, 760.0, 982.0, 762.0), client).is_err());
    }

    #[test]
    fn replaces_task_region_line_keeping_comments() {
        let text = "{\n  // 注释要留着\n  \"ocr/task_region\": \"0.1,0.2,0.3,0.4\",\n  \"ocr/upscale\": 2,\n}\n";
        let patched = replace_key_value(text, "ocr/task_region", "0.5,0.6,0.7,0.8");
        assert!(patched.contains("// 注释要留着"), "{patched}");
        assert!(
            patched.contains("\"ocr/task_region\": \"0.5,0.6,0.7,0.8\","),
            "{patched}"
        );
        assert!(patched.contains("\"ocr/upscale\": 2,"), "{patched}");
        assert!(json5::from_str::<serde_json::Value>(&patched).is_ok());
    }

    #[test]
    fn patches_region_presets_both_ways() {
        let existing =
            "{\n  \"ocr/region_presets\": {\n    \"1920x1080\": \"0.1,0.2,0.3,0.4\",\n  },\n}\n";
        let patched = patch_preset(existing, "1920x1080", "0.5,0.6,0.7,0.8");
        assert!(
            patched.contains("\"1920x1080\": \"0.5,0.6,0.7,0.8\""),
            "{patched}"
        );
        let value: serde_json::Value = json5::from_str(&patched).unwrap();
        assert_eq!(value["ocr/region_presets"]["1920x1080"], "0.5,0.6,0.7,0.8");

        let empty = "{\n  \"ocr/region_presets\": {},\n}\n";
        let patched = patch_preset(empty, "812x663", "0.5,0.6,0.7,0.8");
        let value: serde_json::Value = json5::from_str(&patched).unwrap();
        assert_eq!(value["ocr/region_presets"]["812x663"], "0.5,0.6,0.7,0.8");
    }

    #[test]
    fn writes_config_region_only_when_still_valid() {
        let path =
            std::env::temp_dir().join(format!("font-tool-config-{}.json", std::process::id()));
        std::fs::write(
            &path,
            "{\n  \"ocr/task_region\": \"0.1,0.2,0.3,0.4\",\n  \"ocr/region_presets\": {},\n}\n",
        )
        .unwrap();
        write_config_region(&path, "0.5,0.6,0.7,0.8", "812x663").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let value: serde_json::Value = json5::from_str(&text).unwrap();
        assert_eq!(value["ocr/task_region"], "0.5,0.6,0.7,0.8");
        assert_eq!(value["ocr/region_presets"]["812x663"], "0.5,0.6,0.7,0.8");

        // 原文件本来就坏了：宁可报错也不能写坏。
        std::fs::write(&path, "{ 这不是合法 json5").unwrap();
        assert!(write_config_region(&path, "0.9,0.9,0.9,0.95", "812x663").is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ 这不是合法 json5"
        );
        let _ = std::fs::remove_file(&path);
    }

    /// 真实配置的写法：多行 presets，**最后一项没有逗号**。
    /// 之前直接插一行会写出非法 JSON5（上一行缺逗号），这条就是回归用例。
    #[test]
    fn patches_preset_when_last_entry_has_no_trailing_comma() {
        let existing = "{\n  \"ocr/region_presets\": {\n    \"1218x994\": \"0.795,0.292,0.985,0.538\",\n    \"978x814\": \"0.742,0.35,0.987,0.645\"\n  },\n  \"ocr/task_region\": \"0.795,0.292,0.985,0.538\",\n}\n";
        let patched = patch_preset(existing, "1628x1326", "0.5,0.6,0.7,0.8");
        let value: serde_json::Value = json5::from_str(&patched)
            .unwrap_or_else(|err| panic!("写出非法 JSON5：{err}\n{patched}"));
        assert_eq!(value["ocr/region_presets"]["1628x1326"], "0.5,0.6,0.7,0.8");
        assert_eq!(
            value["ocr/region_presets"]["978x814"],
            "0.742,0.35,0.987,0.645"
        );
        assert_eq!(value["ocr/task_region"], "0.795,0.292,0.985,0.538");
    }
}
