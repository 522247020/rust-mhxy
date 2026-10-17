//! 用户配置的读写。为了兼容 Python 版，支持 JSON 里的 ``//`` 与 ``/* */`` 注释，
//! 并且对布尔值的字符串写法（"true" / "false"）做容错。
use crate::core::{parse_region_setting, TaskRegion, DEFAULT_EXTRA_REGION, DEFAULT_TASK_REGION};
use crate::gate::{parse_hex, to_hex, ColorPoint};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

pub const CONFIG_FILE_NAME: &str = "config.json";
pub const CONFIG_PATH_ENV: &str = "MHXY_CONFIG_PATH";

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
    pub zhuagui_enabled: bool,
    pub ghost_king_enabled: bool,
    pub generic_enabled: bool,
    pub horizontal_layout: bool,
    pub sidebar_expanded: bool,
    pub log_level: String,
    pub color_gate_enabled: bool,
    pub color_gate_interval_ms: u32,
    pub color_gate_mode: String,
    pub color_gate_tolerance: u8,
    pub color_gate_points: Vec<ColorPoint>,
    pub on_top: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ocr_mode: "local".into(),
            interval_seconds: 5,
            task_region: DEFAULT_TASK_REGION,
            extra_task_region: DEFAULT_EXTRA_REGION,
            extra_region_enabled: false,
            api_format: "multipart".into(),
            api_url: String::new(),
            api_token: String::new(),
            zhuagui_enabled: true,
            ghost_king_enabled: true,
            generic_enabled: false,
            horizontal_layout: false,
            sidebar_expanded: true,
            log_level: "simple".into(),
            color_gate_enabled: false,
            color_gate_interval_ms: 300,
            color_gate_mode: "all".into(),
            color_gate_tolerance: 12,
            color_gate_points: Vec::new(),
            on_top: true,
        }
    }
}

impl Config {
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
        config.zhuagui_enabled = get_bool(map, "task/option/抓鬼", true);
        config.ghost_king_enabled = get_bool(map, "task/option/鬼王", true);
        // 通用任务原来的键是 task/option/降妖伏魔，读旧配置时兼容一下。
        config.generic_enabled = get_bool(
            map,
            "task/option/通用",
            get_bool(map, "task/option/降妖伏魔", false),
        );
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
        insert("task/option/抓鬼", Value::Bool(self.zhuagui_enabled));
        insert("task/option/鬼王", Value::Bool(self.ghost_king_enabled));
        insert("task/option/通用", Value::Bool(self.generic_enabled));
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
            color_gate_points: vec![ColorPoint::new(100, 200, [0xd4, 0xa0, 0x17], 15, 1)],
            ..Default::default()
        };
        config.save(&path).unwrap();
        let loaded = Config::load(&path);
        assert_eq!(loaded.api_url, "http://192.168.3.207:8008");
        assert!(!loaded.ghost_king_enabled);
        assert_eq!(loaded.log_level, "detailed");
        assert!(loaded.color_gate_enabled);
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
