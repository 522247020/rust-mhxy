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
use std::sync::atomic::{AtomicU64, Ordering};

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

/// 一次复核的统计：用来告诉用户「字库到底参与了吗、效果如何」。
///
/// - `candidates`：文本里有多少段长得像坐标（值得复核的）
/// - `matched`：字库能确认的段数（读出的字数与 OCR 那段一致）
/// - `agreed`：字库读出来和 OCR 一模一样（说明 OCR 本来就对）
/// - `fixes`：字库真正改写了 OCR 的段数（`fixes = matched - agreed`）
///
/// `candidates == 0` 说明这一帧根本没坐标，不怪字库；
/// `candidates > 0` 但 `matched == 0` 才是「字库参与了但不够确信」。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LibReport {
    pub candidates: usize,
    pub matched: usize,
    pub agreed: usize,
    pub fixes: usize,
}

/// 百分比，分母 0 时给 `—`（避免除零，也别把「没数据」说成 0%）。
fn pct(part: usize, whole: usize) -> String {
    if whole == 0 {
        "—".to_string()
    } else {
        format!("{:.0}%", part as f32 * 100.0 / whole as f32)
    }
}

impl LibReport {
    /// 字库「敢下结论」的比例：确认数 / 候选数。低就说明字模不够或预处理不对。
    pub fn match_rate(&self) -> String {
        pct(self.matched, self.candidates)
    }

    /// 字库改写 OCR 的比例：改写数 / 确认数。这就是「字库真正干了多少活」。
    pub fn fix_rate(&self) -> String {
        pct(self.fixes, self.matched)
    }

    /// OCR 本来就认对的比例：一致数 / 确认数。
    pub fn agree_rate(&self) -> String {
        pct(self.agreed, self.matched)
    }

    /// 写详细日志用的一行。
    pub fn summary(&self) -> String {
        if self.candidates == 0 {
            "本帧没有坐标形状的文字，字库未参与".to_string()
        } else {
            format!(
                "坐标段 {}｜确认 {}（成功率 {}）｜一致 {}｜改写 {}（改写率 {}）｜无法确认 {}",
                self.candidates,
                self.matched,
                self.match_rate(),
                self.agreed,
                self.fixes,
                self.fix_rate(),
                self.candidates.saturating_sub(self.matched)
            )
        }
    }
}

// ---------------------------------------------------------------- 累计统计
//
// 用户常问「我怎么知道字库真的在用？」。累计计数器就是答案：
// 侧边栏直接显示「跑了几帧 / 见过几个坐标段 / 确认几个 / 改写几个」，
// 不用去翻详细日志。

/// 进程内累计。单线程读取、后台识别线程写入，用原子量足够。
pub struct Totals {
    pub frames: u64,
    pub candidates: u64,
    pub matched: u64,
    pub agreed: u64,
    pub fixes: u64,
}

impl Totals {
    pub fn report(&self) -> LibReport {
        LibReport {
            candidates: self.candidates as usize,
            matched: self.matched as usize,
            agreed: self.agreed as usize,
            fixes: self.fixes as usize,
        }
    }
}

static FRAMES: AtomicU64 = AtomicU64::new(0);
static CANDIDATES: AtomicU64 = AtomicU64::new(0);
static MATCHED: AtomicU64 = AtomicU64::new(0);
static AGREED: AtomicU64 = AtomicU64::new(0);
static FIXES: AtomicU64 = AtomicU64::new(0);
static LAST_CANDIDATES: AtomicU64 = AtomicU64::new(0);
static LAST_MATCHED: AtomicU64 = AtomicU64::new(0);
static LAST_AGREED: AtomicU64 = AtomicU64::new(0);
static LAST_FIXES: AtomicU64 = AtomicU64::new(0);

fn record(report: &LibReport) {
    FRAMES.fetch_add(1, Ordering::Relaxed);
    CANDIDATES.fetch_add(report.candidates as u64, Ordering::Relaxed);
    MATCHED.fetch_add(report.matched as u64, Ordering::Relaxed);
    AGREED.fetch_add(report.agreed as u64, Ordering::Relaxed);
    FIXES.fetch_add(report.fixes as u64, Ordering::Relaxed);
    LAST_CANDIDATES.store(report.candidates as u64, Ordering::Relaxed);
    LAST_MATCHED.store(report.matched as u64, Ordering::Relaxed);
    LAST_AGREED.store(report.agreed as u64, Ordering::Relaxed);
    LAST_FIXES.store(report.fixes as u64, Ordering::Relaxed);
}

pub fn totals() -> Totals {
    Totals {
        frames: FRAMES.load(Ordering::Relaxed),
        candidates: CANDIDATES.load(Ordering::Relaxed),
        matched: MATCHED.load(Ordering::Relaxed),
        agreed: AGREED.load(Ordering::Relaxed),
        fixes: FIXES.load(Ordering::Relaxed),
    }
}

/// 最近一帧的统计（侧边栏用来显示「上一帧干了什么」）。
pub fn last_frame() -> LibReport {
    LibReport {
        candidates: LAST_CANDIDATES.load(Ordering::Relaxed) as usize,
        matched: LAST_MATCHED.load(Ordering::Relaxed) as usize,
        agreed: LAST_AGREED.load(Ordering::Relaxed) as usize,
        fixes: LAST_FIXES.load(Ordering::Relaxed) as usize,
    }
}

pub fn reset_totals() {
    FRAMES.store(0, Ordering::Relaxed);
    CANDIDATES.store(0, Ordering::Relaxed);
    MATCHED.store(0, Ordering::Relaxed);
    AGREED.store(0, Ordering::Relaxed);
    FIXES.store(0, Ordering::Relaxed);
    LAST_CANDIDATES.store(0, Ordering::Relaxed);
    LAST_MATCHED.store(0, Ordering::Relaxed);
    LAST_AGREED.store(0, Ordering::Relaxed);
    LAST_FIXES.store(0, Ordering::Relaxed);
}

/// 字库复核的结果：新文本 + 替换记录 + 统计。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Corrected {
    pub text: String,
    pub fixes: Vec<Fix>,
    pub report: LibReport,
}

/// 对一整段 OCR 结果做字库复核，返回（新文本, 替换记录）。
#[cfg(test)]
pub fn correct(
    panel: &RgbImage,
    text: &str,
    regions: &[OcrRegion],
    lib: &GlyphLib,
    cfg: MatchConfig,
    padding: u32,
) -> (String, Vec<Fix>) {
    let corrected = correct_with_report(panel, text, regions, lib, cfg, padding);
    (corrected.text, corrected.fixes)
}

/// 同 [`correct`]，额外返回「字库参与程度」的统计，用来在 UI / 日志里证明它确实在跑。
pub fn correct_with_report(
    panel: &RgbImage,
    text: &str,
    regions: &[OcrRegion],
    lib: &GlyphLib,
    cfg: MatchConfig,
    padding: u32,
) -> Corrected {
    let mut report = LibReport::default();
    if lib.is_empty() || regions.is_empty() {
        return Corrected {
            text: text.to_string(),
            fixes: Vec::new(),
            report,
        };
    }
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut fixes = Vec::new();
    for region in regions {
        let Some(local) = coordinate_shape(&region.text) else {
            continue;
        };
        report.candidates += 1;
        let local = widen_over_confusables(&region.text, local);
        let Some((x, y, width, height)) = padded_box(region, padding, panel) else {
            continue;
        };
        let Ok(crop) = font_lib::crop(panel, x, y, width, height) else {
            continue;
        };
        let gray = font_lib::binarize_text(&crop);
        // 跨行框：两行的字交错，读出来一定不是坐标。
        if font_lib::row_spans(&gray, cfg.row_gap).len() != 1 {
            continue;
        }
        // 读法一：整块就是一个坐标（OCR 把坐标单独检出来时最准）。
        // 读法二：整行混着中文时，只认**窄的字形**（数字约中文一半宽），
        //         挑出连着的数字串——这就是任务栏最常见的形态。
        let read = {
            let whole = font_lib::read_line_gray(&gray, lib, cfg);
            let whole_chars = whole.text.chars().count();
            if whole.confident
                && coordinate_shape(&whole.text) == Some(0..whole_chars)
                && whole_chars == local.len()
            {
                Some(whole.text)
            } else {
                font_lib::read_narrow_runs(&gray, lib, cfg)
                    .into_iter()
                    .map(|run| run.iter().map(|m| m.label.as_str()).collect::<String>())
                    .find(|text| {
                        let chars = text.chars().count();
                        coordinate_shape(text) == Some(0..chars)
                    })
            }
        };
        let Some(read) = read else {
            continue;
        };
        // 字数必须与 OCR 原来那段一致：只修「认错一位」，不允许凭空加减字符。
        // 字库一旦有脏模板，这条护栏能挡住「595，115 → 919,15」这种改写。
        if read.chars().count() != local.len() {
            continue;
        }
        report.matched += 1;
        let before: String = region
            .text
            .chars()
            .skip(local.start)
            .take(local.len())
            .collect();
        if before == read {
            // 字库读出来和 OCR 一样：OCR 本来就对，这里不算“改写”。
            report.agreed += 1;
            continue;
        }
        let start = region.range.start + local.start;
        let end = region.range.start + local.end;
        edits.push((start..end, read.clone()));
        fixes.push(Fix {
            before,
            after: read,
        });
    }
    report.fixes = fixes.len();
    record(&report);
    Corrected {
        text: apply_edits(text, edits),
        fixes,
        report,
    }
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
    fn report_counts_candidates_matched_and_fixes() {
        let lib = trained_lib();
        // 一段认错一位（会被改写）、一段本来就对、一段没有坐标形状。
        let (panel, mut regions, _) = panel_with("79,26", "79,2G", (60, 40));
        let mut second = regions[0].clone();
        second.text = "12,34".into();
        second.range = 8..13;
        let mut third = regions[0].clone();
        third.text = "附近".into();
        third.range = 13..15;
        regions.push(second);
        regions.push(third);
        let text = "79,2G 12,34 附近".to_string();
        let corrected = correct_with_report(
            &panel,
            &text,
            &regions,
            &lib,
            DEFAULT_MATCH,
            DEFAULT_PADDING,
        );
        // 两段像坐标（第三段「附近」不算）。
        assert_eq!(corrected.report.candidates, 2);
        assert_eq!(corrected.report.fixes, corrected.fixes.len());
        assert!(!corrected.report.summary().contains("未参与"));
    }

    #[test]
    fn report_marks_a_frame_without_coordinates() {
        let lib = trained_lib();
        let (panel, regions, text) = panel_with("79,26", "附近", (60, 40));
        let corrected =
            correct_with_report(&panel, &text, &regions, &lib, DEFAULT_MATCH, DEFAULT_PADDING);
        assert_eq!(corrected.report.candidates, 0);
        assert!(corrected.report.summary().contains("未参与"));
    }

    #[test]
    fn report_separates_agreement_from_rewrites() {
        let lib = trained_lib();
        // OCR 本来就对：算「一致」，不算「改写」，成功率/一致率都 100%。
        let (panel, regions, text) = panel_with("79,26", "79,26", (60, 40));
        let corrected =
            correct_with_report(&panel, &text, &regions, &lib, DEFAULT_MATCH, DEFAULT_PADDING);
        assert_eq!(corrected.report.candidates, 1);
        assert_eq!(corrected.report.matched, 1);
        assert_eq!(corrected.report.agreed, 1);
        assert_eq!(corrected.report.fixes, 0);
        assert_eq!(corrected.report.match_rate(), "100%");
        assert_eq!(corrected.report.agree_rate(), "100%");
        assert_eq!(corrected.report.fix_rate(), "0%");

        // OCR 认错一位：一致 0、改写 1，改写率 100%。
        let (panel, regions, text) = panel_with("79,26", "79,2G", (60, 40));
        let corrected =
            correct_with_report(&panel, &text, &regions, &lib, DEFAULT_MATCH, DEFAULT_PADDING);
        assert_eq!(corrected.report.agreed, 0);
        assert_eq!(corrected.report.fixes, 1);
        assert_eq!(corrected.report.fix_rate(), "100%");
    }

    #[test]
    fn totals_accumulate_across_frames() {
        let lib = trained_lib();
        let before = totals();
        for _ in 0..2 {
            let (panel, regions, text) = panel_with("79,26", "79,2G", (60, 40));
            let _ =
                correct_with_report(&panel, &text, &regions, &lib, DEFAULT_MATCH, DEFAULT_PADDING);
        }
        let after = totals();
        // 其他测试也在跑，所以只断言「至少增加了这么多」。
        assert!(after.frames >= before.frames + 2, "帧数应累加");
        assert!(after.candidates >= before.candidates + 2, "坐标段应累加");
        assert!(after.matched >= before.matched + 2, "确认数应累加");
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

#[cfg(test)]
mod length_guard_tests {
    use super::*;
    use crate::font_lib::{binarize, build_from_lines, fixture, DEFAULT_CELL_H, DEFAULT_CELL_W};

    /// 字库读出来的坐标长度和 OCR 那段不一致时，绝不能替换
    /// （否则脏模板会把「595，115」改成「919,15」这种更糟的结果）。
    #[test]
    fn refuses_replacement_when_length_differs() {
        // 字库只学了 0-9，用「79,26」建库。
        let samples = vec![(
            "0123456789.,".to_string(),
            binarize(&fixture::render("0123456789.,", 4, 1)),
        )];
        let (lib, _) = build_from_lines(
            DEFAULT_CELL_W,
            DEFAULT_CELL_H,
            &samples,
            MatchConfig::default(),
        );
        // 面板上画的是「12,34」（5 个字符），OCR 却说成「12,3」（4 个字符）。
        let line = fixture::render("12,34", 5, 1);
        let mut panel = image::RgbImage::from_pixel(240, 60, image::Rgb([255, 255, 255]));
        image::imageops::overlay(&mut panel, &line, 10, 10);
        let region = crate::ocr::OcrRegion {
            text: "12,3".into(),
            score: 0.9,
            left: 10.0,
            top: 10.0,
            right: 10.0 + line.width() as f32,
            bottom: 10.0 + line.height() as f32,
            range: 0..4,
        };
        let text = "坐标12,3处".to_string();
        let (fixed, fixes) = correct(
            &panel,
            &text,
            &[region],
            &lib,
            DEFAULT_MATCH,
            DEFAULT_PADDING,
        );
        assert_eq!(fixed, text, "长度不一致时不该改");
        assert!(fixes.is_empty());
    }
}
