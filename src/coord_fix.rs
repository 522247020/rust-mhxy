//! 用点阵字库复核 OCR 认出来的坐标数字。
//!
//! 分工：OCR 负责定位和「这段文字大概是什么」，字库负责「这几个数字到底是几」。
//! 只替换**字库高置信**、且形状刚好是完整坐标（`d{1,3}[,.、]d{1,3}`）的区域，
//! 其余一律保持 OCR 原文 —— 字库认错也不会把文本弄坏，退回原来的行为。
//!
//! 文本替换后的字符串仍然走 `core::parse_tasks` + 地图边界校验，
//! 所以这里不需要自己判断「这个坐标合不合理」。

use crate::font_lib::{self, GlyphLib, MatchConfig};
use crate::ocr::OcrRegion;
use image::RgbImage;
use std::ops::Range;

/// 复核用的默认匹配参数。和建库时的参数保持一致即可（建库用 `MatchConfig::default()`）。
pub const DEFAULT_MATCH: MatchConfig = MatchConfig {
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
};

/// 裁剪检测框时向外扩几个像素（送 OCR 的图可能已放大，2px 足够吃掉抗锯齿边缘）。
pub const DEFAULT_PADDING: u32 = 2;

/// 一次坐标替换的记录，用来写日志。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    pub before: String,
    pub after: String,
}

/// 坐标分隔符：和 `core::is_separator` 保持一致（OCR 常把逗号认成句号）。
fn is_separator(ch: char) -> bool {
    matches!(ch, ',' | '，' | '、' | '.' | '．' | '。')
}

/// 在文本里找第一个「1~3 位数字 + 分隔符 + 1~3 位数字」，返回字符下标区间。
///
/// 冒号、分号不算分隔符，避免把「剩余12:30」当成坐标。
pub fn coordinate_shape(text: &str) -> Option<Range<usize>> {
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if !chars[index].is_ascii_digit() {
            index += 1;
            continue;
        }
        let start = index;
        while index < chars.len() && chars[index].is_ascii_digit() {
            index += 1;
        }
        if index - start > 3 || index >= chars.len() || !is_separator(chars[index]) {
            continue;
        }
        index += 1;
        let second_start = index;
        while index < chars.len() && chars[index].is_ascii_digit() {
            index += 1;
        }
        let second_len = index - second_start;
        if (1..=3).contains(&second_len) {
            return Some(start..index);
        }
    }
    None
}

/// OCR 有时会把数字认成字母（`6→G`、`0→O`、`1→l`）。这些字符不算坐标的一部分，
/// 但会把坐标区间的长度对不上，所以替换时允许把它们一起吞进去。
fn ocr_garbage(ch: char) -> bool {
    ch.is_ascii_alphabetic() || matches!(ch, '|' | '?' | '!' | '_')
}

/// 把坐标区间两边向外扩到「紧贴着的 OCR 错字」。
///
/// 例如 `79,2G` 的坐标区间只有 `79,2`，右边界要吞掉 `G`；`G9,26` 左边界要吐回 `G`。
/// 中文和全角标点不吞，避免把「坐标（79,26）附近」这种结构拆错。
pub fn widen_over_confusables(text: &str, shape: Range<usize>) -> Range<usize> {
    let chars: Vec<char> = text.chars().collect();
    let mut start = shape.start;
    let mut end = shape.end;
    while start > 0 && ocr_garbage(chars[start - 1]) {
        start -= 1;
    }
    while end < chars.len() && ocr_garbage(chars[end]) {
        end += 1;
    }
    start..end
}

/// 对一整段 OCR 结果做字库复核，返回（新文本, 替换记录）。
pub fn correct(
    panel: &RgbImage,
    text: &str,
    regions: &[OcrRegion],
    lib: &GlyphLib,
    cfg: MatchConfig,
    padding: u32,
) -> (String, Vec<Fix>) {
    if lib.is_empty() || regions.is_empty() {
        return (text.to_string(), Vec::new());
    }
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut fixes = Vec::new();
    for region in regions {
        let Some(local) = coordinate_shape(&region.text) else {
            continue;
        };
        let local = widen_over_confusables(&region.text, local);
        let Some((x, y, width, height)) = padded_box(region, padding, panel) else {
            continue;
        };
        let Ok(crop) = font_lib::crop(panel, x, y, width, height) else {
            continue;
        };
        let read = font_lib::read_line_gray(&font_lib::binarize(&crop), lib, cfg);
        if !read.confident {
            continue;
        }
        // 整块必须就是一个坐标：中文混排、多段数字之类一律不动。
        // 字数也要对得上（OCR 把一位数字认成字母时字数不变）。
        let read_chars = read.text.chars().count();
        if coordinate_shape(&read.text) != Some(0..read_chars) || local.len() != read_chars {
            continue;
        }
        let before: String = region
            .text
            .chars()
            .skip(local.start)
            .take(local.len())
            .collect();
        if before == read.text {
            continue;
        }
        let start = region.range.start + local.start;
        let end = region.range.start + local.end;
        edits.push((start..end, read.text.clone()));
        fixes.push(Fix {
            before,
            after: read.text,
        });
    }
    (apply_edits(text, edits), fixes)
}

/// 检测框向外扩 `padding` 后夹到图片范围内。空框返回 None。
///
/// 运行时复核和建库采样都走这里：裁剪方式一致，训练出来的字模和识别时的输入同分布。
pub fn padded_box(
    region: &OcrRegion,
    padding: u32,
    panel: &RgbImage,
) -> Option<(u32, u32, u32, u32)> {
    let pad = padding as f32;
    let left = (region.left - pad).max(0.0);
    let top = (region.top - pad).max(0.0);
    let right = (region.right + pad).min(panel.width() as f32);
    let bottom = (region.bottom + pad).min(panel.height() as f32);
    if !left.is_finite() || !top.is_finite() || !right.is_finite() || !bottom.is_finite() {
        return None;
    }
    let width = (right - left).round() as u32;
    let height = (bottom - top).round() as u32;
    if width == 0 || height == 0 {
        return None;
    }
    Some((left.round() as u32, top.round() as u32, width, height))
}

/// 按字符区间替换，从后往前改，避免前面的替换把后面的下标搞乱。
fn apply_edits(text: &str, mut edits: Vec<(Range<usize>, String)>) -> String {
    if edits.is_empty() {
        return text.to_string();
    }
    edits.sort_by_key(|(range, _)| range.start);
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0usize;
    for (range, replacement) in edits {
        // 区间重叠（理论上不会出现）就跳过后面那个，宁可少改也不写坏文本。
        if range.start < cursor || range.end > chars.len() {
            continue;
        }
        out.extend(&chars[cursor..range.start]);
        out.push_str(&replacement);
        cursor = range.end;
    }
    out.extend(&chars[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::font_lib::{binarize, build_from_lines, fixture, DEFAULT_CELL_H, DEFAULT_CELL_W};

    fn trained_lib() -> GlyphLib {
        let samples = vec![
            (
                "0123456789.,".to_string(),
                binarize(&fixture::render("0123456789.,", 4, 1)),
            ),
            (
                "0123456789.,".to_string(),
                binarize(&fixture::render("0123456789.,", 6, 1)),
            ),
        ];
        let (lib, warnings) = build_from_lines(
            DEFAULT_CELL_W,
            DEFAULT_CELL_H,
            &samples,
            MatchConfig::default(),
        );
        assert!(warnings.is_empty(), "建库警告：{warnings:?}");
        lib
    }

    /// 在白色画布上画一行字，返回画布、这行的矩形和它对应的 OCR 区域。
    fn panel_with(
        text: &str,
        ocr_text: &str,
        at: (u32, u32),
    ) -> (RgbImage, Vec<OcrRegion>, String) {
        let line = fixture::render(text, 5, 1);
        let mut panel = RgbImage::from_pixel(400, 120, image::Rgb([255, 255, 255]));
        fixture::place(&mut panel, &line, at.0, at.1);
        let region = OcrRegion {
            text: ocr_text.to_string(),
            score: 0.9,
            left: at.0 as f32,
            top: at.1 as f32,
            right: (at.0 + line.width()) as f32,
            bottom: (at.1 + line.height()) as f32,
            range: 0..ocr_text.chars().count(),
        };
        (panel, vec![region], ocr_text.to_string())
    }

    #[test]
    fn finds_coordinate_shapes() {
        assert_eq!(coordinate_shape("79,26"), Some(0..5));
        // 「去长寿村（79,26）附近」：全角括号不算坐标，区间只包数字本身。
        assert_eq!(coordinate_shape("去长寿村（79,26）附近"), Some(5..10));
        assert_eq!(coordinate_shape("79、26"), Some(0..5));
        // 冒号不算分隔符，避免把倒计时当坐标。
        assert_eq!(coordinate_shape("剩余12:30"), None);
        // 超过 3 位数字不算。
        assert_eq!(coordinate_shape("1234,5"), None);
        assert_eq!(coordinate_shape("没有数字"), None);
    }

    #[test]
    fn widens_over_confusable_letters() {
        assert_eq!(widen_over_confusables("79,2G", 0..4), 0..5);
        assert_eq!(widen_over_confusables("G9,26", 1..5), 0..5);
        assert_eq!(widen_over_confusables("坐标79,2G处", 2..6), 2..7);
        // 中文/全角标点不吞。
        assert_eq!(
            widen_over_confusables("去长寿村（79,26）附近", 5..10),
            5..10
        );
        // 后面又是逗号时不吞（「1,2,3」这种多段数字）。
        assert_eq!(widen_over_confusables("1,2,3", 0..3), 0..3);
    }

    #[test]
    fn replaces_misread_digits() {
        let lib = trained_lib();
        let (panel, regions, text) = panel_with("79,26", "79,2G", (60, 40));
        let (fixed, fixes) = correct(
            &panel,
            &text,
            &regions,
            &lib,
            DEFAULT_MATCH,
            DEFAULT_PADDING,
        );
        assert_eq!(fixed, "79,26");
        assert_eq!(
            fixes,
            vec![Fix {
                before: "79,2G".into(),
                after: "79,26".into()
            }]
        );
    }

    #[test]
    fn replaces_only_the_coordinate_inside_a_longer_line() {
        let lib = trained_lib();
        let line = "79,26";
        let at = (100, 30);
        let (panel, mut regions, _) = panel_with(line, "坐标79,2G处", at);
        regions[0].text = "坐标79,2G处".into();
        regions[0].range = 3..10;
        let text = "抓鬼 坐标79,2G处".to_string();
        let (fixed, fixes) = correct(
            &panel,
            &text,
            &regions,
            &lib,
            DEFAULT_MATCH,
            DEFAULT_PADDING,
        );
        assert_eq!(fixed, "抓鬼 坐标79,26处");
        assert_eq!(fixes.len(), 1);
        assert_eq!(fixes[0].after, "79,26");
    }

    #[test]
    fn keeps_text_when_library_disagrees_on_shape() {
        let lib = trained_lib();
        // 区域里其实是 "1,2,3,4"，整块不是单个坐标，不能替换。
        let (panel, regions, text) = panel_with("1,2,3,4", "1,2,3,4", (60, 40));
        let (fixed, fixes) = correct(
            &panel,
            &text,
            &regions,
            &lib,
            DEFAULT_MATCH,
            DEFAULT_PADDING,
        );
        assert_eq!(fixed, text);
        assert!(fixes.is_empty());
    }

    #[test]
    fn keeps_text_when_crop_has_no_digits() {
        let lib = trained_lib();
        // 框位置偏到空白处：字库读不出可信结果，保持 OCR 原文。
        let (panel, mut regions, text) = panel_with("79,26", "79,2G", (60, 40));
        regions[0].left = 300.0;
        regions[0].right = 340.0;
        regions[0].top = 90.0;
        regions[0].bottom = 110.0;
        let (fixed, fixes) = correct(
            &panel,
            &text,
            &regions,
            &lib,
            DEFAULT_MATCH,
            DEFAULT_PADDING,
        );
        assert_eq!(fixed, text);
        assert!(fixes.is_empty());
    }

    #[test]
    fn does_nothing_without_regions_or_lib() {
        let lib = trained_lib();
        let panel = RgbImage::from_pixel(10, 10, image::Rgb([255, 255, 255]));
        let (fixed, fixes) = correct(&panel, "79,2G", &[], &lib, DEFAULT_MATCH, DEFAULT_PADDING);
        assert_eq!(fixed, "79,2G");
        assert!(fixes.is_empty());

        let empty = GlyphLib::new(DEFAULT_CELL_W, DEFAULT_CELL_H);
        let (_, mut regions, _) = panel_with("79,26", "79,2G", (10, 10));
        regions[0].range = 0..5;
        let (fixed, fixes) = correct(
            &panel,
            "79,2G",
            &regions,
            &empty,
            DEFAULT_MATCH,
            DEFAULT_PADDING,
        );
        assert_eq!(fixed, "79,2G");
        assert!(fixes.is_empty());
    }

    #[test]
    fn keeps_text_when_library_agrees() {
        let lib = trained_lib();
        let (panel, regions, text) = panel_with("79,26", "79,26", (60, 40));
        let (fixed, fixes) = correct(
            &panel,
            &text,
            &regions,
            &lib,
            DEFAULT_MATCH,
            DEFAULT_PADDING,
        );
        assert_eq!(fixed, "79,26");
        assert!(fixes.is_empty(), "一致时不该记录成替换");
    }

    #[test]
    fn applies_multiple_edits_in_order() {
        let chars: Vec<char> = "aa79,2Gbb31,4Zcc".chars().collect();
        let edits = vec![(2..7, "79,26".to_string()), (9..14, "31,45".to_string())];
        let text: String = chars.into_iter().collect();
        assert_eq!(apply_edits(&text, edits), "aa79,26bb31,45cc");
    }
}
