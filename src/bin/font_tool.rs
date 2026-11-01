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
  font-tool snap    --out <面板.png> [--window 梦幻西游] [--region 左,上,右,下]
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
capture：自动采集——截当前游戏窗口、OCR 拿标签、多帧聚类投票后直接写字库。
         默认只学数字和分隔符（坐标用），--all 连中文一起学。
         不给 --region/--preprocess 时，读 config.json 里的现有设置（含分辨率预设）。
build  ：目录下每个 xxx.png（一行文字的截图）+ 同名 xxx.txt（这行的准确文字）。
read   ：默认按行切分整块面板；--line 表示整张图就是一行。"
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
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
        let windows = capture::list_windows()?;
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

/// 只截一张面板存成 PNG（不跑 OCR）：调区域 / 预处理时用，秒出结果。
fn snap(args: &[String]) -> Result<(), String> {
    let flags = parse_flags(args);
    let out: PathBuf = required(&flags, "out")?.into();
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
    if lib.len() < 12 {
        eprintln!(
            "提醒：只有 {} 个字模，可能还没覆盖全部数字，建议多跑几帧",
            lib.len()
        );
    }
    Ok(())
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
    Ok(&windows[0])
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
}
