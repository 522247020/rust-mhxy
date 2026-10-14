//! 图色门控：用少量点位的颜色快速判断「当前有没有任务」。
//!
//! 战斗、任务完成时任务栏没有任务，点位不命中，就跳过 OCR，省 CPU。
use image::RgbImage;

#[derive(Clone, Debug, PartialEq)]
pub struct ColorPoint {
    /// 游戏客户区像素坐标（左上角 0,0），和截图同一坐标系。
    pub x: i32,
    pub y: i32,
    pub color: [u8; 3],
    /// 每通道允许的差值 0~255。
    pub tolerance: u8,
    /// 取以 (x,y) 为中心、半径 radius 的方块，块内任一像素命中即算命中。
    pub radius: u8,
    pub label: String,
}

impl ColorPoint {
    pub fn new(x: i32, y: i32, color: [u8; 3], tolerance: u8, radius: u8) -> Self {
        Self {
            x,
            y,
            color,
            tolerance,
            radius,
            label: String::new(),
        }
    }

    pub fn hex(&self) -> String {
        to_hex(self.color)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateMode {
    All,
    Any,
}

impl GateMode {
    pub fn from_str(value: &str) -> Self {
        match value {
            "any" => GateMode::Any,
            _ => GateMode::All,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct GateResult {
    /// 本次是否判定为「有任务」（允许 OCR）。
    pub active: bool,
    pub matched: Vec<bool>,
    /// 每个点位中心像素的实际颜色（越界为 None）。
    pub sampled: Vec<Option<[u8; 3]>>,
}

pub fn channel_match(sample: [u8; 3], expected: [u8; 3], tolerance: u8) -> bool {
    let tol = tolerance as i16;
    (sample[0] as i16 - expected[0] as i16).abs() <= tol
        && (sample[1] as i16 - expected[1] as i16).abs() <= tol
        && (sample[2] as i16 - expected[2] as i16).abs() <= tol
}

/// 所有点位的外接矩形（含 radius 余量），返回 (x, y, w, h)，单位为客户区像素。
pub fn bounding_box(points: &[ColorPoint]) -> Option<(i32, i32, i32, i32)> {
    if points.is_empty() {
        return None;
    }
    let mut min_x = i32::MAX;
    let mut min_y = i32::MAX;
    let mut max_x = i32::MIN;
    let mut max_y = i32::MIN;
    for point in points {
        let margin = point.radius as i32 + 1;
        min_x = min_x.min(point.x - margin);
        min_y = min_y.min(point.y - margin);
        max_x = max_x.max(point.x + margin);
        max_y = max_y.max(point.y + margin);
    }
    Some((min_x, min_y, (max_x - min_x + 1).max(1), (max_y - min_y + 1).max(1)))
}

/// 在截取的包围盒图上判断门控状态。
///
/// `origin` 是这张图左上角对应的客户区坐标。
pub fn evaluate(
    image: &RgbImage,
    origin: (i32, i32),
    points: &[ColorPoint],
    mode: GateMode,
) -> GateResult {
    if points.is_empty() {
        return GateResult {
            active: true,
            matched: Vec::new(),
            sampled: Vec::new(),
        };
    }
    let width = image.width() as i32;
    let height = image.height() as i32;
    let mut matched = Vec::with_capacity(points.len());
    let mut sampled = Vec::with_capacity(points.len());
    for point in points {
        let local_x = point.x - origin.0;
        let local_y = point.y - origin.1;
        let center = sample(image, local_x, local_y);
        sampled.push(center);
        let radius = point.radius as i32;
        let mut hit = false;
        'block: for dy in -radius..=radius {
            for dx in -radius..=radius {
                let px = local_x + dx;
                let py = local_y + dy;
                if px < 0 || py < 0 || px >= width || py >= height {
                    continue;
                }
                let pixel = image.get_pixel(px as u32, py as u32);
                if channel_match([pixel[0], pixel[1], pixel[2]], point.color, point.tolerance) {
                    hit = true;
                    break 'block;
                }
            }
        }
        matched.push(hit);
    }
    let active = match mode {
        GateMode::All => matched.iter().all(|hit| *hit),
        GateMode::Any => matched.iter().any(|hit| *hit),
    };
    GateResult {
        active,
        matched,
        sampled,
    }
}

fn sample(image: &RgbImage, x: i32, y: i32) -> Option<[u8; 3]> {
    if x < 0 || y < 0 || x >= image.width() as i32 || y >= image.height() as i32 {
        return None;
    }
    let pixel = image.get_pixel(x as u32, y as u32);
    Some([pixel[0], pixel[1], pixel[2]])
}

pub fn to_hex(color: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", color[0], color[1], color[2])
}

/// 解析 `#RRGGBB` / `RRGGBB`（也接受 `#RGB`）。
pub fn parse_hex(text: &str) -> Option<[u8; 3]> {
    let text = text.trim().trim_start_matches('#');
    if text.len() == 3 {
        let mut out = [0u8; 3];
        for (slot, ch) in out.iter_mut().zip(text.chars()) {
            let value = ch.to_digit(16)? as u8;
            *slot = value * 17;
        }
        return Some(out);
    }
    if text.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(text, 16).ok()?;
    Some([
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        assert_eq!(parse_hex("#d4a017"), Some([0xd4, 0xa0, 0x17]));
        assert_eq!(parse_hex("d4a017"), Some([0xd4, 0xa0, 0x17]));
        assert_eq!(to_hex([0xd4, 0xa0, 0x17]), "#d4a017");
    }

    #[test]
    fn matches_with_tolerance_and_block() {
        let mut image = RgbImage::from_pixel(4, 4, image::Rgb([0, 0, 0]));
        image.put_pixel(2, 2, image::Rgb([10, 20, 30]));
        let point = ColorPoint::new(2, 2, [12, 18, 33], 5, 0);
        let result = evaluate(&image, (0, 0), &[point.clone()], GateMode::All);
        assert!(result.active);
        assert_eq!(result.sampled[0], Some([10, 20, 30]));

        // 半径 1 时，即使中心被盖住，块内命中也算
        let mut image2 = RgbImage::from_pixel(4, 4, image::Rgb([0, 0, 0]));
        image2.put_pixel(1, 2, image::Rgb([200, 10, 10]));
        let point2 = ColorPoint::new(2, 2, [200, 10, 10], 5, 1);
        assert!(evaluate(&image2, (0, 0), &[point2], GateMode::All).active);

        let miss = ColorPoint::new(2, 2, [255, 255, 255], 5, 0);
        assert!(!evaluate(&image, (0, 0), &[miss], GateMode::All).active);
    }
}
