//! 本地飞行旗配置。合成旗是多个飞行点的集合，同一合成旗共享地图标注编号。
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const CONFIG_PATH_ENV: &str = "MHXY_FLIGHT_FLAGS_PATH";
pub const CONFIG_FILE_NAME: &str = "flight_flags.json";
pub const SUPPORTED_MAPS: [&str; 4] = ["长安城", "傲来国", "长寿村", "朱紫国"];

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct FlightPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CompositeFlag {
    pub name: String,
    #[serde(default)]
    pub points: Vec<FlightPoint>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq)]
pub struct FlightFlagStyle {
    pub background: [f32; 4],
    #[serde(default = "opaque_alpha")]
    pub background_normal_alpha: f32,
    pub text: [f32; 4],
    #[serde(default = "opaque_alpha")]
    pub text_normal_alpha: f32,
    pub size: u8,
}

fn opaque_alpha() -> f32 {
    1.0
}

impl Default for FlightFlagStyle {
    fn default() -> Self {
        Self {
            background: [0.5, 0.0, 0.0, 1.0],
            background_normal_alpha: 1.0,
            text: [0.0, 0.0, 1.0, 1.0],
            text_normal_alpha: 1.0,
            size: 20,
        }
    }
}

impl FlightFlagStyle {
    fn migrate_legacy_rgba(&mut self) {
        if self.background.iter().any(|channel| *channel > 1.0) {
            self.background = [
                self.background[0] / 255.0,
                self.background[1] / 255.0,
                self.background[2] / 255.0,
                1.0,
            ];
        }
        if self.text.iter().any(|channel| *channel > 1.0) {
            self.text = [
                self.text[0] / 255.0,
                self.text[1] / 255.0,
                self.text[2] / 255.0,
                1.0,
            ];
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct FlightFlags {
    #[serde(default)]
    pub maps: BTreeMap<String, Vec<CompositeFlag>>,
    #[serde(default)]
    pub style: FlightFlagStyle,
}

#[derive(Deserialize)]
struct LegacyFlightFlags {
    #[serde(default)]
    maps: BTreeMap<String, Vec<FlightPoint>>,
}

impl FlightFlags {
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text =
            std::fs::read_to_string(path).map_err(|err| format!("读取飞行旗配置失败：{err}"))?;
        let mut flags: Self = match serde_json::from_str(&text) {
            Ok(flags) => flags,
            Err(current_error) => {
                let legacy: LegacyFlightFlags = serde_json::from_str(&text)
                    .map_err(|_| format!("飞行旗配置格式错误：{current_error}"))?;
                Self {
                    style: FlightFlagStyle::default(),
                    maps: legacy
                        .maps
                        .into_iter()
                        .filter(|(_, points)| !points.is_empty())
                        .map(|(map, points)| {
                            (
                                map,
                                vec![CompositeFlag {
                                    name: "合成旗 1".to_string(),
                                    points,
                                }],
                            )
                        })
                        .collect(),
                }
            }
        };
        flags.normalize();
        Ok(flags)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("创建飞行旗配置目录失败：{err}"))?;
        }
        let payload = serde_json::to_string_pretty(self)
            .map_err(|err| format!("生成飞行旗配置失败：{err}"))?;
        let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
        std::fs::write(&temporary, format!("{payload}\n"))
            .map_err(|err| format!("写入飞行旗配置失败：{err}"))?;
        if path.exists() {
            std::fs::remove_file(path).map_err(|err| format!("替换飞行旗配置失败：{err}"))?;
        }
        std::fs::rename(&temporary, path).map_err(|err| {
            let _ = std::fs::remove_file(&temporary);
            format!("保存飞行旗配置失败：{err}")
        })
    }

    pub fn flags_for(&self, map: &str) -> &[CompositeFlag] {
        self.maps
            .get(normalize_map_name(map))
            .map_or(&[], Vec::as_slice)
    }

    pub fn add_flag(&mut self, map: &str, name: &str) -> Result<usize, String> {
        let map = checked_map(map)?;
        let flags = self.maps.entry(map.to_string()).or_default();
        let name = name.trim();
        let name = if name.is_empty() {
            format!("合成旗 {}", flags.len() + 1)
        } else {
            name.to_string()
        };
        if flags.iter().any(|flag| flag.name == name) {
            return Err(format!("{map} 已有名为“{name}”的合成旗"));
        }
        flags.push(CompositeFlag {
            name,
            points: Vec::new(),
        });
        Ok(flags.len() - 1)
    }

    pub fn remove_flag(&mut self, map: &str, flag_index: usize) -> bool {
        let map = normalize_map_name(map);
        let Some(flags) = self.maps.get_mut(map) else {
            return false;
        };
        if flag_index >= flags.len() {
            return false;
        }
        flags.remove(flag_index);
        if flags.is_empty() {
            self.maps.remove(map);
        }
        true
    }

    pub fn add_point(
        &mut self,
        map: &str,
        flag_index: usize,
        point: FlightPoint,
    ) -> Result<(), String> {
        let map = checked_map(map)?;
        if point.x < 0 || point.y < 0 {
            return Err("飞行旗坐标不能小于 0".to_string());
        }
        let flag = self
            .maps
            .get_mut(map)
            .and_then(|flags| flags.get_mut(flag_index))
            .ok_or_else(|| "请先新建或选择一个合成旗".to_string())?;
        if flag.points.contains(&point) {
            return Err(format!("点位 ({}, {}) 已存在", point.x, point.y));
        }
        flag.points.push(point);
        Ok(())
    }

    pub fn remove_point(&mut self, map: &str, flag_index: usize, point_index: usize) -> bool {
        let Some(flag) = self
            .maps
            .get_mut(normalize_map_name(map))
            .and_then(|flags| flags.get_mut(flag_index))
        else {
            return false;
        };
        if point_index >= flag.points.len() {
            return false;
        }
        flag.points.remove(point_index);
        true
    }

    fn normalize(&mut self) {
        self.style.migrate_legacy_rgba();
        if let Some(flags) = self.maps.remove("朱子国") {
            self.maps
                .entry("朱紫国".to_string())
                .or_default()
                .extend(flags);
        }
        self.maps.retain(|map, flags| {
            if !SUPPORTED_MAPS.contains(&map.as_str()) {
                return false;
            }
            flags.retain_mut(|flag| {
                flag.name = flag.name.trim().to_string();
                flag.points.retain(|point| point.x >= 0 && point.y >= 0);
                let mut unique = Vec::with_capacity(flag.points.len());
                flag.points.retain(|point| {
                    if unique.contains(point) {
                        false
                    } else {
                        unique.push(*point);
                        true
                    }
                });
                !flag.name.is_empty()
            });
            !flags.is_empty()
        });
    }
}

fn checked_map(map: &str) -> Result<&str, String> {
    let map = normalize_map_name(map);
    if SUPPORTED_MAPS.contains(&map) {
        Ok(map)
    } else {
        Err(format!("{map} 不支持飞行旗标注"))
    }
}

pub fn normalize_map_name(map: &str) -> &str {
    if map == "朱子国" {
        "朱紫国"
    } else {
        map
    }
}

pub fn default_config_path() -> PathBuf {
    if let Some(path) = std::env::var_os(CONFIG_PATH_ENV).filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    #[cfg(windows)]
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(dir).join("rust-mhxy").join(CONFIG_FILE_NAME);
    }
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("rust-mhxy").join(CONFIG_FILE_NAME);
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home)
            .join(".config")
            .join("rust-mhxy")
            .join(CONFIG_FILE_NAME);
    }
    PathBuf::from(CONFIG_FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_composite_flag_groups_multiple_points_under_one_number() {
        let mut flags = FlightFlags::default();
        let first = flags.add_flag("朱子国", "酒店旗").unwrap();
        flags
            .add_point("朱紫国", first, FlightPoint { x: 10, y: 20 })
            .unwrap();
        flags
            .add_point("朱紫国", first, FlightPoint { x: 30, y: 40 })
            .unwrap();
        assert_eq!(flags.flags_for("朱紫国")[0].points.len(), 2);
        assert_eq!(flags.flags_for("朱紫国")[0].name, "酒店旗");
    }

    #[test]
    fn different_composite_flags_keep_separate_numbers() {
        let mut flags = FlightFlags::default();
        assert_eq!(flags.add_flag("傲来国", "一号旗").unwrap(), 0);
        assert_eq!(flags.add_flag("傲来国", "二号旗").unwrap(), 1);
        flags
            .add_point("傲来国", 1, FlightPoint { x: 12, y: 34 })
            .unwrap();
        assert_eq!(flags.flags_for("傲来国").len(), 2);
        assert_eq!(flags.flags_for("傲来国")[1].points.len(), 1);
    }

    #[test]
    fn migrates_legacy_point_lists_into_one_composite_flag() {
        let path = std::env::temp_dir().join(format!(
            "rust-mhxy-flight-flags-legacy-{}.json",
            std::process::id()
        ));
        std::fs::write(
            &path,
            r#"{"maps":{"长寿村":[{"x":10,"y":20},{"x":30,"y":40}]}}"#,
        )
        .unwrap();
        let flags = FlightFlags::load(&path).unwrap();
        assert_eq!(flags.flags_for("长寿村").len(), 1);
        assert_eq!(flags.flags_for("长寿村")[0].name, "合成旗 1");
        assert_eq!(flags.flags_for("长寿村")[0].points.len(), 2);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn style_settings_round_trip_and_legacy_files_use_defaults() {
        let path = std::env::temp_dir().join(format!(
            "rust-mhxy-flight-flags-style-{}.json",
            std::process::id()
        ));
        let flags = FlightFlags {
            style: FlightFlagStyle {
                background: [0.8, 0.7, 0.6, -1.0],
                background_normal_alpha: 0.35,
                text: [0.2, 0.3, 0.4, 0.65],
                text_normal_alpha: 0.65,
                size: 32,
            },
            ..FlightFlags::default()
        };
        flags.save(&path).unwrap();
        assert_eq!(FlightFlags::load(&path).unwrap().style, flags.style);
        let _ = std::fs::remove_file(path);

        let legacy: FlightFlags = serde_json::from_str(
            r#"{"maps":{},"style":{"background":[13,148,160,255],"text":[255,255,255,255],"size":20}}"#,
        )
        .unwrap();
        let mut legacy_style = legacy.style;
        legacy_style.migrate_legacy_rgba();
        assert_eq!(
            legacy_style.background,
            [13.0 / 255.0, 148.0 / 255.0, 160.0 / 255.0, 1.0]
        );
        assert_eq!(legacy_style.text, [1.0; 4]);
        assert_eq!(legacy_style.size, FlightFlagStyle::default().size);
    }

    #[test]
    fn round_trips_local_file() {
        let path = std::env::temp_dir().join(format!(
            "rust-mhxy-flight-flags-{}-{}.json",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let mut flags = FlightFlags::default();
        let index = flags.add_flag("傲来国", "测试旗").unwrap();
        flags
            .add_point("傲来国", index, FlightPoint { x: 12, y: 34 })
            .unwrap();
        flags.save(&path).unwrap();
        assert_eq!(FlightFlags::load(&path).unwrap(), flags);
        let _ = std::fs::remove_file(path);
    }
}
