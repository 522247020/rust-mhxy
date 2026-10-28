//! 用户配置的读写。配置文件使用 JSON5，支持 ``//`` / ``/* */`` 注释与尾逗号；
//! 读取时兼容旧版的扁平键，以及布尔值的字符串写法（"true" / "false"）。
use crate::core::{
    parse_region, parse_region_setting, TaskRegion, DEFAULT_EXTRA_REGION, DEFAULT_TASK_REGION,
};
use crate::gate::{parse_hex, to_hex, ColorPoint};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub const CONFIG_FILE_NAME: &str = "config.json";
pub const CONFIG_PATH_ENV: &str = "MHXY_CONFIG_PATH";
/// 预处理默认文字颜色：RGB u8 (234, 255, 101)，即 #eaff65。
pub const DEFAULT_PREPROCESS_COLOR: [u8; 3] = [234, 255, 101];

/// 可保存的识别方案：识别区域 + 图色门控 + 预处理设置，方便一键切换。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    /// 主识别区域；旧方案没有这一项时保持当前区域不变。
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "optional_region_serde"
    )]
    pub task_region: Option<TaskRegion>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "optional_region_serde"
    )]
    pub extra_task_region: Option<TaskRegion>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "de_optional_bool"
    )]
    pub extra_region_enabled: Option<bool>,
    #[serde(deserialize_with = "de_profile_bool")]
    pub gate_enabled: bool,
    #[serde(deserialize_with = "de_profile_gate_interval")]
    pub gate_interval_ms: u32,
    #[serde(deserialize_with = "de_profile_gate_mode")]
    pub gate_mode: String,
    #[serde(deserialize_with = "de_profile_gate_tolerance")]
    pub gate_tolerance: u8,
    #[serde(deserialize_with = "de_points")]
    pub gate_points: Vec<ColorPoint>,
    #[serde(deserialize_with = "de_profile_preprocess_mode")]
    pub preprocess_mode: String,
    #[serde(with = "profile_color_serde")]
    pub preprocess_color: [u8; 3],
    #[serde(deserialize_with = "de_profile_preprocess_tolerance")]
    pub preprocess_tolerance: u8,
    #[serde(deserialize_with = "de_profile_preprocess_mode")]
    pub extra_preprocess_mode: String,
    #[serde(with = "profile_color_serde")]
    pub extra_preprocess_color: [u8; 3],
    #[serde(deserialize_with = "de_profile_preprocess_tolerance")]
    pub extra_preprocess_tolerance: u8,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: String::new(),
            task_region: None,
            extra_task_region: None,
            extra_region_enabled: None,
            gate_enabled: false,
            gate_interval_ms: 300,
            gate_mode: "all".to_string(),
            gate_tolerance: 12,
            gate_points: Vec::new(),
            preprocess_mode: "off".to_string(),
            preprocess_color: DEFAULT_PREPROCESS_COLOR,
            preprocess_tolerance: 24,
            extra_preprocess_mode: "off".to_string(),
            extra_preprocess_color: DEFAULT_PREPROCESS_COLOR,
            extra_preprocess_tolerance: 24,
        }
    }
}

impl Profile {
    /// 用当前配置生成一个方案（名称可指定）。
    pub fn from_config(config: &Config, name: String) -> Self {
        Self {
            name,
            task_region: Some(config.task_region),
            extra_task_region: Some(config.extra_task_region),
            extra_region_enabled: Some(config.extra_region_enabled),
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

    fn normalize(&mut self) {
        self.gate_interval_ms = self.gate_interval_ms.clamp(50, 5000);
        self.gate_mode = if self.gate_mode == "any" {
            "any".to_string()
        } else {
            "all".to_string()
        };
        self.preprocess_mode = canonical_preprocess_mode(&self.preprocess_mode, false);
        self.extra_preprocess_mode = canonical_preprocess_mode(&self.extra_preprocess_mode, false);
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct Config {
    #[serde(rename = "ocr/mode")]
    pub ocr_mode: String,
    #[serde(rename = "ocr/interval_seconds")]
    pub interval_seconds: u32,
    #[serde(rename = "ocr/task_region", with = "task_region_serde")]
    pub task_region: TaskRegion,
    /// 送 OCR 前把截图放大几倍（1 = 不放大）。
    #[serde(rename = "ocr/upscale")]
    pub ocr_upscale: u32,
    /// 高召回检测：降低 PP-OCR 检测阈值，代价是可能多出噪声框。
    #[serde(rename = "ocr/high_recall")]
    pub ocr_high_recall: bool,
    /// 多帧投票：地图取最近几帧众数、坐标取中位数。
    #[serde(rename = "ocr/vote_frames")]
    pub ocr_vote_frames: bool,
    /// 按分辨率保存的识别区域：键是 "1920x1080"，值是 "0.65,0.23,0.98,0.50"。
    #[serde(rename = "ocr/region_presets", deserialize_with = "de_region_presets")]
    pub region_presets: std::collections::BTreeMap<String, String>,
    #[serde(rename = "ocr/extra_task_region", with = "extra_region_serde")]
    pub extra_task_region: TaskRegion,
    #[serde(rename = "ocr/extra_region_enabled")]
    pub extra_region_enabled: bool,
    #[serde(rename = "api/format")]
    pub api_format: String,
    #[serde(rename = "api/url")]
    pub api_url: String,
    #[serde(rename = "api/token")]
    pub api_token: String,
    #[serde(rename = "ocr/trigger")]
    pub trigger_mode: String,
    #[serde(rename = "ocr/hotkey")]
    pub ocr_hotkey: String,
    #[serde(rename = "task/option/抓鬼")]
    pub zhuagui_enabled: bool,
    #[serde(rename = "task/option/鬼王")]
    pub ghost_king_enabled: bool,
    #[serde(rename = "task/option/通用", alias = "task/option/降妖伏魔")]
    pub generic_enabled: bool,
    #[serde(rename = "task/high_chance")]
    pub high_chance: bool,
    #[serde(rename = "task/high_chance/鬼王")]
    pub ghost_king_high_chance: bool,
    #[serde(rename = "task/horizontal_layout")]
    pub horizontal_layout: bool,
    #[serde(rename = "window/sidebar_expanded")]
    pub sidebar_expanded: bool,
    #[serde(rename = "log/level")]
    pub log_level: String,
    #[serde(rename = "color_gate/enabled")]
    pub color_gate_enabled: bool,
    #[serde(rename = "color_gate/interval_ms")]
    pub color_gate_interval_ms: u32,
    #[serde(rename = "color_gate/mode")]
    pub color_gate_mode: String,
    #[serde(rename = "color_gate/tolerance")]
    pub color_gate_tolerance: u8,
    #[serde(rename = "color_gate/points", deserialize_with = "de_points")]
    pub color_gate_points: Vec<ColorPoint>,
    /// OCR 前预处理：off / gray / binary / color。
    #[serde(rename = "ocr/preprocess")]
    pub preprocess_mode: String,
    /// 按颜色提取时保留的文字颜色。
    #[serde(rename = "ocr/preprocess_color", with = "preprocess_color_serde")]
    pub preprocess_color: [u8; 3],
    /// 按颜色提取时的颜色容差。
    #[serde(rename = "ocr/preprocess_tolerance")]
    pub preprocess_tolerance: u8,
    /// 备用区域独立的预处理设置。
    #[serde(rename = "ocr/extra_preprocess")]
    pub extra_preprocess_mode: String,
    #[serde(rename = "ocr/extra_preprocess_color", with = "preprocess_color_serde")]
    pub extra_preprocess_color: [u8; 3],
    #[serde(rename = "ocr/extra_preprocess_tolerance")]
    pub extra_preprocess_tolerance: u8,
    /// 界面主题：system（跟随系统）/ light / dark。
    #[serde(rename = "window/theme")]
    pub theme: String,
    /// 侧栏各分组折叠状态，逗号分隔的 id（如 "window,task"）。
    #[serde(rename = "window/sidebar/folded", default)]
    pub sidebar_folded: String,
    /// 打坐倒计时：时长（秒）、显示格式、悬浮窗样式。
    #[serde(rename = "meditation/duration_seconds")]
    pub meditation_duration_seconds: u32,
    /// 悬浮窗显示分:秒（false = 只显示秒数）。
    #[serde(rename = "meditation/clock_format")]
    pub meditation_clock_format: bool,
    /// 悬浮窗背景不透明度 0~1。
    #[serde(rename = "meditation/opacity")]
    pub meditation_opacity: f32,
    /// 悬浮窗字号（像素），0 = 跟随窗口高度。
    #[serde(rename = "meditation/font_px")]
    pub meditation_font_px: u32,
    #[serde(rename = "meditation/text_color", with = "text_color_serde")]
    pub meditation_text_color: [u8; 3],
    /// 开始 / 重置倒计时的全局快捷键。
    #[serde(rename = "meditation/hotkey")]
    pub meditation_hotkey: String,
    #[serde(rename = "meditation/on_top")]
    pub meditation_on_top: bool,
    /// 锁定后悬浮窗鼠标穿透。
    #[serde(rename = "meditation/locked")]
    pub meditation_locked: bool,
    /// 启用打坐计时：显示悬浮窗并注册全局快捷键。
    #[serde(rename = "meditation/enabled", alias = "meditation/visible")]
    pub meditation_enabled: bool,
    /// 悬浮窗文字加粗。
    #[serde(rename = "meditation/bold")]
    pub meditation_bold: bool,
    /// 悬浮窗外框位置（逻辑像素），None = 交给系统摆放。
    #[serde(rename = "meditation/position", with = "position_serde")]
    pub meditation_position: Option<(f32, f32)>,

    /// 独立地图浮窗：把地图从主窗口里弹出来，无边框、可拖动缩放。
    #[serde(rename = "map_window/enabled")]
    pub map_window_enabled: bool,
    /// 地图浮窗整体不透明度 0~1（1 = 完全不透明）。
    #[serde(rename = "map_window/opacity")]
    pub map_window_opacity: f32,
    #[serde(rename = "map_window/on_top")]
    pub map_window_on_top: bool,
    /// 锁定后浮窗鼠标穿透，只能在主窗口取消。
    #[serde(rename = "map_window/locked")]
    pub map_window_locked: bool,
    /// 浮窗位置 / 大小（逻辑像素），None = 使用默认值。
    #[serde(rename = "map_window/position", with = "position_serde")]
    pub map_window_position: Option<(f32, f32)>,
    #[serde(rename = "map_window/size", with = "position_serde")]
    pub map_window_size: Option<(f32, f32)>,

    /// 启动时检查更新。
    #[serde(rename = "update/enabled")]
    pub update_enabled: bool,
    /// 更新清单地址（自建站点）。
    #[serde(rename = "update/manifest_url")]
    pub update_manifest_url: String,
    /// 保存的识别方案与当前方案名。
    #[serde(rename = "profiles/items", default, deserialize_with = "de_profiles")]
    pub profiles: Vec<Profile>,
    #[serde(rename = "profiles/active", default)]
    pub active_profile: String,
    /// 识别区域窗口各分组的折叠状态。
    #[serde(rename = "window/region/scope_open")]
    pub region_scope_open: bool,
    #[serde(rename = "window/region/gate_open")]
    pub region_gate_open: bool,
    #[serde(rename = "window/region/preprocess_open")]
    pub region_preprocess_open: bool,
    #[serde(rename = "window/coordinate/on_top")]
    pub on_top: bool,
}

impl Default for Config {
    fn default() -> Self {
        let mut config = Self {
            ocr_mode: "local".into(),
            interval_seconds: 5,
            task_region: DEFAULT_TASK_REGION,
            ocr_upscale: 2,
            ocr_high_recall: true,
            ocr_vote_frames: true,
            region_presets: std::collections::BTreeMap::new(),
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
            // 新用户默认收起「高级设置」，侧栏只留常用项。
            sidebar_folded: "advanced".to_string(),
            meditation_duration_seconds: crate::meditation::DEFAULT_DURATION_SECONDS,
            meditation_clock_format: false,
            meditation_opacity: 0.85,
            meditation_font_px: 0,
            meditation_text_color: [255, 255, 255],
            meditation_hotkey: "F9".to_string(),
            meditation_on_top: true,
            meditation_locked: false,
            meditation_enabled: false,
            meditation_bold: false,
            meditation_position: None,
            map_window_enabled: false,
            map_window_opacity: 1.0,
            map_window_on_top: true,
            map_window_locked: false,
            map_window_position: None,
            map_window_size: None,
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
        let Ok(raw) = std::fs::read_to_string(path) else {
            return Config::default();
        };
        let Ok(value) = json5::from_str::<Value>(&raw) else {
            return Config::default();
        };
        let Value::Object(mut map) = value else {
            return Config::default();
        };
        prepare_flat_values(&mut map);
        let mut config = serde_json::from_value::<Config>(Value::Object(map)).unwrap_or_default();
        config.normalize();
        config
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(|err| format!("创建配置目录失败：{err}"))?;
        }
        // 先转成 Value，保持旧版的键排序（serde_json 默认按 BTreeMap 输出）。
        let value = serde_json::to_value(self).map_err(|err| format!("序列化配置失败：{err}"))?;
        let payload =
            serde_json::to_string_pretty(&value).map_err(|err| format!("序列化配置失败：{err}"))?;
        let text = format!("{CONFIG_HEADER}{payload}\n");
        let temp = path.with_extension("json.tmp");
        std::fs::write(&temp, text).map_err(|err| format!("写入配置失败：{err}"))?;
        std::fs::rename(&temp, path).map_err(|err| format!("保存配置失败：{err}"))?;
        Ok(())
    }

    fn normalize(&mut self) {
        self.ocr_mode = if self.ocr_mode == "remote" {
            "remote".to_string()
        } else {
            "local".to_string()
        };
        self.interval_seconds = self.interval_seconds.clamp(1, 60);
        self.ocr_upscale = self.ocr_upscale.clamp(1, 3);
        self.region_presets
            .retain(|_, region| parse_region(region).is_some());
        self.api_format = if self.api_format == "raw" {
            "raw".to_string()
        } else {
            "multipart".to_string()
        };
        self.trigger_mode = if self.trigger_mode == "manual" {
            "manual".to_string()
        } else {
            "auto".to_string()
        };
        self.log_level = if self.log_level == "detailed" {
            "detailed".to_string()
        } else {
            "simple".to_string()
        };
        self.color_gate_interval_ms = self.color_gate_interval_ms.clamp(50, 5000);
        self.color_gate_mode = if self.color_gate_mode == "any" {
            "any".to_string()
        } else {
            "all".to_string()
        };
        self.preprocess_mode = canonical_preprocess_mode(&self.preprocess_mode, true);
        self.extra_preprocess_mode = canonical_preprocess_mode(&self.extra_preprocess_mode, false);
        self.theme = match self.theme.as_str() {
            "light" => "light".to_string(),
            "dark" => "dark".to_string(),
            _ => "system".to_string(),
        };
        self.meditation_duration_seconds =
            crate::meditation::clamp_duration(self.meditation_duration_seconds);
        self.meditation_opacity = self.meditation_opacity.clamp(0.0, 1.0);
        self.meditation_font_px = self.meditation_font_px.min(200);
        self.map_window_opacity = self.map_window_opacity.clamp(0.0, 1.0);
        if let Some((width, height)) = self.map_window_size {
            // 尺寸过小会让浮窗缩成一条线，既看不清也拖不回来。
            self.map_window_size = Some((width.max(160.0), height.max(120.0)));
        }
        // 通用任务与抓鬼/鬼王互斥；手改配置或旧配置里同时为真时，以通用任务为准。
        if self.generic_enabled {
            self.zhuagui_enabled = false;
            self.ghost_king_enabled = false;
        }
        for profile in &mut self.profiles {
            profile.normalize();
        }
        self.profiles
            .retain(|profile| !profile.name.trim().is_empty());
        if self.profiles.is_empty() {
            self.profiles
                .push(Profile::from_config(self, "默认".to_string()));
        }
        if !self
            .profiles
            .iter()
            .any(|profile| profile.name == self.active_profile)
        {
            self.active_profile = self.profiles[0].name.clone();
        }
    }
}

const CONFIG_HEADER: &str = "// 梦幻西游任务坐标辅助（rust-mhxy）用户配置\n\
// 界面里改的设置会自动写到这里，下次启动读回；也可以手工编辑后重启程序。\n\
// 支持 // 和 /* */ 注释。\n";

pub(crate) fn region_to_string(region: TaskRegion) -> String {
    format!("{},{},{},{}", region.0, region.1, region.2, region.3)
}

fn canonical_preprocess_mode(value: &str, strict: bool) -> String {
    match value {
        "gray" | "grayscale" => "gray".to_string(),
        "binary" | "binarize" => "binary".to_string(),
        "color" => "color".to_string(),
        "off" => "off".to_string(),
        other if !strict => other.to_string(),
        _ => "off".to_string(),
    }
}

fn bool_from_value(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::String(value) => {
            let value = value.trim().to_ascii_lowercase();
            Some(!matches!(value.as_str(), "" | "false" | "0" | "off" | "no"))
        }
        Value::Number(value) => value.as_i64().map(|value| value != 0),
        _ => None,
    }
}

fn legacy_string(value: Option<&Value>, default: &str) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        _ => default.to_string(),
    }
}

fn resolve_bool_alias(map: &mut Map<String, Value>, new_key: &str, old_key: &str) {
    let new_value = map.get(new_key).and_then(bool_from_value);
    let old_value = map.get(old_key).and_then(bool_from_value);
    match (new_value, old_value) {
        (Some(_), _) => {
            map.remove(old_key);
        }
        (None, Some(_)) => {
            map.remove(new_key);
        }
        (None, None) => {
            map.remove(new_key);
            map.remove(old_key);
        }
    }
}

fn fill_ghost_king_high_chance(map: &mut Map<String, Value>) {
    let high_chance = map
        .get("task/high_chance")
        .and_then(bool_from_value)
        .unwrap_or(false);
    let has_ghost_value = map
        .get("task/high_chance/鬼王")
        .and_then(bool_from_value)
        .is_some();
    if !has_ghost_value {
        map.insert(
            "task/high_chance/鬼王".to_string(),
            Value::Bool(high_chance),
        );
    }
}

/// 旧配置的备用区域预处理会继承主区域；把继承结果先补成显式值。
fn fill_extra_preprocess(map: &mut Map<String, Value>) {
    let primary_mode =
        canonical_preprocess_mode(&legacy_string(map.get("ocr/preprocess"), "off"), true);
    let primary_color = parse_hex(&legacy_string(
        map.get("ocr/preprocess_color"),
        &to_hex(DEFAULT_PREPROCESS_COLOR),
    ))
    .unwrap_or(DEFAULT_PREPROCESS_COLOR);
    let primary_tolerance = map
        .get("ocr/preprocess_tolerance")
        .and_then(Value::as_u64)
        .map(|value| value.min(255) as u8)
        .unwrap_or(24);

    let extra_mode_is_scalar = matches!(
        map.get("ocr/extra_preprocess"),
        Some(Value::String(_) | Value::Number(_))
    );
    if !extra_mode_is_scalar {
        map.insert(
            "ocr/extra_preprocess".to_string(),
            Value::String(primary_mode),
        );
    }

    let extra_color = match map.get("ocr/extra_preprocess_color") {
        Some(Value::String(value)) => parse_hex(value).unwrap_or(primary_color),
        Some(Value::Number(value)) => parse_hex(&value.to_string()).unwrap_or(primary_color),
        _ => primary_color,
    };
    map.insert(
        "ocr/extra_preprocess_color".to_string(),
        Value::String(to_hex(extra_color)),
    );

    let extra_tolerance = match map.get("ocr/extra_preprocess_tolerance") {
        Some(Value::Number(value)) => value
            .as_u64()
            .map(|value| value.min(255) as u8)
            .unwrap_or(primary_tolerance),
        _ => primary_tolerance,
    };
    map.insert(
        "ocr/extra_preprocess_tolerance".to_string(),
        Value::Number(extra_tolerance.into()),
    );
}

const STRING_KEYS: &[&str] = &[
    "ocr/mode",
    "api/format",
    "api/url",
    "api/token",
    "ocr/trigger",
    "ocr/hotkey",
    "log/level",
    "color_gate/mode",
    "ocr/preprocess",
    "ocr/extra_preprocess",
    "window/theme",
    "window/sidebar/folded",
    "meditation/hotkey",
    "update/manifest_url",
    "profiles/active",
];

const BOOL_KEYS: &[&str] = &[
    "ocr/extra_region_enabled",
    "task/option/抓鬼",
    "task/option/鬼王",
    "task/option/通用",
    "task/option/降妖伏魔",
    "task/high_chance",
    "task/high_chance/鬼王",
    "task/horizontal_layout",
    "window/sidebar_expanded",
    "color_gate/enabled",
    "meditation/clock_format",
    "meditation/on_top",
    "meditation/locked",
    "meditation/enabled",
    "meditation/visible",
    "meditation/bold",
    "update/enabled",
    "window/region/scope_open",
    "window/region/gate_open",
    "window/region/preprocess_open",
    "window/coordinate/on_top",
];

const U32_KEYS: &[&str] = &[
    "ocr/interval_seconds",
    "color_gate/interval_ms",
    "meditation/duration_seconds",
    "meditation/font_px",
];

const U8_KEYS: &[&str] = &[
    "color_gate/tolerance",
    "ocr/preprocess_tolerance",
    "ocr/extra_preprocess_tolerance",
];

fn coerce_string(map: &mut Map<String, Value>, key: &str) {
    let replacement = match map.get(key) {
        Some(Value::String(_)) | None => return,
        Some(Value::Number(value)) => Some(Value::String(value.to_string())),
        Some(_) => None,
    };
    match replacement {
        Some(value) => {
            map.insert(key.to_string(), value);
        }
        None => {
            map.remove(key);
        }
    }
}

fn coerce_bool(map: &mut Map<String, Value>, key: &str) {
    match map.get(key).and_then(bool_from_value) {
        Some(value) => {
            map.insert(key.to_string(), Value::Bool(value));
        }
        None => {
            map.remove(key);
        }
    }
}

fn coerce_u32(map: &mut Map<String, Value>, key: &str) {
    let value = match map.get(key) {
        Some(Value::Number(value)) => value.as_u64().and_then(|value| u32::try_from(value).ok()),
        Some(Value::String(value)) => value.trim().parse::<u32>().ok(),
        _ => None,
    };
    match value {
        Some(value) => {
            map.insert(key.to_string(), Value::Number(value.into()));
        }
        None => {
            map.remove(key);
        }
    }
}

fn coerce_u8(map: &mut Map<String, Value>, key: &str) {
    let value = map
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| value.min(255) as u8);
    match value {
        Some(value) => {
            map.insert(key.to_string(), Value::Number(value.into()));
        }
        None => {
            map.remove(key);
        }
    }
}

fn coerce_f32(map: &mut Map<String, Value>, key: &str) {
    let value = match map.get(key) {
        Some(Value::Number(value)) => value
            .as_f64()
            .map(|value| value as f32)
            .filter(|value| value.is_finite()),
        Some(Value::String(value)) => value
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite()),
        _ => None,
    };
    match value.and_then(|value| serde_json::Number::from_f64(f64::from(value))) {
        Some(value) => {
            map.insert(key.to_string(), Value::Number(value));
        }
        None => {
            map.remove(key);
        }
    }
}

/// 在反序列化前把旧版允许的宽松写法收敛成 serde 能直接读取的扁平键值。
fn prepare_flat_values(map: &mut Map<String, Value>) {
    resolve_bool_alias(map, "task/option/通用", "task/option/降妖伏魔");
    resolve_bool_alias(map, "meditation/enabled", "meditation/visible");
    fill_ghost_king_high_chance(map);
    fill_extra_preprocess(map);
    for key in STRING_KEYS {
        coerce_string(map, key);
    }
    for key in BOOL_KEYS {
        coerce_bool(map, key);
    }
    for key in U32_KEYS {
        coerce_u32(map, key);
    }
    for key in U8_KEYS {
        coerce_u8(map, key);
    }
    coerce_f32(map, "meditation/opacity");
}

mod task_region_serde {
    use super::*;

    pub fn serialize<S>(region: &TaskRegion, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&region_to_string(*region))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<TaskRegion, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Ok(parse_region_setting(
            value.as_str().unwrap_or(""),
            DEFAULT_TASK_REGION,
        ))
    }
}

mod extra_region_serde {
    use super::*;

    pub fn serialize<S>(region: &TaskRegion, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&region_to_string(*region))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<TaskRegion, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Ok(parse_region_setting(
            value.as_str().unwrap_or(""),
            DEFAULT_EXTRA_REGION,
        ))
    }
}

mod optional_region_serde {
    use super::*;

    pub fn serialize<S>(region: &Option<TaskRegion>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match region {
            Some(region) => serializer.serialize_str(&region_to_string(*region)),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<TaskRegion>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Ok(value.as_str().and_then(parse_region))
    }
}

mod preprocess_color_serde {
    use super::*;

    pub fn serialize<S>(color: &[u8; 3], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&to_hex(*color))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 3], D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Ok(color_from_value(&value).unwrap_or(DEFAULT_PREPROCESS_COLOR))
    }
}

mod profile_color_serde {
    use super::*;

    pub fn serialize<S>(color: &[u8; 3], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&to_hex(*color))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 3], D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Ok(value
            .as_str()
            .and_then(parse_hex)
            .unwrap_or(DEFAULT_PREPROCESS_COLOR))
    }
}

mod text_color_serde {
    use super::*;

    pub fn serialize<S>(color: &[u8; 3], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&to_hex(*color))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 3], D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Ok(color_from_value(&value).unwrap_or([255, 255, 255]))
    }
}

fn color_from_value(value: &Value) -> Option<[u8; 3]> {
    match value {
        Value::String(value) => parse_hex(value),
        Value::Number(value) => parse_hex(&value.to_string()),
        _ => None,
    }
}

mod position_serde {
    use super::*;

    pub fn serialize<S>(position: &Option<(f32, f32)>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let text = position
            .map(|(x, y)| format!("{x:.0},{y:.0}"))
            .unwrap_or_default();
        serializer.serialize_str(&text)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<(f32, f32)>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        Ok(position_from_value(&value))
    }
}

fn position_from_value(value: &Value) -> Option<(f32, f32)> {
    let (x, y) = value.as_str()?.split_once(',')?;
    let x = x.trim().parse::<f32>().ok()?;
    let y = y.trim().parse::<f32>().ok()?;
    if x.is_finite() && y.is_finite() {
        Some((x, y))
    } else {
        None
    }
}

fn de_region_presets<'de, D>(
    deserializer: D,
) -> Result<std::collections::BTreeMap<String, String>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(value
        .as_object()
        .map(|items| {
            items
                .iter()
                .filter_map(|(key, value)| {
                    let region = value.as_str()?;
                    parse_region(region)?;
                    Some((key.clone(), region.to_string()))
                })
                .collect()
        })
        .unwrap_or_default())
}

fn de_points<'de, D>(deserializer: D) -> Result<Vec<ColorPoint>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(value
        .as_array()
        .map(|items| items.iter().filter_map(point_from_value).collect())
        .unwrap_or_default())
}

fn de_profiles<'de, D>(deserializer: D) -> Result<Vec<Profile>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(value
        .as_array()
        .map(|items| items.iter().filter_map(profile_from_value).collect())
        .unwrap_or_default())
}

fn de_optional_bool<'de, D>(deserializer: D) -> Result<Option<bool>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(value.as_bool())
}

fn de_profile_bool<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(value.as_bool().unwrap_or(false))
}

fn de_profile_gate_interval<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(value.as_u64().map(|value| value as u32).unwrap_or(300))
}

fn de_profile_gate_mode<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(if value.as_str() == Some("any") {
        "any".to_string()
    } else {
        "all".to_string()
    })
}

fn de_profile_gate_tolerance<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(value
        .as_u64()
        .map(|value| value.min(255) as u8)
        .unwrap_or(12))
}

fn de_profile_preprocess_mode<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(canonical_preprocess_mode(
        value.as_str().unwrap_or("off"),
        false,
    ))
}

fn de_profile_preprocess_tolerance<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(value
        .as_u64()
        .map(|value| value.min(255) as u8)
        .unwrap_or(24))
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

fn profile_from_value(value: &Value) -> Option<Profile> {
    let mut profile: Profile = serde_json::from_value(value.clone()).ok()?;
    let name = profile.name.trim().to_string();
    if name.is_empty() {
        return None;
    }
    profile.name = name;
    profile.normalize();
    Some(profile)
}

impl Serialize for ColorPoint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("ColorPoint", 6)?;
        state.serialize_field("x", &self.x)?;
        state.serialize_field("y", &self.y)?;
        state.serialize_field("color", &to_hex(self.color))?;
        state.serialize_field("tolerance", &self.tolerance)?;
        state.serialize_field("radius", &self.radius)?;
        state.serialize_field("label", &self.label)?;
        state.end()
    }
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
    fn reads_json5_with_comments_and_trailing_commas() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-json5-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r##"{
                // 行注释
                "ocr/mode": "remote", /* 块注释 */
                "ocr/interval_seconds": 9,
                "color_gate/points": [
                    { x: 100, y: 200, color: "#d4a017", tolerance: 15, radius: 1, label: "点1", },
                ],
            }"##,
        )
        .unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.ocr_mode, "remote");
        assert_eq!(loaded.interval_seconds, 9);
        assert_eq!(loaded.color_gate_points.len(), 1);
        assert_eq!(loaded.color_gate_points[0].x, 100);
        assert_eq!(loaded.color_gate_points[0].color, [0xd4, 0xa0, 0x17]);
        assert_eq!(loaded.color_gate_points[0].label, "点1");
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
        std::fs::write(&path, r#"{"window/theme": "unknown"}"#).unwrap();
        assert_eq!(Config::load(&path).theme, "system");
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
    fn new_generic_key_wins_over_legacy_alias() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-legacy-alias-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r#"{
                "task/option/通用": false,
                "task/option/降妖伏魔": true,
            }"#,
        )
        .unwrap();
        let loaded = Config::load(&path);
        assert!(!loaded.generic_enabled);
        assert!(loaded.zhuagui_enabled);
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

    #[test]
    fn legacy_keys_and_values_keep_working() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-legacy-values-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r#"{
                // 旧版只有 visible 和 high_chance，没有细分字段
                "task/option/降妖伏魔": true,
                "window/theme": "light",
                "meditation/visible": true,
                "task/high_chance": true,
            }"#,
        )
        .unwrap();
        let loaded = Config::load(&path);
        assert!(loaded.generic_enabled);
        assert!(!loaded.zhuagui_enabled);
        assert!(!loaded.ghost_king_enabled);
        assert_eq!(loaded.theme, "light");
        assert!(loaded.meditation_enabled);
        assert!(loaded.ghost_king_high_chance);
    }

    #[test]
    fn legacy_extra_preprocess_inherits_primary() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-extra-inherit-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r##"{
                "ocr/preprocess": "binary",
                "ocr/preprocess_color": "#123456",
                "ocr/preprocess_tolerance": 33,
                "ocr/extra_preprocess_color": "#zzzzzz",
            }"##,
        )
        .unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.preprocess_mode, "binary");
        assert_eq!(loaded.extra_preprocess_mode, "binary");
        assert_eq!(loaded.extra_preprocess_color, [0x12, 0x34, 0x56]);
        assert_eq!(loaded.extra_preprocess_tolerance, 33);
    }

    #[test]
    fn missing_profile_fields_use_defaults() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-profile-defaults-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r#"{
                "profiles/items": [{ "name": "旧方案", "gate_enabled": true }],
            }"#,
        )
        .unwrap();
        let loaded = Config::load(&path);
        // 没有 profiles/active 时回退到第一个方案。
        assert_eq!(loaded.active_profile, "旧方案");
        assert_eq!(loaded.profiles.len(), 1);
        let profile = &loaded.profiles[0];
        assert!(profile.gate_enabled);
        assert_eq!(profile.task_region, None);
        assert_eq!(profile.extra_task_region, None);
        assert_eq!(profile.extra_region_enabled, None);
        assert_eq!(profile.gate_interval_ms, 300);
        assert_eq!(profile.gate_mode, "all");
        assert_eq!(profile.gate_tolerance, 12);
        assert!(profile.gate_points.is_empty());
        assert_eq!(profile.preprocess_mode, "off");
        assert_eq!(profile.preprocess_color, DEFAULT_PREPROCESS_COLOR);
        assert_eq!(profile.preprocess_tolerance, 24);
        assert_eq!(profile.extra_preprocess_mode, "off");
        assert_eq!(profile.extra_preprocess_color, DEFAULT_PREPROCESS_COLOR);
        assert_eq!(profile.extra_preprocess_tolerance, 24);
        // 旧版 load 对缺失的侧栏折叠字段使用空字符串，而不是 Config::default。
        assert_eq!(loaded.sidebar_folded, "");
    }

    #[test]
    fn creates_default_profile_from_loaded_settings() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-default-profile-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        std::fs::write(
            &path,
            r##"{
                "color_gate/enabled": true,
                "color_gate/interval_ms": 777,
                "color_gate/points": [{ "x": 1, "y": 2, "color": "#112233" }],
                "ocr/preprocess": "color",
                "ocr/region_presets": {
                    "ok": "0.1,0.1,0.9,0.9",
                    "bad": "0.9,0.1,0.2,0.9",
                },
            }"##,
        )
        .unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.region_presets.len(), 1);
        assert_eq!(
            loaded.region_presets.get("ok").map(String::as_str),
            Some("0.1,0.1,0.9,0.9")
        );
        assert_eq!(loaded.profiles.len(), 1);
        let profile = &loaded.profiles[0];
        assert_eq!(profile.name, "默认");
        assert_eq!(loaded.active_profile, "默认");
        assert!(profile.gate_enabled);
        assert_eq!(profile.gate_interval_ms, 777);
        assert_eq!(profile.gate_points.len(), 1);
        assert_eq!(profile.preprocess_mode, "color");
        assert_eq!(profile.extra_preprocess_mode, "color");
    }

    #[test]
    fn map_window_settings_fall_back_to_defaults_and_are_sanitized() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-map-window-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        // 旧配置没有 map_window/*：使用默认值（关闭、不透明、置顶）。
        std::fs::write(&path, r#"{ "window/theme": "dark" }"#).unwrap();
        let loaded = Config::load(&path);
        assert!(!loaded.map_window_enabled);
        assert_eq!(loaded.map_window_opacity, 1.0);
        assert!(loaded.map_window_on_top);
        assert!(!loaded.map_window_locked);
        assert_eq!(loaded.map_window_position, None);
        assert_eq!(loaded.map_window_size, None);

        // 手改配置写出越界值时要收敛到可用范围。
        std::fs::write(
            &path,
            r##"{
                "map_window/enabled": true,
                "map_window/opacity": 2.5,
                "map_window/position": "120,240",
                "map_window/size": "10,20",
            }"##,
        )
        .unwrap();
        let loaded = Config::load(&path);
        assert!(loaded.map_window_enabled);
        assert_eq!(loaded.map_window_opacity, 1.0);
        assert_eq!(loaded.map_window_position, Some((120.0, 240.0)));
        assert_eq!(loaded.map_window_size, Some((160.0, 120.0)));
    }

    #[test]
    fn save_output_keeps_flat_key_format() {
        let dir = std::env::temp_dir().join("rust-mhxy-config-output-format-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        let mut config = Config {
            theme: "dark".into(),
            meditation_position: Some((12.4, 56.6)),
            map_window_position: Some((40.4, 50.6)),
            map_window_size: Some((520.2, 480.7)),
            map_window_opacity: 0.75,
            color_gate_points: vec![ColorPoint::new(100, 200, [0xd4, 0xa0, 0x17], 15, 1)],
            ..Default::default()
        };
        config
            .region_presets
            .insert("1920x1080".into(), "0.65,0.23,0.98,0.50".into());
        config.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(CONFIG_HEADER));
        assert!(text.ends_with("}\n"));
        let body = text.strip_prefix(CONFIG_HEADER).unwrap();
        // 旧版用 serde_json::Map（BTreeMap）输出，键按字母序排列。
        assert!(body.starts_with("{\n  \"api/format\":"));

        let value: Value = json5::from_str(&text).unwrap();
        let object = value.as_object().unwrap();
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        let mut expected = [
            "api/format",
            "api/token",
            "api/url",
            "color_gate/enabled",
            "color_gate/interval_ms",
            "color_gate/mode",
            "color_gate/points",
            "color_gate/tolerance",
            "log/level",
            "map_window/enabled",
            "map_window/locked",
            "map_window/on_top",
            "map_window/opacity",
            "map_window/position",
            "map_window/size",
            "meditation/bold",
            "meditation/clock_format",
            "meditation/duration_seconds",
            "meditation/enabled",
            "meditation/font_px",
            "meditation/hotkey",
            "meditation/locked",
            "meditation/on_top",
            "meditation/opacity",
            "meditation/position",
            "meditation/text_color",
            "ocr/extra_preprocess",
            "ocr/extra_preprocess_color",
            "ocr/extra_preprocess_tolerance",
            "ocr/extra_region_enabled",
            "ocr/extra_task_region",
            "ocr/high_recall",
            "ocr/hotkey",
            "ocr/interval_seconds",
            "ocr/mode",
            "ocr/preprocess",
            "ocr/preprocess_color",
            "ocr/preprocess_tolerance",
            "ocr/region_presets",
            "ocr/task_region",
            "ocr/trigger",
            "ocr/upscale",
            "ocr/vote_frames",
            "profiles/active",
            "profiles/items",
            "task/high_chance",
            "task/high_chance/鬼王",
            "task/horizontal_layout",
            "task/option/抓鬼",
            "task/option/通用",
            "task/option/鬼王",
            "update/enabled",
            "update/manifest_url",
            "window/coordinate/on_top",
            "window/region/gate_open",
            "window/region/preprocess_open",
            "window/region/scope_open",
            "window/sidebar/folded",
            "window/sidebar_expanded",
            "window/theme",
        ];
        expected.sort_unstable();
        assert_eq!(keys, expected);

        assert_eq!(value["ocr/task_region"], "0.7,0.24,0.99,0.7");
        assert_eq!(value["ocr/extra_task_region"], "0.7,0.71,0.99,0.9");
        assert_eq!(value["ocr/preprocess_color"], "#eaff65");
        assert_eq!(value["meditation/text_color"], "#ffffff");
        assert_eq!(value["meditation/position"], "12,57");
        assert_eq!(value["map_window/position"], "40,51");
        assert_eq!(value["map_window/size"], "520,481");
        assert_eq!(value["map_window/opacity"], 0.75);
        assert_eq!(value["window/theme"], "dark");
        assert_eq!(
            value["ocr/region_presets"]["1920x1080"],
            "0.65,0.23,0.98,0.50"
        );
        assert_eq!(value["color_gate/points"][0]["color"], "#d4a017");
        assert_eq!(
            value["profiles/items"][0]["task_region"],
            "0.7,0.24,0.99,0.7"
        );
        assert_eq!(value["profiles/items"][0]["preprocess_color"], "#eaff65");
        assert!(!object.contains_key("ocr"));
        assert!(!object.contains_key("task/option/降妖伏魔"));
        assert!(!text.contains("\"ocr\": {"));
    }
}
