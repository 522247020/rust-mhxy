//! 本地合成旗点位配置。该文件放在用户数据目录，不随安装包分发或更新。
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

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct FlightFlags {
    #[serde(default)]
    pub maps: BTreeMap<String, Vec<FlightPoint>>,
}

impl FlightFlags {
    pub fn load(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text =
            std::fs::read_to_string(path).map_err(|err| format!("读取飞行旗配置失败：{err}"))?;
        let mut flags: Self =
            serde_json::from_str(&text).map_err(|err| format!("飞行旗配置格式错误：{err}"))?;
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

    pub fn points_for(&self, map: &str) -> &[FlightPoint] {
        self.maps
            .get(normalize_map_name(map))
            .map_or(&[], Vec::as_slice)
    }

    pub fn add(&mut self, map: &str, point: FlightPoint) -> Result<(), String> {
        let map = normalize_map_name(map);
        if !SUPPORTED_MAPS.contains(&map) {
            return Err(format!("{map} 不支持飞行旗标注"));
        }
        if point.x < 0 || point.y < 0 {
            return Err("飞行旗坐标不能小于 0".to_string());
        }
        let points = self.maps.entry(map.to_string()).or_default();
        if points.contains(&point) {
            return Err(format!("点位 ({}, {}) 已存在", point.x, point.y));
        }
        points.push(point);
        Ok(())
    }

    pub fn remove(&mut self, map: &str, index: usize) -> bool {
        let map = normalize_map_name(map);
        let Some(points) = self.maps.get_mut(map) else {
            return false;
        };
        if index >= points.len() {
            return false;
        }
        points.remove(index);
        if points.is_empty() {
            self.maps.remove(map);
        }
        true
    }

    fn normalize(&mut self) {
        if let Some(points) = self.maps.remove("朱子国") {
            self.maps
                .entry("朱紫国".to_string())
                .or_default()
                .extend(points);
        }
        self.maps.retain(|map, points| {
            SUPPORTED_MAPS.contains(&map.as_str()) && {
                points.retain(|point| point.x >= 0 && point.y >= 0);
                let mut unique = Vec::with_capacity(points.len());
                points.retain(|point| {
                    if unique.contains(point) {
                        false
                    } else {
                        unique.push(*point);
                        true
                    }
                });
                !points.is_empty()
            }
        });
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
    fn adds_numbers_and_normalizes_supported_maps() {
        let mut flags = FlightFlags::default();
        flags.add("朱子国", FlightPoint { x: 10, y: 20 }).unwrap();
        flags.add("朱紫国", FlightPoint { x: 30, y: 40 }).unwrap();
        assert_eq!(
            flags.points_for("朱紫国"),
            [FlightPoint { x: 10, y: 20 }, FlightPoint { x: 30, y: 40 }]
        );
        assert!(flags.add("建邺城", FlightPoint { x: 1, y: 2 }).is_err());
        assert!(flags.add("朱紫国", FlightPoint { x: 10, y: 20 }).is_err());
    }

    #[test]
    fn round_trips_local_file() {
        let path = std::env::temp_dir().join(format!(
            "rust-mhxy-flight-flags-{}-{}.json",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let mut flags = FlightFlags::default();
        flags.add("傲来国", FlightPoint { x: 12, y: 34 }).unwrap();
        flags.save(&path).unwrap();
        assert_eq!(FlightFlags::load(&path).unwrap(), flags);
        let _ = std::fs::remove_file(path);
    }
}
