//! OCR 前的图像预处理流水线：颜色提取 → 灰度 → 二值化 → 反色，可任意组合。
use image::{Rgb, RgbImage};

/// 预处理步骤组合。顺序固定为：颜色提取 → 灰度 → 二值化 → 反色。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Pipeline {
    /// 只保留指定文字颜色附近的像素（涂黑），其余涂白。
    pub color_key: bool,
    pub grayscale: bool,
    /// Otsu 阈值二值化（内部先转灰度）。
    pub binarize: bool,
    /// 最终黑白颠倒；二值化把文字判成反色时使用。
    pub invert: bool,
}

impl Pipeline {
    /// 兼容旧配置：`off` / `gray` / `binary` / `color`，也支持 `color,binary` 这样的组合。
    pub fn from_config(value: &str) -> Self {
        let mut pipeline = Self::default();
        for step in value.split(',') {
            match step.trim() {
                "color" => pipeline.color_key = true,
                "gray" | "grayscale" => pipeline.grayscale = true,
                "binary" | "binarize" => pipeline.binarize = true,
                "invert" => pipeline.invert = true,
                _ => {}
            }
        }
        pipeline
    }

    pub fn config_value(self) -> String {
        let mut steps = Vec::new();
        if self.color_key {
            steps.push("color");
        }
        if self.grayscale {
            steps.push("gray");
        }
        if self.binarize {
            steps.push("binary");
        }
        if self.invert {
            steps.push("invert");
        }
        if steps.is_empty() {
            "off".to_string()
        } else {
            steps.join(",")
        }
    }

    pub fn is_off(self) -> bool {
        !(self.color_key || self.grayscale || self.binarize || self.invert)
    }

    pub fn summary(self) -> String {
        let mut steps = Vec::new();
        if self.color_key {
            steps.push("颜色提取");
        }
        if self.grayscale {
            steps.push("灰度");
        }
        if self.binarize {
            steps.push("二值化");
        }
        if self.invert {
            steps.push("反色");
        }
        if steps.is_empty() {
            "关闭".to_string()
        } else {
            steps.join(" → ")
        }
    }
}

/// 按固定顺序执行勾选的步骤。完全关闭时直接返回原图，不产生额外拷贝。
pub fn apply(image: RgbImage, pipeline: Pipeline, target: [u8; 3], tolerance: u8) -> RgbImage {
    if pipeline.is_off() {
        return image;
    }
    let mut current = image;
    if pipeline.color_key {
        current = color_key(&current, target, tolerance);
    }
    if pipeline.grayscale {
        current = grayscale(&current);
    }
    if pipeline.binarize {
        current = binarize(&current);
    }
    if pipeline.invert {
        current = invert_image(&current);
    }
    current
}

fn luma(pixel: &Rgb<u8>) -> u8 {
    ((77 * pixel[0] as u32 + 150 * pixel[1] as u32 + 29 * pixel[2] as u32) >> 8) as u8
}

fn grayscale(image: &RgbImage) -> RgbImage {
    RgbImage::from_fn(image.width(), image.height(), |x, y| {
        let value = luma(image.get_pixel(x, y));
        Rgb([value, value, value])
    })
}

/// 保留与目标文字颜色接近的像素并涂黑，其余涂白。
/// 直接输出黑字白底，避免白色文字被涂白后“消失”，也方便叠加灰度 / 二值化。
fn color_key(image: &RgbImage, target: [u8; 3], tolerance: u8) -> RgbImage {
    let tolerance = i16::from(tolerance);
    RgbImage::from_fn(image.width(), image.height(), |x, y| {
        let pixel = *image.get_pixel(x, y);
        let close = (0..3).all(|channel| {
            (i16::from(pixel[channel]) - i16::from(target[channel])).abs() <= tolerance
        });
        let output = if close { 0 } else { 255 };
        Rgb([output, output, output])
    })
}

/// 放大截图：任务栏文字较小时，PP-OCR 的检测更容易漏掉。
/// 用 Lanczos3 保持笔画边缘，放大后不影响后续预处理顺序。
pub fn upscale(image: &RgbImage, factor: u32) -> RgbImage {
    let factor = factor.clamp(1, 3);
    if factor <= 1 {
        return image.clone();
    }
    image::imageops::resize(
        image,
        image.width() * factor,
        image.height() * factor,
        image::imageops::FilterType::Lanczos3,
    )
}

/// 灰度 + Otsu 阈值二值化，输出白底黑字。
/// 背景通常贴着截图边缘，用边框亮度判断哪一侧是背景，比“少数派是文字”更稳；
/// 边框同时混有文字和背景时，退回少数派判断。
fn binarize(image: &RgbImage) -> RgbImage {
    let mut histogram = [0u32; 256];
    for pixel in image.pixels() {
        histogram[luma(pixel) as usize] += 1;
    }
    let threshold = otsu_threshold(&histogram);
    let background = border_luma(image);
    let text_is_light = if background.abs_diff(threshold) <= 4 {
        let mut darker = 0u64;
        let mut lighter = 0u64;
        for (value, count) in histogram.iter().enumerate() {
            if value as u8 <= threshold {
                darker += u64::from(*count);
            } else {
                lighter += u64::from(*count);
            }
        }
        lighter < darker
    } else {
        // 背景在暗侧则文字在亮侧，反之亦然。
        background <= threshold
    };
    RgbImage::from_fn(image.width(), image.height(), |x, y| {
        let value = luma(image.get_pixel(x, y));
        let is_text = if text_is_light {
            value > threshold
        } else {
            value <= threshold
        };
        let output = if is_text { 0 } else { 255 };
        Rgb([output, output, output])
    })
}

/// 截图四周边框像素亮度的中位数，用来判断背景亮暗。
fn border_luma(image: &RgbImage) -> u8 {
    let (width, height) = image.dimensions();
    let mut values = Vec::with_capacity((width * 2 + height * 2) as usize);
    for x in 0..width {
        values.push(luma(image.get_pixel(x, 0)));
        if height > 1 {
            values.push(luma(image.get_pixel(x, height - 1)));
        }
    }
    for y in 1..height.saturating_sub(1) {
        values.push(luma(image.get_pixel(0, y)));
        if width > 1 {
            values.push(luma(image.get_pixel(width - 1, y)));
        }
    }
    if values.is_empty() {
        return 255;
    }
    values.sort_unstable();
    values[values.len() / 2]
}

fn invert_image(image: &RgbImage) -> RgbImage {
    RgbImage::from_fn(image.width(), image.height(), |x, y| {
        let pixel = image.get_pixel(x, y);
        Rgb([255 - pixel[0], 255 - pixel[1], 255 - pixel[2]])
    })
}

/// Otsu 类间方差最大的灰度阈值。
fn otsu_threshold(histogram: &[u32; 256]) -> u8 {
    let total: u64 = histogram.iter().map(|count| u64::from(*count)).sum();
    if total == 0 {
        return 0;
    }
    let sum: u64 = histogram
        .iter()
        .enumerate()
        .map(|(value, count)| value as u64 * u64::from(*count))
        .sum();
    let mut sum_background = 0u64;
    let mut weight_background = 0u64;
    let mut best = 0.0f64;
    let mut threshold = 0u8;
    for (value, count) in histogram.iter().enumerate() {
        weight_background += u64::from(*count);
        if weight_background == 0 {
            continue;
        }
        let weight_foreground = total - weight_background;
        if weight_foreground == 0 {
            break;
        }
        sum_background += value as u64 * u64::from(*count);
        let mean_background = sum_background as f64 / weight_background as f64;
        let mean_foreground = (sum - sum_background) as f64 / weight_foreground as f64;
        let between = weight_background as f64
            * weight_foreground as f64
            * (mean_background - mean_foreground).powi(2);
        if between > best {
            best = between;
            threshold = value as u8;
        }
    }
    threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    const YELLOW: [u8; 3] = [240, 210, 120];

    #[test]
    fn parses_single_steps_and_combinations() {
        assert_eq!(Pipeline::from_config("off"), Pipeline::default());
        assert_eq!(
            Pipeline::from_config("color,gray,binary"),
            Pipeline {
                color_key: true,
                grayscale: true,
                binarize: true,
                invert: false,
            }
        );
        assert_eq!(
            Pipeline::from_config("color,binary,invert").config_value(),
            "color,binary,invert".to_string()
        );
        // 旧配置里的单值仍然可用。
        assert_eq!(
            Pipeline::from_config("binary").config_value(),
            "binary".to_string()
        );
        assert_eq!(Pipeline::default().config_value(), "off");
        assert_eq!(
            Pipeline::from_config("gray,binary").summary(),
            "灰度 → 二值化".to_string()
        );
    }

    #[test]
    fn grayscale_matches_luma_weights() {
        let image = RgbImage::from_pixel(1, 1, Rgb([255, 0, 0]));
        let output = apply(
            image,
            Pipeline {
                grayscale: true,
                ..Default::default()
            },
            [0; 3],
            0,
        );
        assert_eq!(output.get_pixel(0, 0), &Rgb([76, 76, 76]));
    }

    #[test]
    fn color_key_turns_matching_text_black() {
        let mut image = RgbImage::from_pixel(3, 1, Rgb([20, 40, 60]));
        image.put_pixel(1, 0, Rgb(YELLOW));
        let output = apply(
            image,
            Pipeline {
                color_key: true,
                ..Default::default()
            },
            YELLOW,
            12,
        );
        assert_eq!(output.get_pixel(0, 0), &Rgb([255, 255, 255]));
        assert_eq!(
            output.get_pixel(1, 0),
            &Rgb([0, 0, 0]),
            "匹配像素应直接变黑"
        );
        assert_eq!(output.get_pixel(2, 0), &Rgb([255, 255, 255]));
    }

    #[test]
    fn white_text_on_gray_survives_color_key_and_binarize() {
        // 白色文字 + 灰色背景：颜色提取后必须是黑字白底，不能再是“白字白底”。
        let mut image = RgbImage::from_pixel(16, 6, Rgb([120, 120, 120]));
        for x in 3..13 {
            image.put_pixel(x, 2, Rgb([250, 250, 250]));
            image.put_pixel(x, 3, Rgb([250, 250, 250]));
        }
        let pipeline = Pipeline {
            color_key: true,
            binarize: true,
            ..Default::default()
        };
        let output = apply(image, pipeline, [250, 250, 250], 18);
        assert_eq!(output.get_pixel(5, 2), &Rgb([0, 0, 0]), "文字应为黑色");
        assert_eq!(
            output.get_pixel(0, 0),
            &Rgb([255, 255, 255]),
            "背景应为白色"
        );
    }

    #[test]
    fn invert_flips_the_final_result() {
        let mut image = RgbImage::from_pixel(8, 4, Rgb([120, 120, 120]));
        for x in 2..6 {
            image.put_pixel(x, 1, Rgb([250, 250, 250]));
        }
        let black_text = apply(
            image.clone(),
            Pipeline {
                binarize: true,
                ..Default::default()
            },
            [0; 3],
            0,
        );
        let inverted = apply(
            image,
            Pipeline {
                binarize: true,
                invert: true,
                ..Default::default()
            },
            [0; 3],
            0,
        );
        assert_eq!(black_text.get_pixel(3, 1), &Rgb([0, 0, 0]));
        assert_eq!(inverted.get_pixel(3, 1), &Rgb([255, 255, 255]));
        assert_eq!(inverted.get_pixel(0, 0), &Rgb([0, 0, 0]));
    }

    #[test]
    fn color_key_then_binarize_gives_black_text_on_white() {
        let mut image = RgbImage::from_pixel(8, 2, Rgb([20, 40, 60]));
        for x in 2..6 {
            image.put_pixel(x, 1, Rgb(YELLOW));
        }
        let output = apply(
            image,
            Pipeline {
                color_key: true,
                binarize: true,
                ..Default::default()
            },
            YELLOW,
            12,
        );
        assert_eq!(output.get_pixel(3, 1), &Rgb([0, 0, 0]));
        assert_eq!(output.get_pixel(0, 0), &Rgb([255, 255, 255]));
    }

    #[test]
    fn binarize_turns_minority_pixels_into_black_text() {
        // 深色背景 + 少量亮色文字：输出应为白底黑字。
        let mut image = RgbImage::from_pixel(4, 4, Rgb([10, 10, 10]));
        image.put_pixel(1, 1, Rgb([235, 235, 235]));
        image.put_pixel(2, 1, Rgb([235, 235, 235]));
        let output = apply(
            image,
            Pipeline {
                binarize: true,
                ..Default::default()
            },
            [0; 3],
            0,
        );
        assert_eq!(output.get_pixel(1, 1), &Rgb([0, 0, 0]));
        assert_eq!(output.get_pixel(0, 0), &Rgb([255, 255, 255]));

        // 反过来：浅色背景 + 少量深色文字，同样输出白底黑字。
        let mut image = RgbImage::from_pixel(4, 4, Rgb([240, 240, 240]));
        image.put_pixel(1, 1, Rgb([30, 30, 30]));
        let output = apply(
            image,
            Pipeline {
                binarize: true,
                ..Default::default()
            },
            [0; 3],
            0,
        );
        assert_eq!(output.get_pixel(1, 1), &Rgb([0, 0, 0]));
        assert_eq!(output.get_pixel(0, 0), &Rgb([255, 255, 255]));
    }

    #[test]
    fn upscale_changes_dimensions() {
        let image = RgbImage::from_pixel(10, 4, Rgb([1, 2, 3]));
        assert_eq!(upscale(&image, 1).dimensions(), (10, 4));
        assert_eq!(upscale(&image, 2).dimensions(), (20, 8));
        assert_eq!(upscale(&image, 3).dimensions(), (30, 12));
        // 超出范围时钳制到 3 倍。
        assert_eq!(upscale(&image, 9).dimensions(), (30, 12));
    }

    #[test]
    fn off_pipeline_keeps_pixels() {
        let image = RgbImage::from_pixel(2, 2, Rgb([1, 2, 3]));
        let output = apply(image, Pipeline::default(), [0; 3], 0);
        assert_eq!(output.get_pixel(0, 0), &Rgb([1, 2, 3]));
    }

    #[test]
    fn binarize_ignores_background_color_changes() {
        // 同样文字、不同背景亮度：二值化后完全一致，避免背景变化反复触发 OCR。
        let mut dark = RgbImage::from_pixel(40, 20, Rgb([18, 18, 18]));
        let mut warm = RgbImage::from_pixel(40, 20, Rgb([42, 34, 28]));
        for x in 7..17 {
            for y in 6..16 {
                dark.put_pixel(x, y, Rgb([232, 232, 232]));
                warm.put_pixel(x, y, Rgb([232, 232, 232]));
            }
        }
        let pipeline = Pipeline {
            binarize: true,
            ..Default::default()
        };
        let a = apply(dark, pipeline, [0; 3], 0);
        let b = apply(warm, pipeline, [0; 3], 0);
        assert_eq!(a.as_raw(), b.as_raw());
    }
}
