//! 自动更新：从自建站点读取版本清单，下载并校验安装包。
//!
//! 清单示例（`latest.json`）：
//! ```json
//! {
//!   "version": "v0.1.7",
//!   "notes": "修复若干问题",
//!   "platforms": {
//!     "windows-x86_64": {
//!       "url": "https://xyq.mhwy.work/downloads/rust-mhxy-v0.1.7-windows-x86_64.zip",
//!       "sha256": "…",
//!       "size": 44369540
//!     }
//!   }
//! }
//! ```
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

/// 默认清单地址（国内访问比 GitHub 快）。
pub const DEFAULT_MANIFEST_URL: &str = "https://xyq.mhwy.work/latest.json";
/// 下载站首页，用于“打开发布页”。
pub const HOME_URL: &str = "https://xyq.mhwy.work";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: String,
    #[serde(default)]
    platforms: HashMap<String, PlatformEntry>,
    // 兼容没有 platforms 的扁平清单。
    #[serde(default)]
    url: String,
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    size: u64,
}

#[derive(Clone, Deserialize)]
struct PlatformEntry {
    url: String,
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    size: u64,
}

fn agent(timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build();
    ureq::Agent::new_with_config(config)
}

/// 拉取清单；有新版本时返回更新信息。
pub fn check(manifest_url: &str, current_version: &str) -> Result<Option<UpdateInfo>, String> {
    if manifest_url.trim().is_empty() {
        return Err("未配置更新地址".to_string());
    }
    let mut response = agent(Duration::from_secs(12))
        .get(&cache_busted(manifest_url.trim()))
        .header("User-Agent", user_agent())
        .call()
        .map_err(|err| format!("读取版本清单失败：{err}"))?;
    let body = response
        .body_mut()
        .read_to_string()
        .map_err(|err| format!("读取版本清单失败：{err}"))?;
    // 记事本等编辑器保存的 JSON 可能带 UTF-8 BOM，serde_json 会直接报错。
    let body = body.trim_start_matches('\u{feff}').trim();
    if body.is_empty() {
        return Err("版本清单是空内容（检查更新地址是否返回了 latest.json）".to_string());
    }
    let manifest: Manifest =
        serde_json::from_str(body).map_err(|err| format!("版本清单格式错误：{err}"))?;
    if !is_newer(&manifest.version, current_version) {
        return Ok(None);
    }
    let platform = resolve_platform(&manifest);
    if platform.url.trim().is_empty() {
        return Err(if manifest.platforms.is_empty() {
            "清单里没有下载地址（需要 platforms 或顶层 url 字段）".to_string()
        } else {
            format!(
                "清单里没有 {} 的下载地址（现有：{}）",
                platform_key(),
                manifest
                    .platforms
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        });
    }
    validate_version(&manifest.version)?;
    validate_sha256(&platform.sha256)?;
    Ok(Some(UpdateInfo {
        version: manifest.version,
        notes: manifest.notes,
        url: resolve_download_url(manifest_url, &platform.url),
        sha256: platform.sha256,
        size: platform.size,
    }))
}

/// 清单里写相对地址（如 `downloads/a.zip`）时，按清单地址补全。
fn resolve_download_url(manifest_url: &str, target: &str) -> String {
    let target = target.trim();
    if target.starts_with("http://") || target.starts_with("https://") {
        return target.to_string();
    }
    if let Some((prefix, _)) = manifest_url.rsplit_once('/') {
        return format!("{prefix}/{}", target.trim_start_matches('/'));
    }
    target.to_string()
}

/// 找到当前平台对应的下载信息：优先精确匹配，其次常见别名，
/// 最后如果清单里只有一个平台就直接用它（Windows-only 项目很常见）。
fn resolve_platform(manifest: &Manifest) -> PlatformEntry {
    for key in platform_aliases() {
        if let Some(entry) = manifest.platforms.get(*key) {
            return entry.clone();
        }
    }
    if manifest.platforms.len() == 1 {
        if let Some(entry) = manifest.platforms.values().next() {
            return entry.clone();
        }
    }
    PlatformEntry {
        url: manifest.url.clone(),
        sha256: manifest.sha256.clone(),
        size: manifest.size,
    }
}

fn platform_aliases() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        &[
            "windows-x86_64",
            "windows-x64",
            "windows-amd64",
            "win-x64",
            "windows",
            "win64",
            "win",
        ]
    } else if cfg!(target_os = "macos") {
        &["macos-x86_64", "macos", "darwin", "osx"]
    } else {
        &["linux-x86_64", "linux-x64", "linux"]
    }
}

pub fn platform_key() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows-x86_64"
    } else if cfg!(target_os = "macos") {
        "macos-x86_64"
    } else {
        "linux-x86_64"
    }
}

/// 给清单地址加时间戳，避免静态站点/CDN 返回缓存里的旧清单。
fn cache_busted(url: &str) -> String {
    let separator = if url.contains('?') { '&' } else { '?' };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("{url}{separator}t={stamp}")
}

fn user_agent() -> String {
    format!("rust-mhxy/{}", env!("CARGO_PKG_VERSION"))
}

/// 下载安装包并校验 SHA256。`progress` 记录已下载字节数，`cancel` 可中断。
pub fn download_to(
    url: &str,
    destination: &Path,
    expected_sha256: &str,
    progress: &AtomicU64,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if url.trim().is_empty() {
        return Err("下载地址为空".to_string());
    }
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|err| format!("创建临时目录失败：{err}"))?;
    }
    let response = agent(Duration::from_secs(600))
        .get(url.trim())
        .header("User-Agent", user_agent())
        .call()
        .map_err(|err| format!("下载失败：{err}"))?;
    let mut reader = response.into_body().into_reader();
    let mut file =
        BufWriter::new(File::create(destination).map_err(|err| format!("创建安装包失败：{err}"))?);
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = std::fs::remove_file(destination);
            return Err("已取消下载".to_string());
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|err| format!("下载中断：{err}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|err| format!("写入安装包失败：{err}"))?;
        progress.fetch_add(read as u64, Ordering::Relaxed);
    }
    file.flush()
        .map_err(|err| format!("写入安装包失败：{err}"))?;
    verify_sha256(destination, expected_sha256)
}

/// 计算文件的 SHA256（小写十六进制）。
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|err| format!("读取安装包失败：{err}"))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|err| format!("读取安装包失败：{err}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn validate_sha256(value: &str) -> Result<&str, String> {
    let value = value.trim();
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("版本清单缺少有效的 SHA256（必须是 64 位十六进制）".to_string());
    }
    Ok(value)
}

fn validate_version(value: &str) -> Result<(), String> {
    let trimmed = value.trim();
    if parse_version(trimmed).is_none()
        || !trimmed
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
    {
        return Err(format!("版本清单里的版本号无效：{value}"));
    }
    Ok(())
}

fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let expected = validate_sha256(expected)?;
    let actual = sha256_file(path)?;
    if !actual.eq_ignore_ascii_case(expected) {
        let _ = std::fs::remove_file(path);
        return Err(format!(
            "安装包校验失败（期望 {expected}，实际 {actual}），已删除"
        ));
    }
    Ok(())
}

fn parse_version(value: &str) -> Option<(u64, u64, u64)> {
    let value = value.trim().trim_start_matches(['v', 'V']);
    let mut parts = value.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().unwrap_or("0").parse().ok()?;
    let patch = parts
        .next()
        .unwrap_or("0")
        .split(['-', '+'])
        .next()?
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

/// `candidate` 是否比 `current` 新（忽略 `v` 前缀与预发布后缀）。
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

/// 更新脚本：等待本进程退出后解压覆盖，确认新版本真的跑起来再清理安装包。
/// 单独抽出来是为了在非 Windows 上也能断言脚本里的步骤顺序。
#[cfg(any(windows, test))]
const UPDATE_SCRIPT: &str = r#"param([int]$TargetPid, [string]$Zip, [string]$InstallDir, [string]$Exe)
$ErrorActionPreference = 'Stop'
$base = if ($env:LOCALAPPDATA) {
    Join-Path $env:LOCALAPPDATA 'rust-mhxy\updates'
} else {
    Join-Path $env:TEMP 'rust-mhxy-updates'
}
New-Item -ItemType Directory -Path $base -Force | Out-Null
$log = Join-Path $base 'update.log'
function Write-Log([string]$Message) {
    try {
        $stamp = Get-Date -Format 'yyyy-MM-dd HH:mm:ss'
        Add-Content -LiteralPath $log -Value ("[$stamp] $Message") -Encoding utf8
    } catch {}
}
try {
    Write-Log "开始更新：zip=$Zip install=$InstallDir exe=$Exe pid=$TargetPid"
    try { Wait-Process -Id $TargetPid -Timeout 120 -ErrorAction SilentlyContinue } catch {}
    Start-Sleep -Milliseconds 800
    if (-not (Test-Path -LiteralPath $Zip)) { throw "安装包不存在：$Zip" }

    $temp = Join-Path $env:TEMP ('mhxy-update-' + [Guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $temp | Out-Null
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    [System.IO.Compression.ZipFile]::ExtractToDirectory($Zip, $temp)
    Write-Log "已解压到 $temp"

    $root = Get-ChildItem -LiteralPath $temp -Directory | Select-Object -First 1
    if (-not $root) { $root = Get-Item -LiteralPath $temp }

    foreach ($item in (Get-ChildItem -LiteralPath $root.FullName -Force)) {
        $target = Join-Path $InstallDir $item.Name
        $copied = $false
        for ($attempt = 1; $attempt -le 20; $attempt++) {
            try {
                Copy-Item -LiteralPath $item.FullName -Destination $target -Recurse -Force -ErrorAction Stop
                $copied = $true
                break
            } catch {
                if ($attempt -eq 20) { throw "覆盖失败：$($item.Name)：$($_.Exception.Message)" }
                Start-Sleep -Milliseconds 500
            }
        }
        if (-not $copied) { throw "覆盖失败：$($item.Name)" }
    }
    Write-Log "已覆盖安装目录：$InstallDir"
    Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue

    if (-not (Test-Path -LiteralPath $Exe)) { throw "找不到可执行文件：$Exe" }

    # 先启动新版本并确认它没有立刻退出，再删除安装包：
    # 万一新版本启动失败，安装包还在，可以直接重试安装。
    $proc = Start-Process -FilePath $Exe -WorkingDirectory $InstallDir -PassThru
    Start-Sleep -Seconds 3
    if (-not (Get-Process -Id $proc.Id -ErrorAction SilentlyContinue)) {
        throw "新版本启动后立即退出（pid=$($proc.Id)），安装包已保留：$Zip"
    }
    Write-Log "已启动新版本并确认运行：$Exe (pid=$($proc.Id))"

    Remove-Item -LiteralPath $Zip -Force -ErrorAction SilentlyContinue
    # 清理以前残留的安装包，只保留当前这一个之外的文件。
    Get-ChildItem -LiteralPath $base -Filter 'rust-mhxy-*-update.zip' -File -ErrorAction SilentlyContinue |
        Where-Object { $_.FullName -ne $Zip } |
        Remove-Item -Force -ErrorAction SilentlyContinue
    Write-Log "已清理旧安装包"
} catch {
    Write-Log ("更新失败：" + $_.Exception.Message)
    try {
        Add-Type -AssemblyName PresentationFramework
        $hint = "自动更新失败：" + $_.Exception.Message + "`n`n日志：" + $log + "`n安装包：" + $Zip + "`n`n安装包和旧版本都还在，可以重试安装或手动解压覆盖。"
        if ($InstallDir -like '*Program Files*') {
            $hint = $hint + "`n程序装在 Program Files，可能需要用管理员权限运行一次。"
        }
        [System.Windows.MessageBox]::Show($hint, 'rust-mhxy 更新失败') | Out-Null
    } catch {}
}
"#;

/// 生成更新脚本，等待本进程退出后解压覆盖并重启。仅 Windows 可用。
#[cfg(windows)]
pub fn apply_and_restart(zip: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let exe = std::env::current_exe().map_err(|err| format!("无法定位程序：{err}"))?;
    let install_dir = exe.parent().ok_or_else(|| "无法定位程序目录".to_string())?;
    let script = update_dir().join("update.ps1");
    // Windows PowerShell 5.1 需要 BOM 才能正确识别 UTF-8 路径。
    std::fs::write(&script, format!("\u{feff}{UPDATE_SCRIPT}"))
        .map_err(|err| format!("写入更新脚本失败：{err}"))?;
    std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-WindowStyle",
            "Hidden",
            "-File",
        ])
        .arg(&script)
        .arg("-TargetPid")
        .arg(std::process::id().to_string())
        .arg("-Zip")
        .arg(zip)
        .arg("-InstallDir")
        .arg(install_dir)
        .arg("-Exe")
        .arg(&exe)
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|err| format!("启动更新程序失败：{err}"))?;
    Ok(())
}

/// 非 Windows 平台不做自动替换，提示用户手动下载。
#[cfg(not(windows))]
pub fn apply_and_restart(_zip: &Path) -> Result<(), String> {
    Err("自动更新仅支持 Windows，请打开发布页手动下载".to_string())
}

/// 安装包与更新日志的存放目录。
///
/// Windows 放在 `%LOCALAPPDATA%\rust-mhxy\updates`，不会被系统清理临时文件时误删；
/// 其他平台放在临时目录下的 `rust-mhxy-updates`。
pub fn update_dir() -> PathBuf {
    #[cfg(windows)]
    if let Some(dir) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(dir).join("rust-mhxy").join("updates");
    }
    std::env::temp_dir().join("rust-mhxy-updates")
}

/// 更新脚本写的日志文件。
pub fn update_log_path() -> PathBuf {
    update_dir().join("update.log")
}

/// 下载后的安装包路径。
pub fn temp_package(version: &str) -> PathBuf {
    update_dir().join(format!(
        "rust-mhxy-{}-update.zip",
        version.trim_start_matches(['v', 'V'])
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_script_verifies_new_process_before_deleting_package() {
        let start = UPDATE_SCRIPT.find("Start-Process").expect("应启动新版本");
        let verify = UPDATE_SCRIPT
            .find("Get-Process -Id $proc.Id")
            .expect("应确认新进程仍在运行");
        let delete = UPDATE_SCRIPT
            .find("Remove-Item -LiteralPath $Zip -Force")
            .expect("应清理安装包");
        assert!(start < verify, "必须先启动再检查进程");
        assert!(verify < delete, "确认新版本运行后才能删除安装包");
        assert!(
            UPDATE_SCRIPT.contains("-PassThru"),
            "启动时要拿到进程对象才能确认存活"
        );
        assert!(
            UPDATE_SCRIPT.contains("安装包已保留"),
            "启动失败时要提示安装包仍然保留"
        );
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.1.7", "v0.1.6"));
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("v1.0", "v0.9.9"));
        assert!(is_newer("v0.1.6", "v0.1.5"));
        assert!(!is_newer("v0.1.6", "v0.1.6"));
        assert!(!is_newer("v0.1.5", "v0.1.6"));
        assert!(!is_newer("garbage", "v0.1.6"));
        assert!(is_newer("v0.1.7-beta", "v0.1.6"));
    }

    #[test]
    fn hashes_known_content() {
        let dir = std::env::temp_dir().join("rust-mhxy-update-hash-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("payload.bin");
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(
            sha256_file(&path).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(verify_sha256(
            &path,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        )
        .is_ok());
        assert!(verify_sha256(
            &path,
            "0000000000000000000000000000000000000000000000000000000000000000"
        )
        .is_err());
        assert!(!path.exists(), "校验失败应删除文件");
    }

    #[test]
    fn checks_manifest_over_http() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        // 只声明 Windows 平台的清单：单平台时其它系统也应直接使用。
        let body = r#"{"version":"v9.9.9","notes":"test","platforms":{"windows-x86_64":{"url":"http://example.com/a.zip","sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad","size":1}}}"#.to_string();
        let server = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buffer = [0u8; 2048];
                let _ = stream.read(&mut buffer);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        let info = check(&format!("http://{address}/latest.json"), "v0.1.6")
            .expect("读取清单")
            .expect("应有新版本");
        assert_eq!(info.version, "v9.9.9");
        assert_eq!(info.url, "http://example.com/a.zip");
        assert_eq!(
            info.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        server.join().unwrap();
    }

    #[test]
    fn rejects_unsafe_update_metadata() {
        assert!(validate_sha256("").is_err());
        assert!(validate_sha256("abc").is_err());
        assert!(validate_sha256(
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        )
        .is_ok());

        assert!(validate_version("v0.2.3").is_ok());
        assert!(validate_version("v0.2.3-beta+1").is_ok());
        assert!(validate_version("../../payload").is_err());
        assert!(validate_version("v0.2.3/update").is_err());
    }

    #[test]
    fn resolves_relative_download_urls() {
        assert_eq!(
            resolve_download_url("https://xyq.mhwy.work/latest.json", "downloads/a.zip"),
            "https://xyq.mhwy.work/downloads/a.zip"
        );
        assert_eq!(
            resolve_download_url("https://xyq.mhwy.work/latest.json", "/downloads/a.zip"),
            "https://xyq.mhwy.work/downloads/a.zip"
        );
        assert_eq!(
            resolve_download_url("https://cdn.example.com/x/", "a.zip"),
            "https://cdn.example.com/x/a.zip"
        );
        assert_eq!(
            resolve_download_url("https://cdn.example.com/latest.json", "https://a/b.zip"),
            "https://a/b.zip"
        );
    }

    #[test]
    fn resolves_single_platform_and_alias() {
        let single: Manifest = serde_json::from_str(
            r#"{"version":"v0.2.7","platforms":{"windows-x86_64":{"url":"https://example.com/a.zip","sha256":"abc","size":1}}}"#,
        )
        .unwrap();
        // 清单里只有一个平台时，任何系统都直接使用它。
        assert_eq!(resolve_platform(&single).url, "https://example.com/a.zip");

        let alias: Manifest = serde_json::from_str(
            r#"{"version":"v0.2.7","platforms":{"windows":{"url":"https://example.com/b.zip"},"linux":{"url":"https://example.com/l.zip"}}}"#,
        )
        .unwrap();
        let resolved = resolve_platform(&alias);
        assert!(resolved.url.ends_with(".zip"));
    }

    #[test]
    fn parses_platform_manifest() {
        let manifest: Manifest = serde_json::from_str(
            r#"{
                "version": "v0.1.7",
                "notes": "修复抖动",
                "platforms": {
                    "windows-x86_64": {
                        "url": "https://example.com/a.zip",
                        "sha256": "abc",
                        "size": 123
                    }
                }
            }"#,
        )
        .unwrap();
        assert_eq!(manifest.version, "v0.1.7");
        let entry = manifest.platforms.get("windows-x86_64").unwrap();
        assert_eq!(entry.url, "https://example.com/a.zip");
        assert_eq!(entry.size, 123);
    }
}
