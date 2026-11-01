//! 点阵字库（大漠式「字库」）：闭集模板匹配，用来替代/校验 OCR 对坐标数字的识别。
//!
//! 只依赖 `image` + `serde`，可以单独编译成 `font-tool` 做建库和调试。
//!
//! 流程：灰度 → Otsu 二值化（黑字白底）→ 竖直投影切字 → 按**行高**归一化裁剪
//! → 缩放到固定 cell → zNCC 打分（±N 像素平移取最优）→ argmax + margin 判定。
//!
//! 和 OCR 的分工：OCR 负责「这段文字大概是什么」，字库负责「这几个数字到底是几」。
//! 字库是闭集（0-9、逗号、句点…），只需要 argmax，不需要绝对精度，所以小字号、
//! 低对比度下比通用识别稳得多。
//!
//! 建库和识别必须用同一套归一化：裁剪框是「字的 x 跨度 × 整行行高」，而不是
//! 各自的墨迹外框 —— 否则逗号和句点归一化后会变成同一个墨团。

// 这个模块同时被 GUI 和 `font-tool` 两个 target 编译：建库、多行读取这些只给工具用，
// 在 GUI target 里必然是一片 dead_code，所以整模块放开。
#![allow(dead_code)]

use image::{GrayImage, Luma, RgbImage};

/// 默认 cell 尺寸。取 16x24 是因为任务栏数字大致是 1:1.5 的竖长比例。
pub const DEFAULT_CELL_W: u32 = 16;
pub const DEFAULT_CELL_H: u32 = 24;

// ---------------------------------------------------------------- 图像预处理

/// RGB → 灰度（和 `preprocess::luma` 同一组权重，保证两条路径结果一致）。
pub fn to_gray(image: &RgbImage) -> GrayImage {
    GrayImage::from_fn(image.width(), image.height(), |x, y| {
        let pixel = image.get_pixel(x, y);
        let value =
            ((77 * pixel[0] as u32 + 150 * pixel[1] as u32 + 29 * pixel[2] as u32) >> 8) as u8;
        Luma([value])
    })
}

/// Otsu 阈值。字库自带一份，避免为了一个工具 bin 把整个 crate 拉起来。
pub fn otsu(gray: &GrayImage) -> u8 {
    let mut histogram = [0u32; 256];
    for pixel in gray.pixels() {
        histogram[pixel[0] as usize] += 1;
    }
    let total: u32 = gray.width() * gray.height();
    if total == 0 {
        return 128;
    }
    let mut sum_all = 0f64;
    for (value, count) in histogram.iter().enumerate() {
        sum_all += value as f64 * *count as f64;
    }
    let (mut sum_back, mut weight_back, mut best, mut threshold) = (0f64, 0u32, 0f64, 128u8);
    for (value, count) in histogram.iter().enumerate() {
        weight_back += *count;
        if weight_back == 0 {
            continue;
        }
        let weight_fore = total - weight_back;
        if weight_fore == 0 {
            break;
        }
        sum_back += value as f64 * *count as f64;
        let mean_back = sum_back / weight_back as f64;
        let mean_fore = (sum_all - sum_back) / weight_fore as f64;
        let between = weight_back as f64 * weight_fore as f64 * (mean_back - mean_fore).powi(2);
        if between > best {
            best = between;
            threshold = value as u8;
        }
    }
    threshold
}

/// 二值化成「黑字白底」，字库的输入约定就是这个形态。
///
/// 自动判极性：预处理里勾了反色（白字黑底）时，黑像素会占多数，此时翻转输出，
/// 这样字库不需要关心上游有没有反色。
pub fn binarize(image: &RgbImage) -> GrayImage {
    let gray = to_gray(image);
    let threshold = otsu(&gray);
    let mut dark = 0u32;
    for pixel in gray.pixels() {
        if pixel[0] <= threshold {
            dark += 1;
        }
    }
    let inverted = dark * 2 > gray.width() * gray.height();
    GrayImage::from_fn(gray.width(), gray.height(), |x, y| {
        let value = gray.get_pixel(x, y)[0];
        let is_text = (value <= threshold) != inverted;
        Luma([if is_text { 0 } else { 255 }])
    })
}

/// 从截图里裁一块（工具用；越界返回错误，便于发现坐标写错）。
pub fn crop(image: &RgbImage, x: u32, y: u32, width: u32, height: u32) -> Result<RgbImage, String> {
    let (image_w, image_h) = (image.width(), image.height());
    if x >= image_w || y >= image_h || width == 0 || height == 0 {
        return Err(format!(
            "裁剪区域 {x},{y} {width}x{height} 超出图片 {image_w}x{image_h}"
        ));
    }
    let width = width.min(image_w - x);
    let height = height.min(image_h - y);
    Ok(image::imageops::crop_imm(image, x, y, width, height).to_image())
}

// ------------------------------------------------------------------ 字库数据

/// 一个字模：固定 cell 内的灰度点阵（0=黑，255=白）。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Glyph {
    pub label: String,
    pub pixels: Vec<u8>,
}

/// 字库：同一 cell 尺寸下的一组字模。同 label 可以有多个样本（不同字号/粗细），
/// 匹配时取分数最高的那个，不额外合并。
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct GlyphLib {
    pub cell_w: u32,
    pub cell_h: u32,
    pub glyphs: Vec<Glyph>,
}

impl GlyphLib {
    pub fn new(cell_w: u32, cell_h: u32) -> Self {
        Self {
            cell_w: cell_w.max(1),
            cell_h: cell_h.max(1),
            glyphs: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.glyphs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.glyphs.is_empty()
    }

    /// 直接加入一个已经归一化好的 cell 点阵。
    pub fn add_pixels(&mut self, label: impl Into<String>, pixels: Vec<u8>) {
        let expected = (self.cell_w * self.cell_h) as usize;
        assert_eq!(
            pixels.len(),
            expected,
            "字模尺寸必须是 {}x{}",
            self.cell_w,
            self.cell_h
        );
        self.glyphs.push(Glyph {
            label: label.into(),
            pixels,
        });
    }

    /// 把一张单字图裁到 cell 后加入（要求 `gray` 已经是黑字白底）。
    pub fn add_glyph(&mut self, label: impl Into<String>, gray: &GrayImage) {
        let pixels = resize_to_cell(gray, self.cell_w, self.cell_h);
        self.add_pixels(label, pixels);
    }

    /// 各 label 的样本数，方便建库后自查。
    pub fn label_counts(&self) -> Vec<(String, usize)> {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for glyph in &self.glyphs {
            match counts.iter_mut().find(|(label, _)| *label == glyph.label) {
                Some((_, count)) => *count += 1,
                None => counts.push((glyph.label.clone(), 1)),
            }
        }
        counts.sort_by(|a, b| a.0.cmp(&b.0));
        counts
    }

    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|err| format!("读取字库 {} 失败：{err}", path.display()))?;
        serde_json::from_str(&text)
            .map_err(|err| format!("解析字库 {} 失败：{err}", path.display()))
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        let text =
            serde_json::to_string_pretty(self).map_err(|err| format!("序列化字库失败：{err}"))?;
        std::fs::write(path, text).map_err(|err| format!("写入字库 {} 失败：{err}", path.display()))
    }
}

// ------------------------------------------------------------------ 切字布局

/// 一个字的 x 跨度（半开区间）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub x0: u32,
    pub x1: u32,
}

impl Span {
    pub fn width(&self) -> u32 {
        self.x1.saturating_sub(self.x0)
    }
}

/// 最终参与匹配的字框：x 跨度 + 统一的行高区间。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlyphBox {
    pub span: Span,
    pub top: u32,
    pub bottom: u32,
}

/// 匹配参数。建库和识别必须用同一组值，否则切字/归一化会不一致。
#[derive(Clone, Copy, Debug)]
pub struct MatchConfig {
    /// 一列至少要有多少个黑像素才算笔画（滤掉 JPEG/缩放噪点）。
    pub min_ink: u32,
    /// 列间空隙不超过这个值就并成同一个字。
    pub gap_merge: u32,
    /// 单字最佳分数低于该值时，尝试在中间劈开再匹配（处理粘连字）。
    pub split_when_below: f32,
    /// 劈开后两个字都要比整块高这么多分才真的拆（防止把宽笔画字拆坏）。
    pub split_gain: f32,
    /// 最大递归拆分深度。
    pub max_split_depth: u32,
    /// 单字宽度低于该像素数就不尝试拆分。
    pub min_split_width: u32,
    /// 匹配时允许的 ±像素平移，吸收抗锯齿和基线抖动。
    pub shift: i32,
    /// 最优分数低于该值视为不可信。
    pub min_score: f32,
    /// 最优与次优的差值低于该值视为不可信（形近字）。
    pub min_margin: f32,
    /// 行切分时行间空隙容差。
    pub row_gap: u32,
}

impl Default for MatchConfig {
    fn default() -> Self {
        Self {
            min_ink: 1,
            gap_merge: 1,
            split_when_below: 0.95,
            split_gain: 0.05,
            max_split_depth: 3,
            min_split_width: 4,
            shift: 2,
            min_score: 0.5,
            min_margin: 0.05,
            row_gap: 2,
        }
    }
}

/// 按行切分（横向投影），返回 (top, bottom) 半开区间。
pub fn row_spans(gray: &GrayImage, gap_merge: u32) -> Vec<(u32, u32)> {
    let (width, height) = (gray.width() as usize, gray.height() as usize);
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let mut spans: Vec<(u32, u32)> = Vec::new();
    let mut current: Option<(u32, u32)> = None;
    let mut gap = 0u32;
    for y in 0..height {
        let mut ink = false;
        for x in 0..width {
            if gray.get_pixel(x as u32, y as u32)[0] < 128 {
                ink = true;
                break;
            }
        }
        match (&mut current, ink) {
            (None, true) => current = Some((y as u32, y as u32 + 1)),
            (Some(span), true) => {
                span.1 = y as u32 + 1;
                gap = 0;
            }
            (Some(span), false) => {
                gap += 1;
                if gap > gap_merge {
                    spans.push(*span);
                    current = None;
                }
            }
            (None, false) => {}
        }
    }
    if let Some(span) = current {
        spans.push(span);
    }
    spans
}

/// 单行切字：竖直投影得到 x 跨度，再统一成整行的竖直区间。
pub fn analyze(gray: &GrayImage, cfg: MatchConfig) -> Vec<GlyphBox> {
    let (width, height) = (gray.width() as usize, gray.height() as usize);
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let ink: Vec<bool> = gray.pixels().map(|pixel| pixel[0] < 128).collect();

    // 1) 列投影 → 粗跨度。空隙超过 gap_merge 才断开，避免抗锯齿把同一个字撕成两半。
    let mut spans: Vec<Span> = Vec::new();
    let mut current: Option<Span> = None;
    let mut gap = 0u32;
    for x in 0..width {
        let count = (0..height).filter(|&y| ink[y * width + x]).count() as u32;
        let has_ink = count >= cfg.min_ink;
        match (&mut current, has_ink) {
            (None, true) => {
                current = Some(Span {
                    x0: x as u32,
                    x1: x as u32 + 1,
                });
                gap = 0;
            }
            (Some(span), true) => {
                span.x1 = x as u32 + 1;
                gap = 0;
            }
            (Some(span), false) => {
                gap += 1;
                if gap > cfg.gap_merge {
                    spans.push(*span);
                    current = None;
                }
            }
            (None, false) => {}
        }
    }
    if let Some(span) = current {
        spans.push(span);
    }

    // 2) 每个跨度的竖直墨迹范围，用来推整行行高。
    let mut boxes: Vec<GlyphBox> = Vec::new();
    for span in spans {
        let mut top = u32::MAX;
        let mut bottom = 0u32;
        for x in span.x0..span.x1 {
            for y in 0..height {
                if ink[y * width + x as usize] {
                    top = top.min(y as u32);
                    bottom = bottom.max(y as u32 + 1);
                }
            }
        }
        if bottom > top {
            boxes.push(GlyphBox { span, top, bottom });
        }
    }
    if boxes.is_empty() {
        return Vec::new();
    }

    // 3) 行高取「够高的那些字」的外包络：逗号/句点/小数点太矮，不参与定基线，
    //    但它们最终也用这条基线裁剪，因此逗号和句点在 cell 里的位置不同，可以区分。
    let max_height = boxes.iter().map(|b| b.bottom - b.top).max().unwrap_or(1) as f32;
    let line_top = boxes
        .iter()
        .filter(|b| ((b.bottom - b.top) as f32) >= 0.5 * max_height)
        .map(|b| b.top)
        .min()
        .unwrap_or(0);
    let line_bottom = boxes
        .iter()
        .filter(|b| ((b.bottom - b.top) as f32) >= 0.5 * max_height)
        .map(|b| b.bottom)
        .max()
        .unwrap_or(gray.height());

    // 4) 统一使用行高区间：逗号/句点靠「在 cell 里的高低位置」区分，不能被拉到同一高度。
    boxes
        .into_iter()
        .map(|b| GlyphBox {
            span: b.span,
            top: line_top,
            bottom: line_bottom,
        })
        .collect()
}

/// 把一个字框裁出来缩放到固定 cell。
pub fn cell_of(gray: &GrayImage, b: &GlyphBox, cell_w: u32, cell_h: u32) -> Vec<u8> {
    let width = b
        .span
        .width()
        .max(1)
        .min(gray.width().saturating_sub(b.span.x0).max(1));
    let top = b.top.min(gray.height().saturating_sub(1));
    let height = (b.bottom.saturating_sub(top))
        .max(1)
        .min(gray.height() - top);
    let crop = image::imageops::crop_imm(gray, b.span.x0.min(gray.width() - 1), top, width, height)
        .to_image();
    resize_to_cell(&crop, cell_w, cell_h)
}

fn resize_to_cell(gray: &GrayImage, cell_w: u32, cell_h: u32) -> Vec<u8> {
    let resized =
        image::imageops::resize(gray, cell_w, cell_h, image::imageops::FilterType::Triangle);
    resized.pixels().map(|pixel| pixel[0]).collect()
}

// -------------------------------------------------------------------- 匹配

/// 一个字的匹配结果。
#[derive(Clone, Debug)]
pub struct GlyphMatch {
    pub label: String,
    pub score: f32,
    /// 最优与次优的差值：形近字（0/O、1/l、8/B）靠它暴露风险。
    pub margin: f32,
    pub x: u32,
    pub width: u32,
}

/// 一整行的识别结果。
#[derive(Clone, Debug, Default)]
pub struct LineRead {
    pub text: String,
    pub glyphs: Vec<GlyphMatch>,
    /// 每个字都过了分数和差值阈值。
    pub confident: bool,
    pub top: u32,
}

/// 零均值归一化互相关，带 ±shift 的平移搜索取最大值。
fn score_shifted(template: &[u8], patch: &[u8], width: usize, height: usize, shift: i32) -> f32 {
    let mut best = f32::MIN;
    for dy in -shift..=shift {
        for dx in -shift..=shift {
            let score = ncc(template, patch, width, height, dx, dy);
            if score > best {
                best = score;
            }
        }
    }
    if best.is_finite() {
        best
    } else {
        0.0
    }
}

fn ncc(template: &[u8], patch: &[u8], width: usize, height: usize, dx: i32, dy: i32) -> f32 {
    let count = (width * height) as f64;
    if count == 0.0 {
        return 0.0;
    }
    let mut sum_t = 0f64;
    let mut sum_p = 0f64;
    for y in 0..height {
        for x in 0..width {
            sum_t += template[y * width + x] as f64;
            sum_p += patch[shifted_index(x, y, width, height, dx, dy)] as f64;
        }
    }
    let mean_t = sum_t / count;
    let mean_p = sum_p / count;
    let mut numerator = 0f64;
    let mut var_t = 0f64;
    let mut var_p = 0f64;
    for y in 0..height {
        for x in 0..width {
            let a = template[y * width + x] as f64 - mean_t;
            let b = patch[shifted_index(x, y, width, height, dx, dy)] as f64 - mean_p;
            numerator += a * b;
            var_t += a * a;
            var_p += b * b;
        }
    }
    if var_t <= 1e-9 || var_p <= 1e-9 {
        // 纯色块（空白或全黑）没法用相关性区分，直接判 0 分，交给上层当不可信。
        return 0.0;
    }
    (numerator / (var_t.sqrt() * var_p.sqrt())) as f32
}

fn shifted_index(x: usize, y: usize, width: usize, height: usize, dx: i32, dy: i32) -> usize {
    let px = (x as i32 + dx).clamp(0, width as i32 - 1) as usize;
    let py = (y as i32 + dy).clamp(0, height as i32 - 1) as usize;
    py * width + px
}

/// 单个字框的最佳候选。`margin` 是「最优 label」与「次优 label」的分数差，
/// 同一个 label 的多个样本不算作弊，否则多建几个样本就会把自己的 margin 压到 0。
fn best_match(
    gray: &GrayImage,
    b: &GlyphBox,
    lib: &GlyphLib,
    cfg: MatchConfig,
) -> Option<(String, f32, f32)> {
    let (cell_w, cell_h) = (lib.cell_w as usize, lib.cell_h as usize);
    let cell = cell_of(gray, b, lib.cell_w, lib.cell_h);
    let mut per_label: Vec<(&str, f32)> = Vec::with_capacity(lib.glyphs.len());
    for glyph in &lib.glyphs {
        let score = score_shifted(&glyph.pixels, &cell, cell_w, cell_h, cfg.shift);
        match per_label
            .iter_mut()
            .find(|(label, _)| *label == glyph.label.as_str())
        {
            Some((_, best)) => {
                if score > *best {
                    *best = score;
                }
            }
            None => per_label.push((glyph.label.as_str(), score)),
        }
    }
    per_label.sort_by(|a, b| b.1.total_cmp(&a.1));
    let (label, best) = *per_label.first()?;
    let second = per_label.get(1).map_or(-1.0, |item| item.1);
    Some((label.to_string(), best, best - second))
}

/// 递归落盘：整块分数不够高时尝试从中间劈开，两半都明显更准时才接受拆分。
/// 这一步处理「11」「00」这类粘连字，又不会把本来就匹配得很好的宽字拆坏。
fn collect_matches(
    gray: &GrayImage,
    b: &GlyphBox,
    lib: &GlyphLib,
    cfg: MatchConfig,
    depth: u32,
    out: &mut Vec<GlyphMatch>,
) {
    let Some((label, score, margin)) = best_match(gray, b, lib, cfg) else {
        return;
    };
    if depth < cfg.max_split_depth
        && b.span.width() >= cfg.min_split_width
        && score < cfg.split_when_below
    {
        let middle = b.span.x0 + b.span.width() / 2;
        let left = GlyphBox {
            span: Span {
                x0: b.span.x0,
                x1: middle,
            },
            ..*b
        };
        let right = GlyphBox {
            span: Span {
                x0: middle,
                x1: b.span.x1,
            },
            ..*b
        };
        let (Some((_, left_score, _)), Some((_, right_score, _))) = (
            best_match(gray, &left, lib, cfg),
            best_match(gray, &right, lib, cfg),
        ) else {
            out.push(GlyphMatch {
                label,
                score,
                margin,
                x: b.span.x0,
                width: b.span.width(),
            });
            return;
        };
        if left_score.min(right_score) > score + cfg.split_gain {
            collect_matches(gray, &left, lib, cfg, depth + 1, out);
            collect_matches(gray, &right, lib, cfg, depth + 1, out);
            return;
        }
    }
    out.push(GlyphMatch {
        label,
        score,
        margin,
        x: b.span.x0,
        width: b.span.width(),
    });
}

/// 识别单行（输入必须是黑字白底的单行图）。
pub fn read_line_gray(gray: &GrayImage, lib: &GlyphLib, cfg: MatchConfig) -> LineRead {
    let mut line = LineRead::default();
    if lib.is_empty() {
        return line;
    }
    for b in analyze(gray, cfg) {
        collect_matches(gray, &b, lib, cfg, 0, &mut line.glyphs);
    }
    for glyph in &line.glyphs {
        line.text.push_str(&glyph.label);
    }
    line.confident = !line.glyphs.is_empty()
        && line
            .glyphs
            .iter()
            .all(|g| g.score >= cfg.min_score && g.margin >= cfg.min_margin);
    line
}

/// 识别整块面板：先按行切，再逐行识别。空行会被丢掉。
pub fn read_lines(panel: &RgbImage, lib: &GlyphLib, cfg: MatchConfig) -> Vec<LineRead> {
    let gray = binarize(panel);
    let mut out = Vec::new();
    for (top, bottom) in row_spans(&gray, cfg.row_gap) {
        let height = bottom.saturating_sub(top).max(1);
        if height > gray.height() - top {
            break;
        }
        let line_image = image::imageops::crop_imm(&gray, 0, top, gray.width(), height).to_image();
        let mut line = read_line_gray(&line_image, lib, cfg);
        if line.glyphs.is_empty() {
            continue;
        }
        line.top = top;
        out.push(line);
    }
    out
}

// -------------------------------------------------------------------- 建库

/// 从「一行截图 + 这行的准确文字」建库。
///
/// 建库和识别共用同一套切字/归一化代码，保证字模和运行时输入同分布。
/// 切出来的字数与文字个数不一致的样本会被跳过并记一条 warning（通常是裁剪时
/// 切掉了字，或者文字里混进了空格）。
pub fn build_from_lines(
    cell_w: u32,
    cell_h: u32,
    samples: &[(String, GrayImage)],
    cfg: MatchConfig,
) -> (GlyphLib, Vec<String>) {
    let mut lib = GlyphLib::new(cell_w, cell_h);
    let mut warnings = Vec::new();
    for (text, gray) in samples {
        let chars: Vec<char> = text.chars().filter(|ch| !ch.is_whitespace()).collect();
        let boxes = analyze(gray, cfg);
        if boxes.is_empty() {
            warnings.push(format!("「{text}」没切出任何字，已跳过"));
            continue;
        }
        if boxes.len() != chars.len() {
            warnings.push(format!(
                "「{text}」切出 {} 个字，文字有 {} 个字，已跳过（检查裁剪范围）",
                boxes.len(),
                chars.len()
            ));
            continue;
        }
        for (b, ch) in boxes.iter().zip(&chars) {
            lib.add_pixels(*ch, cell_of(gray, b, cell_w, cell_h));
        }
    }
    (lib, warnings)
}

/// 从目录建库：`dir/*.png` 配一个同名 `*.txt`（该行截图对应的准确文字）。
pub fn build_from_dir(
    dir: &std::path::Path,
    cell_w: u32,
    cell_h: u32,
    cfg: MatchConfig,
) -> Result<(GlyphLib, Vec<String>), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|err| format!("读取样本目录 {} 失败：{err}", dir.display()))?;
    let mut samples: Vec<(String, GrayImage)> = Vec::new();
    let mut warnings = Vec::new();
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(err) => {
                warnings.push(format!("跳过无法读取的目录项：{err}"));
                continue;
            }
        };
        if path.extension().and_then(|ext| ext.to_str()) != Some("png") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let text_path = path.with_file_name(format!("{stem}.txt"));
        let Ok(text) = std::fs::read_to_string(&text_path) else {
            warnings.push(format!("{} 缺少同名 .txt，已跳过", path.display()));
            continue;
        };
        let text = text.trim().to_string();
        let Ok(image) = image::open(&path) else {
            warnings.push(format!("读取图片 {} 失败，已跳过", path.display()));
            continue;
        };
        samples.push((text, binarize(&image.to_rgb8())));
    }
    let (lib, build_warnings) = build_from_lines(cell_w, cell_h, &samples, cfg);
    warnings.extend(build_warnings);
    Ok((lib, warnings))
}

/// 测试夹具：用 5x7 点阵字合成“游戏截图”。
///
/// `coord_fix` 的测试也用它，所以 `pub(crate)` + `cfg(test)`，不进入发布二进制。
#[cfg(test)]
pub(crate) mod fixture {
    use super::*;
    use image::Rgb;

    /// 5x7 点阵字，用来合成测试图；真实场景换成游戏字体截图即可。
    pub fn pattern(ch: char) -> Option<[&'static str; 7]> {
        let lines = match ch {
            '0' => [
                "01110", "10001", "10001", "10001", "10001", "10001", "01110",
            ],
            '1' => [
                "00100", "01100", "00100", "00100", "00100", "00100", "01110",
            ],
            '2' => [
                "01110", "10001", "00001", "00010", "00100", "01000", "11111",
            ],
            '3' => [
                "11111", "00010", "00100", "00010", "00001", "10001", "01110",
            ],
            '4' => [
                "00010", "00110", "01010", "10010", "11111", "00010", "00010",
            ],
            '5' => [
                "11111", "10000", "11110", "00001", "00001", "10001", "01110",
            ],
            '6' => [
                "00110", "01000", "10000", "11110", "10001", "10001", "01110",
            ],
            '7' => [
                "11111", "00001", "00010", "00100", "01000", "01000", "01000",
            ],
            '8' => [
                "01110", "10001", "10001", "01110", "10001", "10001", "01110",
            ],
            '9' => [
                "01110", "10001", "10001", "01111", "00001", "00010", "01100",
            ],
            ',' => [
                "00000", "00000", "00000", "00000", "00000", "00100", "01000",
            ],
            '.' => [
                "00000", "00000", "00000", "00000", "00000", "01100", "01100",
            ],
            _ => return None,
        };
        Some(lines)
    }

    /// 把一行文字渲染成黑字白底的图（scale 模拟字号，gap 模拟字间距）。
    pub fn render(text: &str, scale: u32, gap: u32) -> RgbImage {
        let chars: Vec<char> = text.chars().collect();
        let advance = 5 + gap;
        let width = chars.len() as u32 * advance * scale;
        let height = 7 * scale;
        let mut image = RgbImage::from_pixel(width.max(1), height.max(1), Rgb([255, 255, 255]));
        for (index, ch) in chars.iter().enumerate() {
            let lines = pattern(*ch).expect("测试字库缺少该字符");
            let origin_x = index as u32 * advance * scale;
            for (row, line) in lines.iter().enumerate() {
                for (col, bit) in line.chars().enumerate() {
                    if bit != '1' {
                        continue;
                    }
                    for dy in 0..scale {
                        for dx in 0..scale {
                            image.put_pixel(
                                origin_x + col as u32 * scale + dx,
                                row as u32 * scale + dy,
                                Rgb([20, 20, 20]),
                            );
                        }
                    }
                }
            }
        }
        image
    }

    /// 把一行文字画到一张更大的白底画布的 (x, y) 处，用来模拟“面板里的一块区域”。
    pub fn place(canvas: &mut RgbImage, line: &RgbImage, x: u32, y: u32) {
        image::imageops::overlay(canvas, line, x as i64, y as i64);
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::render;
    use super::*;
    use image::Rgb;

    fn trained_lib() -> GlyphLib {
        // 同一行文字用两种字号各建一次库，验证缩放归一化是否有效。
        let samples = vec![
            (
                "0123456789.,".to_string(),
                binarize(&render("0123456789.,", 4, 1)),
            ),
            (
                "0123456789.,".to_string(),
                binarize(&render("0123456789.,", 6, 1)),
            ),
        ];
        let (lib, warnings) = build_from_lines(
            DEFAULT_CELL_W,
            DEFAULT_CELL_H,
            &samples,
            MatchConfig::default(),
        );
        assert!(warnings.is_empty(), "建库警告：{warnings:?}");
        assert_eq!(lib.len(), 24, "12 个字 × 2 个字号");
        lib
    }

    #[test]
    fn builds_font_lib_from_lines() {
        let lib = trained_lib();
        let counts = lib.label_counts();
        assert_eq!(counts.len(), 12);
        assert!(counts.iter().all(|(_, count)| *count == 2));
        assert_eq!(lib.cell_w, DEFAULT_CELL_W);
    }

    #[test]
    fn reads_unseen_scale_and_position() {
        let lib = trained_lib();
        // 训练时没见过 scale=5，也没见过 "79,26" 这串内容。
        let line = binarize(&render("79,26", 5, 1));
        let read = read_line_gray(&line, &lib, MatchConfig::default());
        assert_eq!(read.text, "79,26");
        assert!(read.confident, "分数/margin 不足：{:?}", read.glyphs);
    }

    #[test]
    fn tolerates_pixel_noise() {
        let lib = trained_lib();
        let mut image = render("104,38", 5, 1);
        // 确定性地在字笔画边缘制造噪点，模拟抗锯齿和压缩失真。
        for y in 0..image.height() {
            for x in 0..image.width() {
                if (x * 7 + y * 13) % 23 == 0 {
                    image.put_pixel(x, y, Rgb([200, 200, 200]));
                }
            }
        }
        let read = read_line_gray(&binarize(&image), &lib, MatchConfig::default());
        assert_eq!(read.text, "104,38");
    }

    #[test]
    fn separates_comma_from_period() {
        let lib = trained_lib();
        let comma = read_line_gray(
            &binarize(&render("1,2", 5, 1)),
            &lib,
            MatchConfig::default(),
        );
        let period = read_line_gray(
            &binarize(&render("1.2", 5, 1)),
            &lib,
            MatchConfig::default(),
        );
        assert_eq!(comma.text, "1,2");
        assert_eq!(period.text, "1.2");
    }

    #[test]
    fn splits_touching_glyphs() {
        let lib = trained_lib();
        // gap=0 让两个字粘在一起；靠「劈开后两半都明显更像单字」判定拆分。
        let read = read_line_gray(&binarize(&render("00", 5, 0)), &lib, MatchConfig::default());
        assert_eq!(read.text, "00");
    }

    #[test]
    fn reads_multiple_rows() {
        let lib = trained_lib();
        let top = render("12,34", 4, 1);
        let bottom = render("56,78", 4, 1);
        let mut panel = RgbImage::from_pixel(top.width(), top.height() * 3, Rgb([255, 255, 255]));
        image::imageops::overlay(&mut panel, &top, 0, 0);
        image::imageops::overlay(&mut panel, &bottom, 0, (top.height() * 2) as i64);
        let lines = read_lines(&panel, &lib, MatchConfig::default());
        let texts: Vec<&str> = lines.iter().map(|line| line.text.as_str()).collect();
        assert_eq!(texts, ["12,34", "56,78"]);
    }

    /// 预处理里勾了反色（白字黑底）时，字库也要能读：binarize 自动判极性。
    #[test]
    fn reads_inverted_text() {
        let lib = trained_lib();
        let normal = render("79,26", 5, 1);
        let mut inverted = normal.clone();
        for pixel in inverted.pixels_mut() {
            pixel.0 = [255 - pixel.0[0], 255 - pixel.0[1], 255 - pixel.0[2]];
        }
        let read = read_line_gray(&binarize(&inverted), &lib, MatchConfig::default());
        assert_eq!(read.text, "79,26");
    }

    #[test]
    fn flags_low_confidence_on_empty_input() {
        let lib = trained_lib();
        let blank = RgbImage::from_pixel(64, 24, Rgb([255, 255, 255]));
        let lines = read_lines(&blank, &lib, MatchConfig::default());
        assert!(lines.is_empty());
        let line = read_line_gray(&binarize(&blank), &lib, MatchConfig::default());
        assert!(!line.confident);
    }

    #[test]
    fn round_trips_through_json() {
        let lib = trained_lib();
        let text = serde_json::to_string(&lib).expect("序列化");
        let restored: GlyphLib = serde_json::from_str(&text).expect("反序列化");
        assert_eq!(restored.cell_w, lib.cell_w);
        assert_eq!(restored.cell_h, lib.cell_h);
        assert_eq!(restored.len(), lib.len());
        let read = read_line_gray(
            &binarize(&render("79,26", 5, 1)),
            &restored,
            MatchConfig::default(),
        );
        assert_eq!(read.text, "79,26");
    }

    /// 走一遍真实落地路径：样本 PNG + .txt → build_from_dir → 存盘 → 读盘 → 识别 PNG。
    #[test]
    fn builds_from_directory_and_reads_png() {
        let dir = std::env::temp_dir().join(format!("rust-mhxy-fontlib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建临时目录");
        for (name, scale) in [("a", 4), ("b", 6)] {
            render("0123456789.,", scale, 1)
                .save(dir.join(format!("{name}.png")))
                .expect("保存样本");
            std::fs::write(dir.join(format!("{name}.txt")), "0123456789.,").expect("写标注");
        }
        let (lib, warnings) =
            build_from_dir(&dir, DEFAULT_CELL_W, DEFAULT_CELL_H, MatchConfig::default())
                .expect("从目录建库");
        assert!(warnings.is_empty(), "建库警告：{warnings:?}");
        assert_eq!(lib.len(), 24);

        let lib_path = dir.join("fontlib.json");
        lib.save(&lib_path).expect("保存字库");
        let lib = GlyphLib::load(&lib_path).expect("加载字库");

        let panel_path = dir.join("panel.png");
        render("79,26", 5, 1).save(&panel_path).expect("保存面板");
        let panel = image::open(&panel_path).expect("读面板").to_rgb8();
        let lines = read_lines(&panel, &lib, MatchConfig::default());
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text, "79,26");
        assert!(lines[0].confident, "{lines:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 把演示样本写到 target/fontlib-demo，方便手工跑 `font-tool`（不参与 CI）。
    #[test]
    #[ignore = "生成演示用的样本和面板，不参与自动测试"]
    fn dump_demo_samples() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/fontlib-demo");
        let samples = root.join("samples");
        std::fs::create_dir_all(&samples).expect("创建演示目录");
        for (name, scale) in [("line1", 4), ("line2", 6)] {
            render("0123456789.,", scale, 1)
                .save(samples.join(format!("{name}.png")))
                .expect("保存演示样本");
            std::fs::write(samples.join(format!("{name}.txt")), "0123456789.,").expect("写标注");
        }
        let panel = render("79,26", 5, 1);
        panel.save(root.join("panel.png")).expect("保存演示面板");
        println!("演示数据已写到 {}", root.display());
    }

    /// 真实截图冒烟：`MHXY_FONT_SAMPLES` 是带 .txt 标注的样本目录，
    /// `MHXY_FONT_IMAGE` 是待识别的面板图（可选 `MHXY_FONT_CROP=x,y,w,h`）。
    #[test]
    #[ignore = "需要 MHXY_FONT_SAMPLES 与 MHXY_FONT_IMAGE 指向真实截图"]
    fn reads_real_screenshot() {
        let samples = std::env::var("MHXY_FONT_SAMPLES").expect("缺少 MHXY_FONT_SAMPLES");
        let image_path = std::env::var("MHXY_FONT_IMAGE").expect("缺少 MHXY_FONT_IMAGE");
        let (lib, warnings) = build_from_dir(
            std::path::Path::new(&samples),
            DEFAULT_CELL_W,
            DEFAULT_CELL_H,
            MatchConfig::default(),
        )
        .expect("建库");
        for warning in &warnings {
            println!("警告：{warning}");
        }
        println!("字库：{:?}", lib.label_counts());
        assert!(!lib.is_empty(), "字库为空，检查样本目录");

        let image = image::open(&image_path).expect("读图").to_rgb8();
        let image = match std::env::var("MHXY_FONT_CROP") {
            Ok(spec) => {
                let parts: Vec<u32> = spec
                    .split(',')
                    .filter_map(|part| part.trim().parse().ok())
                    .collect();
                assert_eq!(parts.len(), 4, "MHXY_FONT_CROP 需要 x,y,w,h");
                crop(&image, parts[0], parts[1], parts[2], parts[3]).expect("裁剪")
            }
            Err(_) => image,
        };
        let started = std::time::Instant::now();
        let lines = read_lines(&image, &lib, MatchConfig::default());
        for line in &lines {
            let detail: Vec<String> = line
                .glyphs
                .iter()
                .map(|glyph| format!("{}({:.2}/{:.2})", glyph.label, glyph.score, glyph.margin))
                .collect();
            println!(
                "y={:<4} {:<16} {}{}",
                line.top,
                line.text,
                if line.confident { "" } else { "不可信 " },
                detail.join(" ")
            );
        }
        println!("耗时 {:.1} ms", started.elapsed().as_secs_f64() * 1000.0);
    }
}
