//! `font-pick`：完全手动的点阵字库工具。
//!
//! 和自动采集的 `font-studio` 相反：这里**不跑 OCR、不自动切字**。
//! 流程只有一句话：填一个字符 → 在截图上拖框圈住这个字 → 存进字库。
//!
//! 为什么会有这个工具：OCR 的检测框/文字对齐在混排中文上不可靠，自动切字一错，
//! 字库就被污染。手动圈字虽然慢，但每个样本的标签和边界都是人确认过的。
//!
//! 圈字默认会「吸附墨迹」：取框内连通块的并集外接矩形（大部分在框里的连通块才算），
//! 这样手抖画大了不容易把旁边的字带进来，画小了也会把伸出去的笔画找回来。
//! 吸附后的框会在图上画出来，和最终 16x24 字模一起可见，错了直接删。

// 这些模块从主程序共享：截图 / 任务区域类型 / 预处理 / 字库格式与归一化。
#![allow(dead_code)]

#[path = "../capture.rs"]
mod capture;
#[path = "../core.rs"]
mod core;
#[path = "../font_lib.rs"]
mod font_lib;
#[path = "../preprocess.rs"]
mod preprocess;

use font_lib::{Glyph, GlyphBox, GlyphLib, MatchConfig, Span};
use image::{GrayImage, Luma, RgbImage};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() -> eframe::Result {
    // 必须在创建窗口/截图之前：系统有缩放时两套坐标会差一个倍数。
    enable_dpi_awareness();
    // 可选：`font-pick path/to/shot.png` 启动即打开一张图（方便从资源管理器拖命令）。
    let initial_image = std::env::args().nth(1);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("font-pick · 手动字库")
            .with_inner_size([1360.0, 860.0])
            .with_min_inner_size([960.0, 620.0])
            // 默认开在屏幕左上角附近：避免被系统摆到任务栏下面，底部按钮点不到。
            .with_position([60.0, 40.0]),
        ..Default::default()
    };
    eframe::run_native(
        "font-pick",
        options,
        Box::new(|cc| {
            install_cjk(&cc.egui_ctx);
            Ok(Box::new(Picker::new(initial_image.clone())))
        }),
    )
}

// ---------------------------------------------------------------- 平台小工具

/// 把本进程设成 per-monitor-v2 DPI 感知，和主程序一致。
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

/// 中文字体（工具自己的说明文字要用）。
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

/// 游戏窗口：标题含「梦幻西游」或 MHXY，排除本项目的终端窗口。
fn game_windows() -> Result<Vec<capture::WindowInfo>, String> {
    let all = capture::list_windows()?;
    let matched: Vec<capture::WindowInfo> = all
        .into_iter()
        .filter(|window| {
            let title = &window.title;
            title.contains("梦幻西游")
                || (title.to_lowercase().contains("mhxy")
                    && !title.to_lowercase().contains("rust-mhxy"))
        })
        .collect();
    if matched.is_empty() {
        Err("没找到游戏窗口（标题要含「梦幻西游」或 MHXY）".into())
    } else {
        Ok(matched)
    }
}

fn assets_dir() -> PathBuf {
    if let Ok(path) = std::env::var("MHXY_ASSETS_DIR") {
        if !path.is_empty() {
            return PathBuf::from(path);
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

/// 地图名用 assets/maps 里的文件名，保证和地图图鉴一致。
fn load_map_names() -> Vec<String> {
    let dir = assets_dir().join("maps");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let is_png = path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("png"));
            if !is_png {
                return None;
            }
            path.file_stem()
                .map(|stem| stem.to_string_lossy().to_string())
        })
        .collect();
    names.sort();
    names
}

// ---------------------------------------------------------------- 字模工具

/// 从字符串里取标签：去掉空白后只保留第一个字符。
fn label_of(text: &str) -> Option<char> {
    text.trim().chars().next()
}

fn sequence_len(text: &str) -> usize {
    text.chars().filter(|ch| !ch.is_whitespace()).count()
}

fn sequence_label_at(text: &str, index: usize) -> Option<char> {
    text.chars().filter(|ch| !ch.is_whitespace()).nth(index)
}

/// 当前框会用到的标签。
///
/// - 坐标 / 地图单字：序列下一个字（没填序列就用当前单字）；
/// - 地图整名 / 任务类型：直接取整条文本，允许多字。
fn annotation_label(
    kind: LibraryKind,
    unit: MapUnit,
    sequence: &str,
    sequence_index: usize,
    label_input: &str,
) -> Option<String> {
    match (kind, unit) {
        (LibraryKind::Coordinates, _) | (LibraryKind::MapText, MapUnit::Character) => {
            if sequence.trim().is_empty() {
                label_of(label_input).map(|ch| ch.to_string())
            } else {
                sequence_label_at(sequence, sequence_index).map(|ch| ch.to_string())
            }
        }
        (LibraryKind::MapText, MapUnit::WholeName) | (LibraryKind::TaskText, _) => {
            let text = label_input.trim();
            if text.is_empty() {
                None
            } else {
                Some(text.to_string())
            }
        }
    }
}

fn kind_name(kind: LibraryKind) -> &'static str {
    match kind {
        LibraryKind::Coordinates => "坐标数字",
        LibraryKind::MapText => "地图文字",
        LibraryKind::TaskText => "任务类型",
    }
}

/// 字库体检：每个样本拿去和「不含它自己」的字库比。
///
/// 最像别的标签（且分数够高）→ 这个样本很可能是画歪了/标错了。
/// 它不看截图，只检查字库内部是否自洽，所以能抓出「把 4 画成了 1」这类错误。
fn check_samples(lib: &GlyphLib) -> Vec<SampleCheck> {
    let cfg = MatchConfig::default();
    let mut out = Vec::new();
    if lib.glyphs.len() < 2 {
        return out;
    }
    for (index, glyph) in lib.glyphs.iter().enumerate() {
        let gray = GrayImage::from_fn(lib.cell_w, lib.cell_h, |x, y| {
            Luma([glyph.pixels[(y * lib.cell_w + x) as usize]])
        });
        let glyph_box = GlyphBox {
            span: Span {
                x0: 0,
                x1: lib.cell_w,
            },
            top: 0,
            bottom: lib.cell_h,
        };
        let mut others = lib.clone();
        others.glyphs.remove(index);
        if others.is_empty() {
            continue;
        }
        if let Some((best_label, score, margin)) =
            font_lib::classify_group(&gray, &glyph_box, &others, cfg)
        {
            // 分数太低时可能只是「都不像」，不算确定标错；0.4 以上才提示。
            if best_label != glyph.label && score >= 0.4 {
                out.push(SampleCheck {
                    index,
                    label: glyph.label.clone(),
                    best_label,
                    score,
                    margin,
                });
            }
        }
    }
    out
}

/// 框内墨迹的紧外接矩形（`rect` 是半开区间 x0,y0,x1,y1）。
fn tight_ink_box(gray: &GrayImage, rect: (u32, u32, u32, u32)) -> Option<GlyphBox> {
    let (x0, y0, x1, y1) = rect;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for y in y0..y1.min(gray.height()) {
        for x in x0..x1.min(gray.width()) {
            if gray.get_pixel(x, y)[0] < 128 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x + 1);
                max_y = max_y.max(y + 1);
            }
        }
    }
    if min_x == u32::MAX {
        None
    } else {
        Some(GlyphBox {
            span: Span {
                x0: min_x,
                x1: max_x,
            },
            top: min_y,
            bottom: max_y,
        })
    }
}

/// 全图连通块（面积 ≥ 4 的才算字，和运行时 `analyze_components` 的过滤一致）。
fn components_of(gray: &GrayImage) -> Vec<GlyphBox> {
    let (width, height) = (gray.width() as usize, gray.height() as usize);
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let ink: Vec<bool> = gray.pixels().map(|pixel| pixel[0] < 128).collect();
    let mut seen = vec![false; ink.len()];
    let mut out = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..ink.len() {
        if !ink[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1, mut area) =
            (usize::MAX, usize::MAX, 0usize, 0usize, 0usize);
        while let Some(index) = stack.pop() {
            let (x, y) = (index % width, index / width);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
            area += 1;
            for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || ny < 0 || nx >= width as i32 || ny >= height as i32 {
                    continue;
                }
                let next = ny as usize * width + nx as usize;
                if ink[next] && !seen[next] {
                    seen[next] = true;
                    stack.push(next);
                }
            }
        }
        if area >= 4 && x1 > x0 && y1 > y0 {
            out.push(GlyphBox {
                span: Span {
                    x0: x0 as u32,
                    x1: x1 as u32,
                },
                top: y0 as u32,
                bottom: y1 as u32,
            });
        }
    }
    out
}

fn union_box(a: GlyphBox, b: GlyphBox) -> GlyphBox {
    GlyphBox {
        span: Span {
            x0: a.span.x0.min(b.span.x0),
            x1: a.span.x1.max(b.span.x1),
        },
        top: a.top.min(b.top),
        bottom: a.bottom.max(b.bottom),
    }
}

/// 连通块的外接矩形有多少比例落在框里。
///
/// 用比例而不是「中心在框内」：字形可能是几块不连通的笔画（点阵字体、中文），
/// 每一块都应该算进来；而隔壁字只被框到一角时，比例很小，不会被误并。
fn overlap_ratio(component: &GlyphBox, rect: (u32, u32, u32, u32)) -> f32 {
    let (x0, y0, x1, y1) = rect;
    let ix0 = x0.max(component.span.x0);
    let iy0 = y0.max(component.top);
    let ix1 = x1.min(component.span.x1);
    let iy1 = y1.min(component.bottom);
    if ix1 <= ix0 || iy1 <= iy0 {
        return 0.0;
    }
    let intersection = ((ix1 - ix0) * (iy1 - iy0)) as f32;
    let area =
        (component.span.width() * component.bottom.saturating_sub(component.top)).max(1) as f32;
    intersection / area
}

/// 把手画的框吸附到「大部分落在框里的连通块」的并集外接矩形。
///
/// 画小了（笔画伸出框）：连通块仍有一大半在框里 → 完整取回来；
/// 画大了（带上隔壁字一角）：隔壁连通块大部分在框外 → 不会被并进来。
fn snap_box(
    gray: &GrayImage,
    components: &[GlyphBox],
    rect: (u32, u32, u32, u32),
) -> Option<GlyphBox> {
    let mut picked: Option<GlyphBox> = None;
    for component in components {
        if overlap_ratio(component, rect) >= 0.5 {
            picked = Some(match picked {
                Some(current) => union_box(current, *component),
                None => *component,
            });
        }
    }
    picked.or_else(|| tight_ink_box(gray, rect))
}

/// 图像内坐标 → 画布坐标；以及它的逆变换。
///
/// 分开成纯函数是为了能单测来回变换：这两个函数必须是严格互逆的，
/// 否则鼠标框和画出来的框就会整体错位（踩过一次：`image_pos` 多加了 `canvas.min`）。
fn screen_from_image(
    image: egui::Pos2,
    canvas: egui::Rect,
    origin: egui::Vec2,
    zoom: f32,
) -> egui::Pos2 {
    canvas.min + origin + image.to_vec2() * zoom
}

fn image_from_screen(
    screen: egui::Pos2,
    canvas: egui::Rect,
    origin: egui::Vec2,
    zoom: f32,
) -> egui::Pos2 {
    let local = (screen - canvas.min - origin) / zoom.max(0.0001);
    egui::pos2(local.x, local.y)
}

/// 框选范围（浮点、可能反向）→ 图像像素半开区间。
fn drag_rect(start: egui::Pos2, end: egui::Pos2, width: u32, height: u32) -> (u32, u32, u32, u32) {
    let x0 = start.x.min(end.x).floor().clamp(0.0, width as f32 - 1.0) as u32;
    let x1 = start.x.max(end.x).ceil().clamp(1.0, width as f32) as u32;
    let y0 = start.y.min(end.y).floor().clamp(0.0, height as f32 - 1.0) as u32;
    let y1 = start.y.max(end.y).ceil().clamp(1.0, height as f32) as u32;
    (x0, y0, x1.max(x0 + 1), y1.max(y0 + 1))
}

enum TextureKind<'a> {
    Rgb(&'a RgbImage),
    Gray(&'a GrayImage),
}

impl TextureKind<'_> {
    fn to_color_image(&self) -> egui::ColorImage {
        let mut rgba: Vec<u8> = match self {
            Self::Rgb(image) => {
                let mut out = Vec::with_capacity((image.width() * image.height() * 4) as usize);
                for pixel in image.pixels() {
                    out.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
                }
                out
            }
            Self::Gray(image) => {
                let mut out = Vec::with_capacity((image.width() * image.height() * 4) as usize);
                for pixel in image.pixels() {
                    out.extend_from_slice(&[pixel[0], pixel[0], pixel[0], 255]);
                }
                out
            }
        };
        let size = match self {
            Self::Rgb(image) => [image.width() as usize, image.height() as usize],
            Self::Gray(image) => [image.width() as usize, image.height() as usize],
        };
        if rgba.is_empty() {
            rgba.extend_from_slice(&[0, 0, 0, 255]);
            return egui::ColorImage::from_rgba_unmultiplied([1, 1], &rgba);
        }
        egui::ColorImage::from_rgba_unmultiplied(size, &rgba)
    }
}

/// 首次建贴图，之后复用同一个 texture id 就地更新。
///
/// 每帧 `load_texture` 会新建 GPU 纹理（拖容差滑块时非常卡），所以这里只在
/// 尺寸可能变化时才重建，其余情况用 `set` 覆盖。
fn set_texture(
    ctx: &egui::Context,
    slot: &mut Option<egui::TextureHandle>,
    name: &str,
    kind: TextureKind<'_>,
) {
    let image = kind.to_color_image();
    // 放大看像素时不要糊成一片。
    let options = egui::TextureOptions::NEAREST;
    match slot {
        Some(handle) if handle.size() == [image.width(), image.height()] => {
            handle.set(image, options);
        }
        _ => {
            *slot = Some(ctx.load_texture(name, image, options));
        }
    }
}

fn cell_texture(
    ctx: &egui::Context,
    pixels: &[u8],
    cell_w: u32,
    cell_h: u32,
    name: &str,
) -> egui::TextureHandle {
    let mut rgba = Vec::with_capacity((cell_w * cell_h * 4) as usize);
    for value in pixels {
        rgba.extend_from_slice(&[*value, *value, *value, 255]);
    }
    let image = egui::ColorImage::from_rgba_unmultiplied([cell_w as usize, cell_h as usize], &rgba);
    ctx.load_texture(name, image, egui::TextureOptions::NEAREST)
}

/// 点图取文字颜色：取邻域里「最饱和/最亮」的像素，避开抗锯齿边缘。
fn pick_stroke_color(image: &RgbImage, x: i32, y: i32, radius: i32) -> Option<[u8; 3]> {
    let mut best: Option<([u8; 3], i32)> = None;
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let px = x + dx;
            let py = y + dy;
            if px < 0 || py < 0 || px >= image.width() as i32 || py >= image.height() as i32 {
                continue;
            }
            let pixel = image.get_pixel(px as u32, py as u32);
            let color = [pixel[0], pixel[1], pixel[2]];
            // 用「最大通道 + 通道差」当分数：亮且彩色的像素更像文字核心。
            let max = *color.iter().max().unwrap_or(&0) as i32;
            let min = *color.iter().min().unwrap_or(&0) as i32;
            let score = max + (max - min);
            if best.map_or(true, |(_, best_score)| score > best_score) {
                best = Some((color, score));
            }
        }
    }
    best.map(|(color, _)| color)
}

// ---------------------------------------------------------------- 数据

/// 一个字模样本。`glyph` 只有本次手动画的才有（用来在图上画框）；
/// 从已有字库读进来的样本没有框，只能在下方字模格子里管理。
#[derive(Clone)]
struct Sample {
    label: String,
    pixels: Vec<u8>,
    glyph: Option<GlyphBox>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// 拖框圈字。
    Draw,
    /// 拖拽平移（滚轮缩放不受模式影响）。
    Pan,
    /// 点图取文字颜色。
    PickColor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Raw,
    Processed,
    Binary,
}

/// 采集对象：坐标数字、地图文字、任务类型各用一套字库，互不影响。
#[derive(Clone, Copy, PartialEq, Eq)]
enum LibraryKind {
    Coordinates,
    MapText,
    TaskText,
}

impl LibraryKind {
    fn is_map(self) -> bool {
        matches!(self, Self::MapText)
    }

    fn is_task(self) -> bool {
        matches!(self, Self::TaskText)
    }
}

/// 任务类型是闭集：只有这两个词。
pub const TASK_LABELS: [&str; 2] = ["抓鬼", "鬼王"];

/// 地图文字的标注粒度：整个地名一个样本，或一个框一个字。
#[derive(Clone, Copy, PartialEq, Eq)]
enum MapUnit {
    WholeName,
    Character,
}

fn library_file(kind: LibraryKind, unit: MapUnit) -> &'static str {
    match (kind, unit) {
        (LibraryKind::Coordinates, _) => "fontlib.json",
        (LibraryKind::MapText, MapUnit::WholeName) => "mapfontlib.json",
        (LibraryKind::MapText, MapUnit::Character) => "mapcharlib.json",
        (LibraryKind::TaskText, _) => "tasklib.json",
    }
}

/// 坐标是窄长数字（16x24）；整段地名宽一些；单个汉字用方形 cell。
fn library_cell(kind: LibraryKind, unit: MapUnit) -> (u32, u32) {
    match (kind, unit) {
        (LibraryKind::Coordinates, _) => (font_lib::DEFAULT_CELL_W, font_lib::DEFAULT_CELL_H),
        (LibraryKind::MapText, MapUnit::WholeName) => (64, 24),
        (LibraryKind::MapText, MapUnit::Character) => (24, 24),
        (LibraryKind::TaskText, _) => (40, 24),
    }
}

struct DragBox {
    start: egui::Pos2,
    end: egui::Pos2,
}

struct Picker {
    // 图像来源
    windows: Vec<capture::WindowInfo>,
    window_index: usize,
    image_path: String,
    pending_open: Option<String>,
    raw: Option<Arc<RgbImage>>,
    raw_tex: Option<egui::TextureHandle>,
    processed: Option<RgbImage>,
    processed_tex: Option<egui::TextureHandle>,
    gray: Option<GrayImage>,
    binary_tex: Option<egui::TextureHandle>,
    /// 二值用的是局部对比度（画面还没取色），界面上会提示一句。
    binary_adaptive: bool,
    components: Vec<GlyphBox>,
    components_dirty: bool,

    // 预处理
    color_key: bool,
    grayscale: bool,
    binarize: bool,
    invert: bool,
    color: [u8; 3],
    color_hex: String,
    tolerance: u8,
    /// 和主程序 `ocr/upscale` 保持一致：字模必须在同一个放大倍数下取，
    /// 否则运行时（默认 2x）和手动画的 1x 字模对不上。
    upscale: u32,
    view: View,
    /// 预处理参数变了之后延迟重算：拖容差滑块时不要每一帧都重建大图贴图。
    reprocess_due: Option<Instant>,

    // 视图
    zoom: f32,
    origin: egui::Vec2,
    fit_pending: bool,
    drag: Option<DragBox>,
    last_canvas: Option<egui::Rect>,
    mode: Mode,
    snap: bool,
    show_boxes: bool,
    /// 底部日志默认收起，免得窗口被撑满；需要排查时勾上。
    show_log: bool,

    // 建库
    label_input: String,
    /// 可选的「这一行文字」：填了就按顺序给每个框取标签，不用一个个点。
    sequence: String,
    sequence_index: usize,
    samples: Vec<Sample>,
    selected: Option<usize>,
    relabel: String,
    glyph_textures: Vec<Option<egui::TextureHandle>>,
    /// 撤销栈：样本 + 整行文字进度（撤销一个框时进度也要退回去）。
    history: Vec<(Vec<Sample>, usize)>,
    cell_w: u32,
    cell_h: u32,
    lib_path: String,
    kind: LibraryKind,
    /// 两种库各自的路径：切换采集对象时不要把另一个的路径覆盖掉。
    coord_path: String,
    map_path: String,
    map_char_path: String,
    task_path: String,
    map_unit: MapUnit,
    /// assets/maps 里的地图名，用来当「地图名」下拉，减少手输错字。
    map_names: Vec<String>,
    lib_dirty: bool,
    /// 「体检」发现的可疑样本：最像别的字的那些。
    health: Vec<SampleCheck>,
    /// 「验证」时用来对比的期望文本（可选）。
    expected: String,
    validation: Option<Validation>,

    logs: VecDeque<String>,
}

/// 一个样本的体检结果：拿它跟「不含它自己」的字库比，最像哪个标签。
#[derive(Clone)]
struct SampleCheck {
    index: usize,
    label: String,
    best_label: String,
    score: f32,
    margin: f32,
}

struct Validation {
    summary: String,
    /// Some(true)=和期望一致，Some(false)=不一致，None=没填期望、只做识别。
    ok: Option<bool>,
}

impl Picker {
    fn new(initial_image: Option<String>) -> Self {
        let image_path = initial_image.clone().unwrap_or_default();
        let assets = assets_dir();
        let coord_path = assets
            .join(library_file(LibraryKind::Coordinates, MapUnit::WholeName))
            .display()
            .to_string();
        let map_path = assets
            .join(library_file(LibraryKind::MapText, MapUnit::WholeName))
            .display()
            .to_string();
        let map_char_path = assets
            .join(library_file(LibraryKind::MapText, MapUnit::Character))
            .display()
            .to_string();
        let task_path = assets
            .join(library_file(LibraryKind::TaskText, MapUnit::WholeName))
            .display()
            .to_string();
        let (cell_w, cell_h) = library_cell(LibraryKind::Coordinates, MapUnit::WholeName);
        let mut picker = Self {
            windows: Vec::new(),
            window_index: 0,
            image_path,
            pending_open: initial_image,
            raw: None,
            raw_tex: None,
            processed: None,
            processed_tex: None,
            gray: None,
            binary_tex: None,
            binary_adaptive: false,
            components: Vec::new(),
            components_dirty: true,
            // 默认不猜文字颜色：先让人看到原图，点「取色」再开颜色提取。
            // 一上来就用错误的黄色做 color key，二值图会是全白，圈字会「框里没有墨迹」。
            color_key: false,
            grayscale: false,
            binarize: false,
            invert: false,
            color: [0xea, 0xff, 0x65],
            color_hex: "#eaff65".into(),
            tolerance: 48,
            upscale: 2,
            view: View::Raw,
            reprocess_due: None,
            zoom: 1.0,
            origin: egui::Vec2::ZERO,
            fit_pending: true,
            drag: None,
            last_canvas: None,
            mode: Mode::Draw,
            snap: true,
            show_boxes: true,
            show_log: false,
            label_input: "0".into(),
            sequence: String::new(),
            sequence_index: 0,
            samples: Vec::new(),
            selected: None,
            relabel: String::new(),
            glyph_textures: Vec::new(),
            history: Vec::new(),
            cell_w,
            cell_h,
            lib_path: coord_path.clone(),
            kind: LibraryKind::Coordinates,
            coord_path,
            map_path,
            map_char_path,
            task_path,
            map_unit: MapUnit::Character,
            map_names: Vec::new(),
            lib_dirty: false,
            health: Vec::new(),
            expected: String::new(),
            validation: None,
            logs: VecDeque::new(),
        };
        picker.reload_windows();
        picker.map_names = load_map_names();
        picker.load_lib();
        picker.log("手动流程：填标签（或整行文字），在图上拖框圈字，松手就加进字库。");
        picker.log("整行文字会按顺序自动取标签；滚轮缩放、中键平移。");
        picker.log("坐标和地图文字是两套库：在「4. 字库 → 采集对象」切换。");
        picker
    }

    fn log(&mut self, message: impl Into<String>) {
        let time = chrono::Local::now().format("%H:%M:%S");
        self.logs.push_back(format!("[{time}] {}", message.into()));
        while self.logs.len() > 300 {
            self.logs.pop_front();
        }
    }

    /// 样本变了：体检/验证结果都作废。
    fn invalidate_checks(&mut self) {
        self.health.clear();
        self.validation = None;
    }

    /// 只换了图/预处理：字库没变，体检结果保留，验证结果作废。
    fn invalidate_validation(&mut self) {
        self.validation = None;
    }

    fn reload_windows(&mut self) {
        match game_windows() {
            Ok(windows) => {
                self.log(format!("找到 {} 个游戏窗口", windows.len()));
                self.windows = windows;
                self.window_index = 0;
            }
            Err(err) => {
                self.windows.clear();
                self.log(err);
            }
        }
    }

    fn pipeline(&self) -> preprocess::Pipeline {
        preprocess::Pipeline {
            color_key: self.color_key,
            grayscale: self.grayscale,
            binarize: self.binarize,
            invert: self.invert,
        }
    }

    fn image_size(&self) -> Option<egui::Vec2> {
        self.raw.as_ref().map(|raw| {
            let scale = self.upscale.max(1) as f32;
            egui::vec2(raw.width() as f32 * scale, raw.height() as f32 * scale)
        })
    }

    // ------------------------------------------------------------ 图像

    fn set_raw(&mut self, ctx: &egui::Context, image: RgbImage) {
        let image = Arc::new(image);
        self.raw = Some(image);
        // 框是「这张截图上的坐标」，换图后旧框没有意义；字模本身保留。
        let had_boxes = self.samples.iter().any(|sample| sample.glyph.is_some());
        for sample in &mut self.samples {
            sample.glyph = None;
        }
        if had_boxes {
            self.selected = None;
            self.log("换图：旧图上的框不再显示（字模本身还在字库里）");
        }
        self.reprocess(ctx, true);
    }

    /// 按当前预处理参数重算。`refit` = 换图时把视图缩放到适应窗口。
    fn reprocess(&mut self, ctx: &egui::Context, refit: bool) {
        self.reprocess_due = None;
        let Some(raw) = self.raw.clone() else {
            return;
        };
        let panel = preprocess::apply((*raw).clone(), self.pipeline(), self.color, self.tolerance);
        let panel = preprocess::upscale(&panel, self.upscale);
        // 原图不单独放大一份贴图（整窗口截图放大后会多占一块大纹理，还得多跑一遍
        // 插值），直接用 1x 贴图画到同一个矩形上，坐标仍然是 2x 空间的。
        let (gray, adaptive) = if self.color_key || self.binarize {
            (font_lib::binarize_text(&panel), false)
        } else {
            // 还没取色：整张彩图用局部对比度兜底（背景保持白，不会整片变黑）。
            (font_lib::binarize_adaptive(&panel, 16, 32), true)
        };
        self.binary_adaptive = adaptive;
        set_texture(ctx, &mut self.raw_tex, "pick-raw", TextureKind::Rgb(&raw));
        set_texture(
            ctx,
            &mut self.processed_tex,
            "pick-processed",
            TextureKind::Rgb(&panel),
        );
        set_texture(ctx, &mut self.binary_tex, "pick-binary", TextureKind::Gray(&gray));
        self.processed = Some(panel);
        self.gray = Some(gray);
        self.invalidate_validation();
        // 连通块只在真正圈字时才需要：拖容差滑块时不做全图标记，避免大图卡顿。
        self.components_dirty = true;
        self.components.clear();
        if refit {
            self.fit_pending = true;
        }
        self.log(format!(
            "预处理：{}｜文字颜色 {}｜容差 {}｜放大 {}x",
            self.pipeline().summary(),
            self.color_hex,
            self.tolerance,
            self.upscale
        ));
    }

    fn ensure_components(&mut self) {
        if !self.components_dirty {
            return;
        }
        let components = self.gray.as_ref().map(components_of);
        if let Some(components) = components {
            self.components = components;
            self.components_dirty = false;
        }
    }

    fn capture_window(&mut self, ctx: &egui::Context) {
        let Some(window) = self.windows.get(self.window_index) else {
            self.log("没有可截的窗口，先点「刷新窗口」");
            return;
        };
        // 整块客户区都截下来，具体圈哪一块由人决定。
        match capture::capture_client(window.handle, (0.0, 0.0, 1.0, 1.0)) {
            Ok(image) => {
                self.log(format!(
                    "截到「{}」客户区 {}x{}",
                    window.title,
                    image.width(),
                    image.height()
                ));
                self.set_raw(ctx, image);
            }
            Err(err) => self.log(err),
        }
    }

    fn capture_desktop(&mut self, ctx: &egui::Context) {
        match capture::capture_desktop() {
            Ok(image) => {
                self.log(format!("截到整屏 {}x{}", image.width(), image.height()));
                self.set_raw(ctx, image);
            }
            Err(err) => self.log(err),
        }
    }

    fn open_image(&mut self, ctx: &egui::Context) {
        let path = self.image_path.trim();
        if path.is_empty() {
            self.log("先填图片路径");
            return;
        }
        match image::open(path) {
            Ok(image) => {
                let image = image.to_rgb8();
                self.log(format!(
                    "已打开 {path}（{}x{}）",
                    image.width(),
                    image.height()
                ));
                self.set_raw(ctx, image);
            }
            Err(err) => self.log(format!("打开 {path} 失败：{err}")),
        }
    }

    // ------------------------------------------------------------ 建库

    fn lib(&self) -> GlyphLib {
        let mut lib = GlyphLib::new(self.cell_w, self.cell_h);
        for sample in &self.samples {
            lib.glyphs.push(Glyph {
                label: sample.label.clone(),
                pixels: sample.pixels.clone(),
            });
        }
        lib
    }

    fn push_history(&mut self) {
        self.history
            .push((self.samples.clone(), self.sequence_index));
        if self.history.len() > 40 {
            self.history.remove(0);
        }
    }

    fn save_active_path(&mut self) {
        match (self.kind, self.map_unit) {
            (LibraryKind::Coordinates, _) => self.coord_path = self.lib_path.clone(),
            (LibraryKind::MapText, MapUnit::WholeName) => self.map_path = self.lib_path.clone(),
            (LibraryKind::MapText, MapUnit::Character) => {
                self.map_char_path = self.lib_path.clone()
            }
            (LibraryKind::TaskText, _) => self.task_path = self.lib_path.clone(),
        }
    }

    fn path_for(&self, kind: LibraryKind, unit: MapUnit) -> String {
        match (kind, unit) {
            (LibraryKind::Coordinates, _) => self.coord_path.clone(),
            (LibraryKind::MapText, MapUnit::WholeName) => self.map_path.clone(),
            (LibraryKind::MapText, MapUnit::Character) => self.map_char_path.clone(),
            (LibraryKind::TaskText, _) => self.task_path.clone(),
        }
    }

    /// 换库前清空当前状态，再按目标库的路径/cell 重新加载。
    fn reset_for_library(&mut self, kind: LibraryKind, unit: MapUnit) {
        self.kind = kind;
        self.map_unit = unit;
        let (cell_w, cell_h) = library_cell(kind, unit);
        self.cell_w = cell_w;
        self.cell_h = cell_h;
        self.samples.clear();
        self.glyph_textures.clear();
        self.history.clear();
        self.selected = None;
        self.sequence.clear();
        self.sequence_index = 0;
        self.health.clear();
        self.validation = None;
        self.expected.clear();
        self.lib_dirty = false;
        self.label_input = if kind.is_task() {
            TASK_LABELS[0].to_string()
        } else if kind.is_map() {
            String::new()
        } else {
            "0".into()
        };
        self.lib_path = self.path_for(kind, unit);
        self.load_lib();
    }

    /// 切换「坐标数字 / 地图文字」。不同库的路径、cell 尺寸、标签方式都不一样，
    /// 切换前必须先把当前库保存/清空，避免未保存的标注被直接替换。
    fn switch_kind(&mut self, next: LibraryKind) {
        if next == self.kind {
            return;
        }
        if self.lib_dirty {
            self.log("有未保存的改动：先「保存」或「清空」，再切换采集对象");
            return;
        }
        self.save_active_path();
        self.reset_for_library(next, self.map_unit);
        self.log(format!(
            "采集对象切到「{}」：{}（{}x{}）",
            kind_name(next),
            self.lib_path,
            self.cell_w,
            self.cell_h
        ));
        self.log_kind_hint();
    }

    /// 地图文字内部再切「整个地名 / 单个字」。
    fn switch_map_unit(&mut self, next: MapUnit) {
        if !self.kind.is_map() || next == self.map_unit {
            return;
        }
        if self.lib_dirty {
            self.log("有未保存的改动：先「保存」或「清空」，再切换标注粒度");
            return;
        }
        self.save_active_path();
        self.reset_for_library(LibraryKind::MapText, next);
        self.log(format!(
            "地图标注粒度切到「{}」：{}（{}x{}）",
            if next == MapUnit::Character {
                "单个字"
            } else {
                "整个地名"
            },
            self.lib_path,
            self.cell_w,
            self.cell_h
        ));
        self.log_kind_hint();
    }

    fn log_kind_hint(&mut self) {
        match (self.kind, self.map_unit) {
            (LibraryKind::Coordinates, _) => {}
            (LibraryKind::MapText, MapUnit::WholeName) => {
                self.log("整名：框住整个地名，选好「地图名」后拖一下；一个框一条样本")
            }
            (LibraryKind::MapText, MapUnit::Character) => {
                self.log("单字：在地名输入框填整条地名，从左到右每个字拖一个框，标签自动按字前进")
            }
            (LibraryKind::TaskText, _) => self.log(
                "任务类型：选「抓鬼」或「鬼王」，框住任务栏里那两个彩色字（或整块）拖一下",
            ),
        }
    }

    /// 任务类型库里每个标签的样本数，用来提示还缺哪个。
    fn missing_task_labels(&self) -> Vec<&'static str> {
        TASK_LABELS
            .iter()
            .copied()
            .filter(|label| {
                !self
                    .samples
                    .iter()
                    .any(|sample| sample.label == *label)
            })
            .collect()
    }

    fn undo(&mut self) {
        if let Some((previous, sequence_index)) = self.history.pop() {
            self.samples = previous;
            self.sequence_index = sequence_index;
            self.glyph_textures.clear();
            self.selected = None;
            self.invalidate_checks();
            self.lib_dirty = true;
            self.log("撤销上一步");
        } else {
            self.log("没有可撤销的操作");
        }
    }

    fn load_lib(&mut self) {
        if self.lib_dirty {
            self.log("注意：读取会覆盖当前未保存的改动");
        }
        let path = PathBuf::from(self.lib_path.trim());
        match GlyphLib::load(&path) {
            Ok(lib) => {
                self.cell_w = lib.cell_w.max(1);
                self.cell_h = lib.cell_h.max(1);
                self.samples = lib
                    .glyphs
                    .iter()
                    .map(|glyph| Sample {
                        label: glyph.label.clone(),
                        pixels: glyph.pixels.clone(),
                        glyph: None,
                    })
                    .collect();
                self.glyph_textures.clear();
                self.selected = None;
                self.history.clear();
                self.sequence_index = 0;
                self.lib_dirty = false;
                self.invalidate_checks();
                let counts = lib.label_counts();
                self.log(format!(
                    "已读取 {}（{} 个字模，{}x{}）：{}",
                    path.display(),
                    lib.len(),
                    lib.cell_w,
                    lib.cell_h,
                    counts
                        .iter()
                        .map(|(label, count)| format!("{label}×{count}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
                if self.kind.is_map() {
                    let unit = if self.map_unit == MapUnit::Character {
                        "个不同字"
                    } else {
                        "种地名"
                    };
                    self.log(format!("地图文字库：{} {unit}", counts.len()));
                } else if self.kind.is_task() {
                    self.log(format!("任务类型库：{} 种", counts.len()));
                } else {
                    let missing = font_lib::missing_coordinate_labels(&lib);
                    if !missing.is_empty() {
                        self.log(format!(
                            "还缺坐标字符：{}",
                            missing.iter().collect::<String>()
                        ));
                    }
                }
            }
            Err(err) => {
                if path.exists() {
                    self.log(err);
                } else {
                    self.log(format!(
                        "{} 不存在：从空字库开始（新建后点「保存」即可）",
                        path.display()
                    ));
                    self.samples.clear();
                    self.glyph_textures.clear();
                    self.selected = None;
                    self.sequence_index = 0;
                    self.lib_dirty = false;
                }
            }
        }
    }

    fn save_lib(&mut self) {
        let path = PathBuf::from(self.lib_path.trim());
        if self.samples.is_empty() {
            self.log("字库是空的，不保存");
            return;
        }
        // 手动改过的库先留一份上一次的备份，误删/误改还能找回来。
        if path.exists() {
            let backup = path.with_extension("json.bak");
            if let Err(err) = std::fs::copy(&path, &backup) {
                self.log(format!("备份失败（继续保存）：{err}"));
            }
        }
        let lib = self.lib();
        match lib.save(&path) {
            Ok(()) => {
                self.lib_dirty = false;
                self.log(format!(
                    "已保存 {}（{} 个字模，{}x{}）",
                    path.display(),
                    lib.len(),
                    lib.cell_w,
                    lib.cell_h
                ));
            }
            Err(err) => self.log(err),
        }
    }

    fn quick_label(&mut self, ch: char) {
        if !self.sequence.trim().is_empty() {
            self.sequence.clear();
            self.sequence_index = 0;
            self.log("已切回「当前标签」模式（整行文字已清空）");
        }
        self.label_input = ch.to_string();
        self.log(format!("当前标签「{ch}」：在图上拖框圈字"));
    }

    fn sequence_len(&self) -> usize {
        sequence_len(&self.sequence)
    }

    fn next_sequence_label(&self) -> Option<char> {
        sequence_label_at(&self.sequence, self.sequence_index)
    }

    /// 下一个框会用的标签：整行文字优先，否则用「当前标签」。
    fn current_label(&self) -> Option<char> {
        if self.sequence.trim().is_empty() {
            label_of(&self.label_input)
        } else {
            self.next_sequence_label()
        }
    }

    /// 当前是否在用「按字前进」的序列（坐标整行 / 地图单字）。
    fn sequence_mode(&self) -> bool {
        !self.sequence.trim().is_empty()
            && !self.kind.is_task()
            && (!self.kind.is_map() || self.map_unit == MapUnit::Character)
    }

    /// 当前框的完整标签。坐标/地图单字取一个字；地图整名取整条文本。
    fn active_label(&self) -> Option<String> {
        annotation_label(
            self.kind,
            self.map_unit,
            &self.sequence,
            self.sequence_index,
            &self.label_input,
        )
    }

    // ------------------------------------------------------------ 画框

    fn screen_pos(&self, image: egui::Pos2, canvas: egui::Rect) -> egui::Pos2 {
        screen_from_image(image, canvas, self.origin, self.zoom)
    }

    fn image_pos(&self, screen: egui::Pos2, canvas: egui::Rect) -> egui::Pos2 {
        // 返回「图像内坐标」（左上角 = 0,0）：screen_pos 会把它加回去。
        image_from_screen(screen, canvas, self.origin, self.zoom)
    }

    fn fit(&mut self, canvas: egui::Rect, size: egui::Vec2) {
        if size.x <= 0.0 || size.y <= 0.0 || canvas.width() <= 1.0 {
            return;
        }
        let zoom = (canvas.width() / size.x)
            .min(canvas.height() / size.y)
            .clamp(0.05, 16.0)
            * 0.98;
        self.zoom = zoom;
        self.origin = (canvas.size() - size * zoom) / 2.0;
    }

    /// 松手后把这一框变成字模。标签为空就不加，避免建出无名样本。
    fn finish_drag(&mut self, ctx: &egui::Context, start: egui::Pos2, end: egui::Pos2) {
        // 刚改完预处理（还在 120ms 防抖里）就画框：先把新参数算出来再用。
        if self.reprocess_due.is_some() {
            self.reprocess(ctx, false);
        }
        let sequence_active = self.sequence_mode();
        let Some(label_text) = self.active_label() else {
            if self.kind.is_task() {
                self.log("先选「抓鬼」或「鬼王」再拖框");
            } else if self.kind.is_map() {
                if self.map_unit == MapUnit::Character && !self.sequence.trim().is_empty() {
                    self.log("这个地名已经标完了：点「重来」或清空地名再继续");
                } else {
                    self.log("先填「地图名」或单字再拖框");
                }
            } else if sequence_active {
                self.log("整行文字已经标完了：点「重来」或清空它再继续");
            } else {
                self.log("先填标签（或按 0-9 / , / . 键）再拖框");
            }
            return;
        };
        if self.snap {
            self.ensure_components();
        }
        let Some(gray) = self.gray.as_ref() else {
            return;
        };
        let (width, height) = gray.dimensions();
        let rect = drag_rect(start, end, width, height);
        let (x0, y0, x1, y1) = rect;
        if x1.saturating_sub(x0) < 2 || y1.saturating_sub(y0) < 2 {
            self.log("框太小了，画大一点");
            return;
        }
        let glyph = if self.snap {
            snap_box(gray, &self.components, rect)
        } else {
            // 不吸附：字模边界严格等于手画的框。
            Some(GlyphBox {
                span: Span { x0, x1 },
                top: y0,
                bottom: y1,
            })
        };
        let Some(glyph) = glyph else {
            self.log("框里没有墨迹：先调预处理（颜色提取/容差），让文字在二值图里看得见");
            return;
        };
        if glyph.span.width() < 2 || glyph.bottom.saturating_sub(glyph.top) < 2 {
            self.log("吸附后只剩一条线，重画一个大点的框");
            return;
        }
        let cell = font_lib::cell_of(gray, &glyph, self.cell_w, self.cell_h);
        if self
            .samples
            .iter()
            .any(|sample| sample.label == label_text && sample.pixels == cell)
        {
            self.log(format!(
                "「{label_text}」这个字形已经在字库里了（完全一样，没重复加）"
            ));
            return;
        }
        let snapped_wider =
            glyph.span.x0 < x0 || glyph.span.x1 > x1 || glyph.top < y0 || glyph.bottom > y1;
        self.push_history();
        self.samples.push(Sample {
            label: label_text.clone(),
            pixels: cell,
            glyph: Some(glyph),
        });
        self.glyph_textures.push(None);
        self.selected = Some(self.samples.len() - 1);
        self.relabel = label_text.clone();
        self.label_input = label_text.clone();
        self.lib_dirty = true;
        self.invalidate_checks();
        if sequence_active {
            self.sequence_index += 1;
        }
        let ink = self
            .samples
            .last()
            .map(|sample| sample.pixels.iter().filter(|value| **value < 128).count());
        self.log(format!(
            "加入「{label_text}」：框 {}..{} × {}..{}（{}x{}px，{} 个墨点）{}",
            glyph.span.x0,
            glyph.span.x1,
            glyph.top,
            glyph.bottom,
            glyph.span.width(),
            glyph.bottom.saturating_sub(glyph.top),
            ink.unwrap_or(0),
            if snapped_wider {
                "｜吸附后的框比手画的大，看下方缩略图确认"
            } else {
                ""
            }
        ));
        if self.kind.is_map() {
            if self.map_unit == MapUnit::WholeName
                && !self.map_names.is_empty()
                && !self.map_names.iter().any(|name| name == &label_text)
            {
                self.log(format!(
                    "注意：「{label_text}」不在 assets/maps 的地图列表里，确认没打错字"
                ));
            }
            let unique: std::collections::BTreeSet<&str> = self
                .samples
                .iter()
                .map(|sample| sample.label.as_str())
                .collect();
            let unit = if self.map_unit == MapUnit::Character {
                "个不同字"
            } else {
                "种地名"
            };
            self.log(format!(
                "地图文字库：{} {unit}，{} 条样本",
                unique.len(),
                self.samples.len()
            ));
            self.log(format!("记得「保存」；当前库：{}", self.lib_path));
        } else if self.kind.is_task() {
            let missing = self.missing_task_labels();
            if missing.is_empty() {
                self.log("任务类型齐了（抓鬼 / 鬼王），可以「保存」");
            } else {
                self.log(format!("还缺任务类型：{}", missing.join("、")));
            }
        } else {
            let missing = font_lib::missing_coordinate_labels(&self.lib());
            if missing.is_empty() {
                self.log("坐标字符齐了（0-9 , .），可以「保存」");
            } else {
                self.log(format!("还缺：{}", missing.iter().collect::<String>()));
            }
        }
    }

    fn run_health_check(&mut self) {
        if self.samples.len() < 2 {
            self.log("至少两个字模才能体检");
            return;
        }
        let lib = self.lib();
        self.health = check_samples(&lib);
        if self.health.is_empty() {
            self.log(format!(
                "体检通过：{} 个字模都能和字库里的自己对上（没有最像别的字的）",
                lib.len()
            ));
        } else {
            self.log(format!(
                "体检：{} 个字模里有 {} 个可疑（红框标出）",
                lib.len(),
                self.health.len()
            ));
            for finding in self.health.clone() {
                self.log(format!(
                    "#{} 标「{}」，但最像「{}」（{:.2}/{:+.2}）→ 建议删除重画",
                    finding.index, finding.label, finding.best_label, finding.score, finding.margin
                ));
            }
            self.log("点「可疑样本」里的编号可选中它；删除/改标签后重新体检");
        }
    }

    /// 用当前字库在当前截图上读一遍，并把每个字的分数打出来。
    /// 填了「期望文本」还会直接对比，判断字库在这张图上到底能不能用。
    fn run_validate(&mut self) {
        if self.kind.is_map() {
            self.log("地图文字库还没接运行时识别，暂时用「体检」检查字模");
            return;
        }
        if self.kind.is_task() {
            self.log("任务类型库还没接运行时识别，暂时用「体检」检查字模");
            return;
        }
        let Some(gray) = self.gray.as_ref() else {
            self.log("先抓一张图/打开 png，再验证字库");
            return;
        };
        if self.samples.is_empty() {
            self.log("字库是空的，先圈几个字");
            return;
        }
        let lib = self.lib();
        let runs = font_lib::read_narrow_runs(gray, &lib, MatchConfig::default());
        if runs.is_empty() {
            let summary = "没读出数字串（字太少、这张图没有连着的数字，或预处理不对）".to_string();
            self.log(format!("验证：{summary}"));
            self.validation = Some(Validation { summary, ok: None });
            return;
        }
        let mut texts = Vec::new();
        let mut details = Vec::new();
        for run in &runs {
            let text: String = run.iter().map(|item| item.label.as_str()).collect();
            let scores: Vec<f32> = run.iter().map(|item| item.score).collect();
            let average = scores.iter().sum::<f32>() / scores.len() as f32;
            let lowest = scores.iter().copied().fold(f32::INFINITY, f32::min);
            details.push(format!(
                "「{text}」均分 {average:.2} / 最低 {lowest:.2}：{}",
                run.iter()
                    .map(|item| format!("{}@{:.2}/{:+.2}", item.label, item.score, item.margin))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
            texts.push(text);
        }
        let detected = texts.concat();
        let expected: String = self
            .expected
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect();
        let (ok, summary) = if expected.is_empty() {
            (
                None,
                format!(
                    "读出 {} 段：{}",
                    texts.len(),
                    texts
                        .iter()
                        .map(|text| format!("「{text}」"))
                        .collect::<Vec<_>>()
                        .join(" ")
                ),
            )
        } else if detected == expected || texts.iter().any(|text| *text == expected) {
            (
                Some(true),
                format!("与期望「{expected}」一致（读出「{detected}」）"),
            )
        } else {
            (
                Some(false),
                format!("不一致：期望「{expected}」，读出「{detected}」"),
            )
        };
        self.log(format!("验证：{summary}"));
        for detail in details {
            self.log(format!("  {detail}"));
        }
        if expected.is_empty() {
            self.log("提示：填「期望文本」可以做对照（例如 151,12）");
        } else if ok == Some(false) {
            self.log("不一致时先看二值图：字被切碎/漏掉的话，先调预处理");
        }
        self.validation = Some(Validation { summary, ok });
    }

    fn glyph_texture_cached(
        &mut self,
        ctx: &egui::Context,
        index: usize,
        pixels: &[u8],
    ) -> egui::TextureHandle {
        while self.glyph_textures.len() <= index {
            self.glyph_textures.push(None);
        }
        if self.glyph_textures[index].is_none() {
            let name = format!("pick-glyph-{index}");
            self.glyph_textures[index] =
                Some(cell_texture(ctx, pixels, self.cell_w, self.cell_h, &name));
        }
        self.glyph_textures[index].clone().expect("贴图刚刚建好")
    }

    fn delete_sample(&mut self, index: usize) {
        if index >= self.samples.len() {
            return;
        }
        self.push_history();
        let removed = self.samples.remove(index);
        if index < self.glyph_textures.len() {
            self.glyph_textures.remove(index);
        }
        self.selected = None;
        self.lib_dirty = true;
        self.invalidate_checks();
        self.log(format!("删除字模「{}」", removed.label));
    }

    // ------------------------------------------------------------ 快捷键

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.text_edit_focused() {
            return;
        }
        let (undo, save, delete) = ctx.input(|input| {
            (
                input.modifiers.command && input.key_pressed(egui::Key::Z),
                input.modifiers.command && input.key_pressed(egui::Key::S),
                input.key_pressed(egui::Key::Delete) || input.key_pressed(egui::Key::Backspace),
            )
        });
        if undo {
            self.undo();
        }
        if save {
            self.save_lib();
        }
        if delete {
            if let Some(index) = self.selected {
                self.delete_sample(index);
            }
        }
        if self.kind == LibraryKind::Coordinates {
            let keys = [
                (egui::Key::Num0, '0'),
                (egui::Key::Num1, '1'),
                (egui::Key::Num2, '2'),
                (egui::Key::Num3, '3'),
                (egui::Key::Num4, '4'),
                (egui::Key::Num5, '5'),
                (egui::Key::Num6, '6'),
                (egui::Key::Num7, '7'),
                (egui::Key::Num8, '8'),
                (egui::Key::Num9, '9'),
                (egui::Key::Comma, ','),
                (egui::Key::Period, '.'),
            ];
            let pressed = ctx.input(|input| {
                keys.iter()
                    .find(|(key, _)| input.key_pressed(*key))
                    .map(|(_, ch)| *ch)
            });
            if let Some(ch) = pressed {
                self.quick_label(ch);
            }
        }
    }

    // ------------------------------------------------------------ 界面

    fn controls_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.add_space(2.0);
        ui.label(egui::RichText::new("font-pick · 手动圈字建库").heading());
        ui.label(
            egui::RichText::new("不跑 OCR、不自动切字：每个字都由你圈、你标。")
                .small()
                .weak(),
        );
        ui.add_space(8.0);

        ui.label(egui::RichText::new("1. 图像来源").strong());
        ui.horizontal(|ui| {
            let current = self
                .windows
                .get(self.window_index)
                .map(|window| window.title.clone())
                .unwrap_or_else(|| "（没有游戏窗口）".to_string());
            egui::ComboBox::from_id_salt("pick-window")
                .selected_text(current)
                .width(170.0)
                .show_ui(ui, |ui| {
                    for (index, window) in self.windows.iter().enumerate() {
                        ui.selectable_value(&mut self.window_index, index, &window.title);
                    }
                });
            if ui.button("刷新").clicked() {
                self.reload_windows();
            }
        });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.windows.is_empty(), egui::Button::new("抓游戏窗口"))
                .on_hover_text("截整个客户区，再用滚轮缩放到任务栏")
                .clicked()
            {
                self.capture_window(ctx);
            }
            if ui
                .button("抓整屏")
                .on_hover_text("游戏窗口被挡时用这个；截完自己缩放/平移")
                .clicked()
            {
                self.capture_desktop(ctx);
            }
        });
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.image_path)
                    .desired_width(170.0)
                    .hint_text("或者：png 路径"),
            );
            if ui.button("打开图片").clicked() {
                self.open_image(ctx);
            }
        });

        ui.add_space(8.0);
        ui.label(egui::RichText::new("2. 让字看得清").strong());
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            changed |= ui.checkbox(&mut self.color_key, "颜色提取").changed();
            changed |= ui.checkbox(&mut self.grayscale, "灰度").changed();
            changed |= ui.checkbox(&mut self.binarize, "二值化").changed();
            changed |= ui.checkbox(&mut self.invert, "反色").changed();
        });
        ui.horizontal(|ui| {
            if ui.color_edit_button_srgb(&mut self.color).changed() {
                self.color_hex = format!(
                    "#{:02x}{:02x}{:02x}",
                    self.color[0], self.color[1], self.color[2]
                );
                changed = true;
            }
            if ui
                .add(
                    egui::TextEdit::singleline(&mut self.color_hex)
                        .desired_width(70.0)
                        .hint_text("#rrggbb"),
                )
                .changed()
            {
                if let Some(color) = parse_hex(&self.color_hex) {
                    self.color = color;
                    changed = true;
                }
            }
            changed |= ui
                .add(egui::Slider::new(&mut self.tolerance, 0..=120).text("容差"))
                .changed();
        });
        ui.horizontal(|ui| {
            ui.label("放大");
            let before = self.upscale;
            for value in 1..=3 {
                ui.selectable_value(&mut self.upscale, value, format!("{value}x"));
            }
            if self.upscale != before {
                // 图像尺寸变了，重新适应窗口，避免看起来只显示了一个角。
                self.fit_pending = true;
                changed = true;
            }
            ui.label(
                egui::RichText::new("要和主程序 ocr/upscale 一致（config 里现在是 2x）")
                    .small()
                    .weak(),
            );
        });
        ui.horizontal(|ui| {
            ui.label("看：");
            ui.radio_value(&mut self.view, View::Raw, "原图");
            ui.radio_value(&mut self.view, View::Processed, "预处理");
            ui.radio_value(&mut self.view, View::Binary, "二值");
        });
        if changed {
            self.reprocess_due = Some(Instant::now() + Duration::from_millis(120));
            ctx.request_repaint_after(Duration::from_millis(120));
        }
        if self.binary_adaptive {
            ui.label(
                egui::RichText::new("二值现在是「局部对比度」兜底（白底黑字）：还没取色，先点图上文字取色才干净")
                    .small()
                    .color(egui::Color32::from_rgb(0xe6, 0xc2, 0x5c)),
            );
        }
        ui.label(
            egui::RichText::new("先勾「颜色提取」，再点图上文字取色；二值图里应该只剩字、没有背景")
                .small()
                .weak(),
        );

        ui.add_space(8.0);
        ui.label(egui::RichText::new("3. 圈字").strong());
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.mode, Mode::Draw, "拖框圈字");
            ui.radio_value(&mut self.mode, Mode::Pan, "平移");
            ui.radio_value(&mut self.mode, Mode::PickColor, "取色");
        });
        ui.checkbox(&mut self.snap, "吸附到字的墨迹（推荐）")
            .on_hover_text("取框内墨迹的紧外接矩形；关掉则字模边界严格等于你画的框");
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.show_boxes, "显示已圈的框");
            ui.checkbox(&mut self.show_log, "显示日志");
        });

        ui.add_space(4.0);
        if self.kind.is_map() {
            ui.horizontal(|ui| {
                ui.label("标注粒度");
                let mut unit = self.map_unit;
                ui.selectable_value(&mut unit, MapUnit::WholeName, "整个地名");
                ui.selectable_value(&mut unit, MapUnit::Character, "单个字");
                if unit != self.map_unit {
                    self.switch_map_unit(unit);
                }
            });
            if self.map_unit == MapUnit::WholeName {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("地图名").strong());
                    let names = self.map_names.clone();
                    let selected = if self.label_input.trim().is_empty() {
                        "选择地名".to_string()
                    } else {
                        self.label_input.trim().to_string()
                    };
                    egui::ComboBox::from_id_salt("pick-map-name")
                        .selected_text(selected)
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            for name in &names {
                                if ui
                                    .selectable_label(
                                        self.label_input.trim() == name.as_str(),
                                        name,
                                    )
                                    .clicked()
                                {
                                    self.label_input = name.clone();
                                }
                            }
                        });
                    ui.add(
                        egui::TextEdit::singleline(&mut self.label_input)
                            .desired_width(120.0)
                            .hint_text("或手输"),
                    );
                });
                ui.label(
                    egui::RichText::new(
                        "框住整个地名 → 输入完整文本 → 松手保存一条样本；地图库是 mapfontlib.json。",
                    )
                    .small()
                    .weak(),
                );
            } else {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("地名").strong());
                    let names = self.map_names.clone();
                    let selected = if self.sequence.trim().is_empty() {
                        "选择地名".to_string()
                    } else {
                        self.sequence.trim().to_string()
                    };
                    egui::ComboBox::from_id_salt("pick-map-char-name")
                        .selected_text(selected)
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            for name in &names {
                                if ui
                                    .selectable_label(self.sequence.trim() == name.as_str(), name)
                                    .clicked()
                                {
                                    self.sequence = name.clone();
                                    self.sequence_index = 0;
                                }
                            }
                        });
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut self.sequence)
                                .desired_width(120.0)
                                .hint_text("或手输：长寿村"),
                        )
                        .changed()
                    {
                        self.sequence_index = 0;
                    }
                });
                let total = self.sequence_len();
                ui.horizontal(|ui| {
                    let current = self
                        .active_label()
                        .map(|ch| format!("当前字「{ch}」"))
                        .unwrap_or_else(|| {
                            if total > 0 {
                                "这个地名标完了".to_string()
                            } else {
                                "还没填地名".to_string()
                            }
                        });
                    ui.label(
                        egui::RichText::new(current)
                            .color(egui::Color32::from_rgb(0x8a, 0xd9, 0x9a)),
                    );
                    if total > 0 {
                        ui.label(
                            egui::RichText::new(format!(
                                "{}/{}",
                                self.sequence_index.min(total),
                                total
                            ))
                            .small()
                            .weak(),
                        );
                        if ui.small_button("重来").clicked() {
                            self.sequence_index = 0;
                            self.log("地名从头开始");
                        }
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("单字");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.label_input)
                            .desired_width(40.0)
                            .hint_text("字"),
                    );
                    if response.changed() && !self.sequence.trim().is_empty() {
                        self.sequence.clear();
                        self.sequence_index = 0;
                        self.log("已切到单字模式（地名已清空）");
                    }
                    ui.label(
                        egui::RichText::new("不填地名时，可以手填一个字反复框")
                            .small()
                            .weak(),
                    );
                });
                ui.label(
                    egui::RichText::new(
                        "在地名里填整条名字，从左到右每个字拖一个框，标签自动按字前进；地图库是 mapcharlib.json。",
                    )
                    .small()
                    .weak(),
                );
            }
        } else if self.kind.is_task() {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("任务类型").strong());
                for label in TASK_LABELS {
                    let selected = self.label_input.trim() == label;
                    if ui.selectable_label(selected, label).clicked() {
                        self.label_input = label.to_string();
                        self.log(format!("当前任务类型「{label}」：框住任务栏里那两个彩色字"));
                    }
                }
                ui.add(
                    egui::TextEdit::singleline(&mut self.label_input)
                        .desired_width(80.0)
                        .hint_text("或手输"),
                );
            });
            let missing = self.missing_task_labels();
            if missing.is_empty() {
                ui.label(
                    egui::RichText::new("抓鬼 / 鬼王 都有样本了")
                        .color(egui::Color32::from_rgb(0x6d, 0xd9, 0x8a)),
                );
            } else {
                ui.label(
                    egui::RichText::new(format!("还缺：{}", missing.join("、")))
                        .color(egui::Color32::from_rgb(0xe6, 0x8a, 0x5c)),
                );
            }
            ui.label(
                egui::RichText::new(
                    "任务类型是闭集：抓鬼 / 鬼王；框住整个词（两个彩色字），一个框一条样本；库是 tasklib.json。",
                )
                .small()
                .weak(),
            );
        } else {
            ui.horizontal(|ui| {
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.label_input)
                        .desired_width(40.0)
                        .hint_text("字"),
                );
                if response.changed() && !self.sequence.trim().is_empty() {
                    self.sequence.clear();
                    self.sequence_index = 0;
                    self.log("已切回「当前标签」模式（整行文字已清空）");
                }
                let current = self
                    .current_label()
                    .map(|ch| format!("当前「{ch}」"))
                    .unwrap_or_else(|| "还没填标签".to_string());
                ui.label(
                    egui::RichText::new(current).color(egui::Color32::from_rgb(0x8a, 0xd9, 0x9a)),
                );
            });
            ui.horizontal(|ui| {
                ui.label("整行文字");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut self.sequence)
                            .desired_width(130.0)
                            .hint_text("可选：119,60"),
                    )
                    .changed()
                {
                    self.sequence_index = 0;
                }
                let total = self.sequence_len();
                if total > 0 {
                    ui.label(
                        egui::RichText::new(format!(
                            "{}/{}",
                            self.sequence_index.min(total),
                            total
                        ))
                        .small()
                        .weak(),
                    );
                    if ui.small_button("重来").clicked() {
                        self.sequence_index = 0;
                        self.log("整行文字从头开始");
                    }
                }
            });
            ui.label(
                egui::RichText::new(
                    "填了整行文字就按顺序自动取标签（从左到右圈字）；留空则用上面的当前标签。",
                )
                .small()
                .weak(),
            );
            ui.label(
                egui::RichText::new("标签会保留：连着标同一个字的多种字形时，一直拖框即可。")
                    .small()
                    .weak(),
            );
        }

        ui.add_space(8.0);
        ui.label(egui::RichText::new("4. 字库").strong());
        ui.horizontal(|ui| {
            ui.label("采集对象");
            let mut next = self.kind;
            ui.selectable_value(&mut next, LibraryKind::Coordinates, "坐标数字");
            ui.selectable_value(&mut next, LibraryKind::MapText, "地图文字");
            ui.selectable_value(&mut next, LibraryKind::TaskText, "任务类型");
            if next != self.kind {
                self.switch_kind(next);
            }
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.lib_path)
                .desired_width(275.0)
                .hint_text(format!("assets/{}", library_file(self.kind, self.map_unit))),
        );
        ui.horizontal(|ui| {
            if ui.button("读取").clicked() {
                self.load_lib();
            }
            if ui
                .add_enabled(!self.samples.is_empty(), egui::Button::new("保存"))
                .clicked()
            {
                self.save_lib();
            }
            if ui
                .button("清空")
                .on_hover_text("只清空内存，不写盘")
                .clicked()
            {
                self.push_history();
                self.samples.clear();
                self.glyph_textures.clear();
                self.selected = None;
                self.sequence_index = 0;
                self.lib_dirty = true;
                self.invalidate_checks();
                self.log("已清空字模（还没写盘；Ctrl+Z 可撤销）");
            }
            if ui
                .button("体检")
                .on_hover_text("检查每个字模：有没有画歪/标错（最像别的字）")
                .clicked()
            {
                self.run_health_check();
            }
            if ui
                .add_enabled(
                    !self.samples.is_empty()
                        && self.gray.is_some()
                        && self.kind == LibraryKind::Coordinates,
                    egui::Button::new("验证"),
                )
                .on_hover_text(if self.kind == LibraryKind::Coordinates {
                    "用当前字库在这张图上读数字串，可和期望文本对比"
                } else {
                    "地图 / 任务类型还没接入运行时识别；先用「体检」检查字模"
                })
                .clicked()
            {
                self.run_validate();
            }
        });
        if self.kind == LibraryKind::Coordinates {
            ui.horizontal(|ui| {
                ui.label("期望文本");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.expected)
                        .desired_width(120.0)
                        .hint_text("可选：151,12"),
                );
                if response.changed() {
                    self.validation = None;
                }
                ui.label(
                    egui::RichText::new("填了它，「验证」会直接对比")
                        .small()
                        .weak(),
                );
            });
        }
        if self.lib_dirty {
            ui.label(
                egui::RichText::new("有未保存的改动")
                    .small()
                    .color(egui::Color32::from_rgb(0xe6, 0xc2, 0x5c)),
            );
        }
        let lib = self.lib();
        let counts = lib.label_counts();
        ui.label(format!(
            "{} 个字模，{}x{}",
            self.samples.len(),
            self.cell_w,
            self.cell_h
        ));
        if !counts.is_empty() {
            ui.label(
                egui::RichText::new(
                    counts
                        .iter()
                        .map(|(label, count)| format!("{label}×{count}"))
                        .collect::<Vec<_>>()
                        .join("  "),
                )
                .small()
                .weak(),
            );
        }
        if self.kind.is_task() {
            let missing = self.missing_task_labels();
            if missing.is_empty() {
                ui.label(
                    egui::RichText::new("任务类型齐了：抓鬼 / 鬼王")
                        .color(egui::Color32::from_rgb(0x6d, 0xd9, 0x8a)),
                );
            } else {
                ui.label(
                    egui::RichText::new(format!("还缺任务类型：{}", missing.join("、")))
                        .color(egui::Color32::from_rgb(0xe6, 0x8a, 0x5c))
                        .strong(),
                );
            }
            ui.label(
                egui::RichText::new(
                    "运行时还没接任务类型库：先采齐、体检通过；识别接入后再开验证。",
                )
                .small()
                .weak(),
            );
        } else if self.kind.is_map() {
            let unit = if self.map_unit == MapUnit::Character {
                "个不同字"
            } else {
                "种地名"
            };
            ui.label(
                egui::RichText::new(format!("地图文字库：{} {unit}", counts.len()))
                    .color(egui::Color32::from_rgb(0x6d, 0xd9, 0x8a)),
            );
            ui.label(
                egui::RichText::new(
                    "运行时还没接地图字库：先把地名采齐、体检通过；识别接入后再开验证。",
                )
                .small()
                .weak(),
            );
            if self.map_unit == MapUnit::Character {
                let required: std::collections::BTreeSet<char> = self
                    .map_names
                    .iter()
                    .flat_map(|name| name.chars())
                    .collect();
                if required.is_empty() {
                    ui.label(
                        egui::RichText::new("assets/maps 里没有地名，算不了地图字覆盖")
                            .small()
                            .weak(),
                    );
                } else {
                    let have: std::collections::BTreeSet<char> = counts
                        .iter()
                        .filter_map(|(label, _)| {
                            let mut chars = label.chars();
                            match (chars.next(), chars.next()) {
                                (Some(ch), None) => Some(ch),
                                _ => None,
                            }
                        })
                        .collect();
                    let missing: Vec<char> = required.difference(&have).copied().collect();
                    let covered = required.len() - missing.len();
                    ui.label(format!("地图字覆盖：{covered}/{}", required.len()));
                    if missing.is_empty() {
                        ui.label(
                            egui::RichText::new("地图字齐了：assets/maps 里所有地名用到的字都有")
                                .color(egui::Color32::from_rgb(0x6d, 0xd9, 0x8a)),
                        );
                    } else {
                        ui.label(
                            egui::RichText::new(format!(
                                "还缺 {} 个字：{}",
                                missing.len(),
                                missing.iter().collect::<String>()
                            ))
                            .color(egui::Color32::from_rgb(0xe6, 0x8a, 0x5c))
                            .small(),
                        );
                    }
                }
            }
        } else {
            let missing = font_lib::missing_coordinate_labels(&lib);
            if missing.is_empty() {
                ui.label(
                    egui::RichText::new("覆盖齐了：0-9 和分隔符都有")
                        .color(egui::Color32::from_rgb(0x6d, 0xd9, 0x8a)),
                );
            } else {
                ui.label(
                    egui::RichText::new(format!("还缺：{}", missing.iter().collect::<String>()))
                        .color(egui::Color32::from_rgb(0xe6, 0x8a, 0x5c))
                        .strong(),
                );
            }
        }
        if let Some(v) = &self.validation {
            let color = match v.ok {
                Some(true) => egui::Color32::from_rgb(0x6d, 0xd9, 0x8a),
                Some(false) => egui::Color32::from_rgb(0xe6, 0x8a, 0x5c),
                None => egui::Color32::from_rgb(0x9a, 0xa4, 0xb2),
            };
            ui.label(egui::RichText::new(&v.summary).small().color(color));
        }
        let mut pick_health: Option<usize> = None;
        if !self.health.is_empty() {
            egui::CollapsingHeader::new(format!("可疑样本（{}）", self.health.len()))
                .default_open(true)
                .show(ui, |ui| {
                    for finding in &self.health {
                        ui.horizontal(|ui| {
                            if ui
                                .small_button(format!("#{}「{}」", finding.index, finding.label))
                                .clicked()
                            {
                                pick_health = Some(finding.index);
                            }
                            ui.label(
                                egui::RichText::new(format!(
                                    "最像「{}」{:.2}",
                                    finding.best_label, finding.score
                                ))
                                .small()
                                .color(egui::Color32::from_rgb(0xe6, 0x8a, 0x5c)),
                            );
                        });
                    }
                });
        }
        if let Some(index) = pick_health {
            self.selected = Some(index);
            self.relabel = self
                .samples
                .get(index)
                .map(|sample| sample.label.clone())
                .unwrap_or_default();
        }
    }

    fn canvas_ui(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new("滚轮缩放 · 中键平移 · Ctrl+Z 撤销 · Ctrl+S 保存")
                    .small()
                    .weak(),
            );
            ui.separator();
            if ui.button("适应窗口").clicked() {
                self.fit_pending = true;
            }
            ui.label(format!("{:.0}%", self.zoom * 100.0));
            if ui.button("100%").clicked() {
                if let Some(canvas) = self.last_canvas {
                    if self.image_size().is_some() {
                        let center = self.image_pos(canvas.center(), canvas);
                        self.zoom = 1.0;
                        self.origin = (canvas.center() - canvas.min) - center.to_vec2();
                    }
                }
            }
            ui.separator();
            let status = match self.mode {
                Mode::Draw => "拖框圈字",
                Mode::Pan => "拖拽平移",
                Mode::PickColor => "点图上文字取色",
            };
            ui.label(egui::RichText::new(status).color(egui::Color32::from_rgb(0x8a, 0xd9, 0x9a)));
        });
        if self.mode != Mode::Draw {
            self.drag = None;
        }
        let Some(size) = self.image_size() else {
            ui.centered_and_justified(|ui| {
                ui.label("还没有图：左边「抓游戏窗口」或打开一张 png。");
            });
            return;
        };
        let canvas = ui.available_rect_before_wrap();
        let response = ui.allocate_rect(canvas, egui::Sense::click_and_drag());
        self.last_canvas = Some(canvas);
        if self.fit_pending {
            self.fit(canvas, size);
            self.fit_pending = false;
        }
        // 先处理输入、再画画：这一帧的缩放/平移/指针位置会同时作用到图和框上，
        // 不会出现「框慢半拍」或「松手位置没算进去」。
        // 缩放：滚轮以鼠标位置为中心
        if response.hovered() {
            let scroll = ui.input(|input| input.smooth_scroll_delta.y);
            if scroll.abs() > 0.5 {
                let anchor = response.hover_pos().unwrap_or(canvas.center());
                let before = self.image_pos(anchor, canvas);
                let zoom = (self.zoom * (scroll * 0.0015).exp()).clamp(0.05, 16.0);
                self.zoom = zoom;
                self.origin = (anchor - canvas.min) - before.to_vec2() * zoom;
            }
        }

        // 中键平移：任何模式都能用
        if response.dragged_by(egui::PointerButton::Middle) {
            self.origin += response.drag_delta();
        }
        // 左键：平移模式拖图，圈字模式拖框
        if response.dragged_by(egui::PointerButton::Primary) {
            if self.mode == Mode::Pan {
                self.origin += response.drag_delta();
            } else if self.mode == Mode::Draw {
                if let Some(pos) = response.interact_pointer_pos() {
                    let end = self.image_pos(pos, canvas);
                    if let Some(drag) = &mut self.drag {
                        drag.end = end;
                    }
                }
            }
        }
        if response.drag_started_by(egui::PointerButton::Primary) && self.mode == Mode::Draw {
            // 用按下的位置当起点；终点用当前指针（拖拽阈值吃掉的那几像素也能补回来）。
            let press = ui.input(|input| input.pointer.press_origin());
            let current = response.interact_pointer_pos();
            if let Some(pos) = press.or(current) {
                let start = self.image_pos(pos, canvas);
                let end = current
                    .map(|point| self.image_pos(point, canvas))
                    .unwrap_or(start);
                self.drag = Some(DragBox { start, end });
            }
        }
        if response.drag_stopped_by(egui::PointerButton::Primary) && self.mode == Mode::Draw {
            if let Some(mut drag) = self.drag.take() {
                // 松手这一帧也要把终点收到松手位置，否则最后一次移动会丢。
                let release = response
                    .interact_pointer_pos()
                    .or_else(|| ui.input(|input| input.pointer.latest_pos()));
                if let Some(pos) = release {
                    drag.end = self.image_pos(pos, canvas);
                }
                self.finish_drag(ui.ctx(), drag.start, drag.end);
            }
        }
        if response.clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let image = self.image_pos(pos, canvas);
                match self.mode {
                    Mode::PickColor => {
                        let (x, y) = (
                            (image.x / self.upscale.max(1) as f32) as i32,
                            (image.y / self.upscale.max(1) as f32) as i32,
                        );
                        if let Some(color) = self
                            .raw
                            .as_ref()
                            .and_then(|raw| pick_stroke_color(raw, x, y, 3))
                        {
                            self.color = color;
                            self.color_hex =
                                format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2]);
                            self.color_key = true;
                            self.view = View::Binary;
                            if self.tolerance < 40 {
                                self.tolerance = 48;
                            }
                            let ctx = ui.ctx().clone();
                            self.reprocess(&ctx, false);
                            self.log(format!("取色 {}，已勾选颜色提取", self.color_hex));
                        }
                    }
                    Mode::Draw => {
                        // 点已有的框 = 选中它，方便删除/改标签。
                        let hit = self
                            .samples
                            .iter()
                            .enumerate()
                            .filter_map(|(index, sample)| sample.glyph.map(|glyph| (index, glyph)))
                            .find(|(_, glyph)| {
                                image.x >= glyph.span.x0 as f32
                                    && image.x <= glyph.span.x1 as f32
                                    && image.y >= glyph.top as f32
                                    && image.y <= glyph.bottom as f32
                            })
                            .map(|(index, _)| index);
                        if let Some(index) = hit {
                            self.selected = Some(index);
                            self.relabel = self.samples[index].label.clone();
                            self.log(format!(
                                "选中第 {index} 个字模「{}」",
                                self.samples[index].label
                            ));
                        }
                    }
                    Mode::Pan => {}
                }
            }
        }

        let painter = ui.painter_at(canvas);
        painter.rect_filled(canvas, 0.0, egui::Color32::from_rgb(0x11, 0x13, 0x18));

        // 图
        let texture = match self.view {
            View::Raw => self.raw_tex.as_ref(),
            View::Processed => self.processed_tex.as_ref(),
            View::Binary => self.binary_tex.as_ref(),
        };
        if let Some(texture) = texture {
            let image_rect = egui::Rect::from_min_max(
                self.screen_pos(egui::pos2(0.0, 0.0), canvas),
                self.screen_pos(egui::pos2(size.x, size.y), canvas),
            );
            painter.image(
                texture.id(),
                image_rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }

        // 已圈的框
        if self.show_boxes {
            for (index, sample) in self.samples.iter().enumerate() {
                let Some(glyph) = sample.glyph else {
                    continue;
                };
                let rect = egui::Rect::from_min_max(
                    self.screen_pos(egui::pos2(glyph.span.x0 as f32, glyph.top as f32), canvas),
                    self.screen_pos(
                        egui::pos2(glyph.span.x1 as f32, glyph.bottom as f32),
                        canvas,
                    ),
                );
                let selected = self.selected == Some(index);
                let color = if selected {
                    egui::Color32::from_rgb(0xf2, 0xc4, 0x4c)
                } else {
                    egui::Color32::from_rgba_unmultiplied(0x4c, 0xd9, 0x8a, 180)
                };
                painter.rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(if selected { 2.0 } else { 1.0 }, color),
                    egui::StrokeKind::Outside,
                );
                painter.text(
                    rect.left_top() + egui::vec2(2.0, -13.0),
                    egui::Align2::LEFT_TOP,
                    &sample.label,
                    egui::FontId::monospace(11.0),
                    color,
                );
            }
        }

        // 正在画的框
        if let Some(drag) = &self.drag {
            let start = self.screen_pos(drag.start, canvas);
            let end = self.screen_pos(drag.end, canvas);
            let rect = egui::Rect::from_two_pos(start, end);
            painter.rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.5, egui::Color32::from_rgb(0xf2, 0xc4, 0x4c)),
                egui::StrokeKind::Outside,
            );
            painter.text(
                rect.left_top() + egui::vec2(2.0, -13.0),
                egui::Align2::LEFT_TOP,
                self.active_label().unwrap_or_else(|| "?".into()),
                egui::FontId::monospace(12.0),
                egui::Color32::from_rgb(0xf2, 0xc4, 0x4c),
            );
        }

        // 当前鼠标位置（排查框错位用）
        if let Some(pos) = response.hover_pos() {
            let image = self.image_pos(pos, canvas);
            painter.text(
                canvas.left_top() + egui::vec2(8.0, 8.0),
                egui::Align2::LEFT_TOP,
                format!("x={:.0} y={:.0}", image.x, image.y),
                egui::FontId::monospace(11.0),
                egui::Color32::from_rgb(0x9a, 0xa4, 0xb2),
            );
        }
    }

    fn samples_ui(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(format!("字模（{}）", self.samples.len())).strong());
            if self.selected.is_some() {
                ui.separator();
                if let Some(index) = self.selected {
                    ui.label(format!("选中 #{index}"));
                }
                ui.add(egui::TextEdit::singleline(&mut self.relabel).desired_width(40.0));
                let mut relabel: Option<(usize, char)> = None;
                if ui.button("改标签").clicked() {
                    if let (Some(index), Some(label)) = (self.selected, label_of(&self.relabel)) {
                        relabel = Some((index, label));
                    }
                }
                if ui.button("删除选中").clicked() {
                    if let Some(index) = self.selected {
                        self.delete_sample(index);
                    }
                }
                if ui.button("取消选择").clicked() {
                    self.selected = None;
                }
                if let Some((index, label)) = relabel {
                    if index < self.samples.len() {
                        self.push_history();
                        self.samples[index].label = label.to_string();
                        self.selected = Some(index);
                        self.lib_dirty = true;
                        self.invalidate_checks();
                        self.log(format!("第 {index} 个字模改标签为「{label}」"));
                    }
                }
            } else {
                ui.label(
                    egui::RichText::new("点图上的框或下面的字模可选中，再改标签/删除")
                        .small()
                        .weak(),
                );
            }
        });
        if self.samples.is_empty() {
            ui.label("空字库：左边填标签（例如 5），在图上拖一个框圈住数字 5。");
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("pick-glyphs")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let indices: Vec<usize> = (0..self.samples.len()).collect();
                    for index in indices {
                        let (pixels, label, glyph) = {
                            let sample = &self.samples[index];
                            (sample.pixels.clone(), sample.label.clone(), sample.glyph)
                        };
                        let suspect = self
                            .health
                            .iter()
                            .find(|finding| finding.index == index)
                            .cloned();
                        let texture = self.glyph_texture_cached(ctx, index, &pixels);
                        ui.vertical(|ui| {
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(self.cell_w as f32 * 2.0, self.cell_h as f32 * 2.0),
                                egui::Sense::click(),
                            );
                            if response.clicked() {
                                self.selected = Some(index);
                                self.relabel = label.clone();
                            }
                            let mut tip = match glyph {
                                Some(glyph) => format!(
                                    "「{label}」 {}..{} × {}..{}",
                                    glyph.span.x0, glyph.span.x1, glyph.top, glyph.bottom
                                ),
                                None => format!("「{label}」（从已有字库读入）"),
                            };
                            if let Some(finding) = &suspect {
                                tip.push_str(&format!(
                                    "\n体检：最像「{}」（{:.2}/{:+.2}），建议删除重画",
                                    finding.best_label, finding.score, finding.margin
                                ));
                            }
                            response.on_hover_text(tip);
                            ui.painter().image(
                                texture.id(),
                                rect,
                                egui::Rect::from_min_max(
                                    egui::pos2(0.0, 0.0),
                                    egui::pos2(1.0, 1.0),
                                ),
                                egui::Color32::WHITE,
                            );
                            if let Some(_finding) = &suspect {
                                ui.painter().rect_stroke(
                                    rect,
                                    2.0,
                                    egui::Stroke::new(
                                        2.0,
                                        egui::Color32::from_rgb(0xe6, 0x5c, 0x5c),
                                    ),
                                    egui::StrokeKind::Outside,
                                );
                            } else if self.selected == Some(index) {
                                ui.painter().rect_stroke(
                                    rect,
                                    2.0,
                                    egui::Stroke::new(
                                        2.0,
                                        egui::Color32::from_rgb(0xf2, 0xc4, 0x4c),
                                    ),
                                    egui::StrokeKind::Outside,
                                );
                            }
                            let color = if suspect.is_some() {
                                egui::Color32::from_rgb(0xe6, 0x8a, 0x5c)
                            } else {
                                ui.visuals().text_color()
                            };
                            ui.label(
                                egui::RichText::new(&label)
                                    .monospace()
                                    .strong()
                                    .color(color),
                            );
                        });
                    }
                });
            });
    }
}

impl eframe::App for Picker {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.06, 0.07, 0.09, 1.0]
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if let Some(path) = self.pending_open.take() {
            self.image_path = path;
            self.open_image(&ctx);
        }
        if self
            .reprocess_due
            .map_or(false, |due| Instant::now() >= due)
        {
            self.reprocess(&ctx, false);
        }
        self.handle_shortcuts(&ctx);
        if self.show_log {
            egui::Panel::bottom("pick-log")
                .resizable(true)
                .default_size(96.0)
                .min_size(60.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("pick-log-scroll")
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for line in &self.logs {
                                ui.label(egui::RichText::new(line).small().monospace());
                            }
                        });
                });
        }
        egui::Panel::bottom("pick-samples")
            .resizable(true)
            .default_size(160.0)
            .min_size(120.0)
            .max_size(320.0)
            .show(ui, |ui| self.samples_ui(ui, &ctx));
        egui::Panel::left("pick-controls")
            .resizable(false)
            .default_size(300.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("pick-controls-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.controls_ui(ui, &ctx));
            });
        egui::CentralPanel::default().show(ui, |ui| self.canvas_ui(ui, &ctx));
    }
}

fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let text = text.trim().trim_start_matches('#');
    if text.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&text[0..2], 16).ok()?;
    let g = u8::from_str_radix(&text[2..4], 16).ok()?;
    let b = u8::from_str_radix(&text[4..6], 16).ok()?;
    Some([r, g, b])
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Luma;

    fn gray_with_block(width: u32, height: u32, rect: (u32, u32, u32, u32)) -> GrayImage {
        let mut gray = GrayImage::from_pixel(width, height, Luma([255]));
        let (x0, y0, x1, y1) = rect;
        for y in y0..y1 {
            for x in x0..x1 {
                gray.put_pixel(x, y, Luma([0]));
            }
        }
        gray
    }

    #[test]
    fn manual_components_build_a_library_that_reads_the_line() {
        // 用测试字模合成一行「123,45」，模拟：人把每个字圈在一个稍大的框里 →
        // 吸附到字的墨迹 → 归一化成 16x24 → 组成字库。
        // 然后用运行时的窄字形逻辑读回来，验证两套归一化是同分布的。
        let image = font_lib::fixture::render("123,45", 5, 2);
        let gray = font_lib::binarize(&image);
        let components = components_of(&gray);
        let groups = font_lib::analyze_components(&gray, MatchConfig::default());
        assert_eq!(groups.len(), 6, "运行时应切出 6 个字：{groups:?}");
        let mut lib = GlyphLib::new(font_lib::DEFAULT_CELL_W, font_lib::DEFAULT_CELL_H);
        for (group, label) in groups.iter().zip("123,45".chars()) {
            // 模拟人手画的框：比字本身大 3px。
            let rect = (
                group.span.x0.saturating_sub(3),
                group.top.saturating_sub(3),
                group.span.x1 + 3,
                group.bottom + 3,
            );
            let snapped = snap_box(&gray, &components, rect).expect("框里应该有字");
            lib.add_pixels(
                label,
                font_lib::cell_of(&gray, &snapped, lib.cell_w, lib.cell_h),
            );
        }
        let runs = font_lib::read_narrow_runs(&gray, &lib, MatchConfig::default());
        let text: String = runs
            .first()
            .map(|run| run.iter().map(|item| item.label.as_str()).collect())
            .unwrap_or_default();
        assert_eq!(text, "123,45", "手动画的框必须和运行时切字同分布");
    }

    #[test]
    fn snap_includes_all_parts_of_a_split_glyph() {
        // 点阵字体的「2」可能是好几块不连通的笔画，框住整字时要全部取回来。
        let mut gray = GrayImage::from_pixel(30, 20, Luma([255]));
        for y in 3..6 {
            for x in 4..12 {
                gray.put_pixel(x, y, Luma([0]));
            }
        }
        for y in 15..18 {
            for x in 20..26 {
                gray.put_pixel(x, y, Luma([0]));
            }
        }
        let components = components_of(&gray);
        let glyph = snap_box(&gray, &components, (0, 0, 30, 20)).expect("should snap");
        assert_eq!(glyph.span.x0, 4);
        assert_eq!(glyph.span.x1, 26);
        assert_eq!(glyph.top, 3);
        assert_eq!(glyph.bottom, 18);
    }

    #[test]
    #[ignore]
    fn dump_binary_for_inspection() {
        // 把二值结果落盘看极性：cargo test --bin font-pick dump_binary -- --ignored
        let cases = [
            ("target/panel.png", "target/binary-panel.png"),
            ("target/live-1x.png", "target/binary-live.png"),
            ("target/coord-1x.png", "target/binary-coord.png"),
            ("target/live-full.png", "target/binary-full.png"),
        ];
        for (source, out) in cases {
            let path = std::path::Path::new(source);
            if !path.exists() {
                continue;
            }
            let image = image::open(path).expect("open").to_rgb8();
            let gray = font_lib::binarize_text(&image);
            let dark = gray.pixels().filter(|p| p[0] < 128).count();
            let total = gray.pixels().count();
            gray.save(out).expect("save");
            eprintln!(
                "{source}: {}x{} dark={:.1}% -> {out}",
                gray.width(),
                gray.height(),
                dark as f32 * 100.0 / total.max(1) as f32
            );
            for (radius, bias) in [(8u32, 24u8), (16, 32)] {
                let adaptive = font_lib::binarize_adaptive(&image, radius, bias);
                let dark = adaptive.pixels().filter(|p| p[0] < 128).count();
                let out = out.replace(".png", &format!("-adaptive-{radius}-{bias}.png"));
                adaptive.save(&out).expect("save");
                eprintln!(
                    "  adaptive r={radius} bias={bias}: dark={:.1}% -> {out}",
                    dark as f32 * 100.0 / total.max(1) as f32
                );
            }
        }
    }

    #[test]
    #[ignore]
    fn manual_lib_on_panel_screenshot() {
        // 诊断用：用现有字库在 target/panel.png 上跑运行时的窄字形逻辑。
        // cargo test --bin font-pick manual_lib_on_panel -- --ignored --nocapture
        let lib_path = std::path::Path::new("assets/fontlib.json");
        let image_path = std::env::var("MHXY_PROBE_IMAGE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("target/panel.png"));
        let image_path = image_path.as_path();
        if !lib_path.exists() || !image_path.exists() {
            eprintln!("缺少 {lib_path:?} 或 {image_path:?}，跳过");
            return;
        }
        let lib = GlyphLib::load(lib_path).expect("load lib");
        let panel = image::open(image_path).expect("open png").to_rgb8();
        let mut cases: Vec<(String, image::GrayImage)> = Vec::new();
        for (name, color, tolerance) in [
            ("color-red-80", [255, 0, 0], 80),
            ("color-red-130", [255, 0, 0], 130),
            ("color-white-48", [255, 255, 255], 48),
            ("color-yellow-48", [0xea, 0xff, 0x65], 48),
        ] {
            let pipeline = preprocess::Pipeline {
                color_key: true,
                grayscale: false,
                binarize: false,
                invert: false,
            };
            let processed = preprocess::apply(panel.clone(), pipeline, color, tolerance);
            cases.push((name.to_string(), font_lib::binarize_text(&processed)));
        }
        cases.push(("raw".into(), font_lib::binarize_text(&panel)));
        cases.push((
            "adaptive".into(),
            font_lib::binarize_adaptive(&panel, 8, 26),
        ));
        let white_pipeline = preprocess::Pipeline {
            color_key: true,
            grayscale: false,
            binarize: false,
            invert: false,
        };
        let white_panel = preprocess::apply(panel.clone(), white_pipeline, [255, 255, 255], 48);
        cases.push((
            "white-48-up2".into(),
            font_lib::binarize_text(&preprocess::upscale(&white_panel, 2)),
        ));
        cases.push((
            "raw-up2".into(),
            font_lib::binarize_text(&preprocess::upscale(&panel, 2)),
        ));
        for (name, gray) in cases {
            let cfg = MatchConfig::default();
            let boxes = font_lib::analyze_components(&gray, cfg);
            let narrow = font_lib::narrow_groups(&boxes);
            eprintln!("{name}: components={} narrow={}", boxes.len(), narrow.len());
            for &index in narrow.iter().take(40) {
                let b = boxes[index];
                let detail = font_lib::classify_group(&gray, &b, &lib, cfg)
                    .map(|(label, score, margin)| format!("{label}@{score:.2}/{margin:+.2}"))
                    .unwrap_or_else(|| "-".into());
                eprintln!(
                    "  x{}..{} y{}..{} -> {detail}",
                    b.span.x0, b.span.x1, b.top, b.bottom
                );
            }
            let runs = font_lib::read_narrow_runs(&gray, &lib, cfg);
            eprintln!("{name}: runs={}", runs.len());
            for run in &runs {
                eprintln!(
                    "  {}",
                    run.iter()
                        .map(|item| format!("{}@{:.2}/{:+.2}", item.label, item.score, item.margin))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
            }
        }
    }

    #[test]
    #[ignore]
    fn manual_lib_self_consistency() {
        // 诊断用：每个样本拿去和「不含它自己」的字库比，看它最像什么。
        // 标签错/样本炸了会在这里露馅。
        let lib_path = std::path::Path::new("assets/fontlib.json");
        if !lib_path.exists() {
            return;
        }
        let lib = GlyphLib::load(lib_path).expect("load lib");
        let findings = check_samples(&lib);
        eprintln!("字库 {} 个字模，可疑 {}", lib.len(), findings.len());
        for finding in findings {
            eprintln!(
                "#{} 标「{}」→ 最像「{}」{:.2}/{:+.2}",
                finding.index, finding.label, finding.best_label, finding.score, finding.margin
            );
        }
    }

    #[test]
    fn library_kinds_use_separate_files_and_cells() {
        let whole = MapUnit::WholeName;
        let character = MapUnit::Character;
        assert_eq!(
            library_file(LibraryKind::Coordinates, whole),
            "fontlib.json"
        );
        assert_eq!(library_file(LibraryKind::MapText, whole), "mapfontlib.json");
        assert_eq!(
            library_file(LibraryKind::MapText, character),
            "mapcharlib.json"
        );
        assert_eq!(library_cell(LibraryKind::Coordinates, whole), (16, 24));
        assert_eq!(library_cell(LibraryKind::MapText, whole), (64, 24));
        assert_eq!(library_cell(LibraryKind::MapText, character), (24, 24));
        assert_eq!(
            library_file(LibraryKind::TaskText, whole),
            "tasklib.json"
        );
        assert_eq!(library_cell(LibraryKind::TaskText, whole), (40, 24));
        assert!(!LibraryKind::Coordinates.is_map());
        assert!(LibraryKind::MapText.is_map());
        assert!(LibraryKind::TaskText.is_task());
        assert_eq!(TASK_LABELS, ["抓鬼", "鬼王"]);
    }

    #[test]
    fn task_labels_keep_the_whole_word() {
        assert_eq!(
            annotation_label(LibraryKind::TaskText, MapUnit::WholeName, "x", 0, "抓鬼"),
            Some("抓鬼".into())
        );
        assert_eq!(
            annotation_label(LibraryKind::TaskText, MapUnit::WholeName, "", 0, "  鬼王 "),
            Some("鬼王".into())
        );
        assert_eq!(
            annotation_label(LibraryKind::TaskText, MapUnit::WholeName, "", 0, "  "),
            None
        );
    }

    #[test]
    fn check_samples_flags_a_mislabeled_glyph() {
        let image = font_lib::fixture::render("12", 5, 2);
        let gray = font_lib::binarize(&image);
        let boxes = font_lib::analyze_components(&gray, MatchConfig::default());
        assert_eq!(boxes.len(), 2);
        let mut clean = GlyphLib::new(font_lib::DEFAULT_CELL_W, font_lib::DEFAULT_CELL_H);
        // 每个标签放两份：体检会拿掉自己再比，这样同标签还有一份可对照。
        for (b, label) in boxes.iter().zip("12".chars()) {
            let cell = font_lib::cell_of(&gray, b, clean.cell_w, clean.cell_h);
            clean.add_pixels(label, cell.clone());
            clean.add_pixels(label, cell);
        }
        assert!(
            check_samples(&clean).is_empty(),
            "每个标签都有多份的干净字库不应报可疑"
        );

        // 把「2」的字形错标成「7」：错那份应该被抓住，并指出它最像 2。
        let mut lib = clean.clone();
        lib.add_pixels(
            "7",
            font_lib::cell_of(&gray, &boxes[1], lib.cell_w, lib.cell_h),
        );
        let wrong = check_samples(&lib)
            .into_iter()
            .find(|finding| finding.index == 4)
            .expect("错标的样本应该被抓住");
        assert_eq!(wrong.best_label, "2");
        assert!(wrong.score > 0.9);
    }

    #[test]
    fn components_find_separate_blocks() {
        let mut gray = gray_with_block(20, 10, (2, 2, 6, 8));
        for y in 2..8 {
            for x in 12..16 {
                gray.put_pixel(x, y, Luma([0]));
            }
        }
        let components = components_of(&gray);
        assert_eq!(components.len(), 2, "{components:?}");
        assert_eq!(components[0].span.x0, 2);
        assert_eq!(components[1].span.x0, 12);
    }

    #[test]
    fn snap_ignores_neighbour_that_is_mostly_outside() {
        // 两个块：左块 2..6，右块 12..16。框只盖左块 + 右块的一小角。
        let mut gray = gray_with_block(20, 10, (2, 2, 6, 8));
        for y in 2..8 {
            for x in 12..16 {
                gray.put_pixel(x, y, Luma([0]));
            }
        }
        let components = components_of(&gray);
        // 框 0..13：右块中心 14 在框外 → 只吸附左块，不把右块带进来。
        let glyph = snap_box(&gray, &components, (0, 0, 13, 10)).expect("should snap");
        assert_eq!(glyph.span.x0, 2);
        assert_eq!(glyph.span.x1, 6);
    }

    #[test]
    fn snap_keeps_clipped_stroke_when_mostly_inside() {
        // 手画小了：框只盖到左块右半部分，但块大部分在框里 → 仍然取完整块。
        let gray = gray_with_block(20, 10, (2, 2, 10, 8));
        let components = components_of(&gray);
        let glyph = snap_box(&gray, &components, (5, 0, 12, 10)).expect("should snap");
        assert_eq!(glyph.span.x0, 2);
        assert_eq!(glyph.span.x1, 10);
    }

    #[test]
    fn tight_box_falls_back_to_ink_inside_box() {
        let gray = gray_with_block(20, 10, (2, 2, 6, 8));
        let glyph = tight_ink_box(&gray, (0, 0, 20, 10)).expect("ink exists");
        assert_eq!(glyph.span.x0, 2);
        assert_eq!(glyph.span.x1, 6);
        assert_eq!(glyph.top, 2);
        assert_eq!(glyph.bottom, 8);
        assert!(tight_ink_box(&gray, (10, 0, 15, 10)).is_none());
    }

    #[test]
    fn empty_box_returns_none() {
        let gray = gray_with_block(20, 10, (2, 2, 6, 8));
        assert!(tight_ink_box(&gray, (10, 0, 15, 10)).is_none());
        assert!(snap_box(&gray, &components_of(&gray), (10, 0, 15, 10)).is_none());
    }

    #[test]
    fn drag_rect_normalizes_and_clamps() {
        let rect = drag_rect(egui::pos2(12.4, 8.6), egui::pos2(3.2, 2.1), 10, 8);
        assert_eq!(rect, (3, 2, 10, 8));
    }

    #[test]
    fn screen_and_image_transforms_are_inverse() {
        let canvas = egui::Rect::from_min_max(egui::pos2(480.8, 30.3), egui::pos2(1352.0, 532.0));
        let origin = egui::vec2(17.9, 5.0);
        let zoom = 1.848_207_2;
        let screen = egui::pos2(1206.7, 406.0);
        let image = image_from_screen(screen, canvas, origin, zoom);
        assert!((image.x - 383.1).abs() < 0.5, "image={image:?}");
        assert!((image.y - 200.6).abs() < 0.5, "image={image:?}");
        let back = screen_from_image(image, canvas, origin, zoom);
        assert!((back - screen).length() < 0.01, "back={back:?}");
        // 图像左上角必须回到 canvas.min + origin，而不是多加一次 canvas.min。
        let top_left = screen_from_image(egui::Pos2::ZERO, canvas, origin, zoom);
        assert!((top_left - (canvas.min + origin)).length() < 0.01);
    }

    #[test]
    fn label_takes_first_non_space_char() {
        assert_eq!(label_of("  5 "), Some('5'));
        assert_eq!(label_of("，"), Some('，'));
        assert_eq!(label_of("   "), None);
    }

    #[test]
    fn map_character_mode_advances_one_char_per_box() {
        let map = LibraryKind::MapText;
        assert_eq!(
            annotation_label(map, MapUnit::Character, "长寿村", 0, ""),
            Some("长".into())
        );
        assert_eq!(
            annotation_label(map, MapUnit::Character, "长寿村", 1, ""),
            Some("寿".into())
        );
        assert_eq!(
            annotation_label(map, MapUnit::Character, "长寿村", 3, ""),
            None
        );
        // 没填地名时可以手填单字反复框。
        assert_eq!(
            annotation_label(map, MapUnit::Character, "", 0, "村"),
            Some("村".into())
        );
    }

    #[test]
    fn map_whole_name_mode_keeps_the_whole_text() {
        assert_eq!(
            annotation_label(
                LibraryKind::MapText,
                MapUnit::WholeName,
                "长寿村",
                0,
                "长寿村"
            ),
            Some("长寿村".into())
        );
    }

    #[test]
    fn sequence_labels_advance_and_ignore_spaces() {
        let text = " 119,60 ";
        let chars: Vec<char> = (0..sequence_len(text))
            .filter_map(|index| sequence_label_at(text, index))
            .collect();
        assert_eq!(chars, vec!['1', '1', '9', ',', '6', '0']);
        assert_eq!(sequence_len(text), 6);
        assert_eq!(sequence_label_at(text, 6), None);
        assert_eq!(sequence_len("   "), 0);
    }

    #[test]
    fn hex_parser_accepts_with_and_without_hash() {
        assert_eq!(parse_hex("#eaff65"), Some([0xea, 0xff, 0x65]));
        assert_eq!(parse_hex("000000"), Some([0, 0, 0]));
        assert_eq!(parse_hex("#zz0000"), None);
        assert_eq!(parse_hex("fff"), None);
    }
}
