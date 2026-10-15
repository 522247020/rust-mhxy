//! 任务坐标解析与地图几何计算。移植自 Python 版 `mhxy_assistant/core.py`，
//! 只保留「抓鬼 / 鬼王」两种任务。
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const TASK_TYPES: &[&str] = &["抓鬼", "鬼王"];
/// 捉鬼省眼的高概率区域开关只对这两类任务有意义。
pub const HIGH_CHANCE_SHARED_MAPS: &[&str] = &["普陀山", "五庄观"];
/// 捉鬼省眼的 50×50 格子。
pub const HIGH_CHANCE_SIZE: f32 = 50.0;
/// 坐标最多允许超出地图范围的比例，超出后仍贴边绘制。
pub const MAP_COORDINATE_TOLERANCE: f32 = 0.1;

pub type TaskRegion = (f32, f32, f32, f32);
pub const DEFAULT_TASK_REGION: TaskRegion = (0.70, 0.24, 0.99, 0.70);
/// 个别任务会在第二处再显示一次坐标，备用区域放在任务栏下方。
pub const DEFAULT_EXTRA_REGION: TaskRegion = (0.70, 0.71, 0.99, 0.90);

#[derive(Clone, Debug)]
pub struct MapDefinition {
    pub name: String,
    pub aliases: Vec<String>,
    pub image: PathBuf,
    pub left_x: f32,
    pub right_x: f32,
    pub top_y: f32,
    pub bottom_y: f32,
    /// 空表示适用于所有任务；否则只在列出的任务中出现。
    pub tasks: Vec<String>,
}

impl MapDefinition {
    pub fn supports_task(&self, task_type: &str) -> bool {
        self.tasks.is_empty() || self.tasks.iter().any(|task| task == task_type)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TaskLocation {
    pub task_type: String,
    pub map_name: String,
    pub x: i32,
    pub y: i32,
    pub nearby: bool,
}

#[derive(Deserialize)]
struct RawMaps {
    maps: Vec<RawMap>,
}

#[derive(Deserialize)]
struct RawMap {
    name: String,
    #[serde(default)]
    aliases: Vec<String>,
    image: String,
    left_x: f32,
    right_x: f32,
    top_y: f32,
    bottom_y: f32,
    #[serde(default)]
    tasks: Vec<String>,
}

/// 读取 maps.json；地图图片路径按 JSON 所在目录解析。
pub fn load_maps(path: &Path) -> Result<Vec<MapDefinition>, String> {
    let text = std::fs::read_to_string(path).map_err(|err| format!("读取地图配置失败：{err}"))?;
    let payload: RawMaps =
        serde_json::from_str(&text).map_err(|err| format!("maps.json 格式错误：{err}"))?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut maps = Vec::with_capacity(payload.maps.len());
    for raw in payload.maps {
        let name = raw.name.trim().to_string();
        let mut aliases: Vec<String> = vec![name.clone()];
        for alias in raw.aliases {
            let alias = alias.trim().to_string();
            if !alias.is_empty() && !aliases.contains(&alias) {
                aliases.push(alias);
            }
        }
        let tasks: Vec<String> = raw
            .tasks
            .into_iter()
            .map(|task| task.trim().to_string())
            .filter(|task| !task.is_empty())
            .collect();
        if let Some(unknown) = tasks
            .iter()
            .find(|task| !TASK_TYPES.contains(&task.as_str()))
        {
            return Err(format!("地图 {name} 的任务类型无效：{unknown}"));
        }
        if raw.left_x == raw.right_x || raw.top_y == raw.bottom_y {
            return Err(format!("地图 {name} 的坐标范围无效"));
        }
        maps.push(MapDefinition {
            name,
            aliases,
            image: resolve_image(base, &raw.image),
            left_x: raw.left_x,
            right_x: raw.right_x,
            top_y: raw.top_y,
            bottom_y: raw.bottom_y,
            tasks,
        });
    }
    Ok(maps)
}

/// 读取图片并按内容嗅探格式（不依赖扩展名）。
/// 注意：仓库里的地图文件其实是 JPEG 但扩展名是 `.png`，按扩展名解码会失败。
pub fn load_rgb_image(path: &Path) -> Result<image::RgbImage, String> {
    let reader = image::ImageReader::open(path).map_err(|err| err.to_string())?;
    let reader = reader
        .with_guessed_format()
        .map_err(|err| err.to_string())?;
    let image = reader.decode().map_err(|err| err.to_string())?;
    Ok(image.to_rgb8())
}

/// 解析地图图片路径：先按 maps.json 里的相对路径找，找不到再在常见目录里找同名文件。
/// 这样 assets/ 或 exe 旁边的目录结构略有差异时也能找到图片。
fn resolve_image(base: &Path, image: &str) -> PathBuf {
    let raw = Path::new(image);
    let file = raw.file_name().map(PathBuf::from);
    let mut candidates = vec![base.join(raw)];
    if let Some(file) = &file {
        candidates.push(base.join("maps").join(file));
        candidates.push(base.join("assets").join("maps").join(file));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(raw));
            if let Some(file) = &file {
                candidates.push(dir.join("maps").join(file));
                candidates.push(dir.join("assets").join("maps").join(file));
            }
        }
    }
    candidates
        .into_iter()
        .find(|candidate| candidate.exists())
        .unwrap_or_else(|| base.join(raw))
}

fn compact(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_cjk(ch: char) -> bool {
    matches!(ch, '\u{4e00}'..='\u{9fff}')
}

fn is_separator(ch: char) -> bool {
    matches!(ch, ',' | '，' | '、' | '.')
}

/// 归一化后的编辑距离相似度，近似 Python difflib 的匹配比例。
fn similarity(a: &str, b: &str) -> f32 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let max_len = a.len().max(b.len());
    if max_len == 0 {
        return 1.0;
    }
    let distance = levenshtein(&a, &b);
    1.0 - distance as f32 / max_len as f32
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    let mut current = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        current[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current[j + 1] = (previous[j] + cost)
                .min(previous[j + 1] + 1)
                .min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b.len()]
}

fn match_map<'a>(
    name: &str,
    maps: impl Iterator<Item = &'a MapDefinition>,
) -> Option<&'a MapDefinition> {
    let normalized = compact(name);
    if normalized.is_empty() {
        return None;
    }
    let candidates: Vec<&MapDefinition> = maps.collect();
    for definition in &candidates {
        if definition
            .aliases
            .iter()
            .any(|alias| compact(alias) == normalized)
        {
            return Some(definition);
        }
    }
    let mut best: Option<(&MapDefinition, f32)> = None;
    for definition in &candidates {
        for alias in &definition.aliases {
            let alias = compact(alias);
            if alias.is_empty() {
                continue;
            }
            let score = similarity(&normalized, &alias);
            if score >= 0.6 && best.is_none_or(|(_, best_score)| score > best_score) {
                best = Some((definition, score));
            }
        }
    }
    best.map(|(definition, _)| definition)
}

pub fn find_map_definition<'a>(
    name: &str,
    maps: &'a [MapDefinition],
    task_type: Option<&str>,
) -> Option<&'a MapDefinition> {
    match_map(
        name,
        maps.iter().filter(|map| match task_type {
            Some(task) => map.supports_task(task),
            None => true,
        }),
    )
}

/// 在 ``block`` 中寻找第一个「数字 分隔符 数字」坐标，返回 (字节起点, x, y)。
fn find_coordinate(block: &str) -> Option<(usize, i32, i32)> {
    let chars: Vec<char> = block.chars().collect();
    // 记录每个 char 对应的字节位置，方便返回偏移。
    let mut byte_offsets = Vec::with_capacity(chars.len());
    let mut offset = 0usize;
    for ch in &chars {
        byte_offsets.push(offset);
        offset += ch.len_utf8();
    }
    for start in 0..chars.len() {
        if !chars[start].is_ascii_digit() {
            continue;
        }
        let mut index = start;
        let mut first = String::new();
        while index < chars.len() && chars[index].is_ascii_digit() && first.len() < 3 {
            first.push(chars[index]);
            index += 1;
        }
        let mut had_space = false;
        while index < chars.len() && chars[index] == ' ' {
            index += 1;
            had_space = true;
        }
        if index < chars.len() && is_separator(chars[index]) {
            index += 1;
            while index < chars.len() && chars[index] == ' ' {
                index += 1;
            }
        } else if !had_space {
            continue;
        }
        let mut second = String::new();
        while index < chars.len() && chars[index].is_ascii_digit() && second.len() < 3 {
            second.push(chars[index]);
            index += 1;
        }
        if second.is_empty() {
            continue;
        }
        if let (Ok(x), Ok(y)) = (first.parse::<i32>(), second.parse::<i32>()) {
            return Some((byte_offsets[start], x, y));
        }
    }
    None
}

/// 从「去XXX / 前往XXX」这样的描述里推断地图名。
fn infer_map_name(before: &str) -> Option<String> {
    let chars: Vec<char> = before.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let (keyword_len, rest) = if chars[index..].starts_with(&['前', '往']) {
            (2, index + 2)
        } else if chars[index] == '去' {
            (1, index + 1)
        } else {
            index += 1;
            continue;
        };
        let _ = keyword_len;
        // 取紧随其后的连续汉字，要求在 2~8 个之间，且之后不再出现汉字。
        let mut end = rest;
        while end < chars.len() && is_cjk(chars[end]) {
            end += 1;
        }
        let count = end - rest;
        let tail_has_cjk = chars[end..].iter().any(|ch| is_cjk(*ch));
        if (2..=8).contains(&count) && !tail_has_cjk {
            return Some(chars[rest..end].iter().collect());
        }
        index += 1;
    }
    None
}

/// 解析任务文字，返回识别到的抓鬼 / 鬼王坐标。
pub fn parse_tasks(
    text: &str,
    maps: &[MapDefinition],
    include_ghost_king: bool,
) -> Vec<TaskLocation> {
    let normalized = collapse_whitespace(text);
    let chars: Vec<char> = normalized.chars().collect();

    // 记录每个任务关键字出现的位置（等价于 Python 的非重叠匹配 + 前置非汉字断言）。
    let mut matches: Vec<(usize, usize, &str)> = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let keyword = [
            ("抓鬼", ['抓', '鬼']),
            ("捉鬼", ['捉', '鬼']),
            ("鬼王", ['鬼', '王']),
        ]
        .into_iter()
        .find_map(|(keyword, prefix)| chars[index..].starts_with(&prefix).then_some(keyword));
        match keyword {
            Some(keyword) if index == 0 || !is_cjk(chars[index - 1]) => {
                matches.push((index, index + keyword.chars().count(), keyword));
                index += keyword.chars().count();
            }
            _ => index += 1,
        }
    }

    // 记录 char 索引 -> 字节索引，便于切分 UTF-8 字符串。
    let mut byte_of_char = Vec::with_capacity(chars.len() + 1);
    let mut offset = 0usize;
    for ch in &chars {
        byte_of_char.push(offset);
        offset += ch.len_utf8();
    }
    byte_of_char.push(offset);

    let mut tasks: Vec<TaskLocation> = Vec::new();
    for (position, (start, end, keyword)) in matches.iter().enumerate() {
        let task_type = if *keyword == "捉鬼" {
            "抓鬼"
        } else {
            *keyword
        };
        if task_type == "鬼王" && !include_ghost_king {
            continue;
        }
        if tasks.iter().any(|task| task.task_type == task_type) {
            continue;
        }
        let block_end = matches.get(position + 1).map_or(chars.len(), |next| next.0);
        let _ = start;
        let block_bytes = &normalized[byte_of_char[*end]..byte_of_char[block_end]];
        let coordinate = find_coordinate(block_bytes);
        let compact_block = compact(block_bytes);
        let candidates: Vec<&MapDefinition> = maps
            .iter()
            .filter(|map| map.supports_task(task_type))
            .collect();
        let mut definition = candidates.iter().copied().find(|map| {
            map.aliases.iter().any(|alias| {
                let alias = compact(alias);
                !alias.is_empty() && compact_block.contains(&alias)
            })
        });
        let mut inferred_map: Option<String> = None;
        if let Some((coordinate_start, _, _)) = coordinate {
            let before = compact(&block_bytes[..coordinate_start]);
            inferred_map = infer_map_name(&before);
        }
        if definition.is_none() {
            if let Some(inferred) = &inferred_map {
                definition = match_map(inferred, candidates.iter().copied());
            }
        }
        let map_name = definition
            .map(|map| map.name.clone())
            .or_else(|| inferred_map.clone());
        if let (Some((_, x, y)), Some(map_name)) = (coordinate, map_name) {
            tasks.push(TaskLocation {
                task_type: task_type.to_string(),
                map_name,
                x,
                y,
                nearby: compact_block.contains("附近"),
            });
        }
    }
    tasks
}

pub fn coordinate_in_map(definition: &MapDefinition, x: f32, y: f32, tolerance: f32) -> bool {
    let (min_x, max_x) = if definition.left_x < definition.right_x {
        (definition.left_x, definition.right_x)
    } else {
        (definition.right_x, definition.left_x)
    };
    let (min_y, max_y) = if definition.top_y < definition.bottom_y {
        (definition.top_y, definition.bottom_y)
    } else {
        (definition.bottom_y, definition.top_y)
    };
    let x_margin = (max_x - min_x) * tolerance;
    let y_margin = (max_y - min_y) * tolerance;
    min_x - x_margin <= x && x <= max_x + x_margin && min_y - y_margin <= y && y <= max_y + y_margin
}

/// 把略微超出地图的坐标压回地图边缘，保证点位可见。
pub fn clamp_to_map(definition: &MapDefinition, x: f32, y: f32) -> (f32, f32) {
    let (min_x, max_x) = if definition.left_x < definition.right_x {
        (definition.left_x, definition.right_x)
    } else {
        (definition.right_x, definition.left_x)
    };
    let (min_y, max_y) = if definition.top_y < definition.bottom_y {
        (definition.top_y, definition.bottom_y)
    } else {
        (definition.bottom_y, definition.top_y)
    };
    (x.clamp(min_x, max_x), y.clamp(min_y, max_y))
}

/// 捉鬼省眼的高概率区域，返回 50×50 格子的左下角坐标。
pub fn high_chance_areas(map_name: &str, x: f32, y: f32, size: f32) -> Vec<(f32, f32)> {
    let mut corners: Vec<(f32, f32)> = Vec::new();
    if x < y {
        corners.push((x - size, y + size));
    }
    if x > 150.0 && y > 150.0 {
        corners.push((x, y + size));
    }
    if (x < 150.0 && y < 150.0) || HIGH_CHANCE_SHARED_MAPS.contains(&map_name) {
        corners.push((x - size, y));
    }
    if x > y || (x > 150.0 && y < 50.0) {
        corners.push((x, y));
    }
    // 上面算出来的是格子左上角（游戏 y 越大越靠上）；换成左下角绘制。
    corners
        .into_iter()
        .map(|(cx, cy)| (cx, cy - size))
        .collect()
}

pub fn map_point(
    definition: &MapDefinition,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> Option<(f32, f32)> {
    if !coordinate_in_map(definition, x, y, 0.0) {
        return None;
    }
    let x_ratio = (x - definition.left_x) / (definition.right_x - definition.left_x);
    let y_ratio = (y - definition.top_y) / (definition.bottom_y - definition.top_y);
    Some((x_ratio * width, y_ratio * height))
}

/// 像素坐标反算游戏坐标（鼠标悬停用）。
pub fn map_coordinate(
    definition: &MapDefinition,
    pixel_x: f32,
    pixel_y: f32,
    width: f32,
    height: f32,
) -> Option<(i32, i32)> {
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let x = definition.left_x + pixel_x / width * (definition.right_x - definition.left_x);
    let y = definition.top_y + pixel_y / height * (definition.bottom_y - definition.top_y);
    Some((x.round() as i32, y.round() as i32))
}

#[allow(clippy::too_many_arguments)]
pub fn map_rect(
    definition: &MapDefinition,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    width: f32,
    height: f32,
) -> (f32, f32, f32, f32) {
    let (min_x, max_x) = if definition.left_x < definition.right_x {
        (definition.left_x, definition.right_x)
    } else {
        (definition.right_x, definition.left_x)
    };
    let (min_y, max_y) = if definition.top_y < definition.bottom_y {
        (definition.top_y, definition.bottom_y)
    } else {
        (definition.bottom_y, definition.top_y)
    };
    let left = x1.min(x2).clamp(min_x, max_x);
    let right = x1.max(x2).clamp(min_x, max_x);
    let bottom = y1.min(y2).clamp(min_y, max_y);
    let top = y1.max(y2).clamp(min_y, max_y);
    let (px1, py1) = map_point(definition, left, bottom, width, height).unwrap_or((0.0, 0.0));
    let (px2, py2) = map_point(definition, right, top, width, height).unwrap_or((0.0, 0.0));
    (px1.min(px2), py1.min(py2), px1.max(px2), py1.max(py2))
}

/// 以 (x, y) 为中心、边长为 size 的正方形，超出地图的部分会被裁掉。
pub fn map_area(
    definition: &MapDefinition,
    x: f32,
    y: f32,
    size: f32,
    width: f32,
    height: f32,
) -> (f32, f32, f32, f32) {
    let half = size / 2.0;
    map_rect(
        definition,
        x - half,
        y - half,
        x + half,
        y + half,
        width,
        height,
    )
}

/// 把百分比区域换算成图片像素区域。
pub fn task_region(width: f32, height: f32, region: TaskRegion) -> (f32, f32, f32, f32) {
    (
        width * region.0,
        height * region.1,
        width * region.2,
        height * region.3,
    )
}

/// 解析 ``左,上,右,下`` 百分比字符串，非法时返回默认值。
pub fn parse_region_setting(saved: &str, default: TaskRegion) -> TaskRegion {
    let parts: Vec<&str> = saved.split(',').collect();
    if parts.len() != 4 {
        return default;
    }
    let mut values = [0f32; 4];
    for (slot, part) in values.iter_mut().zip(parts.iter()) {
        match part.trim().parse::<f32>() {
            Ok(value) if value.is_finite() => *slot = value,
            Ok(_) => return default,
            Err(_) => return default,
        }
    }
    let (left, top, right, bottom) = (values[0], values[1], values[2], values[3]);
    if !(0.0..1.0).contains(&left)
        || left >= right
        || right > 1.0
        || !(0.0..1.0).contains(&top)
        || top >= bottom
        || bottom > 1.0
    {
        return default;
    }
    (left, top, right, bottom)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maps() -> Vec<MapDefinition> {
        vec![
            MapDefinition {
                name: "长寿村".into(),
                aliases: vec!["长寿村".into()],
                image: PathBuf::from("maps/长寿村.png"),
                left_x: 0.0,
                right_x: 160.0,
                top_y: 210.0,
                bottom_y: 0.0,
                tasks: vec!["抓鬼".into()],
            },
            MapDefinition {
                name: "天宫".into(),
                aliases: vec!["天宫".into()],
                image: PathBuf::from("maps/天宫.png"),
                left_x: 0.0,
                right_x: 253.0,
                top_y: 170.0,
                bottom_y: 0.0,
                tasks: vec!["鬼王".into()],
            },
        ]
    }

    #[test]
    fn parses_ghost_task() {
        let tasks = parse_tasks("抓鬼 长寿村 123,45", &maps(), true);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task_type, "抓鬼");
        assert_eq!(tasks[0].map_name, "长寿村");
        assert_eq!((tasks[0].x, tasks[0].y), (123, 45));
    }

    #[test]
    fn separates_ghost_and_king() {
        let text = "抓鬼 长寿村 12,34 鬼王 天宫 56 78";
        let tasks = parse_tasks(text, &maps(), true);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].map_name, "长寿村");
        assert_eq!(tasks[1].map_name, "天宫");
        let without_king = parse_tasks(text, &maps(), false);
        assert_eq!(without_king.len(), 1);
    }

    #[test]
    fn ignores_embedded_king() {
        let tasks = parse_tasks("嗜血鬼王 天宫 10,20", &maps(), true);
        assert!(tasks.is_empty());
    }

    #[test]
    fn region_setting_defaults_on_bad_input() {
        for input in [
            "bad",
            "0.7,0.2,NaN,0.7",
            "0.7,0.2,0.99,inf",
            "0.7,0.8,0.6,0.9",
        ] {
            assert_eq!(
                parse_region_setting(input, DEFAULT_TASK_REGION),
                DEFAULT_TASK_REGION
            );
        }
        assert_eq!(
            parse_region_setting("0.7,0.2,0.99,0.7", DEFAULT_TASK_REGION),
            (0.7, 0.2, 0.99, 0.7)
        );
    }

    #[test]
    fn all_map_images_load() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/maps.json");
        let maps = load_maps(&path).expect("加载 maps.json");
        assert!(!maps.is_empty());
        for map in &maps {
            assert!(map.image.exists(), "缺少地图图片：{}", map.image.display());
            load_rgb_image(&map.image)
                .unwrap_or_else(|err| panic!("打开地图图片失败 {}：{err}", map.image.display()));
        }
    }
}
