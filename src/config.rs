//! 用户配置的读写。为了兼容 Python 版，支持 JSON 里的 ``//`` 与 ``/* */`` 注释，
//! 并且对布尔值的字符串写法（"true" / "false"）做容错。
use crate::core::{parse_region_setting, TaskRegion, DEFAULT_EXTRA_REGION, DEFAULT_TASK_REGION};
use crate::gate::{parse_hex, to_hex, ColorPoint};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub const CONFIG_FILE_NAME: &str = "config.json";
pub const CONFIG_PATH_ENV: &str = "MHXY_CONFIG_PATH";
/// 预处理默认文字颜色：RGB u8 (234, 255, 101)，即 #eaff65。
pub const DEFAULT_PREPROCESS_COLOR: [u8; 3] = [234, 255, 101];

/// 可保存的识别方案：图色门控 + 预处理设置，方便一键切换。
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub name: String,
    pub gate_enabled: bool,
    pub gate_interval_ms: u32,
    pub gate_mode: String,
    pub gate_tolerance: u8,
    pub gate_points: Vec<ColorPoint>,
    pub preprocess_mode: String,
    pub preprocess_color: [u8; 3],
    pub preprocess_tolerance: u8,
    pub extra_preprocess_mode: String,
    pub extra_preprocess_color: [u8; 3],
    pub extra_preprocess_tolerance: u8,
}

impl Profile {
    /// 用当前配置生成一个方案（名称可指定）。
    pub fn from_config(config: &Config, name: String) -> Self {
        Self {
            name,
            gate_enabled: config.color_gate_enabled,
            gate_interval_ms: config.color_gate_interval_ms,
            gate_mode: config.color_gate_mode.clone(),
            gate_tolerance: config.color_gate_tolerance,
            gate_points: config.color_gate_points.clone(),
            preprocess_mode: config.preprocess_mode.clone(),
            preprocess_color: config.preprocess_color,
            preprocess_tolerance: config.preprocess_tolerance,
            extra_preprocess_mode: config.extra_preprocess_mode.clone(),
            extra_preprocess_color: config.extra_preprocess_color,
            extra_preprocess_tolerance: config.extra_preprocess_tolerance,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub ocr_mode: String,
    pub interval_seconds: u32,
    pub task_region: TaskRegion,
    pub extra_task_region: TaskRegion,
    pub extra_region_enabled: bool,
    pub api_format: String,
    pub api_url: String,
    pub api_token: String,
    pub trigger_mode: String,
    pub ocr_hotkey: String,
    pub zhuagui_enabled: bool,
    pub ghost_king_enabled: bool,
    pub generic_enabled: bool,
    pub high_chance: bool,
    pub ghost_king_high_chance: bool,
    /// 大字体模式：OCR 把多位坐标截断导致 y 轴抖动时开启。
    pub large_font_mode: bool,
    pub horizontal_layout: bool,
    pub sidebar_expanded: bool,
    pub log_level: String,
    pub color_gate_enabled: bool,
    pub color_gate_interval_ms: u32,
    pub color_gate_mode: String,
    pub color_gate_tolerance: u8,
    pub color_gate_points: Vec<ColorPoint>,
    /// OCR 前预处理：off / gray / binary / color。
    pub preprocess_mode: String,
    /// 按颜色提取时保留的文字颜色。
    pub preprocess_color: [u8; 3],
    /// 按颜色提取时的颜色容差。
    pub preprocess_tolerance: u8,
    /// 备用区域独立的预处理设置。
    pub extra_preprocess_mode: String,
    pub extra_preprocess_color: [u8; 3],
    pub extra_preprocess_tolerance: u8,
    /// 界面主题：system（跟随系统）/ light / dark。
    pub theme: String,
    /// 侧栏各分组折叠状态，逗号分隔的 id（如 "window,task"）。
    pub sidebar_folded: String,
    /// 启动时检查更新。
    pub update_enabled: bool,
    /// 更新清单地址（自建站点）。
    pub update_manifest_url: String,
    /// 保存的识别方案与当前方案名。
    pub profiles: Vec<Profile>,
    pub active_profile: String,
    /// 识别区域窗口各分组的折叠状态。
    pub region_scope_open: bool,
    pub region_gate_open: bool,
    pub region_preprocess_open: bool,
    pub on_top: bool,
}

impl Default for Config {
    fn default() -> Self {
        let mut config = Self {
            ocr_mode: "local".into(),
            interval_seconds: 5,
            task_region: DEFAULT_TASK_REGION,
            extra_task_region: DEFAULT_EXTRA_REGION,
            extra_region_enabled: false,
            api_format: "multipart".into(),
            api_url: String::new(),
            api_token: String::new(),
            trigger_mode: "auto".into(),
            ocr_hotkey: "F8".into(),
            zhuagui_enabled: true,
            ghost_king_enabled: true,
            generic_enabled: false,
            high_chance: false,
            ghost_king_high_chance: false,
            large_font_mode: false,
            horizontal_layout: false,
            sidebar_expanded: true,
            log_level: "simple".into(),
            color_gate_enabled: false,
            color_gate_interval_ms: 300,
            color_gate_mode: "all".into(),
            color_gate_tolerance: 12,
            color_gate_points: Vec::new(),
            preprocess_mode: "off".into(),
            preprocess_color: DEFAULT_PREPROCESS_COLOR,
            preprocess_tolerance: 24,
            extra_preprocess_mode: "off".into(),
            extra_preprocess_color: DEFAULT_PREPROCESS_COLOR,
            extra_preprocess_tolerance: 24,
            theme: "system".into(),
            sidebar_folded: String::new(),
            update_enabled: true,
            update_manifest_url: crate::update::DEFAULT_MANIFEST_URL.to_string(),
            region_scope_open: true,
            region_gate_open: true,
            region_preprocess_open: true,
            on_top: true,
            profiles: Vec::new(),
            active_profile: String::new(),
        };
        let profile = Profile::from_config(&config, "默认".to_string());
        config.profiles = vec![profile];
        config.active_profile = "默认".to_string();
        config
    }
}

impl Config {
    /// 用当前设置生成方案（沿用当前方案名）。
    pub fn profile_from_current(&self) -> Profile {
        Profile::from_config(self, self.active_profile.clone())
    }

    pub fn load(path: &Path) -> Config {
        let mut config = Config::default();
        let Ok(raw) = std::fs::read_to_string(path) else {
            return config;
        };
        let Ok(value) = serde_json::from_str::<Value>(&strip_comments(&raw)) else {
            return config;
        };
        let Some(map) = value.as_object() else {
            return config;
        };
        config.ocr_mode = match get_string(map, "ocr/mode", &config.ocr_mode).as_str() {
            "remote" => "remote".into(),
            _ => "local".into(),
        };
        config.interval_seconds =
            get_u32(map, "ocr/interval_seconds", config.interval_seconds).clamp(1, 60);
        config.task_region =
            parse_region_setting(&get_string(map, "ocr/task_region", ""), DEFAULT_TASK_REGION);
        config.extra_task_region = parse_region_setting(
            &get_string(map, "ocr/extra_task_region", ""),
            DEFAULT_EXTRA_REGION,
        );
        config.extra_region_enabled = get_bool(map, "ocr/extra_region_enabled", false);
        config.api_format = match get_string(map, "api/format", &config.api_format).as_str() {
            "raw" => "raw".into(),
            _ => "multipart".into(),
        };
        config.api_url = get_string(map, "api/url", "");
        config.api_token = get_string(map, "api/token", "");
        config.trigger_mode = match get_string(map, "ocr/trigger", &config.trigger_mode).as_str() {
            "manual" => "manual".into(),
            _ => "auto".into(),
        };
        config.ocr_hotkey = get_string(map, "ocr/hotkey", "F8");
        config.zhuagui_enabled = get_bool(map, "task/option/抓鬼", true);
        config.ghost_king_enabled = get_bool(map, "task/option/鬼王", true);
        // 通用任务原来的键是 task/option/降妖伏魔，读旧配置时兼容一下。
        config.generic_enabled = get_bool(
            map,
            "task/option/通用",
            get_bool(map, "task/option/降妖伏魔", false),
        );
        config.high_chance = get_bool(map, "task/high_chance", false);
        // 旧配置只有 task/high_chance，鬼王默认跟随抓鬼的设置。
        config.ghost_king_high_chance = get_bool(map, "task/high_chance/鬼王", config.high_chance);
        config.large_font_mode = get_bool(map, "task/large_font_mode", false);
        config.horizontal_layout = get_bool(map, "task/horizontal_layout", false);
        config.sidebar_expanded = get_bool(map, "window/sidebar_expanded", true);
        config.log_level = match get_string(map, "log/level", &config.log_level).as_str() {
            "detailed" => "detailed".into(),
            _ => "simple".into(),
        };
        config.color_gate_enabled = get_bool(map, "color_gate/enabled", false);
        config.color_gate_interval_ms = get_u32(map, "color_gate/interval_ms", 300).clamp(50, 5000);
        config.color_gate_mode = match get_string(map, "color_gate/mode", "all").as_str() {
            "any" => "any".into(),
            _ => "all".into(),
        };
        config.color_gate_tolerance = map
            .get("color_gate/tolerance")
            .and_then(Value::as_u64)
            .map(|value| value.min(255) as u8)
            .unwrap_or(12);
        config.color_gate_points = get_points(map, "color_gate/points");
        config.preprocess_mode = match get_string(map, "ocr/preprocess", "off").as_str() {
            "gray" | "grayscale" => "gray".into(),
            "binary" | "binarize" => "binary".into(),
            "color" => "color".into(),
            _ => "off".into(),
        };
        config.preprocess_color = parse_hex(&get_string(map, "ocr/preprocess_color", "#eaff65"))
            .unwrap_or(DEFAULT_PREPROCESS_COLOR);
        config.preprocess_tolerance = map
            .get("ocr/preprocess_tolerance")
            .and_then(Value::as_u64)
            .map(|value| value.min(255) as u8)
            .unwrap_or(24);
        config.extra_preprocess_mode =
            match get_string(map, "ocr/extra_preprocess", &config.preprocess_mode).as_str() {
                "gray" | "grayscale" => "gray".into(),
                "binary" | "binarize" => "binary".into(),
                "color" => "color".into(),
                "off" => "off".into(),
                other => other.to_string(),
            };
        config.extra_preprocess_color = parse_hex(&get_string(
            map,
            "ocr/extra_preprocess_color",
            &to_hex(config.preprocess_color),
        ))
        .unwrap_or(config.preprocess_color);
        config.extra_preprocess_tolerance = map
            .get("ocr/extra_preprocess_tolerance")
            .and_then(Value::as_u64)
            .map(|value| value.min(255) as u8)
            .unwrap_or(config.preprocess_tolerance);
        // 旧配置只有 light / dark，没有配置时跟随系统主题。
        config.theme = match get_string(map, "window/theme", "system").as_str() {
            "light" => "light".to_string(),
            "dark" => "dark".to_string(),
            _ => "system".to_string(),
        };
        config.sidebar_folded = get_string(map, "window/sidebar/folded", "");
        config.update_enabled = get_bool(map, "update/enabled", true);
        config.update_manifest_url = get_string(
            map,
            "update/manifest_url",
            crate::update::DEFAULT_MANIFEST_URL,
        );
        config.profiles = map
            .get("profiles/items")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(profile_from_value).collect())
            .unwrap_or_default();
        config.active_profile = get_string(map, "profiles/active", "");
        if config.profiles.is_empty() {
            config
                .profiles
                .push(Profile::from_config(&config, "默认".to_string()));
        }
        if !config
            .profiles
            .iter()
            .any(|profile| profile.name == config.active_profile)
        {
            config.active_profile = config.profiles[0].name.clone();
        }
        config.region_scope_open = get_bool(map, "window/region/scope_open", true);
        config.region_gate_open = get_bool(map, "window/region/gate_open", true);
        config.region_preprocess_open = get_bool(map, "window/region/preprocess_open", true);
        config.on_top = get_bool(map, "window/coordinate/on_top", true);
        // 通用任务与抓鬼/鬼王互斥；手改配置或旧配置里同时为真时，以通用任务为准。
        if config.generic_enabled {
            config.zhuagui_enabled = false;
            config.ghost_king_enabled = false;
        }
        config
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let mut map = Map::new();
        let mut insert = |key: &str, value: Value| {
            map.insert(key.to_string(), value);
        };
        insert("ocr/mode", Value::String(self.ocr_mode.clone()));
        insert("ocr/interval_seconds", Value::from(self.interval_seconds));
        insert(
            "ocr/task_region",
            Value::String(region_to_string(self.task_region)),
        );
        insert(
            "ocr/extra_task_region",
            Value::String(region_to_string(self.extra_task_region)),
        );
        insert(
            "ocr/extra_region_enabled",
            Value::Bool(self.extra_region_enabled),
        );
        insert("api/format", Value::String(self.api_format.clone()));
        insert("api/url", Value::String(self.api_url.clone()));
        insert("api/token", Value::String(self.api_token.clone()));
        insert("ocr/trigger", Value::String(self.trigger_mode.clone()));
        insert("ocr/hotkey", Value::String(self.ocr_hotkey.clone()));
        insert("task/option/抓鬼", Value::Bool(self.zhuagui_enabled));
        insert("task/option/鬼王", Value::Bool(self.ghost_king_enabled));
        insert("task/option/通用", Value::Bool(self.generic_enabled));
        insert("task/high_chance", Value::Bool(self.high_chance));
        insert("task/large_font_mode", Value::Bool(self.large_font_mode));
        insert(
            "task/high_chance/鬼王",
            Value::Bool(self.ghost_king_high_chance),
        );
        insert(
            "task/horizontal_layout",
            Value::Bool(self.horizontal_layout),
        );
        insert(
            "window/sidebar_expanded",
            Value::Bool(self.sidebar_expanded),
        );
        insert("log/level", Value::String(self.log_level.clone()));
        insert("color_gate/enabled", Value::Bool(self.color_gate_enabled));
        insert(
            "color_gate/interval_ms",
            Value::from(self.color_gate_interval_ms),
        );
        insert(
            "color_gate/mode",
            Value::String(self.color_gate_mode.clone()),
        );
        insert(
            "color_gate/tolerance",
            Value::from(self.color_gate_tolerance),
        );
        insert(
            "color_gate/points",
            points_to_value(&self.color_gate_points),
        );
        insert(
            "ocr/preprocess",
            Value::String(self.preprocess_mode.clone()),
        );
        insert(
            "ocr/preprocess_color",
            Value::String(to_hex(self.preprocess_color)),
        );
        insert(
            "ocr/preprocess_tolerance",
            Value::from(self.preprocess_tolerance),
        );
        insert(
            "ocr/extra_preprocess",
            Value::String(self.extra_preprocess_mode.clone()),
        );
        insert(
            "ocr/extra_preprocess_color",
            Value::String(to_hex(self.extra_preprocess_color)),
        );
        insert(
            "ocr/extra_preprocess_tolerance",
            Value::from(self.extra_preprocess_tolerance),
        );
        insert("window/theme", Value::String(self.theme.clone()));
        insert(
            "window/sidebar/folded",
            Value::String(self.sidebar_folded.clone()),
        );
        insert("update/enabled", Value::Bool(self.update_enabled));
        insert(
            "update/manifest_url",
            Value::String(self.update_manifest_url.clone()),
        );
        insert(
            "profiles/active",
            Value::String(self.active_profile.clone()),
        );
        insert(
            "profiles/items",
            Value::Array(self.profiles.iter().map(profile_to_value).collect()),
        );
        insert(
            "window/region/scope_open",
            Value::Bool(self.region_scope_open),
        );
        insert(
            "window/region/gate_open",
            Value::Bool(self.region_gate_open),
        );
        insert(
            "window/region/preprocess_open",
            Value::Bool(self.region_preprocess_open),
        );
        insert("window/coordinate/on_top", Value::Bool(self.on_top));

        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|err| format!("创建配置目录失败：{err}"))?;
        }
        let payload = serde_json::to_string_pretty(&Value::Object(map))
            .map_err(|err| format!("序列化配置失败：{err}"))?;
        let text = format!("{CONFIG_HEADER}{payload}\n");
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, text).map_err(|err| format!("写入配置失败：{err}"))?;
        std::fs::rename(&temp, path).map_err(|err| format!("保存配置失败：{err}"))?;
        Ok(())
    }
}

const CONFIG_HEADER: &str = "// 梦幻西游任务坐标辅助（rust-mhxy）用户配置\n\
// 界面里改的设置会自动写到这里，下次启动读回；也可以手工编辑后重启程序。\n\
// 支持 // 和 /* */ 注释。\n";

fn region_to_string(region: TaskRegion) -> String {
    format!("{},{},{},{}", region.0, region.1, region.2, region.3)
}

fn get_string(map: &Map<String, Value>, key: &str, default: &str) -> String {
    match map.get(key) {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        _ => default.to_string(),
    }
}

fn get_u32(map: &Map<String, Value>, key: &str, default: u32) -> u32 {
    match map.get(key) {
        Some(Value::Number(value)) => value
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(default),
        Some(Value::String(value)) => value.trim().parse().unwrap_or(default),
        _ => default,
    }
}

fn get_bool(map: &Map<String, Value>, key: &str, default: bool) -> bool {
    match map.get(key) {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => {
            let value = value.trim().to_ascii_lowercase();
            !matches!(value.as_str(), "" | "false" | "0" | "off" | "no")
        }
        Some(Value::Number(value)) => value.as_i64().map(|v| v != 0).unwrap_or(default),
        _ => default,
    }
}

fn get_points(map: &Map<String, Value>, key: &str) -> Vec<ColorPoint> {
    map.get(key)
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(point_from_value).collect())
        .unwrap_or_default()
}

fn point_from_value(value: &Value) -> Option<ColorPoint> {
    let object = value.as_object()?;
    let x = object.get("x")?.as_i64()? as i32;
    let y = object.get("y")?.as_i64()? as i32;
    let color = object
        .get("color")
        .and_then(Value::as_str)
        .and_then(parse_hex)
        .unwrap_or([255, 255, 255]);
    let tolerance = object
        .get("tolerance")
        .and_then(Value::as_u64)
        .map(|value| value.min(255) as u8)
        .unwrap_or(12);
    let radius = object
        .get("radius")
        .and_then(Value::as_u64)
        .map(|value| value.min(20) as u8)
        .unwrap_or(0);
    let label = object
        .get("label")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Some(ColorPoint {
        x,
        y,
        color,
        tolerance,
        radius,
        label,
    })
}

fn points_to_value(points: &[ColorPoint]) -> Value {
    Value::Array(
        points
            .iter()
            .map(|point| {
                let mut object = Map::new();
                object.insert("x".into(), Value::from(point.x));
                object.insert("y".into(), Value::from(point.y));
                object.insert("color".into(), Value::String(to_hex(point.color)));
                object.insert("tolerance".into(), Value::from(point.tolerance));
                object.insert("radius".into(), Value::from(point.radius));
                object.insert("label".into(), Value::String(point.label.clone()));
                Value::Object(object)
            })
            .collect(),
    )
}

fn profile_from_value(value: &Value) -> Option<Profile> {
    let object = value.as_object()?;
    let name = object.get("name")?.as_str()?.trim().to_string();
    if name.is_empty() {
        return None;
    }
    Some(Profile {
        name,
        gate_enabled: object
            .get("gate_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        gate_interval_ms: object
            .get("gate_interval_ms")
            .and_then(Value::as_u64)
            .map(|value| (value as u32).clamp(50, 5000))
            .unwrap_or(300),
        gate_mode: match object.get("gate_mode").and_then(Value::as_str) {
            Some("any") => "any".to_string(),
            _ => "all".to_string(),
        },
        gate_tolerance: object
            .get("gate_tolerance")
            .and_then(Value::as_u64)
            .map(|value| value.min(255) as u8)
            .unwrap_or(12),
        gate_points: points_from_value(object.get("gate_points")),
        preprocess_mode: match object.get("preprocess_mode").and_then(Value::as_str) {
            Some("gray" | "grayscale") => "gray".to_string(),
            Some("binary" | "binarize") => "binary".to_string(),
            Some("color") => "color".to_string(),
            Some(value) => value.to_string(),
            None => "off".to_string(),
        },
        preprocess_color: object
            .get("preprocess_color")
            .and_then(Value::as_str)
            .and_then(parse_hex)
            .unwrap_or(DEFAULT_PREPROCESS_COLOR),
        preprocess_tolerance: object
            .get("preprocess_tolerance")
            .and_then(Value::as_u64)
            .map(|value| value.min(255) as u8)
            .unwrap_or(24),
        extra_preprocess_mode: match object.get("extra_preprocess_mode").and_then(Value::as_str) {
            Some("gray" | "grayscale") => "gray".to_string(),
            Some("binary" | "binarize") => "binary".to_string(),
            Some("color") => "color".to_string(),
            Some(value) => value.to_string(),
            None => "off".to_string(),
        },
        extra_preprocess_color: object
            .get("extra_preprocess_color")
            .and_then(Value::as_str)
            .and_then(parse_hex)
            .unwrap_or(DEFAULT_PREPROCESS_COLOR),
        extra_preprocess_tolerance: object
            .get("extra_preprocess_tolerance")
            .and_then(Value::as_u64)
            .map(|value| value.min(255) as u8)
            .unwrap_or(24),
    })
}

fn points_from_value(value: Option<&Value>) -> Vec<ColorPoint> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(point_from_value).collect())
        .unwrap_or_default()
}

fn profile_to_value(profile: &Profile) -> Value {
    let mut object = Map::new();
    object.insert("name".into(), Value::String(profile.name.clone()));
    object.insert("gate_enabled".into(), Value::Bool(profile.gate_enabled));
    object.insert(
        "gate_interval_ms".into(),
        Value::from(profile.gate_interval_ms),
    );
    object.insert("gate_mode".into(), Value::String(profile.gate_mode.clone()));
    object.insert("gate_tolerance".into(), Value::from(profile.gate_tolerance));
    object.insert("gate_points".into(), points_to_value(&profile.gate_points));
    object.insert(
        "preprocess_mode".into(),
        Value::String(profile.preprocess_mode.clone()),
    );
    object.insert(
        "preprocess_color".into(),
        Value::String(to_hex(profile.preprocess_color)),
    );
    object.insert(
        "preprocess_tolerance".into(),
        Value::from(profile.preprocess_tolerance),
    );
    object.insert(
        "extra_preprocess_mode".into(),
        Value::String(profile.extra_preprocess_mode.clone()),
    );
    object.insert(
        "extra_preprocess_color".into(),
        Value::String(to_hex(profile.extra_preprocess_color)),
    );
    object.insert(
        "extra_preprocess_tolerance".into(),
        Value::from(profile.extra_preprocess_tolerance),
    );
    Value::Object(object)
}

/// 去掉 JSON 里的 // 与 /* */ 注释，字符串里的斜杠保持原样。
pub fn strip_comments(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;
    while let Some(ch) = chars.next() {
        if in_string {
            result.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        if ch == '"' {
            in_string = true;
            result.push(ch);
            continue;
        }
        if ch == '/' {
            match chars.peek() {
                Some('/') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '\n' {
                            result.push('\n');
                            break;
                        }
                    }
                    continue;
                }
                Some('*') => {
                    chars.next();
                    let mut previous = '\0';
                    for next in chars.by_ref() {
                        if previous == '*' && next == '/' {
                            break;
                        }
                        previous = next;
                    }
                    continue;
                }
                _ => {}
            }
        }
        result.push(ch);
    }
    result
}

/// 配置默认写在 exe 旁边；开发时（存在 Cargo.toml）写在项目根目录。
pub fn default_config_path() -> PathBuf {
    if let Ok(override_path) = std::env::var(CONFIG_PATH_ENV) {
        if !override_path.is_empty() {
            return PathBuf::from(override_path);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("Cargo.toml").exists() {
            return cwd.join(CONFIG_FILE_NAME);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join(CONFIG_FILE_NAME);
        }
    }
    PathBuf::from(CONFIG_FILE_NAME)
}

/// 资源目录（maps.json / maps/ 所在目录）。
pub fn assets_dir() -> PathBuf {
    if let Ok(override_path) = std::env::var("MHXY_ASSETS_DIR") {
        if !override_path.is_empty() {
            return PathBuf::from(override_path);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if cwd.join("assets").join("maps.json").exists() {
            return cwd.join("assets");
        }
        if cwd.join("maps.json").exists() {
            return cwd;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            if dir.join("assets").join("maps.json").exists() {
                return dir.join("assets");
            }
            return dir.to_path_buf();
        }
    }
    PathBuf::from(".")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_line_and_block_comments() {
        let text = r#"{
            // line
            "a": 1, /* block
            comment */
            "b": "http://x"
        }"#;
        let value: Value = serde_json::from_str(&strip_comments(text)).unwrap();
        assert_eq!(value["a"], 1);
        assert_eq!(value["b"], "http://x");
    }

    #[test]
    fn round_trips_config() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        let config = Config {
            api_url: "http://192.168.3.207:8008".into(),
            ghost_king_enabled: false,
            log_level: "detailed".into(),
            color_gate_enabled: true,
            high_chance: true,
            ghost_king_high_chance: false,
            color_gate_points: vec![ColorPoint::new(100, 200, [0xd4, 0xa0, 0x17], 15, 1)],
            ..Default::default()
        };
        config.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.api_url, "http://192.168.3.207:8008");
        assert!(!loaded.ghost_king_enabled);
        assert_eq!(loaded.log_level, "detailed");
        assert!(loaded.color_gate_enabled);
        assert!(loaded.high_chance);
        assert!(!loaded.ghost_king_high_chance);
        assert_eq!(loaded.color_gate_points.len(), 1);
        assert_eq!(loaded.color_gate_points[0].color, [0xd4, 0xa0, 0x17]);
        assert_eq!(loaded.color_gate_points[0].tolerance, 15);
    }

    #[test]
    fn generic_task_wins_over_zhuagui_in_config() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-exclusive-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        Config {
            zhuagui_enabled: true,
            ghost_king_enabled: true,
            generic_enabled: true,
            ..Default::default()
        }
        .save(&path)
        .unwrap();
        // 手改配置把互斥的任务都写成 true 时，加载后以通用任务为准。
        let loaded = Config::load(&path);
        assert!(loaded.generic_enabled);
        assert!(!loaded.zhuagui_enabled);
        assert!(!loaded.ghost_king_enabled);
    }

    #[test]
    fn round_trips_profiles() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-profile-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        let mut config = Config {
            active_profile: "抓鬼".into(),
            color_gate_enabled: true,
            preprocess_mode: "binary".into(),
            ..Default::default()
        };
        let profile = Config::profile_from_current(&config);
        config.profiles.push(profile);
        config.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.active_profile, "抓鬼");
        let saved = loaded
            .profiles
            .iter()
            .find(|profile| profile.name == "抓鬼")
            .expect("方案应被保存");
        assert!(saved.gate_enabled);
        assert_eq!(saved.preprocess_mode, "binary");
        assert_eq!(loaded.profiles[0].name, "默认");
    }

    #[test]
    fn round_trips_extra_preprocess_settings() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-extra-preprocess-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        Config {
            preprocess_mode: "binary".into(),
            extra_preprocess_mode: "color".into(),
            extra_preprocess_color: [9, 8, 7],
            extra_preprocess_tolerance: 33,
            ..Default::default()
        }
        .save(&path)
        .unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.preprocess_mode, "binary");
        assert_eq!(loaded.extra_preprocess_mode, "color");
        assert_eq!(loaded.extra_preprocess_color, [9, 8, 7]);
        assert_eq!(loaded.extra_preprocess_tolerance, 33);
    }

    #[test]
    fn round_trips_theme_setting() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-theme-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        for mode in ["system", "light", "dark"] {
            Config {
                theme: mode.into(),
                ..Default::default()
            }
            .save(&path)
            .unwrap();
            assert_eq!(Config::load(&path).theme, mode);
        }
    }

    #[test]
    fn theme_defaults_to_system_and_accepts_old_values() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-theme-legacy-test");
        let _ = std::fs::create_dir_all(&dir);
        assert_eq!(Config::default().theme, "system");
        let path = dir.join("legacy.json");
        std::fs::write(&path, r#"{"window/theme": "light"}"#).unwrap();
        assert_eq!(Config::load(&path).theme, "light");
        std::fs::write(&path, r#"{"window/theme": "dark"}"#).unwrap();
        assert_eq!(Config::load(&path).theme, "dark");
        std::fs::write(&path, r#"{}"#).unwrap();
        assert_eq!(Config::load(&path).theme, "system");
    }

    #[test]
    fn round_trips_sidebar_fold_state() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-sidebar-fold-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        Config {
            sidebar_folded: "window,task".into(),
            ..Default::default()
        }
        .save(&path)
        .unwrap();
        assert_eq!(Config::load(&path).sidebar_folded, "window,task");
    }

    #[test]
    fn round_trips_region_fold_state() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-fold-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        Config {
            region_scope_open: false,
            region_gate_open: false,
            region_preprocess_open: true,
            ..Default::default()
        }
        .save(&path)
        .unwrap();
        let loaded = Config::load(&path);
        assert!(!loaded.region_scope_open);
        assert!(!loaded.region_gate_open);
        assert!(loaded.region_preprocess_open);
    }

    #[test]
    fn default_preprocess_color_is_eaff65() {
        let config = Config::default();
        assert_eq!(config.preprocess_color, [234, 255, 101]);
        assert_eq!(to_hex(config.preprocess_color), "#eaff65");
    }

    #[test]
    fn round_trips_preprocess_settings() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-preprocess-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        Config {
            preprocess_mode: "color".into(),
            preprocess_color: [0xd4, 0xa0, 0x17],
            preprocess_tolerance: 31,
            ..Default::default()
        }
        .save(&path)
        .unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.preprocess_mode, "color");
        assert_eq!(loaded.preprocess_color, [0xd4, 0xa0, 0x17]);
        assert_eq!(loaded.preprocess_tolerance, 31);
    }

    #[test]
    fn reads_legacy_jiangyao_fumo_key_as_generic() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-legacy-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        std::fs::write(&path, "{\n  \"task/option/降妖伏魔\": true\n}\n").unwrap();
        // 旧配置里的 task/option/降妖伏魔 仍然按通用任务读取。
        let loaded = Config::load(&path);
        assert!(loaded.generic_enabled);
    }
}
