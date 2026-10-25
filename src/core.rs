//! 任务坐标解析与地图几何计算。移植自 Python 版 `mhxy_assistant/core.py`，
//! 只保留「抓鬼 / 鬼王」两种任务。
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const TASK_TYPES: &[&str] = &["抓鬼", "鬼王", "通用"];

/// 通用任务：不看任务类型，只按「地图名 + 坐标」定位。
pub const GENERIC_TASK: &str = "通用";

/// 解析时允许出现的任务类型。
#[derive(Clone, Copy, Debug)]
pub struct TaskFilter {
    pub zhuagui: bool,
    pub ghost_king: bool,
    pub generic: bool,
}

impl Default for TaskFilter {
    /// 只解析抓鬼（历史默认行为）。
    fn default() -> Self {
        Self {
            zhuagui: true,
            ghost_king: false,
            generic: false,
        }
    }
}

impl TaskFilter {
    /// 抓鬼 + 鬼王（按关键字解析）。
    #[cfg(test)]
    pub const KEYWORD_TASKS: TaskFilter = TaskFilter {
        zhuagui: true,
        ghost_king: true,
        generic: false,
    };

    /// 只启用通用任务。
    #[cfg(test)]
    pub const GENERIC_ONLY: TaskFilter = TaskFilter {
        zhuagui: false,
        ghost_king: false,
        generic: true,
    };

    pub fn enabled(&self, task_type: &str) -> bool {
        match task_type {
            "抓鬼" => self.zhuagui,
            "鬼王" => self.ghost_king,
            GENERIC_TASK => self.generic,
            _ => false,
        }
    }
}

/// 坐标最多允许超出地图范围的比例，超出后仍贴边绘制。
pub const MAP_COORDINATE_TOLERANCE: f32 = 0.1;

/// 捉鬼省眼的高概率区域：50×50 格子。
pub const HIGH_CHANCE_SIZE: f32 = 50.0;
/// 这些地图无论象限都额外画左上角格子。
pub const HIGH_CHANCE_SHARED_MAPS: &[&str] = &["普陀山", "五庄观"];

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
        // 通用任务不看任务类型，任何地图都能用。
        if task_type == GENERIC_TASK {
            return true;
        }
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
    // OCR 经常把坐标的逗号认成句号、冒号或分号，都当作分隔符。
    matches!(
        ch,
        ',' | '，' | '、' | '.' | '．' | '。' | ':' | '：' | ';' | '；' | '·'
    )
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
        // OCR 可能把坐标的第二个数字换行拆开（例如 "79,2" + "6"）。
        // 如果后面隔了空格还有数字，就接上去（最多 3 位）。
        let mut probe = index;
        while second.len() < 3 {
            let mut next = probe;
            while next < chars.len() && chars[next] == ' ' {
                next += 1;
            }
            if next > probe && next < chars.len() && chars[next].is_ascii_digit() {
                second.push(chars[next]);
                probe = next + 1;
            } else {
                break;
            }
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

/// 任务关键字 → 归一化后的任务类型。长的写在前面，同一位置优先匹配更长的关键字。
/// 通用任务不参与关键字匹配，它只看地图名和坐标。
///
/// 最后一个字段表示是否要求关键字前面不是汉字：「鬼王」需要，否则「嗜血鬼王」
/// 这类描述会被误判成鬼王任务；「抓鬼 / 捉鬼」不要求，因为任务栏里常写成
/// 「任务抓鬼」「钟馗抓鬼」，加上前置断言会让它们全部失配。
///
/// 鬼王任务的任务名是「降伏千年厉鬼」，文字里「鬼王」只在坐标之后的
/// 「降伏 xxxx鬼王」里出现，前面一定是汉字，所以额外用「降伏」当鬼王关键字。
const TASK_KEYWORDS: &[(&str, &[char], bool)] = &[
    ("抓鬼", &['抓', '鬼'], false),
    ("抓鬼", &['捉', '鬼'], false),
    ("鬼王", &['降', '伏'], false),
    ("鬼王", &['鬼', '王'], true),
];

pub fn parse_tasks(text: &str, maps: &[MapDefinition], filter: TaskFilter) -> Vec<TaskLocation> {
    let normalized = collapse_whitespace(text);
    let chars: Vec<char> = normalized.chars().collect();

    // 记录每个任务关键字出现的位置（等价于 Python 的非重叠匹配 + 前置非汉字断言）。
    let mut matches: Vec<(usize, usize, &str)> = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let hit = TASK_KEYWORDS.iter().find_map(|(task, prefix, boundary)| {
            chars[index..]
                .starts_with(prefix)
                .then_some((*task, prefix.len(), *boundary))
        });
        match hit {
            Some((task, len, boundary)) if !boundary || index == 0 || !is_cjk(chars[index - 1]) => {
                matches.push((index, index + len, task));
                index += len;
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

    // 通用任务不看任务类型：直接拿整段文字找「地图名 + 坐标」。
    if filter.generic {
        if let Some(location) = locate_in_text(&normalized, maps, GENERIC_TASK) {
            tasks.push(location);
        }
    }

    for (position, (start, end, task_type)) in matches.iter().enumerate() {
        if !filter.enabled(task_type) || tasks.iter().any(|task| task.task_type == *task_type) {
            continue;
        }
        let block_end = matches.get(position + 1).map_or(chars.len(), |next| next.0);
        let block = &normalized[byte_of_char[*end]..byte_of_char[block_end]];
        if let Some(location) = locate_in_text(block, maps, task_type) {
            tasks.push(location);
            continue;
        }
        // OCR 阅读顺序错乱时，地图和坐标可能被排到关键字前面（如「长寿村 123,45 抓鬼」）。
        // 只在关键字后面的文字里完全没有坐标时，才回看前面一小段。
        if find_coordinate(block).is_some() {
            continue;
        }
        let from = byte_of_char[start.saturating_sub(40)];
        let before = &normalized[from..byte_of_char[*start]];
        if let Some(location) = locate_in_text(before, maps, task_type) {
            tasks.push(location);
        }
    }

    tasks
}

/// 二次校验：优先解析「去 … 附近 / 去 … 处」中间的连续文字。
/// 游戏里这段（地图名 + 坐标）本来就是一个整体，OCR 的换行/空格都是噪声，
/// 去掉空白后再取坐标，可以避免把 "79,26" 拆成 "79,2" + "6"。
fn locate_in_text(block: &str, maps: &[MapDefinition], task_type: &str) -> Option<TaskLocation> {
    let nearby = compact(block).contains("附近");
    let mut location =
        travel_segment(block).and_then(|segment| locate_at(&segment, maps, task_type));
    if location.is_none() {
        location = locate_at(block, maps, task_type);
    }
    if let Some(location) = &mut location {
        location.nearby = nearby;
    }
    location
}

/// 取「去 / 前往」到「附近 / 处」之间的文字，并去掉所有空白。
fn travel_segment(block: &str) -> Option<String> {
    let chars: Vec<char> = block.chars().collect();
    let mut start = None;
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '去' {
            start = Some(index + 1);
            break;
        }
        if chars[index] == '前' && index + 1 < chars.len() && chars[index + 1] == '往' {
            start = Some(index + 2);
            break;
        }
        index += 1;
    }
    let start = start?;
    let mut end = start;
    while end < chars.len() {
        let is_nearby = chars[end] == '附' && end + 1 < chars.len() && chars[end + 1] == '近';
        // 「处」必须紧跟在数字后面，避免把「处理」之类的词当成任务定位词。
        let is_place = chars[end] == '处' && prev_char_is_digit(&chars, end);
        if is_nearby || is_place {
            let segment: String = chars[start..end]
                .iter()
                .filter(|ch| !ch.is_whitespace())
                .collect();
            return (!segment.is_empty()).then_some(segment);
        }
        end += 1;
    }
    None
}

fn prev_char_is_digit(chars: &[char], index: usize) -> bool {
    let mut cursor = index;
    while cursor > 0 {
        cursor -= 1;
        if !chars[cursor].is_whitespace() {
            return chars[cursor].is_ascii_digit();
        }
    }
    false
}

/// 在一段（已归一化的）文字里找「地图名 + 坐标」。
fn locate_at(block: &str, maps: &[MapDefinition], task_type: &str) -> Option<TaskLocation> {
    let (coordinate_start, x, y) = find_coordinate(block)?;
    let compact_block = compact(block);
    let candidates: Vec<&MapDefinition> = maps
        .iter()
        .filter(|map| map.supports_task(task_type))
        .collect();
    // 1) 整段里包含某个别名（地图名与坐标之间可能夹着 OCR 噪声）。
    let mut definition = candidates.iter().copied().find(|map| {
        map.aliases.iter().any(|alias| {
            let alias = compact(alias);
            !alias.is_empty() && compact_block.contains(&alias)
        })
    });
    // 2) 别名没命中时，用坐标附近的连续中文段做模糊匹配：
    //    OCR 把「长寿村」认成「长寿材」时仍能对回地图；
    //    地图名出现在坐标后面（阅读顺序错乱）也能处理。
    if definition.is_none() {
        definition = fuzzy_map_in_text(block, &candidates);
    }
    let inferred_map = infer_map_name(&compact(&block[..coordinate_start]));
    // 3) 最后按「去XXX / 前往XXX」推断。
    if definition.is_none() {
        if let Some(inferred) = &inferred_map {
            definition = match_map(inferred, candidates.iter().copied());
        }
    }
    let map_name = definition
        .map(|map| map.name.clone())
        .or_else(|| inferred_map.clone())?;
    Some(TaskLocation {
        task_type: task_type.to_string(),
        map_name,
        x,
        y,
        nearby: compact_block.contains("附近"),
    })
}

/// 在整段文字上按「别名长度 ±1」滑动窗口，用编辑距离找出最像的地图名。
///
/// 比「截取连续汉字段再比对」宽松：OCR 把汉字认成字母（`长y村`）时，
/// 汉字段会被字母切断；2 字地图名错一个字（`夭宫` / `天官`）相似度只有 0.5，
/// 需要单独放低阈值。窗口里出现数字就跳过，避免把坐标和「第1个」这类计数带进来。
fn fuzzy_map_in_text<'a>(
    text: &str,
    candidates: &[&'a MapDefinition],
) -> Option<&'a MapDefinition> {
    let chars: Vec<char> = text.chars().collect();
    let mut best: Option<(&MapDefinition, f32)> = None;
    for definition in candidates {
        for alias in &definition.aliases {
            let alias: Vec<char> = compact(alias).chars().collect();
            if alias.len() < 2 {
                continue;
            }
            // 2 字地图名只错一个字时相似度是 0.5，阈值要跟着名字长度放低。
            let threshold = if alias.len() <= 2 { 0.5 } else { 0.6 };
            let lengths = [alias.len().saturating_sub(1), alias.len(), alias.len() + 1];
            for length in lengths {
                if length < 2 || length > chars.len() {
                    continue;
                }
                for start in 0..=chars.len() - length {
                    let window = &chars[start..start + length];
                    if window.iter().any(|ch| ch.is_ascii_digit()) {
                        continue;
                    }
                    // 至少要两个汉字，避免把零散的字母当成地图名。
                    if window.iter().filter(|ch| is_cjk(**ch)).count() < 2 {
                        continue;
                    }
                    // 没有任何相同字符的窗口不可能达到阈值，先跳过。
                    if !window.iter().any(|ch| alias.contains(ch)) {
                        continue;
                    }
                    let window_text: String = window.iter().collect();
                    let alias_text: String = alias.iter().collect();
                    let score = similarity(&window_text, &alias_text);
                    if score < threshold {
                        continue;
                    }
                    if best.is_none_or(|(_, best_score)| score > best_score) {
                        best = Some((definition, score));
                    }
                }
            }
        }
    }
    best.map(|(definition, _)| definition)
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

/// 捉鬼省眼的「高概率区域」，返回 50×50 格子的左下角坐标（供 `map_rect` 使用）。
///
/// 按任务坐标所在象限推算鬼可能出现的格子：坐标偏左下时取右上方，
/// 整体在左上角（或普陀山、五庄观这类小图）时取左上方，其余情况取右下方。
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
    // 上面算的是格子左上角（游戏 y 越大越靠上），换成左下角。
    corners
        .into_iter()
        .map(|(cx, cy)| (cx, cy - size))
        .collect()
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
            MapDefinition {
                name: "普陀山".into(),
                aliases: vec!["普陀山".into()],
                image: PathBuf::from("maps/普陀山.png"),
                left_x: 0.0,
                right_x: 95.0,
                top_y: 71.0,
                bottom_y: 0.0,
                tasks: vec!["抓鬼".into()],
            },
        ]
    }

    #[test]
    fn rejoins_coordinate_split_across_lines() {
        // 实际 OCR 结果：坐标 79,26 被换行拆成了 "79.2" 和 "6"。
        let text = "捉鬼\n：去普陀山79.2\n6附近抓已时四刻";
        let tasks = parse_tasks(text, &maps(), TaskFilter::default());
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task_type, "抓鬼");
        assert_eq!(tasks[0].map_name, "普陀山");
        assert_eq!((tasks[0].x, tasks[0].y), (79, 26));
        assert!(tasks[0].nearby);
    }

    #[test]
    fn secondary_check_handles_qu_chu_form() {
        // 鬼王常见写法：「去天宫56,78处」，中间连续；换行拆开也能拼回。
        let text = "鬼王\n：去天宫56.7\n8处";
        let filter = TaskFilter {
            zhuagui: false,
            ghost_king: true,
            generic: false,
        };
        let tasks = parse_tasks(text, &maps(), filter);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task_type, "鬼王");
        assert_eq!(tasks[0].map_name, "天宫");
        assert_eq!((tasks[0].x, tasks[0].y), (56, 78));
        assert!(!tasks[0].nearby);
    }

    #[test]
    fn parses_ghost_task() {
        let tasks = parse_tasks("抓鬼 长寿村 123,45", &maps(), TaskFilter::default());
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].task_type, "抓鬼");
        assert_eq!(tasks[0].map_name, "长寿村");
        assert_eq!((tasks[0].x, tasks[0].y), (123, 45));
    }

    #[test]
    fn parses_generic_task_by_map_and_coordinate_only() {
        let maps = maps();
        // 通用任务完全不看任务类型：没有任务名、写着别的任务名，都只按「地图名 + 坐标」定位。
        for text in ["长寿村 88,66", "降妖伏魔 长寿村 88 66", "抓鬼 长寿村 88,66"] {
            let tasks = parse_tasks(text, &maps, TaskFilter::GENERIC_ONLY);
            assert_eq!(tasks.len(), 1, "{text}");
            assert_eq!(tasks[0].task_type, GENERIC_TASK, "{text}");
            assert_eq!(tasks[0].map_name, "长寿村", "{text}");
            assert_eq!((tasks[0].x, tasks[0].y), (88, 66), "{text}");
        }
        // 没勾选通用任务时不解析。
        let tasks = parse_tasks("长寿村 88,66", &maps, TaskFilter::default());
        assert!(tasks.is_empty());
    }

    #[test]
    fn separates_ghost_and_king() {
        let text = "抓鬼 长寿村 12,34 鬼王 天宫 56 78";
        let tasks = parse_tasks(text, &maps(), TaskFilter::KEYWORD_TASKS);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].map_name, "长寿村");
        assert_eq!(tasks[1].map_name, "天宫");
        let without_king = parse_tasks(text, &maps(), TaskFilter::default());
        assert_eq!(without_king.len(), 1);
    }

    #[test]
    fn ignores_embedded_king() {
        let tasks = parse_tasks("嗜血鬼王 天宫 10,20", &maps(), TaskFilter::KEYWORD_TASKS);
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
    fn half_tolerance_accepts_cropped_map_coordinates() {
        let maps = maps();
        let putuo = maps.iter().find(|map| map.name == "普陀山").unwrap();
        // 任务文字里出现 57,94，但地图标定只到 y=71（截图可能只截了地图的一部分）。
        assert!(!coordinate_in_map(
            putuo,
            57.0,
            94.0,
            MAP_COORDINATE_TOLERANCE
        ));
        assert!(coordinate_in_map(putuo, 57.0, 94.0, 0.5));
    }

    #[test]
    fn high_chance_areas_follow_quadrants() {
        // 小坐标（x<150 && y<150）在所有地图上都多画一个左上方格子。
        // 北俱芦洲 73,74：右上方 + 左上方。
        assert_eq!(
            high_chance_areas("北俱芦洲", 73.0, 74.0, 50.0),
            vec![(23.0, 74.0), (23.0, 24.0)]
        );
        // 长寿村 32,61：右上方 + 左上方。
        assert_eq!(
            high_chance_areas("长寿村", 32.0, 61.0, 50.0),
            vec![(-18.0, 61.0), (-18.0, 11.0)]
        );
        // 大坐标只取左上方一格。
        assert_eq!(
            high_chance_areas("麒麟山", 175.0, 175.0, 50.0),
            vec![(175.0, 175.0)]
        );
        // 右下角只取自己那一格。
        assert_eq!(
            high_chance_areas("北俱芦洲", 200.0, 40.0, 50.0),
            vec![(200.0, -10.0)]
        );
        // 普陀山 / 五庄观 x==y 时补左上格。
        assert_eq!(
            high_chance_areas("普陀山", 30.0, 30.0, 50.0),
            vec![(-20.0, -20.0)]
        );
    }

    #[test]
    fn fuzzy_matches_misread_map_names() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/maps.json");
        let maps = load_maps(&path).expect("加载 maps.json");
        let filter = TaskFilter {
            zhuagui: true,
            ghost_king: true,
            generic: false,
        };
        let ghost = |name: &str| {
            format!("降伏千年厉鬼（第1个）：去{name}123,45附近降伏僵尸鬼王,剩余15分钟")
        };
        let catch = |name: &str| format!("钟馗抓鬼（第1个）：去{name}123,45附近抓僵尸。");
        // 抓鬼地图：形近字、认成字母、多认一个字都要能对回正式名。
        for (name, expected) in [
            ("长寿寸", "长寿村"),
            ("长y村", "长寿村"),
            ("长寿打", "长寿村"),
            ("普院山", "普陀山"),
            ("普陀由", "普陀山"),
            ("女儿材", "女儿村"),
            ("五庄现", "五庄观"),
            ("朱紫囯", "朱紫国"),
            ("建邺域", "建邺城"),
        ] {
            let tasks = parse_tasks(&catch(name), &maps, filter);
            assert_eq!(tasks.len(), 1, "{name}");
            assert_eq!(tasks[0].map_name, expected, "{name}");
        }
        // 鬼王地图：3 字别名错一个字，以及 2 字地图名（阈值更低）。
        for (name, expected) in [
            ("北俱芦州", "北俱芦洲"),
            ("北俱卢洲", "北俱芦洲"),
            ("碗子由", "碗子山"),
            ("麒麟由", "麒麟山"),
            ("夭宫", "天宫"),
            ("天官", "天宫"),
        ] {
            let tasks = parse_tasks(&ghost(name), &maps, filter);
            assert_eq!(tasks.len(), 1, "{name}");
            assert_eq!(tasks[0].map_name, expected, "{name}");
        }
    }

    #[test]
    fn parses_real_task_bar_text() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/maps.json");
        let maps = load_maps(&path).expect("加载 maps.json");
        let filter = TaskFilter {
            zhuagui: true,
            ghost_king: true,
            generic: false,
        };
        // 抓鬼：任务名是「钟馗抓鬼」，没开眼用「附近」，开眼用「处」。
        for (text, map_name, (x, y), nearby) in [
            (
                "钟馗抓鬼（第1个）：去长寿村123,45附近抓僵尸。",
                "长寿村",
                (123, 45),
                true,
            ),
            (
                "钟馗抓鬼（第10个）：去长寿村123,45处抓僵尸。",
                "长寿村",
                (123, 45),
                false,
            ),
            (
                "钟馗抓鬼（第1个）：去普陀山79,26附近抓鬼将。",
                "普陀山",
                (79, 26),
                true,
            ),
            (
                "钟馗抓鬼（第1个）：去朱紫国120,4附近抓恶鬼。",
                "朱紫国",
                (120, 4),
                true,
            ),
        ] {
            let tasks = parse_tasks(text, &maps, filter);
            assert_eq!(tasks.len(), 1, "{text}");
            assert_eq!(tasks[0].task_type, "抓鬼", "{text}");
            assert_eq!(tasks[0].map_name, map_name, "{text}");
            assert_eq!((tasks[0].x, tasks[0].y), (x, y), "{text}");
            assert_eq!(tasks[0].nearby, nearby, "{text}");
        }
        // 鬼王：任务名是「降伏千年厉鬼」，「鬼王」只在坐标之后；
        // 开眼时描述里既没有「附近」也没有「处」。
        for (text, map_name, (x, y), nearby) in [
            (
                "降伏千年厉鬼（第1个）：去天宫123,45附近降伏僵尸鬼王,剩余15分钟",
                "天宫",
                (123, 45),
                true,
            ),
            (
                "降伏千年厉鬼（第1个）：去天宫123,45降伏僵尸鬼王,剩余(15)分钟",
                "天宫",
                (123, 45),
                false,
            ),
            (
                "降伏千年厉鬼（第2个）：去北俱芦洲73,74附近降伏厉鬼鬼王,剩余9分钟",
                "北俱芦洲",
                (73, 74),
                true,
            ),
        ] {
            let tasks = parse_tasks(text, &maps, filter);
            assert_eq!(tasks.len(), 1, "{text}");
            assert_eq!(tasks[0].task_type, "鬼王", "{text}");
            assert_eq!(tasks[0].map_name, map_name, "{text}");
            assert_eq!((tasks[0].x, tasks[0].y), (x, y), "{text}");
            assert_eq!(tasks[0].nearby, nearby, "{text}");
        }
        // 两块识别区域拼接后同一段文字里既有抓鬼又有鬼王。
        let text = "钟馗抓鬼（第1个）：去长寿村123,45附近抓僵尸。\
                    降伏千年厉鬼（第1个）：去天宫123,45附近降伏僵尸鬼王,剩余15分钟";
        let tasks = parse_tasks(text, &maps, filter);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].task_type, "抓鬼");
        assert_eq!(tasks[0].map_name, "长寿村");
        assert_eq!(tasks[1].task_type, "鬼王");
        assert_eq!(tasks[1].map_name, "天宫");
    }

    #[test]
    fn parses_task_text_with_ocr_noise() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/maps.json");
        let maps = load_maps(&path).expect("加载 maps.json");
        let filter = TaskFilter {
            zhuagui: true,
            ghost_king: true,
            generic: false,
        };
        let cases = [
            // 任务栏里「抓鬼」前面粘着别的汉字：以前会匹配不到关键字。
            ("任务抓鬼长寿村123,45", "长寿村", (123, 45)),
            ("钟馗抓鬼 长寿村 123,45", "长寿村", (123, 45)),
            ("抓鬼长寿村 123,45", "长寿村", (123, 45)),
            // 地图名被 OCR 认错一个字，模糊匹配回正确地图。
            ("抓鬼\n长寿材 123,45", "长寿村", (123, 45)),
            ("捉鬼\n普院山 79,26", "普陀山", (79, 26)),
            // 坐标分隔符被认成冒号 / 分号 / 句号。
            ("抓鬼\n长寿村 123;45", "长寿村", (123, 45)),
            ("抓鬼\n长寿村 123。45", "长寿村", (123, 45)),
            ("抓鬼\n长寿村 123:45", "长寿村", (123, 45)),
            // 坐标行排在任务名之前（阅读顺序错乱）。
            ("长寿村 123,45 抓鬼", "长寿村", (123, 45)),
            // 任务栏里夹着「1/10」「第1个」这类计数，不能当成坐标。
            ("抓鬼 1/10 长寿村 123,45", "长寿村", (123, 45)),
            ("抓鬼\n第1个：长寿村 123,45", "长寿村", (123, 45)),
        ];
        for (text, map_name, (x, y)) in cases {
            let tasks = parse_tasks(text, &maps, filter);
            assert_eq!(tasks.len(), 1, "{text}");
            assert_eq!(tasks[0].task_type, "抓鬼", "{text}");
            assert_eq!(tasks[0].map_name, map_name, "{text}");
            assert_eq!((tasks[0].x, tasks[0].y), (x, y), "{text}");
        }
    }

    #[test]
    fn embedded_zhuagui_keyword_still_needs_king_boundary() {
        let text = "嗜血鬼王 天宫 10,20";
        let tasks = parse_tasks(text, &maps(), TaskFilter::KEYWORD_TASKS);
        assert!(tasks.is_empty());
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
