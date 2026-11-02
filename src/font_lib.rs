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
    analyze_with_advance(gray, cfg, None)
}

/// 同 [`analyze`]，但可以给一个「每字宽度」的先验。
///
/// 中文是紧贴排版的，列投影会把相邻两个字并成一个跨度。建库时有 OCR 的文字，
/// 字数是已知的，用它算出平均字宽，把过宽的跨度按这个宽度等分，字数就能对上。
pub fn analyze_with_advance(
    gray: &GrayImage,
    cfg: MatchConfig,
    advance: Option<f32>,
) -> Vec<GlyphBox> {
    let (width, height) = (gray.width() as usize, gray.height() as usize);
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

    // 4) 有字宽先验时，把「明显太宽」的跨度按先验等分：中文紧贴排版会把相邻的字并在一起。
    let mut out: Vec<GlyphBox> = Vec::with_capacity(boxes.len());
    for b in boxes {
        let width = b.span.width() as f32;
        let parts = match advance {
            Some(advance) if advance > 2.0 && width > advance * 1.35 => {
                ((width / advance).round() as u32).max(2)
            }
            _ => 1,
        };
        if parts <= 1 {
            out.push(GlyphBox {
                span: b.span,
                top: line_top,
                bottom: line_bottom,
            });
            continue;
        }
        for index in 0..parts {
            let x0 = b.span.x0 + b.span.width() * index / parts;
            let x1 = b.span.x0 + b.span.width() * (index + 1) / parts;
            out.push(GlyphBox {
                span: Span { x0, x1 },
                top: line_top,
                bottom: line_bottom,
            });
        }
    }
    out
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

// ---------------------------------------------------------------- 自动建库

/// 自动建库参数。
#[derive(Clone, Copy, Debug)]
pub struct LearnConfig {
    /// 和已有字模相似到这个程度就当作同一个「字模形态」。
    pub min_similarity: f32,
    /// 聚类时允许的平移像素。
    pub shift: i32,
}

impl Default for LearnConfig {
    fn default() -> Self {
        Self {
            min_similarity: 0.9,
            shift: 1,
        }
    }
}

struct Cluster {
    label: String,
    pixels: Vec<u8>,
    votes: std::collections::BTreeMap<String, usize>,
}

impl Cluster {
    fn new(label: &str, pixels: Vec<u8>) -> Self {
        let mut votes = std::collections::BTreeMap::new();
        votes.insert(label.to_string(), 1usize);
        Self {
            label: label.to_string(),
            pixels,
            votes,
        }
    }

    /// 再投一票。只有**严格多于**当前标签票数才改判，平票保持原判，
    /// 否则两帧错认就能把正确的标签翻掉。
    fn vote(&mut self, label: &str) {
        *self.votes.entry(label.to_string()).or_insert(0) += 1;
        let current = *self.votes.get(&self.label).unwrap_or(&0);
        if *self.votes.get(label).unwrap_or(&0) > current {
            self.label = label.to_string();
        }
    }
}

/// 每个字的统计，建完后打印用。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelSummary {
    pub label: String,
    pub clusters: usize,
    pub votes: usize,
    /// 被投票否决掉的错标次数。
    pub rejected: usize,
}

/// 自动建库器：把「切出来的字模 + 那一刻 OCR 给的标签」累积成字库。
///
/// OCR 会偶尔认错，但同一个字模形态（同字体、同抗锯齿、同字号）每次切出来几乎一样，
/// 所以先按外观聚类，再对每个簇做标签投票：多数帧说得对，单帧错认就被压掉。
pub struct Learner {
    cell_w: u32,
    cell_h: u32,
    cfg: LearnConfig,
    clusters: Vec<Cluster>,
}

impl Learner {
    pub fn new(cell_w: u32, cell_h: u32, cfg: LearnConfig) -> Self {
        Self {
            cell_w: cell_w.max(1),
            cell_h: cell_h.max(1),
            cfg,
            clusters: Vec::new(),
        }
    }

    /// 记录一个字模样本（已归一化到 cell 的点阵）。
    pub fn observe(&mut self, pixels: Vec<u8>, label: &str) {
        assert_eq!(
            pixels.len(),
            (self.cell_w * self.cell_h) as usize,
            "字模尺寸必须是 {}x{}",
            self.cell_w,
            self.cell_h
        );
        let label = label.trim();
        if label.is_empty() {
            return;
        }
        let (cell_w, cell_h) = (self.cell_w as usize, self.cell_h as usize);
        let mut best: Option<(usize, f32)> = None;
        for (index, cluster) in self.clusters.iter().enumerate() {
            let score = score_shifted(&cluster.pixels, &pixels, cell_w, cell_h, self.cfg.shift);
            if best.is_none_or(|(_, best_score)| score > best_score) {
                best = Some((index, score));
            }
        }
        match best {
            Some((index, score)) if score >= self.cfg.min_similarity => {
                self.clusters[index].vote(label);
            }
            _ => self.clusters.push(Cluster::new(label, pixels)),
        }
    }

    /// 字模尺寸（建库时就固定了）。
    pub fn cell(&self) -> (u32, u32) {
        (self.cell_w, self.cell_h)
    }

    pub fn cluster_count(&self) -> usize {
        self.clusters.len()
    }

    pub fn sample_count(&self) -> usize {
        self.clusters
            .iter()
            .map(|cluster| cluster.votes.values().sum::<usize>())
            .sum()
    }

    /// 每个字的簇数 / 票数 / 被否决次数，按字排序。
    pub fn summary(&self) -> Vec<LabelSummary> {
        let mut out: Vec<LabelSummary> = Vec::new();
        for cluster in &self.clusters {
            let votes: usize = cluster.votes.values().sum();
            let kept = *cluster.votes.get(&cluster.label).unwrap_or(&0);
            match out.iter_mut().find(|item| item.label == cluster.label) {
                Some(item) => {
                    item.clusters += 1;
                    item.votes += votes;
                    item.rejected += votes - kept;
                }
                None => out.push(LabelSummary {
                    label: cluster.label.clone(),
                    clusters: 1,
                    votes,
                    rejected: votes - kept,
                }),
            }
        }
        out.sort_by(|a, b| a.label.cmp(&b.label));
        out
    }

    /// 值得人看一醒的冲突：同一簇里另一个标签也拿到了 `min_votes` 以上的票。
    pub fn conflicts(&self, min_votes: usize) -> Vec<String> {
        let mut out = Vec::new();
        for cluster in &self.clusters {
            let kept = *cluster.votes.get(&cluster.label).unwrap_or(&0);
            for (label, count) in &cluster.votes {
                if label != &cluster.label && *count >= min_votes {
                    out.push(format!(
                        "同一字模「{label}」{count} 票、「{}」{kept} 票，取「{}」",
                        cluster.label, cluster.label
                    ));
                }
            }
        }
        out
    }

    /// 建成的字库：每个簇一个字模，标签取投票结果。
    pub fn finish(&self) -> GlyphLib {
        let mut lib = GlyphLib::new(self.cell_w, self.cell_h);
        for cluster in &self.clusters {
            lib.add_pixels(cluster.label.clone(), cluster.pixels.clone());
        }
        lib
    }

    /// 用已有字库初始化（合并再采集）：每个字模当作「已有 1 票」的样本。
    ///
    /// 这样分几次采集（不同任务、不同字号的窗口）能逐步补全数字，
    /// 而不是每次都从零开始、把上一次的成果覆盖掉。
    pub fn seed(&mut self, lib: &GlyphLib) -> Result<(), String> {
        if lib.cell_w != self.cell_w || lib.cell_h != self.cell_h {
            return Err(format!(
                "字库 cell 是 {}x{}，当前是 {}x{}，不能合并",
                lib.cell_w, lib.cell_h, self.cell_w, self.cell_h
            ));
        }
        for glyph in &lib.glyphs {
            self.clusters
                .push(Cluster::new(&glyph.label, glyph.pixels.clone()));
        }
        Ok(())
    }
}

/// 坐标需要的字符集：0-9 加两个常用分隔符。采集后缺哪些可以直接报给用户。
pub const COORDINATE_LABELS: [char; 12] =
    ['0', '1', '2', '3', '4', '5', '6', '7', '8', '9', ',', '.'];

/// 坐标分隔符的等价写法：OCR 经常把逗号认成全角。
pub const COORDINATE_SEPARATORS: [char; 6] = [',', '，', '、', '.', '．', '。'];

/// 字库里还没有的坐标字符。
///
/// 分隔符按「等价类」算：库里只要有 `，` 就不算缺 `,`，
/// 否则会把已经能用的字库报成残缺。
pub fn missing_coordinate_labels(lib: &GlyphLib) -> Vec<char> {
    let single = |ch: char| lib.glyphs.iter().any(|glyph| glyph.label == ch.to_string());
    let mut missing: Vec<char> = ('0'..='9').filter(|digit| !single(*digit)).collect();
    if !COORDINATE_SEPARATORS.iter().any(|ch| single(*ch)) {
        // 用 ASCII ',' 代表「缺分隔符」这个语义。
        missing.push(',');
    }
    missing
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
    /// 中文紧贴排版时列投影会把相邻字并起来；用 OCR 的字数反推字宽就能按字数切开。
    #[test]
    fn advance_hint_splits_merged_glyphs() {
        let gray = binarize(&render("79,26", 5, 0));
        let plain = analyze(&gray, MatchConfig::default());
        assert!(plain.len() < 5, "gap=0 应当被并得更少：{}", plain.len());

        let advance = gray.width() as f32 / 5.0;
        let hinted = analyze_with_advance(&gray, MatchConfig::default(), Some(advance));
        assert_eq!(hinted.len(), 5, "按字宽先验应当恰好切成 5 个");

        // 切开之后用同一套字库读回来，应该还是原文。
        let lib = trained_lib();
        let read = read_line_gray(&gray, &lib, MatchConfig::default());
        assert_eq!(read.text, "79,26");
    }

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

#[cfg(test)]
mod learner_tests {
    use super::fixture::render;
    use super::*;

    /// 从一行合成图里切出字模和标签，模拟真实采集流程。
    fn learn_line(learner: &mut Learner, text: &str, scale: u32) {
        let gray = binarize(&render(text, scale, 1));
        let boxes = analyze(&gray, MatchConfig::default());
        let chars: Vec<char> = text.chars().collect();
        assert_eq!(boxes.len(), chars.len(), "切字数量要和文字一致");
        for (b, ch) in boxes.iter().zip(&chars) {
            learner.observe(
                cell_of(&gray, b, learner.cell_w, learner.cell_h),
                &ch.to_string(),
            );
        }
    }

    fn cell_of_char(text: &str, scale: u32, index: usize) -> Vec<u8> {
        let gray = binarize(&render(text, scale, 1));
        let boxes = analyze(&gray, MatchConfig::default());
        cell_of(&gray, &boxes[index], DEFAULT_CELL_W, DEFAULT_CELL_H)
    }

    #[test]
    fn merges_same_glyph_into_one_cluster() {
        let mut learner = Learner::new(DEFAULT_CELL_W, DEFAULT_CELL_H, LearnConfig::default());
        for _ in 0..3 {
            learn_line(&mut learner, "7", 5);
        }
        assert_eq!(learner.cluster_count(), 1);
        assert_eq!(learner.sample_count(), 3);
        assert_eq!(learner.summary()[0].label, "7");
    }

    #[test]
    fn single_misread_is_outvoted() {
        let cell = cell_of_char("7", 5, 0);
        let mut learner = Learner::new(DEFAULT_CELL_W, DEFAULT_CELL_H, LearnConfig::default());
        for _ in 0..3 {
            learner.observe(cell.clone(), "7");
        }
        // OCR 偶尔把 7 认成 1：一票翻不了盘。
        learner.observe(cell.clone(), "1");
        assert_eq!(learner.summary()[0].label, "7");
        assert_eq!(learner.summary()[0].rejected, 1);

        // 但如果多数帧都说 1，就该改判（不信任单一标签）。
        for _ in 0..3 {
            learner.observe(cell.clone(), "1");
        }
        assert_eq!(learner.summary()[0].label, "1");
    }

    #[test]
    fn separates_different_glyphs() {
        let mut learner = Learner::new(DEFAULT_CELL_W, DEFAULT_CELL_H, LearnConfig::default());
        learn_line(&mut learner, "0123456789.,", 5);
        assert_eq!(learner.cluster_count(), 12);
        let labels: Vec<String> = learner.summary().into_iter().map(|s| s.label).collect();
        assert_eq!(
            labels,
            [",", ".", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]
        );
    }

    #[test]
    fn learned_library_can_read_new_line() {
        let mut learner = Learner::new(DEFAULT_CELL_W, DEFAULT_CELL_H, LearnConfig::default());
        learn_line(&mut learner, "0123456789.,", 4);
        learn_line(&mut learner, "0123456789.,", 6);
        let lib = learner.finish();
        let read = read_line_gray(
            &binarize(&render("79,26", 5, 1)),
            &lib,
            MatchConfig::default(),
        );
        assert_eq!(read.text, "79,26");
        assert!(read.confident);
    }

    #[test]
    fn reports_conflicts_between_close_labels() {
        let cell = cell_of_char("8", 5, 0);
        let mut learner = Learner::new(DEFAULT_CELL_W, DEFAULT_CELL_H, LearnConfig::default());
        for _ in 0..2 {
            learner.observe(cell.clone(), "8");
        }
        for _ in 0..2 {
            learner.observe(cell.clone(), "3");
        }
        // 2:2 平票保持原判，但要能报出来给人看。
        assert_eq!(learner.summary()[0].label, "8");
        let conflicts = learner.conflicts(2);
        assert_eq!(conflicts.len(), 1, "{conflicts:?}");
        assert!(conflicts[0].contains("3"), "{conflicts:?}");
    }
}

// ---------------------------------------------------------- 文字↔切字对齐

/// 一个字符占多少「字宽单位」，按游戏字体的实际规律：
/// **2 个数字 = 1 个汉字宽**，**全角符号（，（）：“”）= 1 个汉字宽**。
///
/// 建库时用它把 OCR 的一行文字对齐到切出来的跨度：中文紧贴排版会把相邻字并成
/// 一个跨度，而数字只有半宽，单纯按「平均字宽等分」是分不对的。
pub fn char_width_units(ch: char) -> f32 {
    if ch.is_ascii() {
        // 半角：数字、字母、半角标点都是半个汉字宽。
        0.5
    } else {
        // 汉字和全角标点都是一个汉字宽。
        1.0
    }
}

/// 把一行文字按字宽先验对齐到切出来的跨度（动态规划，最小化宽度误差）。
///
/// 返回每个跨度对应的那段文字；对齐不上（字数与跨度数量差太多、宽度差异离谱）
/// 返回 None —— 宁可不学，也不要把标签按错。
pub fn align_chars_to_spans(text: &str, span_widths: &[u32]) -> Option<Vec<Vec<char>>> {
    let chars: Vec<char> = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    let spans = span_widths.len();
    if chars.is_empty() || spans == 0 || spans > chars.len() {
        return None;
    }
    let total_units: f32 = chars.iter().map(|ch| char_width_units(*ch)).sum();
    let total_width: u32 = span_widths.iter().sum();
    if total_units <= 0.0 || total_width == 0 {
        return None;
    }
    // 一个像素大约对应多少「字宽单位」。
    let scale = total_units / total_width as f32;

    // 每个跨度最多吃这么多个字：合并跨度里可能是好几个数字。
    const MAX_BLOCK: usize = 8;
    // 单个跨度的宽度误差超过这个值就不做对齐：说明切字本身是坏的
    // （背景没清干净、跨行框、粘连），硬按字数打标签只会把字库弄脏。
    const MAX_ALIGN_ERROR: f32 = 0.5;
    let inf = f32::INFINITY;
    // dp[j][i] = 用前 i 个字填满前 j 个跨度时的最小累计误差。
    let mut dp = vec![vec![inf; chars.len() + 1]; spans + 1];
    let mut from = vec![vec![0usize; chars.len() + 1]; spans + 1];
    dp[0][0] = 0.0;
    for j in 1..=spans {
        let target = span_widths[j - 1] as f32 * scale;
        for i in 1..=chars.len() {
            for block in 1..=MAX_BLOCK.min(i) {
                let start = i - block;
                if dp[j - 1][start] == inf {
                    continue;
                }
                let units: f32 = chars[start..i].iter().map(|ch| char_width_units(*ch)).sum();
                // 相对误差的平方：宽跨度多吃几个字、窄跨度少吃几个字。
                let error = ((units - target) / target.max(0.5)).powi(2);
                let cost = dp[j - 1][start] + error;
                if cost < dp[j][i] {
                    dp[j][i] = cost;
                    from[j][i] = block;
                }
            }
        }
    }
    if dp[spans][chars.len()] == inf {
        return None;
    }
    // 回调出每个跨度对应的文字块，并逐个检查宽度误差。
    let mut blocks: Vec<Vec<char>> = vec![Vec::new(); spans];
    let mut index = chars.len();
    for j in (1..=spans).rev() {
        let block = from[j][index];
        if block == 0 {
            return None;
        }
        let measured = span_widths[j - 1] as f32 * scale;
        let units: f32 = chars[index - block..index]
            .iter()
            .map(|ch| char_width_units(*ch))
            .sum();
        if ((units - measured) / measured.max(0.5)).abs() > MAX_ALIGN_ERROR {
            return None;
        }
        blocks[j - 1] = chars[index - block..index].to_vec();
        index -= block;
    }
    if index != 0 {
        return None;
    }
    Some(blocks)
}

#[cfg(test)]
mod align_tests {
    use super::*;

    /// 中文 30px、数字约 16px：`朱紫国119,60处` 会把 `119,60` 并成一个跨度，
    /// 要靠字宽先验把 6 个字符对到这 1 个跨度上。
    #[test]
    fn aligns_digits_inside_chinese_line() {
        // 朱 紫 国 = 3 个中文跨度，119,60 = 1 个合并跨度，处 = 1 个中文跨度
        let spans = [30u32, 30, 30, 86, 30];
        let blocks = align_chars_to_spans("朱紫国119,60处", &spans).expect("应当对齐");
        assert_eq!(blocks.len(), 5);
        assert_eq!(blocks[0], vec!['朱']);
        assert_eq!(blocks[1], vec!['紫']);
        assert_eq!(blocks[2], vec!['国']);
        assert_eq!(blocks[3], vec!['1', '1', '9', ',', '6', '0']);
        assert_eq!(blocks[4], vec!['处']);
    }

    #[test]
    fn aligns_one_char_per_span() {
        let spans = [30u32, 30, 30];
        let blocks = align_chars_to_spans("鬼王钟", &spans).expect("应当对齐");
        assert_eq!(blocks, vec![vec!['鬼'], vec!['王'], vec!['钟']]);
    }

    #[test]
    fn rejects_hopeless_alignment() {
        // 6 个跨度对 2 个字：差太多，宁可不学。
        assert!(align_chars_to_spans("鬼王", &[30, 30, 30, 30, 30, 30]).is_none());
        assert!(align_chars_to_spans("", &[30]).is_none());
        assert!(align_chars_to_spans("鬼王", &[]).is_none());
    }
}

/// 用连通域切字：先找连通的笔画块，再把「横向挨得近」的块并回一个字。
///
/// 列投影对中文不可靠：`刻` 的 `亥` 和 `刂` 之间有空隙，会被切成两个字；
/// 连通域能把它合回来，代价是 `川` 这种左右分开的字也可能被并在一起——
/// 所以它只做「列投影对不上时」的兜底，不替换默认路径。
pub fn analyze_components(gray: &GrayImage, cfg: MatchConfig) -> Vec<GlyphBox> {
    let (width, height) = (gray.width() as usize, gray.height() as usize);
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let ink: Vec<bool> = gray.pixels().map(|pixel| pixel[0] < 128).collect();

    // 1) 4 连通域标记（显式栈，避免深递归）。
    let mut labels = vec![0u32; width * height];
    let mut boxes: Vec<(u32, u32, u32, u32)> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    for start in 0..ink.len() {
        if !ink[start] || labels[start] != 0 {
            continue;
        }
        let id = boxes.len() as u32 + 1;
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        labels[start] = id;
        stack.push(start);
        while let Some(index) = stack.pop() {
            let (x, y) = (index % width, index / width);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
            for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                let nx = x as i32 + dx;
                let ny = y as i32 + dy;
                if nx < 0 || ny < 0 || nx >= width as i32 || ny >= height as i32 {
                    continue;
                }
                let next = ny as usize * width + nx as usize;
                if ink[next] && labels[next] == 0 {
                    labels[next] = id;
                    stack.push(next);
                }
            }
        }
        boxes.push((x0 as u32, y0 as u32, x1 as u32, y1 as u32));
    }
    if boxes.is_empty() {
        return Vec::new();
    }

    // 2) 只按面积丢噪点：不能用「高度占比」过滤，否则旁边的中文会把
    //    逗号、句点这类矮字当成噪点删掉。
    let mut parts: Vec<(u32, u32, u32, u32)> = boxes
        .into_iter()
        .filter(|(x0, y0, x1, y1)| (x1 - x0) * (y1 - y0) >= 4)
        .collect();
    if parts.is_empty() {
        return Vec::new();
    }

    // 3) 按 x 排序后合并「挨得近」的块（间隙 ≤ gap_merge + 1 视为同一个字）。
    parts.sort_by_key(|(x0, _, _, _)| *x0);
    let merge_gap = cfg.gap_merge.max(1) + 1;
    let mut groups: Vec<(u32, u32, u32, u32)> = Vec::new();
    for (x0, y0, x1, y1) in parts {
        match groups.last_mut() {
            Some(group) if x0 <= group.2 + merge_gap => {
                group.1 = group.1.min(y0);
                group.2 = group.2.max(x1);
                group.3 = group.3.max(y1);
            }
            _ => groups.push((x0, y0, x1, y1)),
        }
    }

    // 4) 每个组用**自己的**竖直范围：否则旁边中文的高度会把数字的归一化拉伸，
    //    学出来和用起来就不一致（实测分数会掉到 0.5）。
    groups
        .into_iter()
        .map(|(x0, y0, x1, y1)| GlyphBox {
            span: Span { x0, x1 },
            top: y0,
            bottom: y1,
        })
        .collect()
}

#[cfg(test)]
mod component_tests {
    use super::fixture::render;
    use super::*;

    /// 一顿画一个「拆开的字」：左右两块之间留 2px 空隙，列投影会切成两半，
    /// 连通域兜底应当合并成一个字。
    #[test]
    fn merges_split_char_components() {
        let mut gray = GrayImage::from_pixel(60, 20, Luma([255]));
        for y in 4..16 {
            for x in 10..14 {
                gray.put_pixel(x, y, Luma([0]));
            }
            for x in 16..20 {
                gray.put_pixel(x, y, Luma([0]));
            }
        }
        let cfg = MatchConfig::default();
        let projected = analyze(&gray, cfg);
        assert_eq!(projected.len(), 2, "列投影会切成两块");
        let merged = analyze_components(&gray, cfg);
        assert_eq!(merged.len(), 1, "连通域兜底应当并回一个字");
    }

    #[test]
    fn components_split_separate_digits() {
        // 两个字之间留 6px：连通域要能分开，别把整行并成一个。
        let gray = binarize(&render("38", 5, 2));
        let cfg = MatchConfig::default();
        let groups = analyze_components(&gray, cfg);
        assert_eq!(groups.len(), 2, "{groups:?}");
    }
}

// ------------------------------------------------- 只认窄字形（中文行里的数字）

/// 一行字里，「窄」的组。
///
/// 中文/全角标点近似方框（宽 ≈ 行高），数字只有一半宽。判据：
/// - 行里**有宽字**（最宽的组接近行高）时，取宽度 ≤ 最宽 72% 的组；
/// - 行里**没有宽字**（整行都是数字，比如 `119,60` 单独一行）时，全部保留。
///
/// 这样既不用猜「文字第 N 个字对应第几个格子」，也不用要求整框就是坐标。
pub fn narrow_groups(boxes: &[GlyphBox]) -> Vec<usize> {
    if boxes.is_empty() {
        return Vec::new();
    }
    let line_height = boxes[0].bottom.saturating_sub(boxes[0].top).max(1) as f32;
    let widest = boxes
        .iter()
        .map(|b| b.span.width())
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    // 最宽的组都不到行高的 0.78：这行没有中文，整行都是窄组。
    let limit = if widest <= line_height * 0.78 {
        widest + 1.0
    } else {
        widest * 0.72
    };
    boxes
        .iter()
        .enumerate()
        .filter(|(_, b)| (b.span.width() as f32) <= limit)
        .map(|(index, _)| index)
        .collect()
}
/// 单组的分类结果：标签、分数、与次优标签的差值。
pub fn classify_group(
    gray: &GrayImage,
    b: &GlyphBox,
    lib: &GlyphLib,
    cfg: MatchConfig,
) -> Option<(String, f32, f32)> {
    if lib.is_empty() {
        return None;
    }
    best_match(gray, b, lib, cfg)
}

/// 把「窄组」按水平间距切成若干段。
///
/// 判据用**相邻组的起点间距**：同一段数字的间距约等于字距（advance），
/// 而「（第2个）」和「1，55」之间的空档远大于它。不切段的话多个数字串会被
/// 拼成 `21,55` 这种垃圾。
pub fn group_runs(boxes: &[GlyphBox], indices: &[usize]) -> Vec<Vec<usize>> {
    if indices.is_empty() {
        return Vec::new();
    }
    // 相邻窄组的起点间距，取中位数当「正常字距」。
    let mut steps: Vec<u32> = indices
        .windows(2)
        .map(|pair| {
            boxes[pair[1]]
                .span
                .x0
                .saturating_sub(boxes[pair[0]].span.x0)
        })
        .filter(|step| *step > 0)
        .collect();
    if steps.is_empty() {
        return vec![indices.to_vec()];
    }
    steps.sort_unstable();
    let advance = steps[steps.len() / 2].max(4) as f32;
    let max_step = (advance * 2.0).max(6.0);
    let mut runs: Vec<Vec<usize>> = Vec::new();
    let mut previous_x0: Option<u32> = None;
    for index in indices {
        let x0 = boxes[*index].span.x0;
        let new_run = match previous_x0 {
            Some(previous) => x0.saturating_sub(previous) as f32 > max_step,
            None => true,
        };
        if new_run {
            runs.push(Vec::new());
        }
        runs.last_mut().expect("run exists").push(*index);
        previous_x0 = Some(x0);
    }
    runs
}
/// 在单行裁剪图里「找数字」：只分类**窄的字形组**，宽字（中文）直接跳过。
///
/// 返回若干「连着的数字串」（每个串是若干 `GlyphMatch`）。中文认不对、
/// 甚至完全没学中文都不影响——这正是它在混排任务文字里能用的原因。
pub fn read_narrow_runs(
    gray: &GrayImage,
    lib: &GlyphLib,
    cfg: MatchConfig,
) -> Vec<Vec<GlyphMatch>> {
    let boxes = analyze_components(gray, cfg);
    let indices = narrow_groups(&boxes);
    if indices.is_empty() {
        return Vec::new();
    }
    let mut runs: Vec<Vec<GlyphMatch>> = Vec::new();
    for run in group_runs(&boxes, &indices) {
        let mut matches: Vec<GlyphMatch> = Vec::with_capacity(run.len());
        let mut usable = true;
        for index in run {
            let b = boxes[index];
            let Some((label, score, margin)) = classify_group(gray, &b, lib, cfg) else {
                usable = false;
                break;
            };
            if score < cfg.min_score || margin < cfg.min_margin || label.chars().count() != 1 {
                usable = false;
                break;
            }
            matches.push(GlyphMatch {
                label,
                score,
                margin,
                x: b.span.x0,
                width: b.span.width(),
            });
        }
        if usable && matches.len() >= 2 {
            runs.push(matches);
        }
    }
    runs
}

#[cfg(test)]
mod narrow_logic_tests {
    use super::*;

    fn boxed(x0: u32, width: u32, top: u32, bottom: u32) -> GlyphBox {
        GlyphBox {
            span: Span { x0, x1: x0 + width },
            top,
            bottom,
        }
    }

    /// 有中文（40 宽）时，只挑 20 宽的数字。
    #[test]
    fn narrow_groups_skips_wide_glyphs() {
        let boxes = vec![
            boxed(0, 40, 0, 48),
            boxed(44, 40, 0, 48),
            boxed(88, 20, 10, 38),
            boxed(112, 20, 10, 38),
            boxed(136, 40, 0, 48),
        ];
        assert_eq!(narrow_groups(&boxes), vec![2, 3]);
    }

    /// 整行都是数字（没有 40 宽的字）时不能把窄的「1」筛掉。
    #[test]
    fn narrow_groups_keeps_all_without_wide_glyph() {
        let boxes = vec![
            boxed(0, 12, 10, 38),
            boxed(24, 20, 10, 38),
            boxed(48, 20, 10, 38),
        ];
        assert_eq!(narrow_groups(&boxes), vec![0, 1, 2]);
    }

    /// 远处另起一段数字：要切开，不能拼成 `21,55`。
    #[test]
    fn group_runs_splits_on_big_gap() {
        let boxes: Vec<GlyphBox> = [0u32, 24, 48, 72]
            .iter()
            .map(|x| boxed(*x, 20, 10, 38))
            .chain(std::iter::once(boxed(400, 20, 10, 38)))
            .chain(std::iter::once(boxed(424, 20, 10, 38)))
            .collect();
        let indices: Vec<usize> = (0..boxes.len()).collect();
        let runs = group_runs(&boxes, &indices);
        assert_eq!(runs.len(), 2, "{runs:?}");
        assert_eq!(runs[0].len(), 4);
        assert_eq!(runs[1].len(), 2);
    }

    /// 紧挨着的一串数字（正常字距）必须保持一段。
    #[test]
    fn group_runs_keeps_tight_digits_together() {
        let boxes: Vec<GlyphBox> = (0..6).map(|i| boxed(i * 24, 18, 10, 38)).collect();
        let indices: Vec<usize> = (0..boxes.len()).collect();
        assert_eq!(group_runs(&boxes, &indices).len(), 1);
    }
}

#[cfg(test)]
mod width_metric_tests {
    use super::*;

    /// 游戏字体规则：2 个数字 = 1 个汉字宽，全角符号 = 1 个汉字宽。
    #[test]
    fn half_width_digits_and_full_width_punctuation() {
        assert_eq!(char_width_units('7'), 0.5);
        assert_eq!(char_width_units('0'), 0.5);
        assert_eq!(char_width_units('a'), 0.5);
        assert_eq!(char_width_units('长'), 1.0);
        assert_eq!(char_width_units('，'), 1.0);
        assert_eq!(char_width_units('（'), 1.0);
        // 「119，60」= 5 个半角 + 1 个全角 = 3.5 个汉字宽。
        let units: f32 = "119，60".chars().map(char_width_units).sum();
        assert!((units - 3.5).abs() < 1e-6, "{units}");
    }

    /// 坐标位数不同（1~3 位）时，行宽就是不一样，对齐要按各自的实际文字来。
    #[test]
    fn aligns_coordinates_of_different_lengths() {
        // 「去 长寿村 7，6」：去=1，长寿村=3，7=0.5， ，=1， 6=0.5 → 共 6.0 单位
        let boxes = 6;
        let span = |units: f32| (units * 30.0) as u32;
        let widths = vec![
            span(1.0),
            span(1.0),
            span(1.0),
            span(1.0),
            span(0.5),
            span(1.5),
        ];
        let blocks = align_chars_to_spans("去长寿村7，6", &widths).expect("应当对齐");
        assert_eq!(blocks.len(), boxes);
        assert_eq!(blocks[0], vec!['去']);
        assert_eq!(blocks[4], vec!['7']);
        assert_eq!(blocks[5], vec!['，', '6']);
    }
}

// ------------------------------------------------------- 按行 + 字宽切字

/// 某一行里这一列有没有墨迹。
fn column_has_ink(gray: &GrayImage, y0: u32, y1: u32, x: u32) -> bool {
    (y0..y1.min(gray.height())).any(|y| x < gray.width() && gray.get_pixel(x, y)[0] < 128)
}

/// 把一行按「字宽规律」切成一格一格。
///
/// 这是按游戏字体的实际排版推的：**2 个数字 = 1 个汉字宽，全角符号 = 1 个汉字宽**。
/// 所以只要知道这一行的文字，就能按字数把行**等比例**切开；坐标 1~3 位导致的行宽
/// 差异自然被吸收（每行都按自己的文字算，不需要等长）。
///
/// `band` 是这一行的矩形 `(x0, y0, x1, y1)`（一般来自 OCR 检测框）。
/// 边界会尽量吸附到「没有墨迹的列」上，避免把笔画切一半。
pub fn split_row_by_text(
    gray: &GrayImage,
    band: (u32, u32, u32, u32),
    text: &str,
) -> Vec<(GlyphBox, char)> {
    let chars: Vec<char> = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    if chars.is_empty() {
        return Vec::new();
    }
    let (bx0, by0, bx1, by1) = (
        band.0.min(gray.width()),
        band.1.min(gray.height()),
        band.2.min(gray.width()),
        band.3.min(gray.height()),
    );
    if bx1 <= bx0 || by1 <= by0 {
        return Vec::new();
    }
    // OCR 框可能比文字略宽：先缩到行内墨迹的左右边界。
    let mut ink: Option<(u32, u32)> = None;
    for x in bx0..bx1 {
        if column_has_ink(gray, by0, by1, x) {
            ink = match ink {
                Some((left, _)) => Some((left, x + 1)),
                None => Some((x, x + 1)),
            };
        }
    }
    let Some((x0, x1)) = ink else {
        return Vec::new();
    };
    let width = (x1 - x0) as f32;
    let units: Vec<f32> = chars.iter().map(|ch| char_width_units(*ch)).collect();
    let total: f32 = units.iter().sum();
    if total <= 0.0 {
        return Vec::new();
    }

    let mut cells = Vec::with_capacity(chars.len());
    let mut consumed = 0.0f32;
    let mut previous = x0;
    for (index, ch) in chars.iter().enumerate() {
        consumed += units[index];
        let is_last = index + 1 == chars.len();
        let mut boundary = if is_last {
            x1
        } else {
            x0 + (width * consumed / total).round() as u32
        };
        if !is_last {
            // 在半格范围内找最近的「空列」，把边界从笔画中间挪到字间。
            let reach = ((width / total * units[index] * 0.4) as u32).max(2);
            for delta in 0..=reach {
                let left = boundary.saturating_sub(delta);
                if column_has_ink(gray, by0, by1, left) {
                    continue;
                }
                if left > previous && left < x1 {
                    boundary = left;
                    break;
                }
                let right = boundary + delta;
                if right < x1 && !column_has_ink(gray, by0, by1, right) && right > previous {
                    boundary = right;
                    break;
                }
            }
        }
        let boundary = boundary.clamp(previous + 1, x1);
        cells.push((
            GlyphBox {
                span: Span {
                    x0: previous,
                    x1: boundary,
                },
                top: by0,
                bottom: by1,
            },
            *ch,
        ));
        previous = boundary;
    }
    cells
}

#[cfg(test)]
mod row_split_tests {
    use super::fixture::render;
    use super::*;

    /// 坐标位数不同 → 行宽不同，但每行都按自己的文字等比例切开。
    #[test]
    fn splits_row_by_character_widths() {
        // 「119,60」6 个半角字符 → 每格宽度应当基本相等。
        let line = binarize(&render("119,60", 5, 1));
        let band = (0, 0, line.width(), line.height());
        let cells = split_row_by_text(&line, band, "119,60");
        assert_eq!(cells.len(), 6, "6 个字应当切 6 格");
        let texts: String = cells.iter().map(|(_, ch)| *ch).collect();
        assert_eq!(texts, "119,60");
        let widths: Vec<u32> = cells.iter().map(|(b, _)| b.span.width()).collect();
        let min = *widths.iter().min().unwrap();
        let max = *widths.iter().max().unwrap();
        // 边界会吸附到空列上，允许几个像素的浮动。
        assert!(max - min <= 6, "半角字符应当等宽：{widths:?}");
    }

    /// 混排：中文宽、数字半宽，全角逗号占一个汉字宽。
    #[test]
    fn wide_and_half_width_glyphs_get_proportional_cells() {
        let mut canvas = image::GrayImage::from_pixel(300, 40, image::Luma([255]));
        // 两个「汉字」宽块 40x40
        for y in 0..40 {
            for x in 0..40 {
                canvas.put_pixel(x, y, image::Luma([0]));
                canvas.put_pixel(x + 40, y, image::Luma([0]));
            }
        }
        // 「，」也按一个汉字宽（40px）
        for y in 28..36 {
            for x in 84..100 {
                canvas.put_pixel(x, y, image::Luma([0]));
            }
        }
        // 两个数字各半宽（20px）
        for y in 10..30 {
            for x in 120..140 {
                canvas.put_pixel(x, y, image::Luma([0]));
            }
            for x in 140..160 {
                canvas.put_pixel(x, y, image::Luma([0]));
            }
        }
        let cells = split_row_by_text(&canvas, (0, 0, 300, 40), "阿阿，12");
        assert_eq!(cells.len(), 5);
        let widths: Vec<u32> = cells.iter().map(|(b, _)| b.span.width()).collect();
        // 前两个和逗号约 1 个字宽，后面两个数字约半个字宽。
        assert!(widths[0] >= 20 && widths[1] >= 20, "{widths:?}");
        assert!(widths[3] <= widths[0], "{widths:?}");
        assert!(widths[4] <= widths[0], "{widths:?}");
    }
}

/// 把矩形裁到里面「真正有墨迹」的范围。
///
/// OCR 的检测框常常比文字宽一圈（尤其放大倍数高、字小的时候），
/// 直接按它画框/平分会出现「框比字宽得多」。这里按墨迹收一遍。
pub fn trim_to_ink(gray: &GrayImage, band: (u32, u32, u32, u32)) -> Option<(u32, u32, u32, u32)> {
    let (x0, y0, x1, y1) = (
        band.0.min(gray.width()),
        band.1.min(gray.height()),
        band.2.min(gray.width()),
        band.3.min(gray.height()),
    );
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    let (mut left, mut top, mut right, mut bottom) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for y in y0..y1 {
        for x in x0..x1 {
            if gray.get_pixel(x, y)[0] < 128 {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
    }
    (right > left && bottom > top).then_some((left, top, right, bottom))
}

#[cfg(test)]
mod trim_tests {
    use super::*;

    #[test]
    fn trims_band_to_ink() {
        let mut gray = GrayImage::from_pixel(100, 40, Luma([255]));
        for y in 10..20 {
            for x in 30..50 {
                gray.put_pixel(x, y, Luma([0]));
            }
        }
        let trimmed = trim_to_ink(&gray, (0, 0, 100, 40)).expect("应当有墨迹");
        assert_eq!(trimmed, (30, 10, 50, 20));

        // 空框返回 None。
        assert!(trim_to_ink(&gray, (60, 0, 100, 40)).is_none());
    }
}
