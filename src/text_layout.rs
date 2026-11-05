//! 任务栏面板图的「投影分行」布局：把整块面板切成若干行，供 rec-only 识别使用。
//!
//! # 为什么不用检测模型
//!
//! 任务栏是深色渐变背景 + 彩色文字（白字/黄字/红字），没有英文单词，通常只有
//! 2~3 行，行距很小。PP-OCR 的检测模型在这种「小字号、低对比度、行间几乎粘连」
//! 的图上容易把上下两行框成一个大框，或者把一行拆成多个碎框；而检测模型本身
//! 又要下载、又要跑推理，启动和耗时都不可控。
//!
//! 这里改用最朴素的**水平投影**：文字行在 y 方向上的墨迹量明显高于行间空隙，
//! 先按阈值切出原始带，再用「合并 + 劈分 + 矮带过滤」修整。投影对颜色不敏感，
//! 只需要一个可靠的二值图；分行结果不可信时调用方可以回退到检测模型，所以本模块
//! 宁可保守，不追求把每个像素都分对。
//!
//! # 算法步骤
//!
//! 1. [`binary`]：按通道最大值做 Otsu，结合边框亮度判断极性，输出黑字白底；
//! 2. [`components`]：4-连通域标记，按面积过滤单像素噪点；
//! 3. 逐行统计保留连通域的墨迹数，得到水平投影；
//! 4. 用 `ink_ratio * 最大投影` 作阈值，切出原始带；
//! 5. 用原始带高度中位数（MAD 剔除离群值）估计行高先验 `h0`；
//! 6. 间隙小于 `merge_ratio * h0` 的相邻带合并；特别地，同一汉字里上下分离的
//!    横笔画（「二」「三」）会先按「横向条带 + 列范围高度重合」合并成一行；
//! 7. 过高的带在内部谷底处递归劈分，谷底不够低就不劈，交给第 10 步兜底；
//! 8. 矮带里横向很长的判为装饰线丢弃，其余保留（可能是一行标点）；
//! 9. 每行左右按墨迹裁剪，并记录行内墨迹数；
//! 10. 校验行数、行高离散度、相邻行中心距；不通过时按连通域 y 中心聚类，
//!     用相邻中心的中点作行界再校验一次；
//! 11. 仍不可信就置 `reliable = false`，让调用方回退检测模型。
//!
//! # 参数
//!
//! [`RowConfig`] 里的比率都以行高 `h0` 为基准，不写死像素：
//! `ink_ratio` 控制投影阈值，`merge_ratio` 控制小间隙合并，`split_ratio` /
//! `valley_ratio` 控制粘连行劈分，`short_ratio` 控制矮带识别，`pad_ratio`
//! 控制裁图留白，`min_area` 过滤噪点。
//!
//! # 可能的误判
//!
//! - 文字行完全粘连且上下笔画连成一个连通域时，投影和连通域都看不到「行」，
//!   只能靠行高先验和聚类兜底，仍可能失败；此时会标记 `reliable = false`。
//! - 单行里只有上下分离的部件（例如孤零零一个「二」）时，投影天然像两行，
//!   本模块用横向条带合并来修复，但两行恰好都是同样的横条时无法区分。
//! - 倾斜文字、粗描边、极淡的抗锯齿灰边会让行带变胖，`h0` 偏大时可能少切行。
//! - 装饰线很粗或很短时可能被当成一行标点保留下来。

// 这个模块的公开 API 会在后续步骤逐步接线；中间状态不要因为 dead_code 报警。
#![allow(dead_code)]

use image::{GrayImage, Luma, Rgb, RgbImage};

// ---------------------------------------------------------------- 二值化

/// 几何用二值化：通道最大值 → Otsu → 边框估背景侧判极性 → 输出黑字白底。
///
/// 用通道最大值而不是亮度，是因为任务栏有黄字、红字：亮色文字在任一通道上
/// 通常都很亮，而深色渐变背景三个通道都暗，最大值能保留最高的文字/背景对比。
/// 边框（面板边缘）大概率是背景，用它的中位数判断背景是亮是暗；如果边框和
/// 阈值分不开，再用「墨迹超过一半就翻转」兜底。输出 `0` 表示墨迹，`255` 表示背景。
pub fn binary(image: &RgbImage) -> GrayImage {
    let gray = channel_max(image);
    let threshold = otsu(&gray);
    let border = border_median(&gray);
    // 背景比阈值亮 → 大概率是浅底深字；否则是深底亮字，需要反相成黑字白底。
    let mut dark_text = border > threshold;
    let total = gray.width() as u64 * gray.height() as u64;
    if total == 0 {
        return gray;
    }
    let is_ink = |value: u8| {
        if dark_text {
            value <= threshold
        } else {
            value > threshold
        }
    };
    let ink: u64 = gray.pixels().filter(|pixel| is_ink(pixel[0])).count() as u64;
    // 前景超过一半说明前面判反了（例如整块面板就是文字截图），翻转一次。
    if ink * 2 > total {
        dark_text = !dark_text;
    }
    GrayImage::from_fn(gray.width(), gray.height(), |x, y| {
        let value = gray.get_pixel(x, y)[0];
        let ink = if dark_text {
            value <= threshold
        } else {
            value > threshold
        };
        Luma([if ink { 0 } else { 255 }])
    })
}

/// 局部对比度二值化：积分图求窗口均值，和均值差超过 `bias` 的像素算墨迹。
///
/// 全局 Otsu 遇到花纹背景（纹理、半透明底图）会把背景也当文字；局部均值只保留
/// 「细、和周围反差大」的笔画。输出同样是黑字白底。`radius` 是窗口半径（像素），
/// `bias` 是灰度差阈值；背景很花时调用方可以传较大的 `bias`。
///
/// 这个函数不参与 [`rows`] 的默认路径，由调用方在全局二值化效果差时替换使用。
pub fn adaptive_binary(image: &RgbImage, radius: u32, bias: u8) -> GrayImage {
    let gray = channel_max(image);
    let (width, height) = (gray.width() as usize, gray.height() as usize);
    if width == 0 || height == 0 {
        return gray;
    }
    // 积分图，多一行一列哨兵，方便窗口求和。
    let stride = width + 1;
    let mut integral = vec![0u64; stride * (height + 1)];
    for y in 0..height {
        let mut row_sum = 0u64;
        for x in 0..width {
            row_sum += gray.get_pixel(x as u32, y as u32)[0] as u64;
            integral[(y + 1) * stride + (x + 1)] = integral[y * stride + (x + 1)] + row_sum;
        }
    }
    let radius = radius.max(1) as i64;
    let bias = bias as i64;
    let mut out = GrayImage::new(width as u32, height as u32);
    for y in 0..height {
        for x in 0..width {
            let x0 = (x as i64 - radius).max(0) as usize;
            let y0 = (y as i64 - radius).max(0) as usize;
            let x1 = ((x as i64 + radius + 1) as usize).min(width);
            let y1 = ((y as i64 + radius + 1) as usize).min(height);
            let area = ((x1 - x0) * (y1 - y0)).max(1) as u64;
            let sum = integral[y1 * stride + x1] + integral[y0 * stride + x0]
                - integral[y0 * stride + x1]
                - integral[y1 * stride + x0];
            let mean = (sum / area) as i64;
            let value = gray.get_pixel(x as u32, y as u32)[0] as i64;
            let ink = (value - mean).abs() > bias;
            out.put_pixel(x as u32, y as u32, Luma([if ink { 0 } else { 255 }]));
        }
    }
    out
}

/// 取每个像素 R/G/B 的最大值，得到一张单通道亮度图。
fn channel_max(image: &RgbImage) -> GrayImage {
    GrayImage::from_fn(image.width(), image.height(), |x, y| {
        let pixel = image.get_pixel(x, y);
        Luma([pixel[0].max(pixel[1]).max(pixel[2])])
    })
}

/// Otsu 阈值；空图或单色图返回 128。
fn otsu(gray: &GrayImage) -> u8 {
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

/// 边框像素的中位数：四条件取并集，角落重复没关系。
fn border_median(gray: &GrayImage) -> u8 {
    let (width, height) = (gray.width(), gray.height());
    if width == 0 || height == 0 {
        return 0;
    }
    let mut values = Vec::with_capacity((width * 2 + height * 2) as usize);
    for x in 0..width {
        values.push(gray.get_pixel(x, 0)[0]);
        values.push(gray.get_pixel(x, height - 1)[0]);
    }
    for y in 0..height {
        values.push(gray.get_pixel(0, y)[0]);
        values.push(gray.get_pixel(width - 1, y)[0]);
    }
    median_u8(&mut values)
}

/// 无符号字节中位数。
fn median_u8(values: &mut [u8]) -> u8 {
    if values.is_empty() {
        return 0;
    }
    let middle = values.len() / 2;
    values.select_nth_unstable(middle);
    values[middle]
}

// ---------------------------------------------------------------- 几何结构

/// 半开区间矩形：`x0 <= x < x1`、`y0 <= y < y1`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    /// 左边界（含）。
    pub x0: u32,
    /// 上边界（含）。
    pub y0: u32,
    /// 右边界（不含）。
    pub x1: u32,
    /// 下边界（不含）。
    pub y1: u32,
}

impl Rect {
    fn width(self) -> u32 {
        self.x1.saturating_sub(self.x0)
    }

    fn height(self) -> u32 {
        self.y1.saturating_sub(self.y0)
    }
}

/// 4-连通域标记，按面积过滤，返回包围盒（按 x0 升序）。
///
/// 只把灰度 `< 128` 的像素当墨迹，所以 [`binary`] / [`adaptive_binary`] 的输出
/// 可以直接喂进来。面积小于 `min_area` 的连通域（单像素噪点、抗锯齿灰边）丢弃。
pub fn components(gray: &GrayImage, min_area: u32) -> Vec<Rect> {
    let (_, blobs) = label_components(gray, false);
    let mut rects: Vec<Rect> = blobs
        .into_iter()
        .filter(|blob| blob.area >= min_area.max(1))
        .map(|blob| blob.rect)
        .collect();
    rects.sort_by_key(|rect| (rect.x0, rect.y0));
    rects
}

/// 一个连通域的几何信息，行投影和聚类都复用它。
#[derive(Clone, Copy, Debug)]
struct Blob {
    rect: Rect,
    area: u32,
    sum_x: u64,
    sum_y: u64,
}

impl Blob {
    fn center_y(self) -> f32 {
        self.sum_y as f32 / self.area.max(1) as f32
    }
}

/// 给二值图的墨迹像素打标签；返回 `(标签图, 连通域列表)`，标签 0 表示背景。
/// `diagonal = true` 时用 8-连通，供内部行聚类使用：点阵字的斜角笔画在 4-连通下
/// 会碎成很多小段，用它们估行高/聚类会把一行拆散；公开的 [`components`] 仍按
/// 需求保持 4-连通。
fn label_components(gray: &GrayImage, diagonal: bool) -> (Vec<u32>, Vec<Blob>) {
    let (width, height) = (gray.width(), gray.height());
    let mut labels = vec![0u32; width as usize * height as usize];
    let mut blobs = Vec::new();
    let mut stack: Vec<(u32, u32)> = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let index = y as usize * width as usize + x as usize;
            if labels[index] != 0 || gray.get_pixel(x, y)[0] >= 128 {
                continue;
            }
            let label = blobs.len() as u32 + 1;
            let mut blob = Blob {
                rect: Rect {
                    x0: x,
                    y0: y,
                    x1: x + 1,
                    y1: y + 1,
                },
                area: 0,
                sum_x: 0,
                sum_y: 0,
            };
            labels[index] = label;
            stack.push((x, y));
            while let Some((cx, cy)) = stack.pop() {
                blob.area += 1;
                blob.sum_x += cx as u64;
                blob.sum_y += cy as u64;
                blob.rect.x0 = blob.rect.x0.min(cx);
                blob.rect.y0 = blob.rect.y0.min(cy);
                blob.rect.x1 = blob.rect.x1.max(cx + 1);
                blob.rect.y1 = blob.rect.y1.max(cy + 1);
                let neighbors = [
                    (cx.wrapping_sub(1), cy),
                    (cx + 1, cy),
                    (cx, cy.wrapping_sub(1)),
                    (cx, cy + 1),
                    (cx.wrapping_sub(1), cy.wrapping_sub(1)),
                    (cx + 1, cy.wrapping_sub(1)),
                    (cx.wrapping_sub(1), cy + 1),
                    (cx + 1, cy + 1),
                ];
                let count = if diagonal { 8 } else { 4 };
                for (nx, ny) in neighbors.into_iter().take(count) {
                    if nx >= width || ny >= height {
                        continue;
                    }
                    let neighbor_index = ny as usize * width as usize + nx as usize;
                    if labels[neighbor_index] != 0 {
                        continue;
                    }
                    if gray.get_pixel(nx, ny)[0] >= 128 {
                        continue;
                    }
                    labels[neighbor_index] = label;
                    stack.push((nx, ny));
                }
            }
            blobs.push(blob);
        }
    }
    (labels, blobs)
}

// ---------------------------------------------------------------- 行配置

/// 投影分行的全部可调参数；默认值针对游戏任务栏的小字号。
#[derive(Clone, Copy, Debug)]
pub struct RowConfig {
    /// 投影阈值 = `ink_ratio * max_ink`，默认 0.05。
    pub ink_ratio: f32,
    /// 相邻带间隙 `<= merge_ratio * 行高` 时合并，默认 0.12。
    pub merge_ratio: f32,
    /// 带高 `> split_ratio * 行高` 时尝试劈分，默认 1.45。
    pub split_ratio: f32,
    /// 谷底墨迹 `< valley_ratio * 带内均值` 才劈，默认 0.35。
    pub valley_ratio: f32,
    /// 带高 `< short_ratio * 行高` 视为矮带，默认 0.35。
    pub short_ratio: f32,
    /// 行裁图上下留白 = `pad_ratio * 行高`，默认 0.18。
    pub pad_ratio: f32,
    /// 连通域最小面积，默认 4。
    pub min_area: u32,
}

impl Default for RowConfig {
    fn default() -> Self {
        Self {
            ink_ratio: 0.05,
            merge_ratio: 0.12,
            split_ratio: 1.45,
            valley_ratio: 0.35,
            short_ratio: 0.35,
            pad_ratio: 0.18,
            min_area: 4,
        }
    }
}

/// 分行来源：投影主路径，还是校验失败后的连通域聚类兜底。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowSource {
    /// 水平投影（含合并、劈分修复）。
    Projection,
    /// 投影不可信后，按连通域 y 中心聚类得到的行。
    Cluster,
}

/// 一行文字：`rect.y` 是行带上下界，`rect.x` 是墨迹左右界（不含 padding）。
#[derive(Clone, Debug)]
pub struct Row {
    /// 行矩形，y 方向是行带，x 方向已按墨迹裁剪。
    pub rect: Rect,
    /// 行内保留连通域的墨迹像素总数。
    pub ink: u32,
}

/// 一次分行得到的完整布局。
#[derive(Clone, Debug)]
pub struct RowLayout {
    /// 行列表，按 y 升序。
    pub rows: Vec<Row>,
    /// 估计行高（像素），空图时为 1。
    pub line_height: u32,
    /// 分行来源。
    pub source: RowSource,
    /// 分行是否可信；`false` 时调用方应回退到检测模型。
    pub reliable: bool,
    /// 不可信或被修复的原因，写日志用。
    pub notes: Vec<String>,
}

// ---------------------------------------------------------------- 主入口

/// 主入口：面板图 → 行布局。
///
/// 步骤见模块文档。所有阈值都从 [`RowConfig`] 的比率推导，行高先验取原始带高度
/// 的中位数（MAD 剔除离群带）。投影结果通过行数/行高离散度/相邻中心距校验；
/// 不通过时用连通域中心聚类兜底，仍不通过则 `reliable = false`。
pub fn rows(image: &RgbImage, cfg: RowConfig) -> RowLayout {
    let fallback = |notes: Vec<String>| RowLayout {
        rows: Vec::new(),
        line_height: 1,
        source: RowSource::Projection,
        reliable: false,
        notes,
    };
    if image.width() == 0 || image.height() == 0 {
        return fallback(vec!["空图，没有可分的行".to_string()]);
    }

    let gray = binary(image);
    let (labels, blobs) = label_components(&gray, true);
    let keep: Vec<bool> = blobs
        .iter()
        .map(|blob| blob.area >= cfg.min_area.max(1))
        .collect();
    if !keep.iter().any(|kept| *kept) {
        return fallback(vec!["没有面积达标的墨迹连通域".to_string()]);
    }

    let mut notes: Vec<String> = Vec::new();
    let ink = row_ink(&labels, &keep, image.width(), image.height());
    let max_ink = ink.iter().copied().max().unwrap_or(0);
    if max_ink == 0 {
        return fallback(vec!["投影全为 0，没有墨迹".to_string()]);
    }

    // 3 点中值平滑，避免单个抗锯齿行把带切断；窗口再大就会把「二」这种
    // 只有两三行的横笔画整条抹掉。
    let smooth = median_smooth(&ink, 1);
    let threshold = ((cfg.ink_ratio * max_ink as f32).ceil() as u32).max(1);
    let mut bands = bands_from_projection(&smooth, threshold);
    if bands.is_empty() {
        return fallback(vec!["投影阈值下没有连续行带".to_string()]);
    }

    // 同一汉字上下分离的横笔画（「二」「三」）先合并，否则会被当成两行。
    merge_stacked_parts(
        &mut bands,
        &labels,
        &keep,
        &blobs,
        image.width(),
        &mut notes,
    );

    let line_h = line_height_prior(&bands);

    // 小间隙合并；保留合并前的 line_h 用于劈分，否则粘连行合并后永远劈不开。
    let before_merge = bands.len();
    bands = merge_bands(bands, cfg.merge_ratio * line_h as f32);
    if bands.len() < before_merge {
        notes.push(format!(
            "合并了 {} 个小于 {:.1} 像素的间隙",
            before_merge - bands.len(),
            cfg.merge_ratio * line_h as f32
        ));
    }

    let bands = split_bands(bands, &ink, &cfg, line_h, &mut notes);
    let projection_rows = finalize_rows(
        &bands,
        &labels,
        &keep,
        image.width(),
        &cfg,
        line_h,
        &mut notes,
    );

    // 完全粘连：上下两行在二值图里连成一个个高瘦连通域，投影没有谷、聚类中心也
    // 不可靠。这种情况宁可整块判不可信让调用方回退检测，也不要悄悄当成一行。
    if let Some(reason) = stacked_lines_reason(&projection_rows, &blobs, &keep) {
        notes.push(format!("疑似多行完全粘连：{reason}"));
        notes.push("无法可靠拆分，标记为不可信".to_string());
        return RowLayout {
            rows: projection_rows,
            line_height: line_h.max(1),
            source: RowSource::Projection,
            reliable: false,
            notes,
        };
    }

    let mut reliability_note = validate_rows(&projection_rows, line_h).err();
    if reliability_note.is_none() {
        reliability_note = suspicious_tall_row(&projection_rows, &blobs, &keep, &cfg);
    }

    if reliability_note.is_none() {
        return RowLayout {
            rows: projection_rows,
            line_height: line_h.max(1),
            source: RowSource::Projection,
            reliable: true,
            notes,
        };
    }

    // 投影不可信：按连通域 y 中心聚类，再用相邻中心的中点做行界。
    let reason = reliability_note.unwrap_or_else(|| "未知原因".to_string());
    notes.push(format!("投影分行不可信：{reason}，改用连通域聚类"));
    let component_h = component_height_quantile(&blobs, &keep, 0.75);
    let cluster_h = if component_h > 0 && component_h < line_h {
        component_h
    } else {
        line_h
    };
    let cluster_rows = cluster_layout(
        &blobs,
        &keep,
        &labels,
        image.width(),
        &cfg,
        cluster_h,
        &mut notes,
    );
    if cluster_rows.is_empty() {
        notes.push("聚类兜底没有产生任何行".to_string());
        return RowLayout {
            rows: Vec::new(),
            line_height: cluster_h.max(1),
            source: RowSource::Cluster,
            reliable: false,
            notes,
        };
    }
    match validate_rows(&cluster_rows, cluster_h) {
        Ok(()) => RowLayout {
            rows: cluster_rows,
            line_height: cluster_h.max(1),
            source: RowSource::Cluster,
            reliable: true,
            notes,
        },
        Err(error) => {
            notes.push(format!("聚类兜底仍不可信：{error}"));
            RowLayout {
                rows: cluster_rows,
                line_height: cluster_h.max(1),
                source: RowSource::Cluster,
                reliable: false,
                notes,
            }
        }
    }
}

// ---------------------------------------------------------------- 投影

/// 一条原始行带，半开区间 `y0..y1`。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Band {
    y0: u32,
    y1: u32,
}

impl Band {
    fn height(self) -> u32 {
        self.y1.saturating_sub(self.y0)
    }
}

/// 逐行统计「保留连通域」的墨迹数。
fn row_ink(labels: &[u32], keep: &[bool], width: u32, height: u32) -> Vec<u32> {
    let mut ink = vec![0u32; height as usize];
    for (y, row_ink) in ink.iter_mut().enumerate().take(height as usize) {
        let row = y * width as usize;
        for x in 0..width as usize {
            let label = labels[row + x] as usize;
            if label != 0 && keep.get(label - 1).copied().unwrap_or(false) {
                *row_ink += 1;
            }
        }
    }
    ink
}

/// 3~5 点中值平滑，窗口在边界处自动收窄。
fn median_smooth(values: &[u32], radius: usize) -> Vec<u32> {
    if values.is_empty() {
        return Vec::new();
    }
    values
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let start = index.saturating_sub(radius);
            let end = (index + radius + 1).min(values.len());
            let mut window = values[start..end].to_vec();
            window.sort_unstable();
            window[window.len() / 2]
        })
        .collect()
}

/// 连续 `>= threshold` 的区间。
fn bands_from_projection(values: &[u32], threshold: u32) -> Vec<Band> {
    let mut bands = Vec::new();
    let mut start: Option<u32> = None;
    for (y, value) in values.iter().enumerate() {
        if *value >= threshold {
            if start.is_none() {
                start = Some(y as u32);
            }
        } else if let Some(y0) = start.take() {
            bands.push(Band { y0, y1: y as u32 });
        }
    }
    if let Some(y0) = start {
        bands.push(Band {
            y0,
            y1: values.len() as u32,
        });
    }
    bands
}

/// 用 MAD 剔除离群带后的行高先验；只有一条带时就用它自己的高度。
fn line_height_prior(bands: &[Band]) -> u32 {
    match bands.len() {
        0 => 1,
        1 => bands[0].height().max(1),
        _ => {
            let heights: Vec<u32> = bands.iter().map(|band| band.height()).collect();
            let median = median_u32(&heights);
            let deviations: Vec<u32> = heights
                .iter()
                .map(|height| height.abs_diff(median))
                .collect();
            let mad = median_u32(&deviations);
            let limit = mad.saturating_mul(5) / 2; // 2.5 * MAD
            let kept: Vec<u32> = heights
                .iter()
                .copied()
                .filter(|height| height.abs_diff(median) <= limit)
                .collect();
            if kept.is_empty() {
                median.max(1)
            } else {
                median_u32(&kept).max(1)
            }
        }
    }
}

/// 无符号整数中位数（偶数个取偏大的中间值）。
fn median_u32(values: &[u32]) -> u32 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[sorted.len() / 2]
}

/// 小的水平区间求交。
fn overlap_len(left: (u32, u32), right: (u32, u32)) -> u32 {
    left.1.min(right.1).saturating_sub(left.0.max(right.0))
}

/// 扫描一条行带里的墨迹，返回 `(左右墨迹界, 墨迹总数)`。
fn scan_band(labels: &[u32], keep: &[bool], width: u32, band: Band) -> (Option<(u32, u32)>, u32) {
    let mut x0 = width;
    let mut x1 = 0u32;
    let mut ink = 0u32;
    for y in band.y0..band.y1 {
        let row = y as usize * width as usize;
        for x in 0..width as usize {
            let label = labels[row + x] as usize;
            if label != 0 && keep.get(label - 1).copied().unwrap_or(false) {
                x0 = x0.min(x as u32);
                x1 = x1.max(x as u32 + 1);
                ink += 1;
            }
        }
    }
    if x1 > x0 {
        (Some((x0, x1)), ink)
    } else {
        (None, ink)
    }
}

/// 带内的连通域是否都是「横向条带」；用于识别汉字上下分离的笔画。
fn band_all_bars(blobs: &[Blob], keep: &[bool], band: Band) -> bool {
    let mut any = false;
    for (index, blob) in blobs.iter().enumerate() {
        if !keep.get(index).copied().unwrap_or(false) {
            continue;
        }
        let center = blob.center_y();
        if center < band.y0 as f32 || center >= band.y1 as f32 {
            continue;
        }
        any = true;
        let rect = blob.rect;
        // 高度不超过宽度一半，才算横笔画；正常字块不会被误判。
        if rect.height() * 2 > rect.width() {
            return false;
        }
    }
    any
}

/// 合并同一汉字里上下分离的横笔画（如「二」）。
///
/// 条件很窄：相邻两带之间的间隙不超过较高带高度的 3 倍、列范围高度重合、
/// 且两带里的连通域全是横向条带。正常两行文字里总会有竖笔画/宽字块，
/// 不会满足「全是横条」，所以这条规则只在孤立的横笔画上生效。
fn merge_stacked_parts(
    bands: &mut Vec<Band>,
    labels: &[u32],
    keep: &[bool],
    blobs: &[Blob],
    width: u32,
    notes: &mut Vec<String>,
) {
    let mut guard = 0;
    loop {
        guard += 1;
        if guard > 16 {
            break;
        }
        let mut found = None;
        for index in 0..bands.len().saturating_sub(1) {
            let left = bands[index];
            let right = bands[index + 1];
            let gap = right.y0.saturating_sub(left.y1);
            if gap == 0 || gap > left.height().max(right.height()).saturating_mul(3) {
                continue;
            }
            let (Some(left_x), Some(right_x)) = (
                band_extent(labels, keep, width, left),
                band_extent(labels, keep, width, right),
            ) else {
                continue;
            };
            let shortest = left_x.1.saturating_sub(left_x.0).min(right_x.1 - right_x.0);
            if shortest == 0 || overlap_len(left_x, right_x) * 10 < shortest * 6 {
                continue;
            }
            if !band_all_bars(blobs, keep, left) || !band_all_bars(blobs, keep, right) {
                continue;
            }
            found = Some(index);
            break;
        }
        let Some(index) = found else {
            break;
        };
        notes.push(format!(
            "合并上下分离的横笔画：y={}..{} 与 y={}..{}",
            bands[index].y0,
            bands[index].y1,
            bands[index + 1].y0,
            bands[index + 1].y1
        ));
        bands[index].y1 = bands[index + 1].y1;
        bands.remove(index + 1);
    }
}

/// 只取一条带的左右墨迹界，不统计墨迹数。
fn band_extent(labels: &[u32], keep: &[bool], width: u32, band: Band) -> Option<(u32, u32)> {
    scan_band(labels, keep, width, band).0
}

/// 合并间隙 `<= gap_limit` 的相邻带。
fn merge_bands(bands: Vec<Band>, gap_limit: f32) -> Vec<Band> {
    let mut merged: Vec<Band> = Vec::with_capacity(bands.len());
    for band in bands {
        if let Some(last) = merged.last_mut() {
            let gap = band.y0.saturating_sub(last.y1);
            if gap as f32 <= gap_limit {
                last.y1 = band.y1;
                continue;
            }
        }
        merged.push(band);
    }
    merged
}

/// 3 点均值平滑，用来找谷底；中值平滑会把 1 像素宽的间隙抹掉。
fn mean_smooth(values: &[u32], radius: usize) -> Vec<u32> {
    if values.is_empty() {
        return Vec::new();
    }
    values
        .iter()
        .enumerate()
        .map(|(index, _)| {
            let start = index.saturating_sub(radius);
            let end = (index + radius + 1).min(values.len());
            let sum: u64 = values[start..end].iter().map(|value| *value as u64).sum();
            (sum / (end - start) as u64) as u32
        })
        .collect()
}

/// 递归劈分过高的带；找不到合格谷底就保留原带并写 note。
fn split_bands(
    bands: Vec<Band>,
    ink: &[u32],
    cfg: &RowConfig,
    line_h: u32,
    notes: &mut Vec<String>,
) -> Vec<Band> {
    let smooth = mean_smooth(ink, 1);
    let mut work = bands;
    let mut output = Vec::new();
    let mut guard = 0;
    while let Some(band) = work.pop() {
        guard += 1;
        if guard > 256 {
            output.push(band);
            continue;
        }
        if band.height() as f32 <= cfg.split_ratio * line_h.max(1) as f32 {
            output.push(band);
            continue;
        }
        match find_valley(band, ink, &smooth, cfg, line_h) {
            Some(cut) => {
                notes.push(format!(
                    "行带 y={}..{} 过高，在谷底 y={cut} 处劈分",
                    band.y0, band.y1
                ));
                work.push(Band {
                    y0: band.y0,
                    y1: cut,
                });
                work.push(Band {
                    y0: cut,
                    y1: band.y1,
                });
            }
            None => {
                notes.push(format!(
                    "行带 y={}..{} 过高但找不到合格谷底，保留整带",
                    band.y0, band.y1
                ));
                output.push(band);
            }
        }
    }
    output.sort_by_key(|band| band.y0);
    output
}

/// 在带内部找最深且离上下沿都足够远的局部极小。
fn find_valley(
    band: Band,
    ink: &[u32],
    smooth: &[u32],
    cfg: &RowConfig,
    line_h: u32,
) -> Option<u32> {
    if band.height() < 4 {
        return None;
    }
    let margin = ((0.35 * line_h.max(1) as f32).ceil() as u32).max(1);
    let sum: u64 = ink[band.y0 as usize..band.y1 as usize]
        .iter()
        .map(|value| *value as u64)
        .sum();
    let mean = sum as f32 / band.height() as f32;
    let mut best: Option<(u32, u32)> = None; // (墨迹, 切点)
    for y in (band.y0 + 1)..(band.y1 - 1) {
        if y.saturating_sub(band.y0) <= margin || band.y1.saturating_sub(y) <= margin {
            continue;
        }
        let value = smooth[y as usize];
        if value > smooth[y as usize - 1] || value > smooth[y as usize + 1] {
            continue;
        }
        if best.is_none_or(|(best_value, _)| value < best_value) {
            best = Some((value, y));
        }
    }
    let (value, cut) = best?;
    if (value as f32) < cfg.valley_ratio * mean {
        Some(cut)
    } else {
        None
    }
}

/// 把行带变成 `Row`：左右按墨迹裁剪、统计墨迹、丢弃又矮又长的装饰线。
fn finalize_rows(
    bands: &[Band],
    labels: &[u32],
    keep: &[bool],
    width: u32,
    cfg: &RowConfig,
    line_h: u32,
    notes: &mut Vec<String>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    for band in bands {
        let (extent, ink) = scan_band(labels, keep, width, *band);
        let Some((x0, x1)) = extent else {
            notes.push(format!("行带 y={}..{} 没有墨迹，丢弃", band.y0, band.y1));
            continue;
        };
        let row_width = x1.saturating_sub(x0);
        let row_height = band.height();
        if (row_height as f32) < cfg.short_ratio * line_h.max(1) as f32
            && (row_width as f32) > 3.0 * line_h.max(1) as f32
        {
            notes.push(format!(
                "丢弃装饰线 y={}..{}（宽 {row_width}，高 {row_height}）",
                band.y0, band.y1
            ));
            continue;
        }
        rows.push(Row {
            rect: Rect {
                x0,
                y0: band.y0,
                x1,
                y1: band.y1,
            },
            ink,
        });
    }
    rows.sort_by_key(|row| row.rect.y0);
    rows
}

/// 校验行数、行高离散度、相邻行中心距；返回不可信原因。
fn validate_rows(rows: &[Row], line_h: u32) -> Result<(), String> {
    if rows.is_empty() {
        return Err("没有检测到任何行".to_string());
    }
    if rows.len() > 8 {
        return Err(format!("行数 {} 超过 8", rows.len()));
    }
    let line_h = line_h.max(1) as f32;
    let heights: Vec<f32> = rows.iter().map(|row| row.rect.height() as f32).collect();
    let mean = heights.iter().sum::<f32>() / heights.len() as f32;
    let variance = heights
        .iter()
        .map(|height| (height - mean).powi(2))
        .sum::<f32>()
        / heights.len() as f32;
    let sigma = variance.sqrt();
    if sigma / line_h >= 0.35 {
        return Err(format!("行高离散度过大：σ/h0 = {:.2}", sigma / line_h));
    }
    for pair in rows.windows(2) {
        let first = (pair[0].rect.y0 + pair[0].rect.y1) as f32 / 2.0;
        let second = (pair[1].rect.y0 + pair[1].rect.y1) as f32 / 2.0;
        let distance = (second - first).abs() / line_h;
        if !(0.8..=3.0).contains(&distance) {
            return Err(format!("相邻行中心距/h0 = {distance:.2}，超出 [0.8, 3.0]"));
        }
    }
    Ok(())
}

/// 检测「上下行完全粘连」：一行里多数墨迹来自又高又瘦的连通域。
///
/// 正常一行的字块宽高比大致在 1:1~1:2.5 之间（点阵数字「1」约 1:2.3）；
/// 两行完全贴在一起时，同一个字的两截会连成 1:3 以上的竖条。只按墨迹面积
/// 占比判断，单个竖直笔画不会把整行带偏。
fn stacked_lines_reason(rows: &[Row], blobs: &[Blob], keep: &[bool]) -> Option<String> {
    for row in rows {
        let mut total = 0u64;
        let mut tall = 0u64;
        for (index, blob) in blobs.iter().enumerate() {
            if !keep.get(index).copied().unwrap_or(false) {
                continue;
            }
            let center = blob.center_y();
            if center < row.rect.y0 as f32 || center >= row.rect.y1 as f32 {
                continue;
            }
            total += blob.area as u64;
            let height = blob.rect.height();
            let width = blob.rect.width().max(1);
            if height >= row.rect.height() / 2 && height * 2 >= width * 5 {
                tall += blob.area as u64;
            }
        }
        if total > 0 && tall * 2 > total {
            return Some(format!(
                "行 y={}..{} 内多数墨迹来自高瘦连通域",
                row.rect.y0, row.rect.y1
            ));
        }
    }
    None
}

/// 一行的高度明显大于其内部典型连通域高度时，怀疑它其实是多行粘连。
fn suspicious_tall_row(
    rows: &[Row],
    blobs: &[Blob],
    keep: &[bool],
    cfg: &RowConfig,
) -> Option<String> {
    for row in rows {
        // 只有横笔画的行（如孤立的「二」）天生就像两行，跳过这种误报。
        let band = Band {
            y0: row.rect.y0,
            y1: row.rect.y1,
        };
        if band_all_bars(blobs, keep, band) {
            continue;
        }
        let mut heights: Vec<u32> = blobs
            .iter()
            .enumerate()
            .filter(|(index, blob)| {
                keep.get(*index).copied().unwrap_or(false)
                    && blob.center_y() >= row.rect.y0 as f32
                    && blob.center_y() < row.rect.y1 as f32
            })
            .map(|(_, blob)| blob.rect.height())
            .collect();
        if heights.is_empty() {
            continue;
        }
        heights.sort_unstable();
        let typical = heights[(heights.len() - 1) * 3 / 4].max(1);
        let limit = cfg.split_ratio * 1.25 * typical as f32;
        if row.rect.height() as f32 > limit {
            return Some(format!(
                "行 y={}..{} 高 {}，约为典型连通域高度的 {:.1} 倍",
                row.rect.y0,
                row.rect.y1,
                row.rect.height(),
                row.rect.height() as f32 / typical as f32
            ));
        }
    }
    None
}

// ---------------------------------------------------------------- 聚类兜底

/// 保留连通域高度的 75% 分位；用来估计「一个字大概多高」。
fn component_height_quantile(blobs: &[Blob], keep: &[bool], quantile: f32) -> u32 {
    let mut heights: Vec<u32> = blobs
        .iter()
        .enumerate()
        .filter(|(index, _)| keep.get(*index).copied().unwrap_or(false))
        .map(|(_, blob)| blob.rect.height())
        .collect();
    if heights.is_empty() {
        return 0;
    }
    heights.sort_unstable();
    let index = ((heights.len() - 1) as f32 * quantile.clamp(0.0, 1.0)).round() as usize;
    heights[index.min(heights.len() - 1)]
}

/// 按连通域 y 中心聚类，再以相邻中心的中点为界得到互不重叠的行。
fn cluster_layout(
    blobs: &[Blob],
    keep: &[bool],
    labels: &[u32],
    width: u32,
    cfg: &RowConfig,
    line_h: u32,
    notes: &mut Vec<String>,
) -> Vec<Row> {
    let height = blobs
        .iter()
        .enumerate()
        .filter(|(index, _)| keep.get(*index).copied().unwrap_or(false))
        .map(|(_, blob)| blob.rect.y1)
        .max()
        .unwrap_or(0);
    let mut items: Vec<(usize, f32)> = blobs
        .iter()
        .enumerate()
        .filter(|(index, _)| keep.get(*index).copied().unwrap_or(false))
        .map(|(index, blob)| (index, blob.center_y()))
        .collect();
    if items.is_empty() {
        return Vec::new();
    }
    items.sort_by(|left, right| left.1.total_cmp(&right.1));

    let threshold = 0.55 * line_h.max(1) as f32;
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut sums: Vec<f32> = Vec::new();
    for (index, center) in items {
        if let Some(last) = groups.last_mut() {
            let mean = sums.last().copied().unwrap_or(center);
            if (center - mean).abs() < threshold {
                last.push(index);
                let n = last.len() as f32;
                if let Some(sum) = sums.last_mut() {
                    *sum = (*sum * (n - 1.0) + center) / n;
                }
                continue;
            }
        }
        groups.push(vec![index]);
        sums.push(center);
    }
    if groups.is_empty() {
        return Vec::new();
    }

    let centers: Vec<f32> = groups
        .iter()
        .map(|group| {
            let sum: f32 = group.iter().map(|index| blobs[*index].center_y()).sum();
            sum / group.len() as f32
        })
        .collect();
    let mut bounds = vec![0u32; groups.len() + 1];
    bounds[0] = groups[0]
        .iter()
        .map(|index| blobs[*index].rect.y0)
        .min()
        .unwrap_or(0);
    bounds[groups.len()] = groups[groups.len() - 1]
        .iter()
        .map(|index| blobs[*index].rect.y1)
        .max()
        .unwrap_or(height);
    for index in 1..groups.len() {
        bounds[index] = ((centers[index - 1] + centers[index]) / 2.0).round() as u32;
    }
    // 保证严格递增、落在图内；极端情况下聚类结果不可用就返回空。
    for index in 1..=groups.len() {
        bounds[index] = bounds[index].max(bounds[index - 1] + 1).min(height);
    }
    if bounds[groups.len()] <= bounds[0] {
        notes.push("聚类行界退化，无法形成有效行".to_string());
        return Vec::new();
    }

    let bands: Vec<Band> = (0..groups.len())
        .map(|index| Band {
            y0: bounds[index],
            y1: bounds[index + 1],
        })
        .collect();
    finalize_rows(&bands, labels, keep, width, cfg, line_h, notes)
}

// ---------------------------------------------------------------- 行裁图

/// 按布局裁每行的图片：灰度化 + 1%~99% 分位对比度拉伸 + 按需反相（保证暗字亮底）
/// + 上下 `pad_ratio * 行高` 留白 + 左右 1~2 像素留白，超出图边界会夹住。
///
/// 输出是 `RgbImage`（三通道同值），方便直接喂识别模型。极性判断用行图边框的
/// 亮度对比整图中位数：边框更暗说明背景暗、字亮，需要反相；两者接近时再看边框
/// 是否低于中灰，避免纯色小图判反。
pub fn row_images(image: &RgbImage, layout: &RowLayout, cfg: RowConfig) -> Vec<RgbImage> {
    if image.width() == 0 || image.height() == 0 {
        return Vec::new();
    }
    let pad_y = (cfg.pad_ratio * layout.line_height.max(1) as f32).round() as u32;
    let mut output = Vec::with_capacity(layout.rows.len());
    for row in &layout.rows {
        let x0 = row.rect.x0.saturating_sub(2);
        let x1 = (row.rect.x1 + 2).min(image.width());
        let y0 = row.rect.y0.saturating_sub(pad_y);
        let y1 = (row.rect.y1 + pad_y).min(image.height());
        if x1 <= x0 || y1 <= y0 {
            continue;
        }
        let crop = image::imageops::crop_imm(image, x0, y0, x1 - x0, y1 - y0).to_image();
        let gray = to_luma(&crop);
        let stretched = stretch_percentile(&gray);
        let border = border_median(&stretched);
        let median = image_median(&stretched);
        let invert = if border.abs_diff(median) <= 1 {
            border < 128
        } else {
            border < median
        };
        let (width, height) = (stretched.width(), stretched.height());
        output.push(RgbImage::from_fn(width, height, |x, y| {
            let value = stretched.get_pixel(x, y)[0];
            let value = if invert { 255 - value } else { value };
            Rgb([value, value, value])
        }));
    }
    output
}

/// RGB → 灰度，权重用 BT.601 近似（和项目其它地方的 luma 一致）。
fn to_luma(image: &RgbImage) -> GrayImage {
    GrayImage::from_fn(image.width(), image.height(), |x, y| {
        let pixel = image.get_pixel(x, y);
        let value =
            ((77 * pixel[0] as u32 + 150 * pixel[1] as u32 + 29 * pixel[2] as u32) >> 8) as u8;
        Luma([value])
    })
}

/// 按 1%/99% 分位做对比度拉伸；分位相等或图为空时原样返回。
fn stretch_percentile(gray: &GrayImage) -> GrayImage {
    let total = gray.width() as u64 * gray.height() as u64;
    if total == 0 {
        return gray.clone();
    }
    let mut histogram = [0u32; 256];
    for pixel in gray.pixels() {
        histogram[pixel[0] as usize] += 1;
    }
    let low_rank = (total as f64 * 0.01).floor() as u64;
    let high_rank = (total as f64 * 0.99).ceil() as u64;
    let mut cumulative = 0u64;
    let mut low = 0u8;
    let mut high = 255u8;
    let mut low_found = false;
    for (value, count) in histogram.iter().enumerate() {
        cumulative += *count as u64;
        if !low_found && cumulative > low_rank {
            low = value as u8;
            low_found = true;
        }
        if cumulative >= high_rank {
            high = value as u8;
            break;
        }
    }
    if high <= low {
        return gray.clone();
    }
    let span = high as i32 - low as i32;
    GrayImage::from_fn(gray.width(), gray.height(), |x, y| {
        let value = gray.get_pixel(x, y)[0] as i32;
        let scaled = ((value - low as i32) * 255 / span).clamp(0, 255) as u8;
        Luma([scaled])
    })
}

/// 全图中位数亮度（直方图累计到一半）。
fn image_median(gray: &GrayImage) -> u8 {
    let total = gray.width() as u64 * gray.height() as u64;
    if total == 0 {
        return 0;
    }
    let mut histogram = [0u32; 256];
    for pixel in gray.pixels() {
        histogram[pixel[0] as usize] += 1;
    }
    let target = total / 2;
    let mut cumulative = 0u64;
    for (value, count) in histogram.iter().enumerate() {
        cumulative += *count as u64;
        if cumulative > target {
            return value as u8;
        }
    }
    255
}

// ---------------------------------------------------------------- 长行分块

/// 长行分块：rec 模型输入约为 3x48x320（宽高比 ≈ 6.7），超宽行要切成多块。
///
/// `gray` 是已经裁好的单行图。`w/h <= ratio` 时直接返回整块；否则沿 x 方向贪心
/// 切分，每次在「目标宽度 ±25%」范围内找列墨迹最少的位置，且保证每块
/// `w/h <= ratio * 1.1`。块之间不重叠、不丢像素，按 x 升序返回半开区间。
pub fn chunks(gray: &GrayImage, ratio: f32) -> Vec<(u32, u32)> {
    let (width, height) = (gray.width(), gray.height());
    if width == 0 || height == 0 || !ratio.is_finite() || ratio <= 0.0 {
        return Vec::new();
    }
    let height_f = height as f32;
    if width as f32 <= ratio * height_f {
        return vec![(0, width)];
    }
    let mut columns = vec![0u32; width as usize];
    for y in 0..height {
        for x in 0..width {
            if gray.get_pixel(x, y)[0] < 128 {
                columns[x as usize] += 1;
            }
        }
    }
    let target = (ratio * height_f).round().max(1.0) as u32;
    let max_width = (ratio * 1.1 * height_f).floor().max(1.0) as u32;
    let span = (target as f32 * 0.25).round() as u32;
    let mut parts = Vec::new();
    let mut start = 0u32;
    while start < width {
        let remaining = width - start;
        if remaining as f32 <= max_width as f32 {
            parts.push((start, width));
            break;
        }
        let target_x = start.saturating_add(target).min(width - 1);
        let low = start
            .saturating_add(target.saturating_sub(span))
            .max(start + 1);
        let high = start
            .saturating_add(target.saturating_add(span))
            .min(start.saturating_add(max_width))
            .min(width - 1);
        let mut cut = if low <= high {
            (low..=high)
                .min_by_key(|x| (columns[*x as usize], x.abs_diff(target_x)))
                .unwrap_or(high)
        } else {
            start.saturating_add(max_width).min(width - 1)
        };
        if cut <= start {
            cut = (start + 1).min(width - 1);
        }
        if cut >= width {
            break;
        }
        parts.push((start, cut));
        start = cut;
    }
    // 极端图（例如宽度方向全是高墨迹列）下保证最后一块收尾。
    if parts.last().is_none_or(|part| part.1 < width) {
        if let Some(last) = parts.last_mut() {
            last.1 = width;
        } else {
            parts.push((0, width));
        }
    }
    parts
}

// ---------------------------------------------------------------- 测试

#[cfg(test)]
mod tests {
    use super::*;

    /// 5x7 点阵，只保留测试要用的字符；「二」是额外的上下分离笔画样例。
    fn pattern(ch: char) -> Option<[&'static str; 7]> {
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
            '二' => [
                "00000", "11111", "00000", "00000", "00000", "11111", "00000",
            ],
            _ => return None,
        };
        Some(lines)
    }

    /// 把一行文字渲染成黑字白底图。
    fn render(text: &str, scale: u32, gap: u32) -> RgbImage {
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

    /// 把一行画到画布的 (x, y) 处。
    fn place(canvas: &mut RgbImage, line: &RgbImage, x: u32, y: u32) {
        image::imageops::overlay(canvas, line, x as i64, y as i64);
    }

    fn white_canvas(width: u32, height: u32) -> RgbImage {
        RgbImage::from_pixel(width, height, Rgb([255, 255, 255]))
    }

    #[test]
    fn splits_three_well_spaced_lines() {
        let mut canvas = white_canvas(220, 90);
        place(&mut canvas, &render("0123", 2, 2), 10, 8);
        place(&mut canvas, &render("4567", 2, 2), 10, 28);
        place(&mut canvas, &render("89", 2, 2), 10, 48);

        let layout = rows(&canvas, RowConfig::default());
        assert_eq!(layout.rows.len(), 3, "notes: {:?}", layout.notes);
        assert!(layout.reliable);
        assert_eq!(layout.source, RowSource::Projection);
        assert!(layout
            .rows
            .windows(2)
            .all(|pair| pair[0].rect.y0 < pair[1].rect.y0));
        assert!(layout.rows.iter().all(|row| row.rect.x1 > row.rect.x0));
    }

    #[test]
    fn splits_two_touching_lines() {
        let mut canvas = white_canvas(220, 40);
        // 两行之间只留 1 像素空隙，并放一个 2x2 的小墨点让这一行也不是纯背景；
        // 墨点面积刚好等于 min_area，不会被当成噪点过滤，但也不和两行字相连。
        place(&mut canvas, &render("0123", 2, 2), 10, 4);
        place(&mut canvas, &render("4567", 2, 2), 10, 19);
        for y in 18..20 {
            for x in 70..72 {
                canvas.put_pixel(x, y, Rgb([20, 20, 20]));
            }
        }

        let layout = rows(&canvas, RowConfig::default());
        assert_eq!(layout.rows.len(), 2, "notes: {:?}", layout.notes);
        assert!(layout.reliable, "notes: {:?}", layout.notes);
        assert!(matches!(
            layout.source,
            RowSource::Projection | RowSource::Cluster
        ));
        assert!(layout.rows[0].rect.y1 <= layout.rows[1].rect.y0);
    }

    #[test]
    fn keeps_single_line() {
        let mut canvas = white_canvas(120, 40);
        place(&mut canvas, &render("0123", 2, 2), 10, 10);

        let layout = rows(&canvas, RowConfig::default());
        assert_eq!(layout.rows.len(), 1, "notes: {:?}", layout.notes);
        assert!(layout.reliable);
        assert_eq!(layout.source, RowSource::Projection);
    }

    #[test]
    fn fully_touching_lines_are_not_silently_one_row() {
        let mut canvas = white_canvas(220, 40);
        // 两行像素直接相邻，二值图里会连成高瘦竖条，投影没有谷底。
        place(&mut canvas, &render("0123", 2, 2), 10, 4);
        place(&mut canvas, &render("4567", 2, 2), 10, 18);

        let layout = rows(&canvas, RowConfig::default());
        assert!(
            layout.rows.len() != 1 || !layout.reliable,
            "完全粘连不能悄悄当成一行：{:?}",
            layout.notes
        );
    }

    #[test]
    fn empty_images_are_unreliable() {
        let blank = white_canvas(80, 40);
        let layout = rows(&blank, RowConfig::default());
        assert!(layout.rows.is_empty());
        assert!(!layout.reliable);
        assert!(!layout.notes.is_empty());

        let empty = RgbImage::new(0, 0);
        let layout = rows(&empty, RowConfig::default());
        assert!(layout.rows.is_empty());
        assert!(!layout.reliable);
        assert_eq!(layout.line_height, 1);
    }

    #[test]
    fn single_pixel_noise_does_not_add_rows() {
        let mut canvas = white_canvas(180, 70);
        place(&mut canvas, &render("0123", 2, 2), 10, 8);
        place(&mut canvas, &render("4567", 2, 2), 10, 38);
        canvas.put_pixel(160, 3, Rgb([20, 20, 20]));
        canvas.put_pixel(170, 65, Rgb([20, 20, 20]));

        let layout = rows(&canvas, RowConfig::default());
        assert_eq!(layout.rows.len(), 2, "notes: {:?}", layout.notes);
        assert!(layout.reliable, "notes: {:?}", layout.notes);
    }

    #[test]
    fn punctuation_line_is_not_treated_as_noise() {
        let mut canvas = white_canvas(80, 50);
        place(&mut canvas, &render(",,", 3, 2), 10, 10);

        let layout = rows(&canvas, RowConfig::default());
        assert_eq!(layout.rows.len(), 1, "notes: {:?}", layout.notes);
        assert!(layout.reliable, "notes: {:?}", layout.notes);
        assert!(layout.rows[0].ink > 0);
    }

    #[test]
    fn chinese_stacked_strokes_stay_in_one_line() {
        let mut canvas = white_canvas(80, 50);
        place(&mut canvas, &render("二", 2, 2), 10, 10);

        let layout = rows(&canvas, RowConfig::default());
        assert_eq!(layout.rows.len(), 1, "notes: {:?}", layout.notes);
        assert!(layout.reliable, "notes: {:?}", layout.notes);
    }

    #[test]
    fn chunks_cover_wide_row_without_gaps() {
        let mut gray = GrayImage::from_pixel(200, 20, Luma([255]));
        for x in (6..194).step_by(12) {
            for y in 3..17 {
                for dx in 0..5 {
                    gray.put_pixel(x + dx, y, Luma([0]));
                }
            }
        }

        let parts = chunks(&gray, 6.0);
        assert!(parts.len() > 1, "parts: {parts:?}");
        assert_eq!(parts.first().map(|part| part.0), Some(0));
        assert_eq!(parts.last().map(|part| part.1), Some(gray.width()));
        assert!(parts.windows(2).all(|pair| pair[0].1 == pair[1].0));
        for part in &parts {
            assert!(part.1 > part.0, "parts: {parts:?}");
            let ratio = (part.1 - part.0) as f32 / gray.height() as f32;
            assert!(ratio <= 6.6, "part {part:?} ratio {ratio}");
        }
    }

    #[test]
    fn row_images_respect_padding_and_bounds() {
        let mut canvas = white_canvas(220, 90);
        place(&mut canvas, &render("0123", 2, 2), 10, 8);
        place(&mut canvas, &render("4567", 2, 2), 10, 28);

        let cfg = RowConfig::default();
        let layout = rows(&canvas, cfg);
        assert_eq!(layout.rows.len(), 2);
        let pad_y = (cfg.pad_ratio * layout.line_height as f32).round() as u32;
        let images = row_images(&canvas, &layout, cfg);
        assert_eq!(images.len(), layout.rows.len());
        for (row, image) in layout.rows.iter().zip(images.iter()) {
            let expected_width =
                (row.rect.x1 + 2).min(canvas.width()) - row.rect.x0.saturating_sub(2);
            let expected_height =
                (row.rect.y1 + pad_y).min(canvas.height()) - row.rect.y0.saturating_sub(pad_y);
            assert_eq!(image.width(), expected_width);
            assert_eq!(image.height(), expected_height);
        }

        // 人为把行放到贴边位置，裁图不能越界。
        let edge = RowLayout {
            rows: vec![Row {
                rect: Rect {
                    x0: 0,
                    y0: 0,
                    x1: canvas.width(),
                    y1: canvas.height(),
                },
                ink: 1,
            }],
            line_height: 20,
            source: RowSource::Projection,
            reliable: true,
            notes: Vec::new(),
        };
        let images = row_images(&canvas, &edge, cfg);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].width(), canvas.width());
        assert_eq!(images[0].height(), canvas.height());
    }

    #[test]
    fn binary_normalizes_both_polarities() {
        let mut dark = RgbImage::from_pixel(40, 20, Rgb([25, 35, 55]));
        for y in 6..14 {
            for x in 8..32 {
                dark.put_pixel(x, y, Rgb([250, 250, 250]));
            }
        }
        let gray = binary(&dark);
        assert!(gray.get_pixel(20, 10)[0] < 128, "亮字应变成黑墨迹");
        assert!(gray.get_pixel(2, 2)[0] > 128, "暗背景应变成白底");

        let mut light = RgbImage::from_pixel(40, 20, Rgb([245, 245, 245]));
        for y in 6..14 {
            for x in 8..32 {
                light.put_pixel(x, y, Rgb([10, 10, 10]));
            }
        }
        let gray = binary(&light);
        assert!(gray.get_pixel(20, 10)[0] < 128, "暗字应变成黑墨迹");
        assert!(gray.get_pixel(2, 2)[0] > 128, "亮背景应变成白底");
    }
}
