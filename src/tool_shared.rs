//! `font-tool`（命令行）和 `font-studio`（图形界面）共用的逻辑。
//!
//! 包含：窗口挑选、从 `config.json` 读采集参数、屏幕选区↔客户区百分比换算、
//! 配置回写（保留注释）、OCR 文字到字模标签的对齐、坐标字符覆盖报告。
//!
//! 这里的每个函数都有单测：两个界面只是壳，真正的规则都在这。

use crate::capture;
use crate::coord_fix;
use crate::core;
use crate::font_lib::{self, GlyphLib};
use crate::gate;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ------------------------------------------------------------------ DPI

/// 把本进程设成 per-monitor-v2 DPI 感知，和主程序（egui/winit）一致。
///
/// 必须在创建任何窗口/截图之前调用：不感知 DPI 时，系统有缩放的情况下
/// 客户区尺寸和屏幕坐标都是「逻辑像素」，会和主程序、和 `region_presets` 差一个倍数。
#[cfg(windows)]
pub fn enable_dpi_awareness() {
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

#[cfg(not(windows))]
pub fn enable_dpi_awareness() {}

// ------------------------------------------------------------------ 窗口

pub fn is_game_title(title: &str) -> bool {
    if title.contains("梦幻西游") {
        return true;
    }
    // 英文客户端/多开工具可能写 MHXY，但本项目自己的终端窗口叫 rust-mhxy，得排掉。
    let lower = title.to_lowercase();
    lower.contains("mhxy") && !lower.contains("rust-mhxy")
}

/// 枚举真正可用的游戏窗口。
///
/// 不「猜一个」：游戏最小化时 `capture::list_windows` 会返回所有窗口，
/// 随手取第一个只会截到终端 / 输入法。找不到就报错并列出可选项。
pub fn game_windows() -> Result<Vec<capture::WindowInfo>, String> {
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

pub fn pick_window<'a>(
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

pub fn window_titles(windows: &[capture::WindowInfo]) -> String {
    windows
        .iter()
        .map(|window| window.title.as_str())
        .collect::<Vec<_>>()
        .join("｜")
}

/// 一个候选窗口：客户区在屏幕上的位置。
#[derive(Clone, Debug)]
pub struct Candidate {
    pub handle: i64,
    pub title: String,
    pub rect: (i32, i32, u32, u32),
}

impl Candidate {
    pub fn area(&self) -> f32 {
        self.rect.2 as f32 * self.rect.3 as f32
    }
}

/// 收集带客户区坐标的候选窗口（可选按标题片段过滤），按面积从大到小。
pub fn candidates(filter: Option<&String>) -> Result<Vec<Candidate>, String> {
    let mut out: Vec<Candidate> = Vec::new();
    for window in game_windows()? {
        if filter.is_some_and(|needle| !window.title.contains(needle)) {
            continue;
        }
        if let Some(rect) = capture::client_rect(window.handle) {
            out.push(Candidate {
                handle: window.handle,
                title: window.title,
                rect,
            });
        }
    }
    out.sort_by(|a, b| b.area().total_cmp(&a.area()));
    if out.is_empty() {
        return Err("没有找到带客户区坐标的窗口（非 Windows 平台不支持）".into());
    }
    Ok(out)
}

/// 选区和哪个窗口重叠最多——不依赖窗口枚举顺序（多开时顺序不可靠）。
pub fn best_candidate(candidates: &[Candidate], selection: Selection) -> Option<&Candidate> {
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

// -------------------------------------------------------------- 选区换算

/// 屏幕像素矩形 `(x0, y0, x1, y1)`。
pub type Selection = (f32, f32, f32, f32);

/// 屏幕像素矩形 → 相对客户区的百分比区域。
pub fn region_from_selection(
    selection: Selection,
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

/// 区域 → `config.json` 里的百分比字符串。
pub fn region_text(region: core::TaskRegion) -> String {
    format!(
        "{:.4},{:.4},{:.4},{:.4}",
        region.0, region.1, region.2, region.3
    )
}

/// 两张同尺寸图的平均像素差；尺寸差太多就没法比。
pub fn mean_abs_diff(a: &image::RgbImage, b: &image::RgbImage) -> Option<f32> {
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

// ------------------------------------------------------- 从 config.json 读参数

/// `config.json` 里和采集相关的几项。读不出来就用默认值（配置本来也可以没有）。
pub struct CaptureDefaults {
    pub region: core::TaskRegion,
    /// 按分辨率保存的识别区域（和主程序一样：1218x994 一套）。
    pub region_presets: HashMap<String, core::TaskRegion>,
    pub preprocess: String,
    pub color: [u8; 3],
    pub tolerance: u8,
    pub upscale: u32,
    pub high_recall: bool,
}

impl CaptureDefaults {
    /// 当前窗口分辨率下该用的区域：优先用分辨率预设，再退到通用区域。
    pub fn region_for(&self, size: Option<(u32, u32)>) -> core::TaskRegion {
        size.and_then(|(width, height)| {
            self.region_presets
                .get(&format!("{width}x{height}"))
                .copied()
        })
        .unwrap_or(self.region)
    }
}

pub fn read_capture_defaults() -> CaptureDefaults {
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
pub fn config_path() -> PathBuf {
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

// ------------------------------------------------------------- 写回 config.json

/// 把区域写回 `config.json`：更新 `ocr/task_region`，并补上当前分辨率的预设。
/// 改写后会先解析一遍，解析不过就放弃（不碰原文件）。
pub fn write_config_region(path: &Path, region: &str, size_key: &str) -> Result<(), String> {
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
pub fn replace_key_value(text: &str, key: &str, value: &str) -> String {
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
pub fn patch_preset(text: &str, size_key: &str, value: &str) -> String {
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

// --------------------------------------------------------------- 标签对齐

pub fn is_digitish(ch: char) -> bool {
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
pub fn labels_for(text: &str, cells: usize, learn_all: bool) -> Option<Vec<Option<char>>> {
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

/// 坐标字符覆盖情况：缺哪些，缺了就基本用不上。
pub fn coverage_report(lib: &GlyphLib) -> Result<String, String> {
    let missing = font_lib::missing_coordinate_labels(lib);
    if missing.is_empty() {
        return Ok("0-9 和 , . 都有了".into());
    }
    Err(missing.iter().collect())
}

/// 中文字体，让 GUI 上的中文不显示成方块。
pub fn install_cjk(ctx: &egui::Context) {
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn region_text_round_trips_through_parse_region() {
        let text = region_text((0.8414, 0.2309, 0.9873, 0.3382));
        assert_eq!(text, "0.8414,0.2309,0.9873,0.3382");
        assert!(core::parse_region(&text).is_some());
    }

    #[test]
    fn picks_window_with_biggest_overlap() {
        let candidates = vec![
            Candidate {
                handle: 1,
                title: "主窗口".into(),
                rect: (0, 0, 800, 600),
            },
            Candidate {
                handle: 2,
                title: "聊天窗口".into(),
                rect: (800, 0, 800, 600),
            },
        ];
        let in_chat = (900.0, 100.0, 1200.0, 300.0);
        assert_eq!(best_candidate(&candidates, in_chat).unwrap().handle, 2);
        let in_main = (100.0, 100.0, 400.0, 300.0);
        assert_eq!(best_candidate(&candidates, in_main).unwrap().handle, 1);
        // 完全在窗口外：不匹配。
        let outside = (10.0, 900.0, 40.0, 950.0);
        assert!(best_candidate(&candidates, outside).is_none());
    }

    #[test]
    fn only_game_titles_count() {
        assert!(is_game_title("梦幻西游 ONLINE - (山东1区)"));
        assert!(is_game_title("MHXY - 3"));
        assert!(!is_game_title("π - rust-mhxy"));
        assert!(!is_game_title("Microsoft Text Input Application"));
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
        let value: serde_json::Value = json5::from_str(&patched).unwrap();
        assert_eq!(value["ocr/region_presets"]["1920x1080"], "0.5,0.6,0.7,0.8");

        let empty = "{\n  \"ocr/region_presets\": {},\n}\n";
        let patched = patch_preset(empty, "812x663", "0.5,0.6,0.7,0.8");
        let value: serde_json::Value = json5::from_str(&patched).unwrap();
        assert_eq!(value["ocr/region_presets"]["812x663"], "0.5,0.6,0.7,0.8");
    }

    /// 真实配置的写法：多行 presets，**最后一项没有逗号**。
    /// 之前直接插一行会写出非法 JSON5，这条是回归用例。
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
        // 混着中文时只学数字那几位（前提：调用方已确认是单行且字数对得上）。
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

/// 从一帧面板里学一批字模，返回学到的数量。
/// 从一帧面板里学一批字模，返回学到的数量。
///
/// `font-tool capture` 和 `font-studio`（固定帧数 / 后台积累）共用这一段：
/// OCR 定位 → 按框裁图 → 切字 → **按字宽把文字对齐到切出来的跨度** →
/// 交给 `Learner` 聚类投票。
///
/// 三道闸门挡住「把中文当数字学进去」：
/// 1. 只收单行（跨行框会把两行的字按 x 交错，按下标对标签必错）；
/// 2. 对齐不上（字数与跨度差太多）就整段跳过；
/// 3. 默认只学数字/分隔符那几位。
pub fn learn_panel(
    learner: &mut font_lib::Learner,
    panel: &image::RgbImage,
    ocr: &crate::ocr::OcrText,
    learn_all: bool,
    skipped: &mut usize,
    padding: u32,
    dump: Option<(&std::path::Path, u32)>,
) -> usize {
    let (cell_w, cell_h) = learner.cell();
    let cfg = font_lib::MatchConfig::default();
    let mut learned = 0usize;
    for (index, region) in ocr.regions.iter().enumerate() {
        let Some((x, y, width, height)) = coord_fix::padded_box(region, padding, panel) else {
            continue;
        };
        let Ok(crop) = font_lib::crop(panel, x, y, width, height) else {
            continue;
        };
        let gray = font_lib::binarize(&crop);
        if font_lib::row_spans(&gray, cfg.row_gap).len() != 1 {
            *skipped += 1;
            continue;
        }
        // ① 列投影 / 字宽 DP 对齐（纯数字行最准）
        let widths_of = |boxes: &[font_lib::GlyphBox]| -> Vec<u32> {
            boxes.iter().map(|b| b.span.width()).collect()
        };
        let mut boxes = font_lib::analyze(&gray, cfg);
        let mut blocks = font_lib::align_chars_to_spans(&region.text, &widths_of(&boxes));
        if blocks.is_none() {
            let fallback = font_lib::analyze_components(&gray, cfg);
            if !fallback.is_empty() {
                let candidate = font_lib::align_chars_to_spans(&region.text, &widths_of(&fallback));
                if candidate.is_some() {
                    boxes = fallback;
                    blocks = candidate;
                }
            }
        }
        // ② 混排中文时对齐基本不可能，改用「窄字形」策略：
        //    一行里数字大约只有中文一半宽，挑出窄组并按间距分段，
        //    段长正好等于坐标长度就打标签 —— 中文认得对不对完全不影响。
        let mut narrow_labels: Option<Vec<(font_lib::GlyphBox, char)>> = None;
        if blocks.is_none() {
            if let Some(shape) = coord_fix::coordinate_shape(&region.text) {
                let coord: Vec<char> = region
                    .text
                    .chars()
                    .skip(shape.start)
                    .take(shape.len())
                    .collect();
                let groups = font_lib::analyze_components(&gray, cfg);
                let indices = font_lib::narrow_groups(&groups);
                let runs: Vec<Vec<usize>> = font_lib::group_runs(&groups, &indices)
                    .into_iter()
                    .filter(|run| run.len() == coord.len())
                    .collect();
                // 只有唯一一段长度对得上才用：多段都对上说明分不清是哪一段。
                if runs.len() == 1 {
                    narrow_labels = Some(
                        runs[0]
                            .iter()
                            .zip(coord)
                            .map(|(index, ch)| (groups[*index], ch))
                            .collect(),
                    );
                }
            }
        }

        if let Some((dir, frame)) = dump {
            let stem = dir.join(format!("f{frame:02}-{index:02}"));
            let _ = crop.save(stem.with_extension("png"));
            let _ = std::fs::write(stem.with_extension("txt"), &region.text);
        }

        if let Some(labels) = narrow_labels {
            for (b, ch) in labels {
                if !learn_all && !is_digitish(ch) {
                    continue;
                }
                let cell = font_lib::cell_of(&gray, &b, cell_w, cell_h);
                learner.observe(cell, &ch.to_string());
                learned += 1;
            }
            continue;
        }

        let Some(blocks) = blocks else {
            *skipped += 1;
            continue;
        };
        for (b, block) in boxes.iter().zip(blocks) {
            for (sub, ch) in split_span_by_block(b, &block).iter().zip(block.iter()) {
                if !learn_all && !is_digitish(*ch) {
                    continue;
                }
                let cell = font_lib::cell_of(&gray, sub, cell_w, cell_h);
                learner.observe(cell, &ch.to_string());
                learned += 1;
            }
        }
    }
    learned
}
/// 一个跨度里可能并了好几个字（中文紧贴、`119,60` 这种连续数字），
/// 这里按块内字宽比例把跨度再切开，保证「一个框 ↔ 一个字」。
fn split_span_by_block(b: &font_lib::GlyphBox, block: &[char]) -> Vec<font_lib::GlyphBox> {
    if block.len() <= 1 {
        return vec![*b];
    }
    let units: Vec<f32> = block
        .iter()
        .map(|ch| font_lib::char_width_units(*ch))
        .collect();
    let total: f32 = units.iter().sum();
    let width = b.span.width().max(1);
    let mut out = Vec::with_capacity(block.len());
    let mut x0 = b.span.x0;
    let mut consumed = 0f32;
    for (index, unit) in units.iter().enumerate() {
        consumed += unit;
        let x1 = if index + 1 == block.len() {
            b.span.x1
        } else {
            b.span.x0 + (width as f32 * consumed / total).round() as u32
        };
        out.push(font_lib::GlyphBox {
            span: font_lib::Span {
                x0,
                x1: x1.max(x0 + 1).min(b.span.x1),
            },
            top: b.top,
            bottom: b.bottom,
        });
        x0 = x1;
    }
    out
}
